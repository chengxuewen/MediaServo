use mediaservo_common::config::ServerConfig;
use mediaservo_common::error::CoreError;
use std::path::Path;

/// Load server configuration from a YAML file into the shared ServerConfig type.
pub fn load(path: impl AsRef<Path>) -> Result<ServerConfig, CoreError> {
    let file = std::fs::File::open(path.as_ref())
        .map_err(|e| CoreError::ConfigParse(format!("Cannot open {}: {}", path.as_ref().display(), e)))?;
    serde_yaml::from_reader(file)
        .map_err(|e| CoreError::ConfigParse(format!("YAML parse error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let yaml = r#"
version: 1
listen:
  host: "0.0.0.0"
  port: 9800
"#;
        let config: ServerConfig = serde_yaml::from_str(yaml).expect("valid yaml");
        assert_eq!(config.version, 1);
        assert_eq!(config.listen.port, 9800);
        assert_eq!(config.room_capacity, 10);
    }

    #[test]
    fn parse_sfu_announced_ips() {
        let yaml = r#"
version: 1
listen:
  host: "0.0.0.0"
  port: 9800
sfu:
  announced_ips:
    - "192.168.2.127"
    - "10.144.0.3"
"#;
        let config: mediaservo_common::config::ServerConfig =
            serde_yaml::from_str(yaml).expect("valid yaml");
        assert_eq!(
            config.sfu.announced_ips,
            vec!["192.168.2.127".to_string(), "10.144.0.3".to_string()]
        );
        // 缺省空（env/探测兜底）
        let minimal: mediaservo_common::config::ServerConfig =
            serde_yaml::from_str("version: 1\nlisten:\n  host: 0.0.0.0\n  port: 9800\n")
                .unwrap();
        assert!(minimal.sfu.announced_ips.is_empty());
    }

    #[test]
    fn parse_full_config() {
        let yaml = r#"
version: 1
listen:
  host: "127.0.0.1"
  port: 9800
room_capacity: 50
psk: "secret-key"
rate_limit: 200
"#;
        let config: ServerConfig = serde_yaml::from_str(yaml).expect("valid yaml");
        assert_eq!(config.listen.host, "127.0.0.1");
        assert_eq!(config.room_capacity, 50);
        assert_eq!(config.psk, Some("secret-key".into()));
        assert_eq!(config.rate_limit, 200);
    }
}
