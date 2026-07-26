//! Runtime deployment profiles. Desktop defaults deliberately match the
//! original CLI; appliance settings are only enabled explicitly.

use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Profile {
    Desktop,
    Appliance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub server: ServerConfig,
    pub ecu: EcuConfig,
    pub logging: LoggingConfig,
    pub engine_shutdown: EngineShutdownConfig,
    pub authentication: AuthenticationConfig,
    pub captive_portal: CaptivePortalConfig,
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: IpAddr,
    pub port: u16,
    pub open_browser: bool,
    pub admin_socket: Option<PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EcuConfig {
    pub device: String,
    pub mode: String,
    pub baud: u32,
    pub poll_ms: u64,
    pub auto_connect: bool,
    pub ini: Option<PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub directory: PathBuf,
    pub auto: bool,
    pub retention_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineShutdownConfig {
    pub enabled: bool,
    pub arm_rpm: f64,
    pub stop_rpm: f64,
    pub delay_seconds: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthenticationConfig {
    pub required: bool,
    pub state_directory: PathBuf,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptivePortalConfig {
    pub enabled: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::desktop()
    }
}
impl RuntimeConfig {
    pub fn desktop() -> Self {
        Self {
            server: ServerConfig::default(),
            ecu: EcuConfig::default(),
            logging: LoggingConfig::default(),
            engine_shutdown: EngineShutdownConfig::default(),
            authentication: AuthenticationConfig::default(),
            captive_portal: CaptivePortalConfig::default(),
            source_path: None,
        }
    }
    pub fn appliance() -> Self {
        Self {
            server: ServerConfig {
                bind: "0.0.0.0".parse().unwrap(),
                port: 80,
                open_browser: false,
                admin_socket: Some("/run/rustytune/admin.sock".into()),
            },
            ecu: EcuConfig {
                device: "/dev/serial0".into(),
                auto_connect: true,
                ..EcuConfig::default()
            },
            logging: LoggingConfig {
                directory: "/var/log/speeduino".into(),
                auto: true,
                retention_bytes: 2 * 1024 * 1024 * 1024,
            },
            engine_shutdown: EngineShutdownConfig::default(),
            authentication: AuthenticationConfig {
                required: true,
                state_directory: "/var/lib/rustytune".into(),
            },
            captive_portal: CaptivePortalConfig { enabled: true },
            source_path: None,
        }
    }
    pub fn load(path: &Path, base: Self) -> Result<Self, String> {
        if !path.exists() {
            let mut config = base;
            config.source_path = Some(path.to_owned());
            return Ok(config);
        }
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        // TOML's missing fields use the type defaults, so merge at the value
        // level to retain profile-specific defaults.
        let mut root = toml::Value::try_from(base).map_err(|e| e.to_string())?;
        let overlay: toml::Value =
            toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
        merge(&mut root, overlay);
        let mut config: Self = root.try_into().map_err(|e| e.to_string())?;
        config.source_path = Some(path.to_owned());
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        let cfg = &self.engine_shutdown;
        if !cfg.stop_rpm.is_finite() || cfg.stop_rpm < 0.0 {
            return Err("engine_shutdown.stop_rpm must be a finite non-negative number".into());
        }
        if !cfg.arm_rpm.is_finite() || cfg.arm_rpm <= cfg.stop_rpm {
            return Err("engine_shutdown.arm_rpm must be greater than stop_rpm".into());
        }
        if !(1..=600).contains(&cfg.delay_seconds) {
            return Err("engine_shutdown.delay_seconds must be between 1 and 600".into());
        }
        Ok(())
    }
}
fn merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(a), toml::Value::Table(b)) => {
            for (k, v) in b {
                if let Some(old) = a.get_mut(&k) {
                    merge(old, v)
                } else {
                    a.insert(k, v);
                }
            }
        }
        (a, b) => *a = b,
    }
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".parse().unwrap(),
            port: 8642,
            open_browser: true,
            admin_socket: None,
        }
    }
}
impl Default for EcuConfig {
    fn default() -> Self {
        Self {
            device: String::new(),
            mode: "primary".into(),
            baud: 115_200,
            poll_ms: 50,
            auto_connect: false,
            ini: None,
        }
    }
}
impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            directory: "logs".into(),
            auto: false,
            retention_bytes: 0,
        }
    }
}
impl Default for EngineShutdownConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            arm_rpm: 500.0,
            stop_rpm: 50.0,
            delay_seconds: 15,
        }
    }
}
impl Default for AuthenticationConfig {
    fn default() -> Self {
        Self {
            required: false,
            state_directory: ".".into(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_are_safe_and_distinct() {
        let d = RuntimeConfig::desktop();
        assert!(d.server.bind.is_loopback());
        assert!(!d.authentication.required);
        assert!(!d.ecu.auto_connect);
        let a = RuntimeConfig::appliance();
        assert!(a.authentication.required);
        assert!(a.ecu.auto_connect);
        assert_eq!(a.logging.retention_bytes, 2 * 1024 * 1024 * 1024);
        assert!(!a.engine_shutdown.enabled);
        assert!(a.validate().is_ok());
    }

    #[test]
    fn engine_shutdown_thresholds_are_validated() {
        let mut cfg = RuntimeConfig::appliance();
        cfg.engine_shutdown.arm_rpm = cfg.engine_shutdown.stop_rpm;
        assert!(cfg.validate().is_err());
        cfg.engine_shutdown.arm_rpm = 500.0;
        cfg.engine_shutdown.delay_seconds = 0;
        assert!(cfg.validate().is_err());
    }
}
