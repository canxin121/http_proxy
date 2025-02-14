use std::{env, net::SocketAddr, path::Path, sync::LazyLock};

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    pub local_address: SocketAddr,
    pub remote_proxy_address: RemoteProxyConfig,
    pub obfuscation: ObfuscationConfig,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ObfuscationConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RemoteProxyConfig {
    pub host: String,
    pub port: u16,
    pub user_agent: String,
    pub x_t5_auth: String,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let config_str = std::fs::read_to_string(path)?;
        let config = toml::from_str(&config_str)?;
        Ok(config)
    }
}

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    let config_path =
        env::var("HTTP_PROXY_CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
    Config::from_file(config_path).expect("Failed to load config file")
});

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_config_serialize() {
        let config = Config {
            local_address: SocketAddr::from(([127, 0, 0, 1], 3000)),
            remote_proxy_address: RemoteProxyConfig {
                host: "110.242.70.68".to_string(),
                port: 443,
                user_agent: "baiduboxapp".to_string(),
                x_t5_auth: "914964676".to_string(),
            },
            obfuscation: ObfuscationConfig {
                host: "gw.alicdn.com".to_string(),
                port: 80,
            },
        };
        let _ = std::fs::write(
            "config.toml",
            toml::to_string_pretty(&config).expect("Failed to serialize config"),
        );
    }
}
