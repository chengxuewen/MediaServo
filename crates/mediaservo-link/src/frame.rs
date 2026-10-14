//! Frame metadata + frame reference + frame stream (latest-slot).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// Frame metadata (fixed-length LE encoding, D243).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameMeta {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    /// Pixel format (0=unknown, 1=I420, 2=NV12, 3=RGBA, 4=JSON payload - not pixels,
    /// e.g. E2 streamer push-status on stats/* topics).
    pub format: u8,
    /// Metadata version (for evolution, D243). encode always writes [`Self::WIRE_VERSION`];
    /// decode **rejects frames** of other versions (N4: silently mis-parsing 36 bytes is worse than dropping).
    pub version: u8,
    pub is_keyframe: bool,
    pub ts_mono_ns: u64,
    pub ts_epoch_ns: u64,
}

impl FrameMeta {
    /// JSON payload format marker (E2 stats topic wire format; not pixel data).
    pub const FORMAT_JSON: u8 = 4;

    /// Current wire-format version. 0 = the historical value (all deployed bytes are v0) - **zero breakage on rollout**;
    /// future evolution bumps this constant and adds a compat window in decode.
    pub const WIRE_VERSION: u8 = 0;

    /// Fixed encoded byte count:
    /// seq(8) + width(4) + height(4) + format(1) + version(1) + keyframe(1) + reserved(1)
    /// + ts_mono_ns(8) + ts_epoch_ns(8) = 36.
    pub const WIRE_LEN: usize = 36;

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut b = [0u8; Self::WIRE_LEN];
        b[0..8].copy_from_slice(&self.seq.to_le_bytes());
        b[8..12].copy_from_slice(&self.width.to_le_bytes());
        b[12..16].copy_from_slice(&self.height.to_le_bytes());
        b[16] = self.format;
        b[17] = self.version;
        b[18] = u8::from(self.is_keyframe);
        b[19] = 0; // reserved
        b[20..28].copy_from_slice(&self.ts_mono_ns.to_le_bytes());
        b[28..36].copy_from_slice(&self.ts_epoch_ns.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self, crate::LinkError> {
        if b.len() < Self::WIRE_LEN {
            return Err(crate::LinkError::Bus(format!(
                "frame meta too short: {} < {}",
                b.len(),
                Self::WIRE_LEN
            )));
        }
        // N4 version gate: reject unknown versions (all fields misaligned after parse = silent corruption, worse than a drop).
        if b[17] != Self::WIRE_VERSION {
            return Err(crate::LinkError::Bus(format!(
                "frame meta wire version {} unsupported (expected {})",
                b[17],
                Self::WIRE_VERSION
            )));
        }
        Ok(Self {
            seq: u64::from_le_bytes(b[0..8].try_into().expect("8 bytes")),
            width: u32::from_le_bytes(b[8..12].try_into().expect("4 bytes")),
            height: u32::from_le_bytes(b[12..16].try_into().expect("4 bytes")),
            format: b[16],
            version: b[17],
            is_keyframe: b[18] != 0,
            ts_mono_ns: u64::from_le_bytes(b[20..28].try_into().expect("8 bytes")),
            ts_epoch_ns: u64::from_le_bytes(b[28..36].try_into().expect("8 bytes")),
        })
    }
}

/// Frame reference (metadata + payload). Phase 2 may evolve into an iceoryx2 SHM zero-copy view.
pub struct FrameRef {
    meta: FrameMeta,
    payload: Vec<u8>,
}

impl FrameRef {
    pub fn new(meta: FrameMeta, payload: Vec<u8>) -> Self {
        Self { meta, payload }
    }
    pub fn meta(&self) -> &FrameMeta {
        &self.meta
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Frame-stream inner state (background thread delivers via [`Weak`]; auto-stops when the stream is dropped).
pub(crate) struct StreamInner {
    slot: Mutex<Option<FrameRef>>,
    notify: tokio::sync::Notify,
    shutdown: AtomicBool,
}

impl StreamInner {
    /// Deliver the latest frame (replaces the old one).
    pub(crate) fn deliver(&self, frame: FrameRef) {
        *self.slot.lock().expect("frame slot lock") = Some(frame);
        self.notify.notify_waiters();
    }

    /// Shutdown (wakes blocked recv calls to return None).
    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
}

/// Frame stream (latest-slot: slow consumers skip to the newest frame, review H5).
///
/// The FrameBus background thread replaces the slot and notifies via [`StreamInner::deliver`] on every frame;
/// consumers take the newest frame after [`Self::recv`] wakes. **Unbounded queues are forbidden** (they reintroduce backlog).
#[derive(Clone)]
pub struct FrameStream {
    inner: Arc<StreamInner>,
}

impl std::fmt::Debug for FrameStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameStream").finish()
    }
}

impl FrameStream {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(StreamInner {
                slot: Mutex::new(None),
                notify: tokio::sync::Notify::new(),
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// Weak handle for the background thread (thread auto-exits when the stream is dropped).
    pub(crate) fn weak_inner(&self) -> Weak<StreamInner> {
        Arc::downgrade(&self.inner)
    }

    /// Inner shared state (FrameBus uses it to shutdown on close).
    pub(crate) fn inner(&self) -> Arc<StreamInner> {
        Arc::clone(&self.inner)
    }

    /// Take the latest frame, waiting when none. Slow consumers auto-skip to the newest (no backlog); returns None after shutdown.
    pub async fn recv(&self) -> Option<FrameRef> {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(f) = self.inner.slot.lock().expect("frame slot lock").take() {
                return Some(f);
            }
            if self.inner.shutdown.load(Ordering::SeqCst) {
                return None; // shut down with no frame
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u64) -> FrameRef {
        FrameRef::new(FrameMeta { seq, ..Default::default() }, vec![seq as u8])
    }

    #[tokio::test]
    async fn latest_slot_skips_to_newest() {
        let s = FrameStream::new();
        s.inner.deliver(frame(1));
        s.inner.deliver(frame(2));
        s.inner.deliver(frame(3));
        let f = s.recv().await.expect("frame");
        assert_eq!(f.meta().seq, 3, "slow consumer should skip to the newest frame");
    }

    #[tokio::test]
    async fn recv_waits_for_delivery() {
        let s = Arc::new(FrameStream::new());
        let s2 = Arc::clone(&s);
        let h = tokio::spawn(async move {
            let f = s2.recv().await.expect("frame");
            f.meta().seq
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        s.inner.deliver(frame(42));
        assert_eq!(h.await.expect("join"), 42);
    }

    #[tokio::test]
    async fn recv_returns_none_after_shutdown() {
        let s = FrameStream::new();
        s.inner().shutdown();
        assert!(s.recv().await.is_none(), "recv should return None after shutdown");
    }

    #[test]
    fn decode_rejects_unknown_wire_version() {
        // N4: nonzero version bit = reject and report the version (36 misaligned bytes are worse than a drop)
        let mut b = FrameMeta { seq: 7, width: 4, ..Default::default() }.encode();
        assert_eq!(FrameMeta::decode(&b).unwrap().version, FrameMeta::WIRE_VERSION);
        b[17] = 9;
        let e = FrameMeta::decode(&b).expect_err("v9 must be rejected");
        assert!(e.to_string().contains("version 9"), "{e}");
    }
}
