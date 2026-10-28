//! host.yaml → oxfile.toml translator (Task A2 + C1).
//!
//! Input: host.yaml text. Output: OxMgr oxfile.toml text (`version = 1` + `[defaults]` +
//! `[[apps]]`, fields aligned with the official [OXFILE.md](https://github.com/Vladimir-Urik/OxMgr).
//! apps cover the 7 host process kinds + one capturer instance per source + one streamer
//! instance per stream (parameterized command). Phase A emits placeholder process skeletons; from C1 the capturer instance appends

use std::path::{Path, PathBuf};

use mediaservo_common::protocol::SignalingMessage;
use serde::Deserialize;

/// host.yaml parse model (Phase A subset: only sources/streams needed for instantiation).
#[derive(Debug, Default, Deserialize)]
struct HostConfig {
    /// Video source list (replaces the legacy cameras key; old key name `cameras` still parsed).
    #[serde(default, alias = "cameras")]
    sources: Vec<Source>,
    #[serde(default)]
    streams: Vec<Stream>,
    #[serde(default)]
    record: Option<RecordSection>,
    #[serde(default)]
    signaling: Option<SignalingSection>,
    /// Shared defaults section (`defaults.streams` / `defaults.sources`) — merge chain
    /// **per-entry config > defaults.<subsection> > builtin default** (D282). Missing section = zero injection: changes no
    /// resolved value (the builtin defaults themselves were separately revised by D282).
    #[serde(default)]
    defaults: DefaultsCfg,
    /// Remote-control execution domain (V5): estop signature key file and other controller process config.
    #[serde(default)]
    control: Option<ControlSection>,
}

/// [control] section (V5/F11 deployment-surface closure).
#[derive(Debug, Default, Deserialize)]
struct ControlSection {
    /// Estop command HMAC signature key file (relative paths resolved against the instance root; rendered as
    /// controller `--hmac-key-file`; the 0600 permission gate is enforced by the loader).
    /// Default = dev migration form (controller passes through + WARN, S4 estop_verdict matrix).
    #[serde(default)]
    hmac_key_file: Option<String>,
}
/// Logical category of a video source (4 mode kinds; `backend`/`input` take effect per mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceMode {
    /// Real camera capture (backend v4l2/mipi; capture backend not implemented this phase).
    Camera,
    /// Test/demo color-bar generation (VideoFrameGenerator produces real frames).
    Generator,
    /// Desktop capture (not implemented this phase).
    Desktop,
    /// Subscribe to an external source (FrameBus/ROS topic; not implemented this phase).
    Subscriber,
}

impl SourceMode {
    /// Config literal (shown in capturer error/ready logs).
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceMode::Camera => "camera",
            SourceMode::Generator => "generator",
            SourceMode::Desktop => "desktop",
            SourceMode::Subscriber => "subscriber",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Source {
    id: String,
    /// Logical category (default generator — legacy configs without a mode field = original stub generation semantics).
    #[serde(default)]
    mode: Option<SourceMode>,
    /// Capture backend (effective only for mode=camera: v4l2 | mipi).
    #[serde(default)]
    #[allow(dead_code)]
    // C36 field in place (backend selection is consumed via the select_backend chain; unread on the struct side = on-record shape)
    backend: Option<String>,
    /// Unified source address field: v4l2=device path / subscriber=FrameBus/ROS topic / desktop=display id (optional).
    #[serde(default)]
    input: Option<String>,
    /// Frame width (default 1280; subscriber may omit — frames carry their own metadata).
    #[serde(default)]
    width: Option<u32>,
    /// Frame height (default 720; subscriber may omit).
    #[serde(default)]
    height: Option<u32>,
    /// Frame rate (default 30).
    #[serde(default)]
    fps: Option<u32>,
    /// Camera reconnect interval ms (reserved; not consumed this phase).
    #[serde(default)]
    reconnect_ms: Option<u64>,
    /// Legacy field (swallowed for compat with old `camera.source: "stub"`; non-"stub" rejected — migration hint).
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Stream {
    id: String,
    /// Referenced source id (default = the stream id itself, topic camera/<id> direct). Old key name `camera` compatible.
    #[serde(default, alias = "camera")]
    source: Option<String>,
    /// Codec (default h264 — decided at the host layer, D282; field PublishOptions own default is still vp8,
    /// only as a fallback for direct callers that bypass host merging).
    #[serde(default)]
    codec: Option<String>,
    /// Encoder backend (auto/software/hardware/nvenc/vaapi; default auto = runtime selection).
    #[serde(default)]
    encoder_backend: Option<String>,
    /// Encode bitrate kbps (default 2000 — field PushConfig default).
    #[serde(default)]
    bitrate_kbps: Option<u32>,
    /// Weak-network policy preset (smooth|balanced|quality; default balanced = current behavior, qos-framerate-priority).
    #[serde(default)]
    stream_mode: Option<String>,
    /// Bitrate floor kbps (explicit override slot; default follows the stream_mode bundle fill).
    #[serde(default)]
    min_bitrate_kbps: Option<u32>,
    /// Keyframe interval seconds (GOP; default 2 — field PushConfig default).
    #[serde(default)]
    keyframe_interval: Option<u32>,
}

/// `defaults:` section (host-stream-defaults D282) — shared defaults for the streams / sources subsections.
///
/// Merge chain **per-entry config > defaults.<subsection> > builtin default**, applied exactly once at each of the two
/// single parse points (`camera_configs()` / `stream_configs()`); downstream (the oxfile decision section, in-process
/// parsing in capturer/streamer) only ever sees fully merged resolved values.
///
/// Deny stance: both sub-structs carry `deny_unknown_fields` (new keys with no legacy baggage — typos error at deploy time,
/// the layer most easily mistyped is the strictest); the top-level [`HostConfig`] does **not** (existing yamls carry
/// deprecated keys like `host:` / `control:` sections and the old `source: "stub"` compat surface — deny would bite).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultsCfg {
    #[serde(default)]
    sources: SourceDefaults,
    #[serde(default)]
    streams: StreamDefaults,
}

/// `defaults.streams` — key names identical to the `streams[]` entry keys (excludes id/source: writing them is
/// rejected at deploy time by deny_unknown_fields; the shared layer has no identity semantics).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamDefaults {
    #[serde(default)]
    codec: Option<String>,
    #[serde(default)]
    encoder_backend: Option<String>,
    #[serde(default)]
    bitrate_kbps: Option<u32>,
    #[serde(default)]
    stream_mode: Option<String>,
    #[serde(default)]
    min_bitrate_kbps: Option<u32>,
    #[serde(default)]
    keyframe_interval: Option<u32>,
}

/// `defaults.sources` — platform/tuning keys (width/height/fps/reconnect_ms/backend).
/// **Identity/topology keys stay out of the shared layer** (D282 revision): `mode` (source type) and `input` (source address)
/// are per-source identity attributes written on entries — making them shared lets "new source missing identity config"
/// silently fall into the wrong shape (e.g. under a global generator a new camera missing mode → color bars replace real video). Writing them errors out via deny.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceDefaults {
    /// Platform-wide capture backend declaration — one backend for the whole machine is legal shared semantics (x86 all v4l2 / Jetson all mipi).
    /// Note: **reserved slot** (entry layer same policy) — the current capturer auto-selects by mode+input and v4l2 is
    /// the only implementation; when the mipi backend is wired, entry + shared layers activate together.
    #[serde(default)]
    #[allow(dead_code)]
    backend: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    fps: Option<u32>,
    #[serde(default)]
    reconnect_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RecordSection {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    out_dir: Option<String>,
}

/// `[signaling]` section (D1: agent gateway local port; I1 review: vehicle-wide room).
#[derive(Debug, Deserialize)]
struct SignalingSection {
    #[serde(default)]
    local_port: Option<u16>,
    /// Vehicle-wide room (D3 TODO closed — host-agent --room; default = agent builtin "vehicle").
    #[serde(default)]
    room: Option<String>,
    /// Remote MediaServo server WS URL (default agent builtin ws://127.0.0.1:9800/ws).
    #[serde(default)]
    server_url: Option<String>,
    /// Signaling PSK (default agent builtin mediaservo-dev).
    #[serde(default)]
    psk: Option<String>,
}

/// Gateway local port ([signaling] local_port; default None → agent builtin 17980).
pub fn signaling_local_port(cfg: &str) -> Result<Option<u16>, String> {
    let cfg: HostConfig =
        serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml parse failed: {e}"))?;
    Ok(cfg.signaling.and_then(|s| s.local_port))
}

/// System hostname, sanitized for use as a room name (lowercase, [a-z0-9-]); None when
/// unreadable or empty. Reads /proc/sys/kernel/hostname — zero new dependencies.
pub fn system_hostname_room() -> Option<String> {
    let raw = std::fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
    let h = raw.trim().to_lowercase();
    let ok =
        !h.is_empty() && h.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if ok { Some(h) } else { None }
}

/// Vehicle-wide room: [signaling] room (manual) > system hostname (C-ruling 2026-10-19,
/// fleet-safe default) > agent builtin "vehicle" (last-resort fallback).
/// Closes the D3 TODO (gateway.rs Default) — translate is responsible for rendering the config into the oxfile.
pub fn signaling_room(cfg: &str) -> Result<Option<String>, String> {
    let cfg: HostConfig =
        serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml parse failed: {e}"))?;
    Ok(cfg.signaling.and_then(|s| s.room).or_else(system_hostname_room))
}

/// [control].hmac_key_file (V5; default None → controller zero perturbation).
pub fn control_hmac_key_file(cfg: &str) -> Result<Option<String>, String> {
    let cfg: HostConfig =
        serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml parse failed: {e}"))?;
    Ok(cfg.control.and_then(|c| c.hmac_key_file))
}

/// Remote server URL ([signaling] server_url; default None → host-agent builtin default).
pub fn signaling_server_url(cfg: &str) -> Result<Option<String>, String> {
    let cfg: HostConfig =
        serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml parse failed: {e}"))?;
    Ok(cfg.signaling.and_then(|s| s.server_url))
}

/// Signaling PSK ([signaling] psk; default None → host-agent builtin default).
pub fn signaling_psk(cfg: &str) -> Result<Option<String>, String> {
    let cfg: HostConfig =
        serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml parse failed: {e}"))?;
    Ok(cfg.signaling.and_then(|s| s.psk))
}

/// Child-process gateway URL (D2): `ws://127.0.0.1:{port}/ws`, port = [signaling] local_port
/// or the default 17980 (matching the host-agent builtin default).
pub fn signaling_gateway_url(cfg: &str) -> Result<String, String> {
    let port = signaling_local_port(cfg)?.unwrap_or(crate::gateway::DEFAULT_LOCAL_PORT);
    Ok(format!("ws://127.0.0.1:{port}/ws"))
}

/// Fixed 5 process base names (no brand prefix — the default brand maps to legacy "host-*", see brand.rs).
const FIXED_APP_BASES: [&str; 5] = ["agent", "recorder", "controller", "emergency", "audio"];

/// app name = brand prefix + base (default "host-" → "host-agent"; branded "cp-agent").
pub(crate) fn app_name(base: &str) -> String {
    format!("{}{}", mediaservo_common::brand::media_brand().app_prefix, base)
}

/// host.yaml → oxfile.toml text.
///
/// Instance names are always `type-id` (e.g. `host-capturer-cam0`) — E4 keeps app identity stable across config evolution:
/// adding/removing cameras/streams must never rename existing instances (a 1↔N boundary rename loses oxmgr history + leaves residual processes).
/// OxMgr validate rejects duplicate app names (CLI.md "duplicate app name" hard error).
/// Pathless variant: capturer instances get only `--camera <id>` (A2 shape, for doctor/tests).
pub fn to_oxfile(cfg: &str) -> Result<String, String> {
    to_oxfile_with_paths(cfg, Path::new(""), Path::new(""))
}

/// host.yaml → oxfile.toml; capturer instances get absolute `--config <dir>/etc/host.yaml`
/// and `--token <dir>/etc/link/<cam>.token` paths appended (Task C1).
pub fn to_oxfile_in_dir(cfg: &str, dir: &Path) -> Result<String, String> {
    let config_path = std::path::absolute(dir.join("etc").join("host.yaml"))
        .unwrap_or_else(|_| dir.join("etc").join("host.yaml"));
    let token_dir = std::path::absolute(dir.join("etc").join("link"))
        .unwrap_or_else(|_| dir.join("etc").join("link"));
    to_oxfile_with_paths(cfg, &config_path, &token_dir)
}

// ── E4 cloud config loop: shared validate/backup/write/apply implementation (host CLI and host-agent same source) ──

/// Validate host.yaml (the same parse path each consuming process uses: camera/stream/record/signaling;
/// plus a duplicate-id guard — duplicates → duplicate oxfile app names → oxmgr apply hard error).
/// F2: id charset guard [A-Za-z0-9_-]+ — illegal characters (quotes/newlines/path traversal) would produce
/// malformed oxfile (push_app does not escape) or poison token paths; must reject.
pub fn validate(cfg: &str) -> Result<(), String> {
    let sources = camera_configs(cfg)?;
    let streams = stream_configs(cfg)?;
    record_config(cfg)?;
    signaling_local_port(cfg)?;
    signaling_room(cfg)?;
    let mut seen = std::collections::HashSet::new();
    for c in &sources {
        check_id("source", &c.id)?;
        if !seen.insert(c.id.clone()) {
            return Err(format!("host.yaml parse failed: duplicate video source id: {}", c.id));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for s in &streams {
        check_id("stream", &s.id)?;
        if !seen.insert(s.id.clone()) {
            return Err(format!("host.yaml parse failed: duplicate stream id: {}", s.id));
        }
    }
    Ok(())
}

/// F2: id charset guard — only [A-Za-z0-9_-]+ allowed (the oxfile app name / token file name /
/// FrameBus topic all interpolate the id directly; illegal chars cause malformed output or path traversal).
fn check_id(kind: &str, id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!(
            "host.yaml parse failed: {kind} id invalid: {id:?} (only [A-Za-z0-9_-]+ allowed)"
        ));
    }
    Ok(())
}

/// Atomic write (tmp + rename) — the agent file-watch bell's fingerprint reads and the daemon reads during an
/// apply invocation only ever see complete files; readers never see half-written state.
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write {} failed: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("write {} failed: {e}", path.display()))
}

/// Validate + translate + atomic write to run/oxfile.toml; returns the oxfile path.
pub fn write_oxfile(cfg: &str, dir: &Path) -> Result<PathBuf, String> {
    validate(cfg)?;
    // V5 deploy-time guard: a configured signature key with a missing file = blocked at render (dying early beats the
    // controller crash-loop × restart_policy storm — a read-failure panic is executor-side discipline; the deploy side should block sooner).
    if let Some(rel) = control_hmac_key_file(cfg)? {
        let p = dir.join(&rel); // when rel is absolute, join takes it directly
        if !p.exists() {
            return Err(format!(
                "[control].hmac_key_file points to a missing file: {}",
                p.display()
            ));
        }
    }
    let ox = to_oxfile_in_dir(cfg, dir)?;
    let run_dir = dir.join("run");
    std::fs::create_dir_all(&run_dir)
        .map_err(|e| format!("create {} failed: {e}", run_dir.display()))?;
    let oxfile = run_dir.join("oxfile.toml");
    atomic_write(&oxfile, &ox)?;
    Ok(oxfile)
}

/// F1: recover the most recently applied version from etc/host.yaml.bak-<version> backups (take the max version).
/// After the agent file-watch bell / config-core re-applies a new version, config_version does not reset to zero —
/// the association contract between the version applied on disk and StatusReport.config_version. No backup → 0.
pub fn recover_config_version(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir.join("etc")) else { return 0 };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("host.yaml.bak-").and_then(|v| v.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0)
}

/// Back up the current host.yaml → etc/host.yaml.bak-<version> (pre-step of applying a ConfigPush).
/// Back up the current host.yaml → etc/host.yaml.bak-<version> (pre-step of applying a ConfigPush).
pub fn backup_host_config(dir: &Path, version: u64) -> Result<PathBuf, String> {
    let cfg_path = dir.join("etc").join("host.yaml");
    let bak = dir.join("etc").join(format!("host.yaml.bak-{version}"));
    std::fs::copy(&cfg_path, &bak)
        .map_err(|e| format!("backup {} → {} failed: {e}", cfg_path.display(), bak.display()))?;
    Ok(bak)
}

/// E4 ConfigPush apply (agent side; pure in-process, unit-testable): validate → backup → write host.yaml →
/// regenerate the oxfile. Any failure returns Err (rejection reason; files are never partially rewritten — validation first).
pub fn apply_config_push(dir: &Path, config: &str, version: u64) -> Result<(), String> {
    validate(config)?;
    backup_host_config(dir, version)?;
    atomic_write(&dir.join("etc").join("host.yaml"), config)?;
    write_oxfile(config, dir)?;
    Ok(())
}

/// oxmgr apply <run/oxfile.toml> (shared by CLI and agent; incremental: new Start / changed Recreate / unchanged Noop).
/// oxmgr binary: the host CLI's own directory first (installed into bin/ by packaging), falling back to PATH.
fn oxmgr_cmd() -> std::process::Command {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("oxmgr")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from("oxmgr"));
    std::process::Command::new(bin)
}

/// Instance daemon home (single source behind oxmgr_env and the config-core survivor scoping).
pub fn oxmgr_home(dir: &Path) -> PathBuf {
    dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("run").join("oxmgr")
}

/// Instance daemon env (OXMGR_HOME-derived port-isolated daemon — multi-instance / parallel tests do not collide).
/// oxmgr_apply, live_host_apps and delete must share this source, otherwise list hits the default daemon and comes back empty.
pub fn oxmgr_env(dir: &Path) -> Vec<(String, String)> {
    let home = oxmgr_home(dir);
    let sum: u32 = home.to_string_lossy().bytes().map(u32::from).sum();
    let port = 18000 + (sum % 400);
    vec![
        ("OXMGR_HOME".to_string(), home.to_string_lossy().into_owned()),
        // C32 legacy name oxmgr remembers — set both for compat (same shape as server oxmgr_cmd :1011)
        ("OXMGR_DATA_DIR".to_string(), home.to_string_lossy().into_owned()),
        ("OXMGR_DAEMON_ADDR".to_string(), format!("127.0.0.1:{port}")),
        ("OXMGR_API_ADDR".to_string(), format!("127.0.0.1:{}", port + 1000)),
    ]
}

pub(crate) fn oxmgr_cmd_with_env(env: &[(String, String)]) -> std::process::Command {
    let mut cmd = oxmgr_cmd();
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

pub fn oxmgr_apply(dir: &Path) -> Result<(), String> {
    let oxfile = dir.join("run").join("oxfile.toml");
    if !oxfile.exists() {
        return Err(format!("{} missing — run write_oxfile/host apply first", oxfile.display()));
    }
    let env = oxmgr_env(dir);
    let out =
        oxmgr_cmd_with_env(&env).arg("apply").arg(&oxfile).output().map_err(|e| {
            format!("oxmgr exec failed: {e} — install OxMgr and put it on PATH first")
        })?;
    if !out.status.success() {
        return Err(format!(
            "oxmgr apply failed (exit {}): {}",
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // Removal path: clean up host-namespace apps no longer present in the new oxfile. oxmgr apply defaults to
    // incremental-only (never removes), while --prune is a full cross-namespace sweep (empirically kills other tools apps) —
    // so delete by name one by one for precise sync.
    for name in removed_apps(&live_host_apps(&env)?, &oxfile_app_names(&oxfile)?) {
        match oxmgr_cmd_with_env(&env).arg("delete").arg(&name).output() {
            Ok(out) if out.status.success() => {
                tracing::info!(app = %name, "oxmgr delete: removed app absent from config");
            }
            Ok(out) => tracing::warn!(
                app = %name,
                "oxmgr delete failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => tracing::warn!(app = %name, "oxmgr delete exec failed: {e}"),
        }
    }
    Ok(())
}

/// Apps to delete = live host namespace minus the apps in the new oxfile (pure function, unit-testable).
fn removed_apps(live: &[String], desired: &[String]) -> Vec<String> {
    live.iter().filter(|n| !desired.contains(n)).cloned().collect()
}

/// App names in the host namespace from oxmgr list --json (removal cleanup + host stop fallback).
/// env shares its source with oxmgr_apply (per-instance daemon port isolation) — without env it connects to the default daemon
/// (empty list → the removal path silently no-ops; PIT-121 same-class proven).
pub fn live_host_apps(env: &[(String, String)]) -> Result<Vec<String>, String> {
    let out = oxmgr_cmd_with_env(env)
        .args(["list", "--json"])
        .output()
        .map_err(|e| format!("oxmgr exec failed: {e} — install OxMgr and put it on PATH first"))?;
    if !out.status.success() {
        return Err(format!(
            "oxmgr list failed (exit {}): {}",
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let val: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("oxmgr list --json parse failed: {e}"))?;
    Ok(val
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|p| {
                    p.get("namespace").and_then(|n| n.as_str())
                        == Some(mediaservo_common::brand::media_brand().namespace)
                })
                .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// Parse the oxfile [[apps]].name list (the desired side of the removal diff).
pub(crate) fn oxfile_app_names(path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("read {} failed: {e}", path.display()))?;
    let val: toml::Value =
        toml::from_str(&text).map_err(|e| format!("parse {} failed: {e}", path.display()))?;
    Ok(val
        .get("apps")
        .and_then(|a| a.as_array())
        .map(|apps| {
            apps.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// `host apply` shared implementation: read host.yaml → validate → translate → write oxfile → oxmgr apply.
pub fn apply_config(dir: &Path) -> Result<(), String> {
    let cfg = std::fs::read_to_string(dir.join("etc").join("host.yaml"))
        .map_err(|e| format!("read host.yaml failed: {e} — run host init <dir> first"))?;
    write_oxfile(&cfg, dir)?;
    oxmgr_apply(dir)
}

/// E4 agent apply body (apply + audit log; oxmgr apply is run by the caller — tests/CLI can separate it).
/// Returns the successfully applied version number; on failure Err(rejection reason) (the audit warn is already logged, C15).
/// F1 stale guard: version <= current_version rejected (stale/duplicate push; audit warn payload).
pub fn handle_config_push(
    dir: &Path,
    current_version: u64,
    push: &SignalingMessage,
) -> Result<u64, String> {
    let SignalingMessage::ConfigPush { config, version, .. } = push else {
        return Err("gateway pending message is not ConfigPush".into());
    };
    if *version <= current_version {
        return Err(format!(
            "ConfigPush version {version} expired (current {current_version}) — reject stale/duplicate push"
        ));
    }
    match apply_config_push(dir, config, *version) {
        Ok(()) => {
            tracing::info!(
                version,
                dir = %dir.display(),
                "ConfigPush applied — host.yaml updated + oxfile regenerated"
            );
            Ok(*version)
        }
        Err(e) => {
            tracing::warn!(version, "ConfigPush rejected: {e} — host.yaml unchanged");
            Err(e)
        }
    }
}

/// smooth preset bitrate floor = constant bytes per frame (3.33 kbps/fps), minimum 50: anchors 15fps→50 /
/// 30fps→100, 24→80 / 60→200 fall out naturally; no cap (high fps deserves a high floor). D282 supersedes
/// D274's fixed 400kbps (too conservative under extreme loss).
fn smooth_bitrate_floor_kbps(fps: u32) -> u32 {
    (fps * 100 / 30).max(50)
}

/// Encoder backend decision truth table (D282 / PIT-156 root fix; codec = the resolved value after the 3-layer merge):
///
/// | backend (merged)                 | codec=h264                   | other codec   |
/// |-------------------------------|------------------------------|-----------------|
/// | None / "auto"                    | pinned `software`            | as-is (no touch)|
/// | explicit software/hardware/nvenc/vaapi | passed through (respected) | as-is         |
///
/// The h264 hardware path has never been usable in this chain (Jetson auto picks MMAPI → negotiation mismatch → black screen), so auto
/// semantics are corrected to "pick the verified path when h264"; a user-explicit backend is never rewritten.
fn resolved_encoder_backend<'a>(codec: &str, backend: Option<&'a str>) -> Option<&'a str> {
    match backend {
        None | Some("auto") if codec == "h264" => Some("software"),
        b => b,
    }
}

fn to_oxfile_with_paths(cfg: &str, config_path: &Path, token_dir: &Path) -> Result<String, String> {
    // capturer instances need the full source config (reconnect_ms passthrough); streams need only their id.
    let sources = camera_configs(cfg)?;
    let (_, streams) = camera_and_stream_ids(cfg)?;
    // T1-e roster tri-state: the render gate reads the RAW [record] enabled flag
    // (record_enabled_raw), NOT the flattened record_config() default — absent/true/false
    // must stay distinguishable here. Only explicit `enabled: false` drops the recorder app.
    let record_enabled = record_enabled_raw(cfg)?;

    let mut out = format!(
        "version = 1\n\n[defaults]\nnamespace = \"{}\"\nrestart_policy = \"always\"\n",
        mediaservo_common::brand::media_brand().namespace
    );

    // T1b7 (A‴): the vendor oxmgr watch is RETIRED — [defaults] renders no watch/watch_delay_secs. Both reload
    // paths converge on config_core (validate + flock + teardown-verification gate + ledger): the E4 cloud loop
    // (one-way ConfigPush → handle_config_push → apply, heartbeat echo unchanged) and local edits via the agent
    // file-watch bell. With no vendor watch, a content-only edit reaches the cluster only through the bell;
    // restart_policy "always" keeps the agent alive — if it is briefly down the daemon relifts it and its boot
    // bell re-reconciles, so there is no orphan gap. cwd is kept on purpose: apps resolve relative paths against
    // it and it underpins the daemon-cwd-independence lesson (absolute log paths below) — it is not
    // watch-specific. The pathless variant (doctor) carries no cwd.
    let inst_root =
        config_path.parent().and_then(|p| p.parent()).map(|d| d.to_string_lossy().into_owned());
    if let Some(cwd) = inst_root.as_deref() {
        out.push_str(&format!("cwd = \"{cwd}\"\n"));
    }
    out.push('\n');
    // PIT: absolute log paths (instance run/logs) — OxMgr resolves relative paths against the daemon cwd, and a leftover
    // daemon with misaligned reuse (cwd pointing at a deleted temp dir) yields failed to create ./run/logs; absolute paths are daemon-cwd independent.
    let log_dir = inst_root.as_deref().map(|d| format!("{d}/run/logs"));

    for base in FIXED_APP_BASES {
        // T1-e: explicit record.enabled=false → no recorder slot at all (no process, no
        // oxmgr entry). Absent/Some(true) render exactly as before (byte-identical roster);
        // the program gate (record_config unwrap_or(false) + expected_process_names) keeps
        // its shape: absent = slot exists + process parks exit-0 = today's behavior.
        if base == "recorder" && record_enabled == Some(false) {
            continue;
        }
        let name = app_name(base);
        let mut cmd = exe_cmd(&name);
        // C3: the fixed recorder app gets --config/--token appended like capturer/streamer
        // (subscribes camera/* for recording; token file recorder.token).
        if name == app_name("recorder") && !config_path.as_os_str().is_empty() {
            cmd.push_str(&format!(
                " --config {} --token {}/recorder.token",
                config_path.display(),
                token_dir.display()
            ));
        }
        // D1: agent gateway local port ([signaling] local_port config; default = agent builtin 17980)
        // E1: agent gets --config appended (topology-monitor desired-state data source, same shape as recorder).
        // G1: agent gets --token agent.token appended (Monitor ACL) — data-flow monitoring is no longer tokenless.
        if name == app_name("agent") {
            if let Some(port) = signaling_local_port(cfg)? {
                cmd.push_str(&format!(" --port {port}"));
            }
            // I1 review (D3 TODO closed): [signaling] room → --room (default = agent builtin "vehicle")
            if let Some(room) = signaling_room(cfg)? {
                cmd.push_str(&format!(" --room {room}"));
            }
            // Remote server config: [signaling] server_url/psk → --remote/--psk (default = agent builtin)
            if let Some(url) = signaling_server_url(cfg)? {
                cmd.push_str(&format!(" --remote {url}"));
            }
            if let Some(psk) = signaling_psk(cfg)? {
                cmd.push_str(&format!(" --psk {psk}"));
            }
            if !config_path.as_os_str().is_empty() {
                cmd.push_str(&format!(" --config {}", config_path.display()));
                cmd.push_str(&format!(" --token {}/agent.token", token_dir.display()));
            }
        }
        // C4: with recorder [record] enabled=false it exits 0 by design — under oxmgr
        // restart_policy=always that becomes a restart storm; use on_failure (restart on crash, not on clean exit).
        let policy = if name == app_name("recorder") { "on_failure" } else { "always" };
        // V5: [control].hmac_key_file → controller --hmac-key-file (unset = zero added args =
        // byte-identical; relative paths are absolutized against the instance root — no controller-cwd dependency at apply time).
        if name == app_name("controller")
            && let Some(rel) = control_hmac_key_file(cfg)?
        {
            let abs = match inst_root.as_deref() {
                Some(r) if !std::path::Path::new(&rel).is_absolute() => {
                    format!("{r}/{rel}")
                }
                _ => rel,
            };
            cmd.push_str(&format!(" --hmac-key-file {abs}"));
        }
        // H2: host-audio must carry --room audio-<room> ([signaling] room or the default vehicle).
        if name == app_name("audio") {
            let room = signaling_room(cfg)?.unwrap_or_else(|| "vehicle".to_string());
            cmd.push_str(&format!(" --room audio-{room}"));
        }
        push_app(&mut out, &name, &cmd, policy, log_dir.as_deref());
    }
    for src in &sources {
        let name = instance_name(&app_name("capturer"), &src.id);
        let mut cmd = format!("{} --camera {}", exe_cmd(&app_name("capturer")), src.id);
        // T7: mode=camera with reconnect_ms configured → pass through --reconnect-ms (generator: never)
        if src.mode == SourceMode::Camera
            && let Some(ms) = src.reconnect_ms
        {
            cmd.push_str(&format!(" --reconnect-ms {ms}"));
        }
        if !config_path.as_os_str().is_empty() {
            cmd.push_str(&format!(
                " --config {} --token {}/{}.token",
                config_path.display(),
                token_dir.display(),
                src.id
            ));
        }
        push_app(&mut out, &name, &cmd, "always", log_dir.as_deref());
    }
    // stream id → config map (encoder-param passthrough; the streams loop works on an id list)
    let stream_cfgs: std::collections::HashMap<String, StreamConfig> =
        stream_configs(cfg)?.into_iter().map(|s| (s.id.clone(), s)).collect();
    // source id → resolved fps (the smooth floor takes its value via the stream→source link; not found → builtin default 30)
    let source_fps: std::collections::HashMap<&str, u32> =
        sources.iter().map(|s| (s.id.as_str(), s.fps)).collect();
    for stream in &streams {
        let name = instance_name(&app_name("streamer"), stream);
        let mut cmd = format!("{} --stream {}", exe_cmd(&app_name("streamer")), stream);
        // D2: child-process WS target = local gateway ([signaling] local_port or default 17980)
        cmd.push_str(&format!(" --gateway {}", signaling_gateway_url(cfg)?));
        // encoder-param passthrough (streams config surface — encoder_backend/bitrate/gop; defaults fall back on the streamer side)
        if let Some(sc) = stream_cfgs.get(stream) {
            // backend decision (truth table): h264 × unset/auto → pin explicit software (PIT-156 root fix)
            if let Some(b) = resolved_encoder_backend(&sc.codec, sc.encoder_backend.as_deref()) {
                cmd.push_str(&format!(" --encoder-backend {b}"));
            }
            // qos-framerate-priority AD-2: the single merge decision point — explicit keys > stream_mode bundle;
            // stream_mode legality was already enforced in stream_configs(), so this expect is only a guard.
            let mode = sc
                .stream_mode
                .as_deref()
                .map(|m| {
                    m.parse::<mediaservo_field::StreamMode>().expect("validated in stream_configs")
                })
                .unwrap_or(mediaservo_field::StreamMode::Balanced);
            let bundle = mode.bundle();
            let bitrate = sc.bitrate_kbps.or(bundle.bitrate_kbps);
            // smooth floor: explicit > fps-linked floor > bundle (the smooth bundle already yields None)
            let floor = (mode == mediaservo_field::StreamMode::Smooth).then(|| {
                let fps = source_fps.get(sc.source.as_str()).copied().unwrap_or(DEFAULT_SOURCE_FPS);
                smooth_bitrate_floor_kbps(fps)
            });
            let min_bitrate = sc.min_bitrate_kbps.or(floor).or(bundle.min_bitrate_kbps);
            use mediaservo_webrtc::rtp::{
                RTCDegradationPreference as Deg, RTCRtpContentHint as Hint,
            };
            if bundle.degradation != Deg::Balanced {
                let d = match bundle.degradation {
                    Deg::Fixed => "fixed",
                    Deg::MaintainFramerate => "framerate",
                    Deg::MaintainResolution => "resolution",
                    Deg::Balanced => "balanced",
                };
                cmd.push_str(&format!(" --degradation {d}"));
            }
            if bundle.content_hint == Hint::Fluid {
                cmd.push_str(" --content-hint fluid");
            }
            if let Some(k) = bitrate {
                cmd.push_str(&format!(" --bitrate-kbps {k}"));
            }
            if let Some(k) = min_bitrate {
                cmd.push_str(&format!(" --min-bitrate-kbps {k}"));
            }
            if let Some(g) = sc.keyframe_interval {
                cmd.push_str(&format!(" --keyframe-interval {g}"));
            }
        }
        if !config_path.as_os_str().is_empty() {
            cmd.push_str(&format!(
                " --config {} --token {}/{}-stream.token",
                config_path.display(),
                token_dir.display(),
                stream
            ));
        }
        push_app(&mut out, &name, &cmd, "always", log_dir.as_deref());
    }
    Ok(out)
}

/// Expected process names (E1 topology-monitor desired state; same instance-naming source as oxfile generation, DRY).
pub fn expected_process_names(cfg: &str) -> Result<Vec<String>, String> {
    let (sources, streams) = camera_and_stream_ids(cfg)?;
    let mut out: Vec<String> = FIXED_APP_BASES.iter().map(|b| app_name(b)).collect();
    // [record] enabled=false (default) → host-recorder exits 0 by design (host-recorder.rs)
    // and oxmgr on_failure does not restart → exclude it from expected, otherwise the default config false-alarms ProcessMissing forever.
    if !record_config(cfg)?.enabled {
        out.retain(|n| n != &app_name("recorder"));
    }
    for src in &sources {
        out.push(instance_name(&app_name("capturer"), src));
    }
    for stream in &streams {
        out.push(instance_name(&app_name("streamer"), stream));
    }
    Ok(out)
}

/// Extract the sources/streams id lists (reused by host init to generate ros_bridge.yaml; single parse point).
pub fn camera_and_stream_ids(cfg: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let cfg: HostConfig =
        serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml parse failed: {e}"))?;
    Ok((
        cfg.sources.into_iter().map(|c| c.id).collect(),
        cfg.streams.into_iter().map(|s| s.id).collect(),
    ))
}

/// Source config (consumed by capturer/recorder/monitor; mode/width/height/fps already fall back to defaults).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceConfig {
    pub id: String,
    pub mode: SourceMode,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// v4l2 device path (required for mode=camera; optional for other modes).
    pub input: Option<String>,
    /// Reconnect interval ms after capture failure (passed through as capturer --reconnect-ms for mode=camera; default 5000 on the capturer side).
    pub reconnect_ms: Option<u64>,
}

/// Default frame width (when width is unset; formerly capturer DEFAULT_WIDTH).
const DEFAULT_SOURCE_WIDTH: u32 = 1280;
/// Default frame height (when height is unset; formerly capturer DEFAULT_HEIGHT).
const DEFAULT_SOURCE_HEIGHT: u32 = 720;
/// Default frame rate (when fps is unset; also the fallback the smooth floor uses when the linked source fps cannot be found).
const DEFAULT_SOURCE_FPS: u32 = 30;

/// Parse all video source configs (formerly camera_configs' duty — function name kept, consumer surface unchanged).
/// The old `source` field other than "stub" is rejected (migration hint); fps=0 is rejected (generator.start(0) panics inside the
/// thread → silent hang, found in the C1 review).
///
/// `defaults.sources` is merged key-by-key here (entry > defaults > builtin, D282) — the single source-config parse point.
pub fn camera_configs(cfg: &str) -> Result<Vec<SourceConfig>, String> {
    let HostConfig { sources, defaults, .. } = serde_yaml::from_str::<HostConfig>(cfg)
        .map_err(|e| format!("host.yaml parse failed: {e}"))?;
    let d = defaults.sources;
    let mut out = Vec::with_capacity(sources.len());
    for c in sources {
        if let Some(src) = &c.source
            && src != "stub"
        {
            return Err(format!(
                "host.yaml parse failed: video source {} uses the legacy field source={src}, unsupported (stub only; for v4l2/mipi switch to mode: camera + backend)",
                c.id
            ));
        }
        let fps = c.fps.or(d.fps).unwrap_or(DEFAULT_SOURCE_FPS);
        if fps == 0 {
            return Err(format!(
                "host.yaml parse failed: video source {} fps=0 invalid (must be > 0)",
                c.id
            ));
        }
        if fps > 60 {
            return Err(format!(
                "host.yaml parse failed: video source {} fps={fps} exceeds the cap (<=60)",
                c.id
            ));
        }
        out.push(SourceConfig {
            id: c.id,
            // the mode identity key lives on entries only (D282 revision); neither set → generator (original stub generation semantics, existing compat)
            mode: c.mode.unwrap_or(SourceMode::Generator),
            width: c.width.or(d.width).unwrap_or(DEFAULT_SOURCE_WIDTH),
            height: c.height.or(d.height).unwrap_or(DEFAULT_SOURCE_HEIGHT),
            fps,
            input: c.input, // the source address is an identity key, entry layer only (D282 revision)
            reconnect_ms: c.reconnect_ms.or(d.reconnect_ms),
        });
    }
    Ok(out)
}
/// Record config (consumed by the recorder process; the [record] section defaults to disabled + a default output dir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordConfig {
    pub enabled: bool,
    pub out_dir: PathBuf,
}

/// Default recording output dir (overridable via host.yaml [record] out_dir; the dev default lives under /tmp).
const DEFAULT_RECORD_DIR: &str = "/tmp/mediaservo-recordings";

/// Parse the record config (for the C3 recorder). Defaults: disabled + /tmp/mediaservo-recordings.
pub fn record_config(cfg: &str) -> Result<RecordConfig, String> {
    let cfg: HostConfig =
        serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml parse failed: {e}"))?;
    let rec = cfg.record.unwrap_or(RecordSection { enabled: None, out_dir: None });
    Ok(RecordConfig {
        enabled: rec.enabled.unwrap_or(false),
        out_dir: PathBuf::from(rec.out_dir.unwrap_or_else(|| DEFAULT_RECORD_DIR.to_string())),
    })
}

/// Raw `[record] enabled` value without the program-side default: None = key omitted,
/// Some(true)/Some(false) = explicit. The T1-e render gate trips ONLY on Some(false);
/// record_config() keeps its `unwrap_or(false)` flattening for the process semantics.
pub fn record_enabled_raw(cfg: &str) -> Result<Option<bool>, String> {
    let cfg: HostConfig =
        serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml parse failed: {e}"))?;
    Ok(cfg.record.and_then(|r| r.enabled))
}

/// Look up a single video source config by id (not found → Ok(None)).
pub fn camera_config(cfg: &str, id: &str) -> Result<Option<SourceConfig>, String> {
    Ok(camera_configs(cfg)?.into_iter().find(|c| c.id == id))
}

/// Stream config (consumed by streamer; source/codec default to id/h264).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamConfig {
    pub id: String,
    /// Referenced source id (determines the FrameBus topic camera/<id>).
    pub source: String,
    /// Codec (aligned with field sfu::codec_spec: vp8/h264/vp9/av1; default h264, D282).
    pub codec: String,
    /// Encoder backend (auto/software/hardware/nvenc/vaapi; None=auto).
    pub encoder_backend: Option<String>,
    /// Encode bitrate kbps (None=field default 2000).
    pub bitrate_kbps: Option<u32>,
    /// Weak-network preset raw string (None=balanced; legality checked in stream_configs(), AD-2).
    pub stream_mode: Option<String>,
    /// Bitrate floor kbps (None=follows the bundle; explicit > preset is decided at the oxfile assembly site).
    pub min_bitrate_kbps: Option<u32>,
    /// Keyframe interval seconds GOP (None=field default 2).
    pub keyframe_interval: Option<u32>,
}

/// Parse all stream configs (for the C2 streamer).
///
/// `defaults.streams` is merged key-by-key here (entry > defaults > builtin, D282) — the single stream-config parse point;
/// the legality gate runs **after** the merge, so all three sources are checked (a bad value surfaces at deploy wherever it is written).
pub fn stream_configs(cfg: &str) -> Result<Vec<StreamConfig>, String> {
    let HostConfig { streams, defaults, .. } = serde_yaml::from_str::<HostConfig>(cfg)
        .map_err(|e| format!("host.yaml parse failed: {e}"))?;
    let d = &defaults.streams;
    streams
        .into_iter()
        .map(|s| {
            let id = s.id.clone();
            let codec = s.codec.or_else(|| d.codec.clone()).unwrap_or_else(|| "h264".into());
            let stream_mode = s.stream_mode.or_else(|| d.stream_mode.clone());
            // qos-framerate-priority AD-2: an illegal stream_mode is intercepted here (Err at deploy/build time,
            // the error message lists the legal set; it never reaches the streamer, avoiding oxmgr restart storms).
            if let Some(m) = &stream_mode
                && m.parse::<mediaservo_field::StreamMode>().is_err()
            {
                return Err(format!("host.yaml parse failed: stream {id} stream_mode={m} illegal (legal: smooth|balanced|quality)"));
            }
            // The legal set aligns with field sfu::codec_spec — unknown values silently fall back to VP8 at the field layer,
            // so a typo is only visible when blocked here (deploy-time error, not inside the streamer).
            if !matches!(codec.as_str(), "vp8" | "h264" | "vp9" | "av1") {
                return Err(format!(
                    "host.yaml parse failed: stream {id} codec={codec} illegal (legal: vp8|h264|vp9|av1)"
                ));
            }
            Ok(StreamConfig {
                id,
                source: s.source.unwrap_or(s.id),
                codec,
                encoder_backend: s.encoder_backend.or_else(|| d.encoder_backend.clone()),
                bitrate_kbps: s.bitrate_kbps.or(d.bitrate_kbps),
                stream_mode,
                min_bitrate_kbps: s.min_bitrate_kbps.or(d.min_bitrate_kbps),
                keyframe_interval: s.keyframe_interval.or(d.keyframe_interval),
            })
        })
        .collect()
}

/// Look up a single stream config by id (not found → Ok(None)).
pub fn stream_config(cfg: &str, id: &str) -> Result<Option<StreamConfig>, String> {
    Ok(stream_configs(cfg)?.into_iter().find(|s| s.id == id))
}

/// Instance app names are uniformly `type-id` (E4 stable identity: config add/remove never renames existing instances,
/// oxmgr restart history / health records stay continuous; the topology desired state shares the same source).
fn instance_name(kind: &str, id: &str) -> String {
    format!("{kind}-{id}")
}

/// Process executable path: the host CLI's own directory (same target artifacts); under test current_exe lives
/// in deps/, so fall back to the bare name (tests only assert command-line substrings).
// ponytail: the bare name relies on PATH; pin absolute paths at the deployment stage (A4 script/packaging).
fn exe_cmd(name: &str) -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name).to_string_lossy().into_owned()))
        .unwrap_or_else(|| name.to_string())
}

fn push_app(
    out: &mut String,
    name: &str,
    command: &str,
    restart_policy: &str,
    log_dir: Option<&str>,
) {
    out.push_str(&format!(
        "[[apps]]\nname = \"{name}\"\ncommand = \"{command}\"\nrestart_policy = \"{restart_policy}\"\n"
    ));
    // D-H13 + PIT: absolute log paths (instance run/logs) — OxMgr resolves relative paths against the daemon cwd,
    // and a leftover daemon with misaligned reuse yields failed to create ./run/logs; the pathless variant (doctor) keeps them relative.
    // Rotation is controlled by the OxMgr daemon env (OXMGR_LOG_MAX_SIZE_MB/MAX_FILES/MAX_DAYS — default 20MB×5×14d)
    let (stdout, stderr) = match log_dir {
        Some(d) => (format!("{d}/{name}.out.log"), format!("{d}/{name}.err.log")),
        None => (format!("logs/{name}.out.log"), format!("logs/{name}.err.log")),
    };
    out.push_str(&format!("[apps.logs]\nstdout = \"{stdout}\"\nstderr = \"{stderr}\"\n\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// New-style config (sources / streams.source).
    const CFG_V0: &str = r#"
sources:
  - id: "cam0"
    mode: "generator"
    fps: 30

streams:
  - id: "s0"
    source: "cam0"
"#;

    /// Old-style config (cameras / source field / streams.camera) — alias-compat gate.
    const CFG_LEGACY: &str = r#"
cameras:
  - id: "cam0"
    source: "stub"
    fps: 30

streams:
  - id: "s0"
    camera: "cam0"
"#;

    fn write_host_yaml(dir: &Path, cfg: &str) {
        let etc = dir.join("etc");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("host.yaml"), cfg).unwrap();
    }

    // ── parsing: new/old keys + field semantics ──

    #[test]
    fn legacy_cameras_and_camera_keys_still_parse() {
        // deployed old host.yaml (cameras / source field / streams.camera) parses into the new shape without breakage
        let cams = camera_configs(CFG_LEGACY).unwrap();
        assert_eq!(cams.len(), 1);
        assert_eq!(cams[0].id, "cam0");
        assert_eq!(
            cams[0].mode,
            SourceMode::Generator,
            "legacy config without mode → generator (original stub semantics)"
        );
        assert_eq!(cams[0].width, 1280, "legacy config missing width → default 1280");
        assert_eq!(cams[0].height, 720, "legacy config missing height → default 720");
        assert_eq!(cams[0].fps, 30);
        let streams = stream_configs(CFG_LEGACY).unwrap();
        assert_eq!(
            streams[0].source, "cam0",
            "the legacy camera key should resolve to source via alias"
        );
        assert!(validate(CFG_LEGACY).is_ok());
        assert!(to_oxfile(CFG_LEGACY).unwrap().contains("host-capturer-cam0"));
    }

    #[test]
    fn legacy_source_non_stub_rejected() {
        let old_v4l2 = "cameras:\n  - id: \"cam0\"\n    source: \"v4l2\"\n    fps: 30\n";
        let err = camera_configs(old_v4l2).unwrap_err();
        assert!(err.contains("v4l2") && err.contains("unsupported"), "{err}");
    }

    #[test]
    fn source_config_resolves_mode_width_height_fps() {
        let cfg = r#"
sources:
  - id: "cam0"
    mode: "camera"
    backend: "v4l2"
    input: "/dev/video0"
    width: 1920
    height: 1080
    fps: 15
  - id: "gen0"
    mode: "generator"
    fps: 30
"#;
        let cams = camera_configs(cfg).unwrap();
        assert_eq!(cams.len(), 2);
        assert_eq!(cams[0].mode, SourceMode::Camera);
        assert_eq!(cams[0].width, 1920);
        assert_eq!(cams[0].height, 1080);
        assert_eq!(cams[0].fps, 15);
        assert_eq!(cams[1].mode, SourceMode::Generator);
        assert_eq!(cams[1].width, 1280, "default width 1280");
        assert_eq!(cams[1].height, 720, "default height 720");
        assert_eq!(cams[1].fps, 30);
        // single lookup
        assert!(camera_config(cfg, "cam0").unwrap().is_some());
        assert!(camera_config(cfg, "nope").unwrap().is_none());
        assert!(camera_configs("not yaml [[[").is_err());
    }

    #[test]
    fn source_missing_mode_defaults_to_generator() {
        let cfg = "sources:\n  - id: \"s0\"\n";
        assert_eq!(camera_configs(cfg).unwrap()[0].mode, SourceMode::Generator);
    }

    #[test]
    fn camera_config_rejects_zero_fps() {
        // C1 review finding: fps=0 → panic inside the generator.start thread → dead thread + main thread blocked forever
        let cfg = "sources:\n  - id: \"cam0\"\n    fps: 0\n";
        let err = camera_configs(cfg).unwrap_err();
        assert!(err.contains("fps") && err.contains("cam0"), "{err}");
        assert!(camera_config(cfg, "cam0").is_err());
    }

    #[test]
    fn camera_config_rejects_fps_over_60() {
        // multi-stream P1: fps>60 cap (default 30 / zero-value rejection already exists)
        let cfg = "sources:\n  - id: \"cam0\"\n    fps: 61\n";
        let err = camera_configs(cfg).unwrap_err();
        assert!(err.contains("60") && err.contains("cam0"), "{err}");
        assert!(camera_config(cfg, "cam0").is_err());
        // boundary 60 is legal
        let ok = "sources:\n  - id: \"cam0\"\n    fps: 60\n";
        assert_eq!(camera_configs(ok).unwrap()[0].fps, 60);
    }

    #[test]
    fn stream_source_refers_to_sources_id() {
        // default source = the stream id itself; explicit references sources[].id (the legacy camera key alias is covered by the legacy test)
        let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n  - id: \"s1\"\n    source: \"cam0\"\n    codec: \"h264\"\n";
        let streams = stream_configs(cfg).unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].source, "s0", "default = the stream id itself");
        assert_eq!(streams[0].codec, "h264", "host-layer default codec = h264 (D282)");
        assert_eq!(streams[1].source, "cam0");
        assert_eq!(streams[1].codec, "h264");
        assert!(stream_config(cfg, "s1").unwrap().is_some());
        assert!(stream_config(cfg, "nope").unwrap().is_none());
        assert!(stream_configs("not yaml [[[").is_err());
    }

    // ── validation ──

    #[test]
    fn validate_rejects_invalid_yaml() {
        let err = validate("host: \"unterminated").unwrap_err();
        assert!(err.contains("parse failed"), "{err}");
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let dup_src = "sources:\n  - id: \"cam0\"\n  - id: \"cam0\"\n";
        assert!(
            validate(dup_src).unwrap_err().contains("duplicate"),
            "duplicate sources id must be rejected"
        );
        let dup_stream = "streams:\n  - id: \"s0\"\n  - id: \"s0\"\n";
        assert!(
            validate(dup_stream).unwrap_err().contains("duplicate"),
            "duplicate stream id must be rejected"
        );
    }

    #[test]
    fn validate_accepts_wellformed_config() {
        validate(CFG_V0).expect("well-formed new-shape config should pass");
        validate(CFG_LEGACY).expect("legacy shape (alias) should also pass");
    }

    #[test]
    fn validate_rejects_non_alnum_source_and_stream_ids() {
        // F2: YAML-legal but ids containing quotes/newlines/path traversal → malformed oxfile (push_app unescaped)
        // or token-path poisoning → must reject (only [A-Za-z0-9_-]+ allowed)
        let quote_src = "sources:\n  - id: \"cam\\\"0\"\n";
        let err = validate(quote_src).unwrap_err();
        assert!(err.contains("invalid"), "quoted source id must be rejected: {err}");
        let path_src = "sources:\n  - id: \"../evil\"\n";
        assert!(
            validate(path_src).unwrap_err().contains("invalid"),
            "path-traversal source id must be rejected"
        );
        let newline_src = "sources:\n  - id: \"cam0\\n1\"\n";
        assert!(
            validate(newline_src).unwrap_err().contains("invalid"),
            "newline source id must be rejected"
        );
        let quote_stream = "streams:\n  - id: \"s\\\"0\"\n";
        assert!(
            validate(quote_stream).unwrap_err().contains("invalid"),
            "quoted stream id must be rejected"
        );
        // normal ids (alphanumeric + hyphen/underscore) pass
        validate("sources:\n  - id: \"cam-A_1\"\nstreams:\n  - id: \"s-2_0\"\n")
            .expect("legal-charset id should pass");
    }

    // ── oxfile translation ──

    #[test]
    fn oxfile_wires_remote_server_and_psk() {
        let cfg = "host:\n  device_id: \"x\"\nsignaling:\n  server_url: \"ws://192.168.2.127:9800/ws\"\n  psk: \"prod-psk\"\nsources:\n  - id: \"cam0\"\nstreams:\n  - id: \"cam0-stream\"\n    source: \"cam0\"\n";
        let ox = to_oxfile(cfg).unwrap();
        assert!(ox.contains("--remote ws://192.168.2.127:9800/ws"));
        assert!(ox.contains("--psk prod-psk"));
        // unset → not generated (agent builtin default)
        let ox2 = to_oxfile("host:\n  device_id: \"x\"\nsources:\n  - id: \"cam0\"\nstreams:\n  - id: \"cam0-stream\"\n    source: \"cam0\"\n").unwrap();
        assert!(!ox2.contains("--remote"));
        assert!(!ox2.contains("--psk"));
    }

    #[test]
    fn defaults_namespace_is_concrete_not_placeholder() {
        // PIT-118 regression gate: the namespace once retained a literal "{ns}" — oxmgr rejects it → apply hangs
        let ox = to_oxfile(CFG_V0).unwrap();
        assert!(
            ox.contains("namespace = \"mediaservo-host\""),
            "default-brand namespace should be mediaservo-host: {ox}"
        );
        assert!(!ox.contains("{ns}"), "no placeholder residue allowed: {ox}");
    }

    #[test]
    fn write_oxfile_writes_translated_file() {
        let dir = tempfile::tempdir().unwrap();
        let oxfile = write_oxfile(CFG_V0, dir.path()).expect("write_oxfile");
        assert_eq!(oxfile, dir.path().join("run").join("oxfile.toml"));
        let ox = std::fs::read_to_string(&oxfile).unwrap();
        assert!(ox.contains("host-capturer"), "source instances should be in the oxfile: {ox}");
        assert!(ox.contains("host-streamer"), "stream instances should be in the oxfile");
    }

    #[test]
    fn oxfile_sets_cwd_without_vendor_watch() {
        // T1b7 contract: the production render pins the instance cwd (apps resolve relative paths;
        // daemon-cwd independence) and carries NO vendor oxmgr watch — the agent file-watch bell invoking
        // config_core is the sole local-edit path.
        let dir = tempfile::tempdir().unwrap();
        let ox = to_oxfile_in_dir(CFG_V0, dir.path()).expect("to_oxfile_in_dir");
        let expected_cwd = format!("cwd = \"{}\"", dir.path().display());
        assert!(ox.contains(&expected_cwd), "the oxfile should pin the instance cwd: {ox}");
        assert!(ox.contains("\n[apps") || ox.contains("[apps"), "roster still renders: {ox}");
        assert!(!ox.contains("watch"), "vendor watch retired — no watch lines may render: {ox}");
        assert!(!ox.contains("watch_delay_secs"), "vendor watch debounce gone with it: {ox}");
        // the pathless variant (for doctor) carries neither cwd nor watch
        let pathless = to_oxfile(CFG_V0).unwrap();
        assert!(!pathless.contains("watch"), "the to_oxfile pathless variant should not watch");
        assert!(!pathless.contains("cwd = \""), "the to_oxfile pathless variant carries no cwd");
    }

    /// G1: host-agent carries --token like the recorder (agent.token, Monitor ACL) —
    /// every oxfile process is fully credentialed; agent data-flow monitoring is no longer tokenless.
    #[test]
    fn oxfile_wires_agent_token() {
        let dir = tempfile::tempdir().unwrap();
        let ox = to_oxfile_in_dir(CFG_V0, dir.path()).expect("to_oxfile_in_dir");
        let agent_line = ox
            .lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent command line: {ox}");
        assert!(
            agent_line.contains("--token") && agent_line.contains("agent.token"),
            "host-agent should carry --token agent.token: {agent_line}"
        );
    }

    #[test]
    fn system_hostname_room_sanitized_k() {
        // live-box sanity: whatever this machine's hostname is, the sanitizer holds.
        if let Some(h) = system_hostname_room() {
            assert!(!h.is_empty());
            assert!(h.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
        }
    }

    #[test]
    fn signaling_room_precedence_manual_over_hostname_k() {
        // manual wins
        let with = "signaling:\n  room: \"ms-car7\"\n";
        assert_eq!(signaling_room(with).unwrap().as_deref(), Some("ms-car7"));
        // unset falls to hostname when readable (never asserts a specific name)
        let unset = "sources: []\n";
        let got = signaling_room(unset).unwrap();
        if system_hostname_room().is_some() {
            assert!(got.is_some(), "hostname fallback must fire when readable");
        }
    }

    /// I1 review (D3 TODO closed): [signaling] room → agent --room; unset → no emit (agent builtin "vehicle").
    #[test]
    fn oxfile_wires_signaling_room_to_agent() {
        let with_room = format!("{CFG_V0}signaling:\n  room: \"ms-car7\"\n");
        let ox = to_oxfile(&with_room).expect("to_oxfile");
        let agent_line = ox
            .lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent command line: {ox}");
        assert!(
            agent_line.contains("--room ms-car7"),
            "host-agent should carry --room ms-car7: {agent_line}"
        );
        assert_eq!(signaling_room(&with_room).unwrap().as_deref(), Some("ms-car7"));
        let ox2 = to_oxfile(CFG_V0).expect("to_oxfile default");
        let agent_line2 = ox2
            .lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent command line: {ox2}");
        // C-ruling 2026-10-19: unset room now falls back to the system hostname
        // (fleet-safe default); "vehicle" only remains when the hostname is unreadable.
        let fallback =
            signaling_room(CFG_V0).unwrap().expect("unset room falls back to hostname/vehicle");
        let agent_line2 = ox2
            .lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent command line: {ox2}");
        assert!(
            agent_line2.contains(&format!("--room {fallback}")),
            "agent must carry the derived room ({fallback}): {agent_line2}"
        );
        assert_eq!(signaling_room(CFG_V0).unwrap().as_deref(), Some(fallback.as_str()));
    }

    /// H2: host-audio must carry --room audio-<vehicle> (the audio room convention).
    #[test]
    fn oxfile_wires_audio_room_to_host_audio() {
        let with_room = format!("{CFG_V0}signaling:\n  room: \"ms-deploy-car1\"\n");
        let ox = to_oxfile(&with_room).expect("to_oxfile");
        let audio_line = ox
            .lines()
            .find(|l| l.contains("command") && l.contains("host-audio"))
            .expect("host-audio command line: {ox}");
        assert!(
            audio_line.contains("--room audio-ms-deploy-car1"),
            "host-audio should carry --room audio-ms-deploy-car1: {audio_line}"
        );
        let ox2 = to_oxfile(CFG_V0).expect("to_oxfile default");
        let audio_line2 = ox2
            .lines()
            .find(|l| l.contains("command") && l.contains("host-audio"))
            .expect("host-audio command line: {ox2}");
        // C-ruling: unset room -> audio-<hostname-fallback> (was audio-vehicle).
        let fallback =
            signaling_room(CFG_V0).unwrap().expect("unset room falls back to hostname/vehicle");
        let audio_line2 = ox2
            .lines()
            .find(|l| l.contains("command") && l.contains("host-audio"))
            .expect("host-audio command line: {ox2}");
        assert!(
            audio_line2.contains(&format!("--room audio-{fallback}")),
            "host-audio must carry the derived audio room: {audio_line2}"
        );
    }

    #[test]
    fn apply_config_push_updates_host_yaml_backs_up_and_regenerates_oxfile() {
        let dir = tempfile::tempdir().unwrap();
        write_host_yaml(dir.path(), CFG_V0);
        let cfg_v1 = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        apply_config_push(dir.path(), cfg_v1, 7).expect("apply_config_push");

        // host.yaml updated
        let now = std::fs::read_to_string(dir.path().join("etc").join("host.yaml")).unwrap();
        assert_eq!(now, cfg_v1, "host.yaml should hold the new config");
        // the backup holds the old config
        let bak = std::fs::read_to_string(dir.path().join("etc").join("host.yaml.bak-7")).unwrap();
        assert_eq!(bak, CFG_V0, "the backup should hold the old config");
        // oxfile regenerated (new source instance)
        let ox = std::fs::read_to_string(dir.path().join("run").join("oxfile.toml")).unwrap();
        assert!(
            ox.contains("host-capturer-cam1"),
            "the new source instance should be in the oxfile: {ox}"
        );
    }

    #[test]
    fn apply_config_push_rejects_invalid_and_leaves_files_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        write_host_yaml(dir.path(), CFG_V0);

        let err = apply_config_push(dir.path(), "host: \"unterminated", 9).unwrap_err();
        assert!(
            err.contains("parse failed"),
            "the rejection reason should mention the parse failure: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("etc").join("host.yaml")).unwrap(),
            CFG_V0,
            "an invalid config must not rewrite host.yaml"
        );
        assert!(
            !dir.path().join("etc").join("host.yaml.bak-9").exists(),
            "an invalid config must not create a backup"
        );
        assert!(
            !dir.path().join("run").join("oxfile.toml").exists(),
            "an invalid config must not rewrite the oxfile"
        );
    }

    #[test]
    fn removed_apps_diff_live_vs_desired() {
        let live =
            vec!["host-agent".into(), "host-capturer-cam0".into(), "host-capturer-cam1".into()];
        let desired = vec!["host-agent".into(), "host-capturer-cam0".into()];
        assert_eq!(removed_apps(&live, &desired), vec!["host-capturer-cam1"]);
        // everything is within the desired set → nothing to delete
        assert!(removed_apps(&live, &live).is_empty());
    }

    #[test]
    fn instance_names_stable_across_count_change() {
        // a single source still carries the id suffix — after adding sources cam0's app name is unchanged (stable identity)
        let ox1 = to_oxfile(CFG_V0).unwrap();
        assert!(ox1.contains("name = \"host-capturer-cam0\""), "single source: {ox1}");
        let v2 = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        let ox2 = to_oxfile(v2).unwrap();
        assert!(
            ox2.contains("name = \"host-capturer-cam0\""),
            "dual-source cam0 name unchanged: {ox2}"
        );
        assert!(ox2.contains("name = \"host-capturer-cam1\""), "dual-source cam1 in oxfile: {ox2}");
    }

    #[test]
    fn recover_config_version_returns_max_backup_version_after_restart() {
        // F1 association contract: after the agent file-watch bell / config-core re-applies, config_version must
        // recover from disk (the max version among the backups), never reset to zero
        let dir = tempfile::tempdir().unwrap();
        write_host_yaml(dir.path(), CFG_V0);
        let v7 = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        let v10 = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\n  - id: \"cam2\"\n    mode: \"generator\"\n    fps: 30\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        apply_config_push(dir.path(), v7, 7).unwrap();
        apply_config_push(dir.path(), v10, 10).unwrap();
        assert_eq!(
            recover_config_version(dir.path()),
            10,
            "after a restart it should recover the max version from backups"
        );
        // no backups → 0
        let fresh = tempfile::tempdir().unwrap();
        write_host_yaml(fresh.path(), CFG_V0);
        assert_eq!(recover_config_version(fresh.path()), 0, "version is 0 with no backups");
    }

    #[test]
    fn handle_config_push_rejects_stale_or_duplicate_versions() {
        // F1 stale guard: version <= current rejected (audit warn payload; files unchanged)
        let dir = tempfile::tempdir().unwrap();
        write_host_yaml(dir.path(), CFG_V0);
        let push = |version: u64| SignalingMessage::ConfigPush {
            room_id: "r".into(),
            target: "p".into(),
            config: CFG_V0.into(),
            version,
            actor: None,
        };
        assert_eq!(handle_config_push(dir.path(), 0, &push(7)).unwrap(), 7);
        let dup = handle_config_push(dir.path(), 7, &push(7)).unwrap_err();
        assert!(dup.contains("expired"), "a same-version duplicate must be rejected: {dup}");
        let stale = handle_config_push(dir.path(), 7, &push(6)).unwrap_err();
        assert!(stale.contains("expired"), "an older version must be rejected: {stale}");
        assert_eq!(
            handle_config_push(dir.path(), 7, &push(8)).unwrap(),
            8,
            "a newer version should be accepted"
        );
        // rejection must not rewrite files (host.yaml stays at the initial config after re-pushing v7)
        assert_eq!(
            std::fs::read_to_string(dir.path().join("etc").join("host.yaml")).unwrap(),
            CFG_V0,
            "rejection must not rewrite host.yaml"
        );
    }
    /// T7: mode=camera + reconnect_ms → the capturer command line passes --reconnect-ms; generator never;
    /// camera sources without reconnect_ms configured emit nothing (capturer-side default 5000).
    #[test]
    fn oxfile_passes_reconnect_ms_for_camera_mode_only() {
        let cfg = r#"
sources:
  - id: "cam0"
    mode: "camera"
    backend: "v4l2"
    input: "/dev/video0"
    width: 1920
    height: 1080
    fps: 30
    reconnect_ms: 3000
  - id: "gen0"
    mode: "generator"
    fps: 30
    reconnect_ms: 9999
streams:
  - id: "s0"
    source: "cam0"
"#;
        let ox = to_oxfile(cfg).unwrap();
        let cam_line = ox
            .lines()
            .find(|l| l.contains("command") && l.contains("--camera cam0"))
            .expect("cam0 capturer command line");
        assert!(
            cam_line.contains("--reconnect-ms 3000"),
            "a camera source should pass reconnect_ms through: {cam_line}"
        );
        let gen_line = ox
            .lines()
            .find(|l| l.contains("command") && l.contains("--camera gen0"))
            .expect("gen0 capturer command line");
        assert!(
            !gen_line.contains("--reconnect-ms"),
            "a generator source must not add --reconnect-ms: {gen_line}"
        );
        // a camera source without reconnect_ms configured → emit nothing (capturer-side default 5000)
        let no_reconnect =
            "sources:\n  - id: \"cam1\"\n    mode: \"camera\"\n    input: \"/dev/video0\"\n";
        let ox2 = to_oxfile(no_reconnect).unwrap();
        let cam1_line = ox2
            .lines()
            .find(|l| l.contains("command") && l.contains("--camera cam1"))
            .expect("cam1 capturer command line");
        assert!(
            !cam1_line.contains("--reconnect-ms"),
            "unset reconnect_ms emits nothing: {cam1_line}"
        );
    }

    /// T7: camera_configs parses input / reconnect_ms (default None — the capturer side falls back to 5000).
    #[test]
    fn camera_config_resolves_input_and_reconnect_ms() {
        let cfg = "sources:\n  - id: \"cam0\"\n    mode: \"camera\"\n    input: \"/dev/v4l/by-id/cam0\"\n    reconnect_ms: 2500\n";
        let cams = camera_configs(cfg).unwrap();
        assert_eq!(cams[0].input.as_deref(), Some("/dev/v4l/by-id/cam0"));
        assert_eq!(cams[0].reconnect_ms, Some(2500));
        let def = "sources:\n  - id: \"cam0\"\n    mode: \"camera\"\n";
        assert_eq!(camera_configs(def).unwrap()[0].reconnect_ms, None);
        assert_eq!(camera_configs(def).unwrap()[0].input, None);
    }

    /// encoder-param passthrough: streams config encoder_backend/bitrate_kbps/keyframe_interval → oxfile command appends;
    /// unset emits nothing (the streamer side falls back to field defaults auto/2000/2).
    #[test]
    fn oxfile_wires_stream_encoder_params() {
        let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n    codec: \"h264\"\n    encoder_backend: \"hardware\"\n    bitrate_kbps: 3500\n    keyframe_interval: 4\n  - id: \"s1\"\n    source: \"cam0\"\n";
        let ox = to_oxfile(cfg).unwrap();
        let s0_line = ox
            .lines()
            .find(|l| l.contains("command") && l.contains("--stream s0"))
            .expect("s0 streamer command line");
        assert!(
            s0_line.contains("--encoder-backend hardware"),
            "encoder_backend should pass through: {s0_line}"
        );
        assert!(s0_line.contains("--bitrate-kbps 3500"), "bitrate should pass through: {s0_line}");
        assert!(s0_line.contains("--keyframe-interval 4"), "gop should pass through: {s0_line}");
        let s1_line = ox
            .lines()
            .find(|l| l.contains("command") && l.contains("--stream s1"))
            .expect("s1 streamer command line");
        // s1 writes no codec → default h264 → the truth table pins software (D282; explicit backends are still passed through)
        assert!(
            s1_line.contains("--encoder-backend software"),
            "default h264 pins software: {s1_line}"
        );
        assert!(!s1_line.contains("--bitrate-kbps"), "unset emits nothing: {s1_line}");
        assert!(!s1_line.contains("--keyframe-interval"), "unset emits nothing: {s1_line}");
    }

    // ── qos-framerate-priority T4: stream_mode bundle merge / flag emission (table-driven) ──

    fn streamer_cmd(ox: &str, stream: &str) -> String {
        ox.lines()
            .find(|l| l.contains("command") && l.contains(&format!("--stream {stream}")))
            .expect("streamer command line")
            .to_string()
    }

    fn qos_cfg(stream_body: &str) -> String {
        format!(
            "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n{stream_body}"
        )
    }

    #[test]
    fn oxfile_expands_smooth_bundle_flags() {
        let ox = to_oxfile(&qos_cfg("    stream_mode: \"smooth\"\n")).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(line.contains("--degradation framerate"), "smooth → framerate: {line}");
        assert!(line.contains("--content-hint fluid"), "smooth → Fluid: {line}");
        // source cam0 has no fps → resolved 30 → floor 100 (D282 linear rule, replaces the fixed 400)
        assert!(line.contains("--min-bitrate-kbps 100"), "smooth → fps-linked floor: {line}");
        assert!(!line.contains("--bitrate-kbps"), "smooth does not move the ceiling: {line}");
    }

    #[test]
    fn oxfile_expands_quality_bundle_flags() {
        let ox = to_oxfile(&qos_cfg("    stream_mode: \"quality\"\n")).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(line.contains("--degradation resolution"), "quality → resolution: {line}");
        assert!(line.contains("--bitrate-kbps 3000"), "quality → bundle 3000 (AD-3): {line}");
        assert!(!line.contains("--content-hint"), "quality has no hint: {line}");
        assert!(!line.contains("--min-bitrate-kbps"), "quality has no floor: {line}");
    }

    #[test]
    fn oxfile_balanced_explicit_emits_no_qos_flags() {
        let ox = to_oxfile(&qos_cfg("    stream_mode: \"balanced\"\n")).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(!line.contains("--degradation"), "balanced emits no degradation: {line}");
        assert!(!line.contains("--content-hint"), "balanced emits no hint: {line}");
        assert!(!line.contains("--min-bitrate-kbps"), "balanced emits no min: {line}");
        assert!(!line.contains("--bitrate-kbps"), "balanced fills no ceiling: {line}");
    }

    #[test]
    fn oxfile_explicit_keys_win_bundle() {
        // explicit bitrate/min override the bundle (smooth writing bitrate is not erased, quality writing bitrate wins over 3000,
        // smooth writing min wins over the fps-linked floor)
        let ox = to_oxfile(&qos_cfg(
            "    stream_mode: \"smooth\"\n    bitrate_kbps: 2500\n    min_bitrate_kbps: 800\n",
        ))
        .unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(line.contains("--bitrate-kbps 2500"), "explicit bitrate wins: {line}");
        assert!(
            line.contains("--min-bitrate-kbps 800"),
            "explicit min wins over floor 100: {line}"
        );
        assert!(
            line.contains("--degradation framerate"),
            "bundle primitives still present: {line}"
        );
        let ox2 =
            to_oxfile(&qos_cfg("    stream_mode: \"quality\"\n    bitrate_kbps: 5000\n")).unwrap();
        assert!(
            streamer_cmd(&ox2, "s0").contains("--bitrate-kbps 5000"),
            "quality explicit wins over bundle 3000"
        );
    }

    #[test]
    fn oxfile_legacy_yaml_emits_no_qos_flags() {
        // a legacy yaml without stream_mode: no --degradation/--content-hint/--*-bitrate-kbps
        // (the balanced zero-perturbation invariant). Existing flags (gateway) are unaffected.
        // Note: since D282 the default codec=h264 pins --encoder-backend software — established backend-gate behavior,
        //     not a QoS perturbation (its explicit-passthrough surface: oxfile_wires_stream_encoder_params).
        let ox = to_oxfile(CFG_V0).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(!line.contains("--degradation"), "status quo unchanged: {line}");
        assert!(!line.contains("--content-hint"), "status quo unchanged: {line}");
        assert!(!line.contains("--min-bitrate-kbps"), "status quo unchanged: {line}");
        assert!(!line.contains("--bitrate-kbps"), "status quo unchanged: {line}");
        assert!(line.contains("--gateway"), "existing flag retained: {line}");
    }

    #[test]
    fn invalid_stream_mode_rejected_at_translate() {
        let e = stream_configs(&qos_cfg("    stream_mode: \"turbo\"\n")).unwrap_err();
        assert!(
            e.contains("smooth|balanced|quality"),
            "the error message contains the legal set: {e}"
        );
        assert!(e.contains("turbo"), "the error message contains the illegal value: {e}");
        // to_oxfile (the deploy main path) blocks in sync
        assert!(to_oxfile(&qos_cfg("    stream_mode: \"turbo\"\n")).is_err());
    }

    // ── host-stream-defaults T1: defaults layered merge (entry > defaults > builtin) ──

    /// All-keys defaults (streams 6 keys + sources platform 5) — the shared-layer input of the priority matrix.
    const ALL_DEFAULTS: &str = r#"
defaults:
  streams:
    codec: "vp9"
    encoder_backend: "vaapi"
    bitrate_kbps: 1500
    stream_mode: "quality"
    min_bitrate_kbps: 300
    keyframe_interval: 5
  sources:
    backend: "v4l2"
    width: 640
    height: 480
    fps: 24
    reconnect_ms: 4000
"#;

    /// All-keys defaults + single source cam0 + single stream s0 (stream_body may be empty = that key comes from defaults/builtin).
    fn defaults_streams(stream_body: &str) -> String {
        format!(
            "{ALL_DEFAULTS}\nsources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n{stream_body}"
        )
    }

    /// Same, with the entry body under the source entry (source_body may be empty).
    fn defaults_sources(source_body: &str) -> String {
        format!(
            "{ALL_DEFAULTS}\nsources:\n  - id: \"cam0\"\n{source_body}streams:\n  - id: \"s0\"\n    source: \"cam0\"\n"
        )
    }

    #[test]
    fn defaults_section_fills_every_key_when_entries_omit_them() {
        let s = &stream_configs(&defaults_streams("")).unwrap()[0];
        assert_eq!(s.codec, "vp9");
        assert_eq!(s.encoder_backend.as_deref(), Some("vaapi"));
        assert_eq!(s.bitrate_kbps, Some(1500));
        assert_eq!(s.stream_mode.as_deref(), Some("quality"));
        assert_eq!(s.min_bitrate_kbps, Some(300));
        assert_eq!(s.keyframe_interval, Some(5));
        let c = &camera_configs(&defaults_sources("")).unwrap()[0];
        assert_eq!(
            c.mode,
            SourceMode::Generator,
            "the mode identity key stays out of defaults — entry default falls to builtin"
        );
        assert_eq!(c.width, 640);
        assert_eq!(c.height, 480);
        assert_eq!(c.fps, 24);
        assert_eq!(c.input, None, "the input identity key stays out of defaults");
        assert_eq!(c.reconnect_ms, Some(4000));
    }

    #[test]
    fn identity_keys_are_denied_in_defaults_sources() {
        // D282 revision: mode/input are source identity — mis-writing them into the shared layer must error at deploy, never be silently ignored.
        for bad in ["mode: \"generator\"", "input: \"/dev/video0\""] {
            let cfg = format!(
                "sources:\n  - id: \"cam0\"\n    mode: camera\ndefaults:\n  sources:\n    {bad}\nstreams: []\n"
            );
            let e = camera_configs(&cfg).unwrap_err();
            assert!(e.contains("unknown field"), "deny should name the unknown field: {e}");
        }
    }

    #[test]
    fn entry_keys_win_over_defaults_for_every_key() {
        let s = &stream_configs(&defaults_streams(
            "    codec: \"av1\"\n    encoder_backend: \"nvenc\"\n    bitrate_kbps: 900\n    stream_mode: \"smooth\"\n    min_bitrate_kbps: 100\n    keyframe_interval: 1\n",
        ))
        .unwrap()[0];
        assert_eq!(s.codec, "av1");
        assert_eq!(s.encoder_backend.as_deref(), Some("nvenc"));
        assert_eq!(s.bitrate_kbps, Some(900));
        assert_eq!(s.stream_mode.as_deref(), Some("smooth"));
        assert_eq!(s.min_bitrate_kbps, Some(100));
        assert_eq!(s.keyframe_interval, Some(1));
        let c = &camera_configs(&defaults_sources(
            "    mode: \"subscriber\"\n    backend: \"mipi\"\n    input: \"ros:///img\"\n    width: 320\n    height: 240\n    fps: 15\n    reconnect_ms: 1000\n",
        ))
        .unwrap()[0];
        assert_eq!(c.mode, SourceMode::Subscriber);
        assert_eq!(c.width, 320);
        assert_eq!(c.height, 240);
        assert_eq!(c.fps, 15);
        assert_eq!(c.input.as_deref(), Some("ros:///img"));
        assert_eq!(c.reconnect_ms, Some(1000));
    }

    #[test]
    fn merge_is_per_key_not_per_section() {
        // an entry writing only one key → that key takes the entry value, the rest still take defaults
        // (whole-section replacement = the most common layered-config implementation bug; this pins per-key merge)
        let s = &stream_configs(&defaults_streams("    codec: \"vp8\"\n")).unwrap()[0];
        assert_eq!(s.codec, "vp8", "the entry key applies");
        assert_eq!(
            s.encoder_backend.as_deref(),
            Some("vaapi"),
            "the remaining keys fall back to defaults"
        );
        assert_eq!(s.bitrate_kbps, Some(1500));
        assert_eq!(s.stream_mode.as_deref(), Some("quality"));
        assert_eq!(s.min_bitrate_kbps, Some(300));
        assert_eq!(s.keyframe_interval, Some(5));
        let c = &camera_configs(&defaults_sources("    fps: 60\n")).unwrap()[0];
        assert_eq!(c.fps, 60, "the entry key applies");
        assert_eq!(c.width, 640, "the remaining keys fall back to defaults");
        assert_eq!(c.mode, SourceMode::Generator);
        assert_eq!(c.reconnect_ms, Some(4000));
    }

    #[test]
    fn defaults_section_absent_equals_builtin_resolution() {
        // missing section = zero injection: resolved values are byte-identical to the builtin defaults (extension of the balanced zero-perturbation invariant)
        let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        assert_eq!(
            camera_configs(cfg).unwrap(),
            vec![SourceConfig {
                id: "cam0".into(),
                mode: SourceMode::Generator,
                width: DEFAULT_SOURCE_WIDTH,
                height: DEFAULT_SOURCE_HEIGHT,
                fps: DEFAULT_SOURCE_FPS,
                input: None,
                reconnect_ms: None,
            }],
            "an absent defaults section must equal the builtin per-field defaults"
        );
        assert_eq!(
            stream_configs(cfg).unwrap(),
            vec![StreamConfig {
                id: "s0".into(),
                source: "cam0".into(),
                codec: "h264".into(),
                encoder_backend: None,
                bitrate_kbps: None,
                stream_mode: None,
                min_bitrate_kbps: None,
                keyframe_interval: None,
            }],
            "an absent defaults section must equal the builtin per-field defaults"
        );
        // an empty defaults section (`defaults: {}`) renders byte-identical to an absent one
        assert_eq!(
            to_oxfile(cfg).unwrap(),
            to_oxfile(&format!("{cfg}\ndefaults: {{}}\n")).unwrap(),
            "an empty defaults section must not change oxfile rendering"
        );
    }

    #[test]
    fn illegal_mode_and_codec_rejected_from_every_layer() {
        // 1) entry layer
        assert!(stream_configs(&qos_cfg("    stream_mode: \"turbo\"\n")).is_err());
        let e = stream_configs(&qos_cfg("    codec: \"h265\"\n")).unwrap_err();
        assert!(
            e.contains("vp8|h264|vp9|av1") && e.contains("h265"),
            "the error message contains the legal set: {e}"
        );
        // 2) defaults layer (checked after the merge → bad values never silently reach the streamer)
        let d_mode = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\ndefaults:\n  streams:\n    stream_mode: \"turbo\"\n";
        let e = stream_configs(d_mode).unwrap_err();
        assert!(e.contains("smooth|balanced|quality") && e.contains("turbo"), "{e}");
        let d_codec = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\ndefaults:\n  streams:\n    codec: \"H264\"\n";
        assert!(
            stream_configs(d_codec).unwrap_err().contains("codec=H264"),
            "case sensitivity = exact legal-set match"
        );
        // 3) builtin layer (no entry key, no defaults → default values are inherently legal)
        assert!(stream_configs(&qos_cfg("")).is_ok());
        // an entry legal value overriding a defaults illegal value → proves the gate runs after the merge
        let ok = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    stream_mode: \"smooth\"\ndefaults:\n  streams:\n    stream_mode: \"turbo\"\n";
        assert_eq!(stream_configs(ok).unwrap()[0].stream_mode.as_deref(), Some("smooth"));
        // the deploy main path blocks in sync
        assert!(to_oxfile(d_mode).is_err());
        assert!(validate(d_codec).is_err());
    }

    // ── host-stream-defaults T2: smooth fps floor + h264 default + auto×h264 decision ──

    #[test]
    fn smooth_floor_is_linear_in_fps_with_50_kbps_minimum() {
        // constant bytes per frame → linear in fps; user-ruled anchors 15→50 / 30→100 (0 floors at 50)
        for (fps, expect) in [(0u32, 50u32), (15, 50), (24, 80), (30, 100), (60, 200)] {
            assert_eq!(smooth_bitrate_floor_kbps(fps), expect, "fps={fps}");
        }
    }

    #[test]
    fn smooth_floor_uses_linked_source_fps() {
        let cfg = |fps: u32| {
            format!(
                "sources:\n  - id: \"cam0\"\n    fps: {fps}\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n    stream_mode: \"smooth\"\n"
            )
        };
        let floor_of = |fps: u32| streamer_cmd(&to_oxfile(&cfg(fps)).unwrap(), "s0");
        assert!(
            floor_of(60).contains("--min-bitrate-kbps 200"),
            "source fps=60 → 200: {}",
            floor_of(60)
        );
        assert!(
            floor_of(15).contains("--min-bitrate-kbps 50"),
            "source fps=15 → 50: {}",
            floor_of(15)
        );
        // the source writes no fps (resolved 30) → 100
        assert!(
            streamer_cmd(&to_oxfile(&qos_cfg("    stream_mode: \"smooth\"\n")).unwrap(), "s0")
                .contains("--min-bitrate-kbps 100"),
            "source default fps → 100"
        );
        // the stream references a nonexistent source → the 30 rule (the existing error path adds no new semantics)
        let orphan = "streams:\n  - id: \"s0\"\n    stream_mode: \"smooth\"\n";
        assert!(
            streamer_cmd(&to_oxfile(orphan).unwrap(), "s0").contains("--min-bitrate-kbps 100"),
            "a missing source is counted as 30"
        );
        // defaults.sources.fps participates in the floor (the merge precedes the linking)
        let via_defaults = "defaults:\n  sources:\n    fps: 60\n  streams:\n    stream_mode: \"smooth\"\nsources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        assert!(
            streamer_cmd(&to_oxfile(via_defaults).unwrap(), "s0")
                .contains("--min-bitrate-kbps 200"),
            "defaults.sources.fps enters the floor"
        );
    }

    #[test]
    fn h264_backend_truth_table_auto_pinned_software() {
        let cell = |backend: &str, codec: &str| {
            to_oxfile(&qos_cfg(&format!(
                "    codec: \"{codec}\"\n    encoder_backend: \"{backend}\"\n"
            )))
        };
        // 1) h264 × auto → pin software (PIT-156 root fix)
        assert!(
            streamer_cmd(&cell("auto", "h264").unwrap(), "s0")
                .contains("--encoder-backend software"),
            "h264×auto → software"
        );
        // 2) h264 × explicit backend → passed through as-is (incl. knowingly trying hardware even though it is unavailable)
        for b in ["software", "hardware", "nvenc", "vaapi"] {
            let line = streamer_cmd(&cell(b, "h264").unwrap(), "s0");
            assert!(line.contains(&format!("--encoder-backend {b}")), "h264×{b} as-is: {line}");
        }
        // 3) other codecs × auto → as-is (no interference with the streamer auto status quo)
        assert!(
            streamer_cmd(&cell("auto", "vp8").unwrap(), "s0").contains("--encoder-backend auto"),
            "vp8×auto as-is"
        );
        // 4) other codecs × unset → emit nothing
        assert!(
            !streamer_cmd(&to_oxfile(&qos_cfg("    codec: \"vp8\"\n")).unwrap(), "s0")
                .contains("--encoder-backend"),
            "vp8 unset emits nothing"
        );
        // pure-function side: four cells
        assert_eq!(resolved_encoder_backend("h264", None), Some("software"));
        assert_eq!(resolved_encoder_backend("h264", Some("auto")), Some("software"));
        assert_eq!(resolved_encoder_backend("vp8", None), None);
        assert_eq!(resolved_encoder_backend("vp8", Some("auto")), Some("auto"));
        assert_eq!(resolved_encoder_backend("h264", Some("hardware")), Some("hardware"));
    }

    #[test]
    fn codec_default_h264_flows_into_backend_pin() {
        // all three layers write no codec → builtin h264 (D282 revision) → incidentally pinned software by the truth table
        assert_eq!(stream_configs(&qos_cfg("")).unwrap()[0].codec, "h264");
        let line = streamer_cmd(&to_oxfile(&qos_cfg("")).unwrap(), "s0");
        assert!(
            line.contains("--encoder-backend software"),
            "default h264 goes through the truth table: {line}"
        );
        // a codec written at the defaults layer also enters the truth table (resolved rule)
        let via_defaults = "defaults:\n  streams:\n    codec: \"vp9\"\nsources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        assert_eq!(stream_configs(via_defaults).unwrap()[0].codec, "vp9");
        assert!(
            !streamer_cmd(&to_oxfile(via_defaults).unwrap(), "s0").contains("--encoder-backend"),
            "vp9 left alone"
        );
    }

    #[test]
    fn defaults_rejects_unknown_and_identity_keys() {
        // the shared layer carries deny_unknown_fields: typos / identity keys error by name at deploy time
        let typo = "sources:\n  - id: \"cam0\"\ndefaults:\n  streams:\n    bitrate_kbps: 1000\n    bitrates: 1000\n";
        let e = camera_configs(typo).unwrap_err();
        assert!(e.contains("bitrates"), "an unknown key must be named: {e}");
        let ident = "sources:\n  - id: \"cam0\"\ndefaults:\n  streams:\n    id: \"x\"\n";
        assert!(
            camera_configs(ident).unwrap_err().contains("id"),
            "id stays out of the shared layer"
        );
        let legacy = "defaults:\n  sources:\n    source: \"stub\"\n";
        assert!(
            camera_configs(legacy).unwrap_err().contains("source"),
            "the legacy compat key stays out of the shared layer"
        );
        // the top level does not deny: existing yaml keys outside HostConfig (host:/control: etc.) must still parse
        assert!(
            camera_configs("host:\n  device_id: \"x\"\ncontrol:\n  enabled: false\nsources: []\n")
                .is_ok()
        );
    }
    // ── V5 [control].hmac_key_file render injection ──

    const CFG_CONTROL: &str = r#"
sources:
  - id: "cam0"
    mode: "generator"
streams:
  - id: "s0"
    source: "cam0"
control:
  hmac_key_file: "etc/control.hmac.key"
"#;

    #[test]
    fn control_key_file_injected_into_controller_app() {
        let ox = to_oxfile_in_dir(CFG_CONTROL, Path::new("/opt/h")).unwrap();
        // the relative path is absolutized against the instance root; only the controller app carries the arg
        assert!(ox.contains("--hmac-key-file /opt/h/etc/control.hmac.key"), "{ox}");
        assert_eq!(ox.matches("--hmac-key-file").count(), 1);
    }

    #[test]
    fn control_absent_means_byte_identical_oxfile() {
        // absent-section zero perturbation (same provable shape as stream_mode balanced): the old config renders without the new arg
        let ox = to_oxfile_in_dir(CFG_V0, Path::new("/opt/h")).unwrap();
        assert!(!ox.contains("--hmac-key-file"));
        assert!(!ox.contains("hmac"));
    }

    #[test]
    fn control_key_file_missing_rejects_at_render() {
        // deploy-time guard: config pointing at a missing file = write_oxfile Err (earlier than the controller process launch)
        let e = write_oxfile(CFG_CONTROL, Path::new("/nonexistent-v5-guard-dir"))
            .expect_err("a missing key file must reject rendering");
        assert!(e.contains("hmac_key_file points to a missing file"), "{e}");
    }

    /// T1-e render-gate tri-state: absent / true keep the recorder in the roster (today's
    /// byte shape); explicit false removes the app entirely.
    #[test]
    fn recorder_roster_tristate_render() {
        let recorder = app_name("recorder");
        assert!(
            to_oxfile(CFG_V0).unwrap().contains(&recorder),
            "record key omitted must render the recorder (legacy shape)"
        );
        let on = format!("{CFG_V0}\nrecord:\n  enabled: true\n");
        assert!(
            to_oxfile(&on).unwrap().contains(&recorder),
            "enabled: true must render the recorder"
        );
        let off = format!("{CFG_V0}\nrecord:\n  enabled: false\n");
        assert!(
            !to_oxfile(&off).unwrap().contains(&recorder),
            "enabled: false must drop the recorder from the managed roster entirely"
        );
    }

    /// T1-e roster↔expected-set alignment pin: the two gate sites must agree —
    /// false = out of both; absent = in roster + out of expected (today's shape, the
    /// split that keeps the parked recorder from false-alarming ProcessMissing).
    #[test]
    fn recorder_roster_expected_set_alignment() {
        let recorder = app_name("recorder");
        let off = format!("{CFG_V0}\nrecord:\n  enabled: false\n");
        assert!(!to_oxfile(&off).unwrap().contains(&recorder));
        assert!(!expected_process_names(&off).unwrap().contains(&recorder));
        let on = format!("{CFG_V0}\nrecord:\n  enabled: true\n");
        assert!(to_oxfile(&on).unwrap().contains(&recorder));
        assert!(expected_process_names(&on).unwrap().contains(&recorder));
        // absent: render gate does not trip (in roster), program gate flattens to false (out of expected)
        assert!(to_oxfile(CFG_V0).unwrap().contains(&recorder));
        assert!(!expected_process_names(CFG_V0).unwrap().contains(&recorder));
    }

    /// record_enabled_raw keeps the tri-state that record_config() flattens away.
    #[test]
    fn record_enabled_raw_keeps_tristate() {
        assert_eq!(record_enabled_raw(CFG_V0).unwrap(), None);
        assert_eq!(
            record_enabled_raw(&format!("{CFG_V0}\nrecord:\n  enabled: true\n")).unwrap(),
            Some(true)
        );
        assert_eq!(
            record_enabled_raw(&format!("{CFG_V0}\nrecord:\n  enabled: false\n")).unwrap(),
            Some(false)
        );
    }
}
