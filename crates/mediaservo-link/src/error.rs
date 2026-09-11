//! link IPC 错误类型。

/// link IPC 错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LinkError {
    /// attach 失败（节点创建 / 验签 / 注册）。
    #[error("attach failed: {0}")]
    Attach(String),

    /// ACL 拒绝（越权 publish/subscribe）。
    #[error("acl denied: {topic}")]
    AclDenied { topic: String },

    /// 单发布者冲突（D239）：topic 已有发布者。
    #[error("topic already has a publisher: {topic}")]
    TopicConflict { topic: String },

    /// 能力令牌无效（验签失败 / 过期 / 格式错）。
    #[error("token invalid: {0}")]
    Token(String),

    /// Registry 操作失败。
    #[error("registry error: {0}")]
    Registry(String),

    /// 帧总线操作失败（loan/send/subscribe 等）。
    #[error("bus error: {0}")]
    Bus(String),

    /// 信令错误（WS 连接/认证/收发）。
    #[error("signal error: {0}")]
    Signal(String),

    /// 设备已入册待管理员批准（device-enroll 手动档，`DeviceAuthPending` 应答）。
    #[error("device enroll pending: {device_id}")]
    EnrollPending { device_id: String },

    /// 已关闭。
    #[error("closed")]
    Closed,
}
