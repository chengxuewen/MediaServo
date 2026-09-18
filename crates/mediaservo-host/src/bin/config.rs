//! Host configuration loader — delegates to mediaservo_common::config::HostConfig.
//!
//! Reads host.conf YAML and deserializes into the shared config schema.
//! Backward-compatible with existing callers via `config::load(&config_path)`.

use mediaservo_common::config::HostConfig;
use mediaservo_common::error::CoreError;
use std::fs;
use std::path::Path;

/// Load host configuration from a YAML file path.
pub fn load<P: AsRef<Path>>(path: P) -> Result<HostConfig, CoreError> {
    let f = fs::File::open(path)
        .map_err(|e| CoreError::ConfigParse(format!("cannot open config: {e}")))?;
    serde_yaml::from_reader(f).map_err(|e| CoreError::ConfigParse(format!("YAML parse error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let yaml = r#"
version: 1
server:
  signaling_url: "ws://localhost:9800/ws"
  ice_servers: []
capture:
  source: "camera"
  resolution: "1280x720"
  framerate: 30
  device: "/dev/video0"
encoder:
  backend: "auto"
  bitrate_kbps: 2000
  keyframe_interval: 2
psk: "mediaservo-dev"
"#;
        let config: HostConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.version, 1);
        assert_eq!(config.server.signaling_url, "ws://localhost:9800/ws");
        assert_eq!(config.capture.source, "camera");
        assert_eq!(config.encoder.backend, "auto");
    }

    #[test]
    fn load_file_uses_coreerror() {
        let result = load("/nonexistent/path/host.conf");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("cannot open config"));
    }
}
