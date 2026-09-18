//! UDP emergency listener on port 9999.
//!
//! Receives "STOP\n" to trigger emergency shutdown. Used for out-of-band
//! control when the normal signaling channel is unavailable.

#![allow(dead_code)] // 双身份文件：host-legacy bin 的 `mod` 形态在用（T1.3 翻案），独立 bin 编译形态全 dead = 结构噪音
use mediaservo_common::error::CoreError;
use tokio::net::UdpSocket;

/// Listens for emergency stop commands on a UDP socket.
pub struct EmergencyListener {
    socket: UdpSocket,
}

impl EmergencyListener {
    /// Bind to the given UDP port.
    pub async fn bind(port: u16) -> Result<Self, CoreError> {
        let addr = format!("0.0.0.0:{port}");
        let socket = UdpSocket::bind(&addr)
            .await
            .map_err(|e| CoreError::Unknown(format!("UDP bind {addr}: {e}")))?;
        tracing::info!("Emergency listener bound on UDP {addr}");
        Ok(EmergencyListener { socket })
    }

    /// Listen for "STOP\n" — returns when shutdown is requested.
    pub async fn listen(&self) -> Result<(), CoreError> {
        let mut buf = [0u8; 16];
        loop {
            let (len, src) = self
                .socket
                .recv_from(&mut buf)
                .await
                .map_err(|e| CoreError::Unknown(format!("UDP recv: {e}")))?;
            let msg = std::str::from_utf8(&buf[..len]).unwrap_or("");
            if msg.trim() == "STOP" {
                tracing::info!("Emergency STOP received from {src}");
                return Ok(());
            }
            tracing::debug!("Emergency listener received {len}B from {src}: {msg}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emergency_listener_binds_port_zero() {
        let listener = EmergencyListener::bind(0).await.unwrap();
        let addr = listener.socket.local_addr().unwrap();
        assert!(addr.port() > 0);
    }
}
