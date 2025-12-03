use futures::StreamExt;
use serde::Deserialize;
use signal_hook::consts::SIGHUP;
use signal_hook_tokio::Signals;
use thiserror::Error;
use tokio::sync::watch;

#[derive(Debug, Deserialize, Clone)]
pub struct RouteConfig {
    pub name: String,
    pub url: String,
    pub auth: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum AuthConfig {
    Bearer { token: String },
    OAuth {
        user: String,
        pass: String,
        url: String,
        client_id: String,
        realm: String,
    },
    Basic { user: String, pass: String },
}

#[derive(Debug, Deserialize, Clone)]
pub struct Auth {
    pub name: String,
    pub config: AuthConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub routes: Vec<RouteConfig>,
    pub auths: Vec<Auth>,
    pub scrape_interval_seconds: u64,
    pub listen_addr: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Read(#[from] std::io::Error),
    #[error("Failed to expand environment variables: {0}")]
    EnvExpansion(String),
    #[error("Failed to parse config YAML: {0}")]
    Parse(#[from] serde_yaml::Error),
}

pub async fn load_config() -> Result<AppConfig, ConfigError> {
    let raw = tokio::fs::read_to_string("config.yaml").await?;
    let expanded = shellexpand::env(&raw)
        .map_err(|e| ConfigError::EnvExpansion(e.to_string()))?
        .to_string();
    let cfg: AppConfig = serde_yaml::from_str(&expanded)?;
    Ok(cfg)
}

/// Returns a watch receiver that emits new config on SIGHUP
pub async fn with_hot_reload() -> Result<watch::Receiver<AppConfig>, ConfigError> {
    let initial = load_config().await?;
    let (tx, rx) = watch::channel(initial);

    let mut signals = Signals::new([SIGHUP]).map_err(|e| {
        tracing::error!("Failed to register signal handler: {}", e);
        ConfigError::Read(e)
    })?;

    tokio::spawn(async move {
        while let Some(sig) = signals.next().await {
            if sig == SIGHUP {
                tracing::info!("Reloading configuration...");
                match load_config().await {
                    Ok(config) => {
                        if tx.send(config).is_err() {
                            tracing::error!("Config receiver dropped");
                            break;
                        }
                        tracing::info!("Configuration reloaded successfully");
                    }
                    Err(e) => {
                        tracing::error!("Failed to reload config, keeping existing: {}", e);
                    }
                }
            }
        }
    });

    Ok(rx)
}
