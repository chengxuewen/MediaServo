//! link IPC error types.

/// link IPC error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LinkError {
    /// attach failed (node creation / signature verify / registration).
    #[error("attach failed: {0}")]
    Attach(String),

    /// ACL denied (unauthorized publish/subscribe).
    #[error("acl denied: {topic}")]
    AclDenied { topic: String },

    /// single-publisher conflict (D239): topic already has a publisher.
    #[error("topic already has a publisher: {topic}")]
    TopicConflict { topic: String },

    /// capability token invalid (verify failed / expired / malformed).
    #[error("token invalid: {0}")]
    Token(String),

    /// Registry operation failed.
    #[error("registry error: {0}")]
    Registry(String),

    /// frame bus operation failed (loan/send/subscribe etc.).
    #[error("bus error: {0}")]
    Bus(String),

    /// signaling error (WS connect/auth/send-recv).
    #[error("signal error: {0}")]
    Signal(String),

    /// device enrolled, awaiting admin approval (device-enroll manual mode, `DeviceAuthPending` reply).
    #[error("device enroll pending: {device_id}")]
    EnrollPending { device_id: String },

    /// Closed.
    #[error("closed")]
    Closed,
}
