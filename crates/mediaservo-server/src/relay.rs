use mediaservo_common::error::CoreError;

#[cfg(feature = "webrtc")]
mod imp {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use mediaservo_common::error::CoreError;
    use mediaservo_webrtc::{RTCPeerConnection, RTCPeerConnectionFactory, RTCConfiguration};

    struct RelaySession {
        host_peer: Option<RTCPeerConnection>,
        remote_peers: Vec<RTCPeerConnection>,
    }

    pub struct Relay {
        factory: RTCPeerConnectionFactory,
        sessions: Arc<Mutex<HashMap<String, RelaySession>>>,
    }
    impl Relay {
        pub fn new() -> Self {
            let factory = RTCPeerConnectionFactory::new();
            tracing::info!("WebRTC relay initialized (mediaservo-webrtc / libwebrtc)");
            Relay {
                factory,
                sessions: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        pub fn register_host(
            &self,
            room_id: &str,
        ) -> Result<RTCPeerConnection, CoreError> {
            let config = RTCConfiguration::default();
            let pc = self
                .factory
                .create_peer_connection(config)
                .map_err(|e| CoreError::PeerConnectionFailure(e.to_string()))?;
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .entry(room_id.to_string())
                .or_insert_with(|| RelaySession {
                    host_peer: None,
                    remote_peers: Vec::new(),
                });
            session.host_peer = Some(pc.clone());
            tracing::info!("Host registered in room {}", room_id);
            Ok(pc)
        }

        pub fn register_remote(
            &self,
            room_id: &str,
        ) -> Result<RTCPeerConnection, CoreError> {
            let config = RTCConfiguration::default();
            let pc = self
                .factory
                .create_peer_connection(config)
                .map_err(|e| CoreError::PeerConnectionFailure(e.to_string()))?;
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .entry(room_id.to_string())
                .or_insert_with(|| RelaySession {
                    host_peer: None,
                    remote_peers: Vec::new(),
                });
            session.remote_peers.push(pc.clone());
            tracing::info!(
                "Remote registered in room {} (total: {})",
                room_id,
                session.remote_peers.len()
            );
            Ok(pc)
        }

        /// Bridge tracks: bind Host video track to Remote PC.
        pub fn bridge_tracks(
            &self,
            _host_pc: &RTCPeerConnection,
            _remote_pc: &RTCPeerConnection,
        ) -> Result<(), CoreError> {
            tracing::info!("WebRTC track bridging (ponytail: stub — needs live track wiring)");
            Ok(())
        }
    }
}

#[cfg(not(feature = "webrtc"))]
mod imp {
    use super::CoreError;

    pub struct Relay;

    impl Relay {
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            tracing::info!("WebRTC relay initialized (stub — signaling-only mode)");
            Relay
        }

        /// ponytail: stub bridge_tracks; requires webrtc feature for real RTCPeerConnection access.
        /// Roadmap: integrate VideoFrameGenerator as test video source (see docs/plans/gen-webrtc-integration/design.md)
        pub async fn bridge_tracks(
            &self,
            _host_pc: &(),
            _remote_pc: &(),
        ) -> Result<(), CoreError> {
            tracing::debug!("bridge_tracks stub — enable webrtc feature for real relay");
            Ok(())
        }
    }
}

pub use imp::Relay;
