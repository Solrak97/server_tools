//! Shared configuration for `server_tools` binaries (network, docker, system).
//!
//! Config merge order (later overrides earlier for keys that appear in each file):
//! 1. Built-in defaults
//! 2. `/etc/server_tools/config.toml` (when present)
//! 3. `$XDG_CONFIG_HOME/server_tools/config.toml` or `~/.config/server_tools/config.toml`
//! 4. Path from `--config` / `SERVER_TOOLS_CONFIG` (when set)
//!
//! Partial TOML files merge at the key level (a file with only `[docker]` does not reset `network`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct ConfigPathsCli {
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerToolsConfig {
    pub global: GlobalConfig,
    pub network: NetworkConfig,
    pub docker: DockerConfig,
    pub system: SystemConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GlobalConfig {
    /// Default log level for `tracing` (`error`, `warn`, `info`, `debug`, `trace`) when `RUST_LOG` is unset.
    pub log_filter: String,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            log_filter: "warn".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Poll interval for the UI loop (milliseconds).
    pub refresh_ms: u64,
    /// Interface names to hide (exact match).
    pub hide_interfaces: Vec<String>,
    /// Show TCP listeners panel (reads `/proc/net/tcp*`).
    pub show_listeners: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            refresh_ms: 1000,
            hide_interfaces: Vec::new(),
            show_listeners: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DockerConfig {
    pub refresh_ms: u64,
    /// Unix socket path; default matches Docker on Debian/Fedora (`/var/run/docker.sock`).
    pub socket_path: String,
    /// Include stopped containers in the list.
    pub list_all_containers: bool,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            refresh_ms: 2000,
            socket_path: "/var/run/docker.sock".to_string(),
            list_all_containers: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SystemConfig {
    pub refresh_ms: u64,
    /// Max rows in the process table.
    pub process_limit: usize,
    /// Show one gauge per CPU core.
    pub show_per_cpu: bool,
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            refresh_ms: 1500,
            process_limit: 18,
            show_per_cpu: true,
        }
    }
}

fn merge_toml(base: toml::Value, overlay: toml::Value) -> toml::Value {
    use toml::Value;
    match (base, overlay) {
        (Value::Table(mut a), Value::Table(b)) => {
            for (k, v) in b {
                let merged = match a.remove(&k) {
                    Some(existing) => merge_toml(existing, v),
                    None => v,
                };
                a.insert(k, merged);
            }
            Value::Table(a)
        }
        (_, overlay) => overlay,
    }
}

fn config_to_toml(cfg: &ServerToolsConfig) -> Result<toml::Value> {
    let s = toml::to_string(cfg)?;
    Ok(toml::from_str(&s)?)
}

/// Resolve ordered config file paths (excluding `--config`, which is applied last by the loader).
pub fn standard_config_files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(unix) {
        v.push(PathBuf::from("/etc/server_tools/config.toml"));
    }
    if let Some(d) = dirs::config_dir() {
        v.push(d.join("server_tools").join("config.toml"));
    }
    v
}

/// Load merged configuration: defaults → standard paths → optional override file.
pub fn load_config(override_path: Option<&Path>) -> Result<ServerToolsConfig> {
    let mut merged = config_to_toml(&ServerToolsConfig::default())?;

    for p in standard_config_files() {
        if p.is_file() {
            let s = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
            let next: toml::Value = toml::from_str(&s).with_context(|| format!("parse {}", p.display()))?;
            merged = merge_toml(merged, next);
        }
    }

    if let Some(p) = override_path {
        if p.is_file() {
            let s = std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
            let next: toml::Value = toml::from_str(&s).with_context(|| format!("parse {}", p.display()))?;
            merged = merge_toml(merged, next);
        }
    }

    let s = toml::to_string(&merged)?;
    let cfg: ServerToolsConfig = toml::from_str(&s)?;
    Ok(cfg)
}

/// Initialize `tracing` from merged config (respects `RUST_LOG` when set).
pub fn init_tracing(global: &GlobalConfig) {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| global.log_filter.clone());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&filter)),
        )
        .try_init();
}
