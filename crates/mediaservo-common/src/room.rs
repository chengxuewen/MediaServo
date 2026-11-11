//! Room kind model (P2, PLAN §4c; docs/modules/26-room-model.md).
//!
//! `kind` is a **data field**, not a name prefix and not a new `RoomType`
//! variant (C29 preserved: the core five-variant enum stays). This module is
//! the pure resolution authority: given a room id (and, later, managed-room
//! metadata), it yields the policy stance every gate must read —
//! `room_kind()` call sites replace the `is_audio_room()` prefix checks.
//!
//! Kind vocabulary is strictly 4 characters (RM8 adjudication): `talk`,
//! `meet`, `desk`, `view`, `ctrl` — plus the deprecated legacy `stream`.
//! Room ids are opaque (`fleet1`, `carA/view`); the legacy forms
//! (`audio-<id>`, `<device>_<stream>`) still resolve, via the fallback chain
//! below, and every fallback is a deprecation surface (warn-once, RM6 fix:
//! `audio-` maps to `meet(video=false)`, never `talk(open)`).

use std::collections::HashSet;
use std::sync::Mutex;

/// The six kind values (five live + deprecated `stream`). Wire form is the
/// lowercase 4-char string itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoomKind {
    /// Floor-enforced conference (exclusive/hybrid only; `talk(open)` banned, RM6).
    Talk,
    /// Full-duplex conference; `video` flag distinguishes voice chat (false).
    Meet,
    /// Remote desktop: screen + explicit control ACL (RM4).
    Desk,
    /// Device-push monitoring; N producers per room, selective subscribe (RM9).
    View,
    /// Data/command only, media forbidden (RM5 — closes the device produce hole).
    Ctrl,
    /// Legacy `<device>_<stream>` room — deprecated, replaced by `view`.
    Stream,
}

impl RoomKind {
    /// Wire string (always 4 chars; the invariant RM8 guards).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Talk => "talk",
            Self::Meet => "meet",
            Self::Desk => "desk",
            Self::View => "view",
            Self::Ctrl => "ctrl",
            Self::Stream => "stream",
        }
    }

    /// Parse an explicit kind label (managed-room config, P4 rooms.yaml).
    /// Unknown labels are an error — no silent fallback (R14 tolerance is a
    /// wire-variant concern, not a config one).
    pub fn parse(label: &str) -> Result<Self, String> {
        match label {
            "talk" => Ok(Self::Talk),
            "meet" => Ok(Self::Meet),
            "desk" => Ok(Self::Desk),
            "view" => Ok(Self::View),
            "ctrl" => Ok(Self::Ctrl),
            "stream" => Ok(Self::Stream),
            other => Err(format!("unknown room kind '{other}'")),
        }
    }

    /// Floor commands are accepted only in `talk` rooms (§4c; O22 negative:
    /// non-talk rooms must never see a floor_*).
    pub fn floor_allowed(self) -> bool {
        matches!(self, Self::Talk)
    }
}

/// What a room id resolves to, including the deprecation notice when the
/// resolution went through a legacy fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomStance {
    pub kind: RoomKind,
    /// `Some(false)` = voice-only meeting (`audio-` legacy mapping, RM6).
    /// `None` = kind default (meet/video rooms carry the flag explicitly via
    /// managed metadata in P4).
    pub video: Option<bool>,
    /// Set when the id itself still carries a legacy form (drives warn-once
    /// logging and the `RoomEntry.deprecated` discovery badge).
    pub deprecated: Option<Deprecation>,
}

impl RoomStance {
    /// Voice-domain room: the policy question the old `is_audio_room()` prefix
    /// check used to answer (C29 amendment -- now a kind read). True for
    /// `talk` rooms (RM6: the floor-controlled speaking room -- PTT without
    /// produce would be meaningless; surfaced by the O26 matrix) and explicit
    /// voice meets, plus the `audio-` legacy form they replace.
    pub fn is_voice_conference(&self) -> bool {
        self.kind == RoomKind::Meet && self.video == Some(false)
            || self.kind == RoomKind::Talk
    }
}

/// Machine-readable deprecation notice for one legacy room form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deprecation {
    /// The legacy form matched: "audio-prefix" or "stream-room".
    pub legacy: &'static str,
    /// Announcement version (PLAN §4c directive round).
    pub since: &'static str,
    /// Removal gate — P6 aftermath, separate round.
    pub remove_after: &'static str,
    /// What to use instead (shown to pilots/admins).
    pub replaced_by: &'static str,
}

/// Prefix convention being retired (C29 amendment; RM6 mapping).
pub const AUDIO_PREFIX: &str = "audio-";

/// O19 producer-label convention (§4c canonical set): camera views inside one
/// `view` room are named from this table so consumers can subscribe selectively
/// (RM9/O21 label filter). The server does not enforce the set (RM10: labels are
/// advisory metadata — devices keep their legacy short forms like "fl"); this is
/// the consumer-side filter vocabulary.
pub const PRODUCER_LABELS: [&str; 5] = ["front", "left", "right", "rear", "ambient"];

pub fn is_canonical_label(label: &str) -> bool {
    PRODUCER_LABELS.contains(&label)
}

/// The deprecation notice for derived stream rooms. The server supplies this
/// when derivation evidence is known (base is a registered device room and
/// the suffix is an announced stream) — an underscore inside an id is NOT
/// sufficient evidence on its own (`vehicle_gen1`, `room_2` are legitimate
/// opaque ids; a raw name-pattern fallback would false-positive on them).
pub fn stream_room_notice() -> Deprecation {
    Deprecation {
        legacy: "stream-room",
        since: "2026-11",
        remove_after: "P6",
        replaced_by: "view room (one device one room, N producers)",
    }
}

/// Resolve a room id through the fallback chain:
/// `audio-` legacy form → `meet(video=false)` (RM6 correction: never `talk(open)`);
/// anything else → `meet` (the neutral default for unmanaged rooms; P4
/// rooms.yaml overrides via `RoomKind::parse`). Stream-room deprecation is
/// NOT inferred here — it needs derivation evidence (see `stream_room_notice`).
pub fn resolve(room_id: &str) -> RoomStance {
    if let Some(rest) = room_id.strip_prefix(AUDIO_PREFIX)
        && !rest.is_empty()
    {
        return RoomStance {
            kind: RoomKind::Meet,
            video: Some(false),
            deprecated: Some(Deprecation {
                legacy: "audio-prefix",
                since: "2026-11",
                remove_after: "P6",
                replaced_by: "kind=meet room (video=false)",
            }),
        };
    }
    // RM6 id convention (O26-surfaced): `talk-` rooms are the floor-controlled
    // speaking rooms. Without this inference the produce gate (roles.rs reads
    // the voice domain) and the floor designation env seeded different truths
    // for the same room -- PTT was impossible. P4 rooms.yaml replaces the
    // inference with an explicit kind field.
    if room_id.starts_with("talk-") {
        return RoomStance { kind: RoomKind::Talk, video: Some(false), deprecated: None };
    }
    RoomStance { kind: RoomKind::Meet, video: None, deprecated: None }
}

/// Resolve with explicit managed metadata (P4 path): the config kind wins;
/// the legacy id forms only contribute the deprecation notice.
pub fn resolve_explicit(room_id: &str, kind: RoomKind) -> RoomStance {
    let mut stance = resolve(room_id);
    stance.kind = kind;
    stance
}

/// Warn-once dedupe set (per room id, per process) — the log surface of the
/// five-surface deprecation plan (PLAN §4c). Not an audit event: deprecation
/// is hygiene, audit is security (M5 ruling).
#[derive(Default)]
pub struct DeprecationLogger {
    seen: Mutex<HashSet<String>>,
}

impl DeprecationLogger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Emits `WARN room_id=... legacy=... replacement=...` on first sighting
    /// only. Returns true when the line was logged.
    pub fn note(&self, room_id: &str, deprecated: &Deprecation) -> bool {
        let first = {
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            seen.insert(room_id.to_string())
        };
        if first {
            tracing::warn!(
                room_id,
                legacy = deprecated.legacy,
                replacement = deprecated.replaced_by,
                "legacy room form deprecated (removal after P6)"
            );
        }
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_wire_strings_are_four_chars() {
        for k in [RoomKind::Talk, RoomKind::Meet, RoomKind::Desk, RoomKind::View, RoomKind::Ctrl] {
            assert_eq!(k.as_str().len(), 4, "{k:?}");
        }
    }

    #[test]
    fn audio_prefix_resolves_to_voice_meet_not_talk() {
        // RM6 correction lock: the old mapping talk(floor=open) is banned.
        let s = resolve("audio-ms-car1");
        assert_eq!(s.kind, RoomKind::Meet);
        assert_eq!(s.video, Some(false));
        assert_eq!(s.deprecated.as_ref().unwrap().legacy, "audio-prefix");
        assert!(!s.kind.floor_allowed(), "meet rooms never take floor");
    }

    #[test]
    fn derived_stream_notice_carries_replacement_line() {
        // The server attaches this where derivation evidence exists (rooms.rs
        // derived set); resolve() alone never guesses from an underscore.
        let d = stream_room_notice();
        assert_eq!(d.legacy, "stream-room");
        assert!(d.replaced_by.contains("view"));
    }

    #[test]
    fn opaque_ids_resolve_to_neutral_meet() {
        let s = resolve("fleet1");
        assert_eq!(s.kind, RoomKind::Meet);
        assert_eq!(s.deprecated, None);
    }

    #[test]
    fn explicit_metadata_wins_over_id_fallback() {
        let s = resolve_explicit("audio-legacy-id", RoomKind::Talk);
        assert_eq!(s.kind, RoomKind::Talk);
        // the notice still rides along so ops sees why the id looks odd
        assert!(s.deprecated.is_some());
    }

    #[test]
    fn parse_roundtrips_and_rejects_unknown() {
        for k in [RoomKind::Talk, RoomKind::Meet, RoomKind::Desk, RoomKind::View, RoomKind::Ctrl] {
            assert_eq!(RoomKind::parse(k.as_str()).unwrap(), k);
        }
        assert!(RoomKind::parse("watch").is_err());
    }

    #[test]
    fn producer_labels_are_the_canonical_five() {
        assert_eq!(PRODUCER_LABELS, ["front", "left", "right", "rear", "ambient"]);
        assert!(is_canonical_label("front"));
        assert!(
            !is_canonical_label("fl"),
            "legacy short forms are not canonical (accepted, not enforced)"
        );
    }

    #[test]
    fn warn_once_fires_per_room_per_process() {
        let logger = DeprecationLogger::new();
        let d = resolve("audio-x").deprecated.unwrap();
        assert!(logger.note("audio-x", &d));
        assert!(!logger.note("audio-x", &d));
        let d2 = resolve("audio-y").deprecated.unwrap();
        assert!(logger.note("audio-y", &d2), "different room still warns");
    }

    #[test]
    fn floor_gate_open_only_for_talk() {
        for k in [RoomKind::Meet, RoomKind::Desk, RoomKind::View, RoomKind::Ctrl, RoomKind::Stream]
        {
            assert!(!k.floor_allowed(), "{k:?} must never accept floor");
        }
        assert!(RoomKind::Talk.floor_allowed());
    }
}
