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
    pub address: SocketAddr,
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
    let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
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
                address: SocketAddr::from(([110, 242, 70, 68], 443)),
                user_agent: "baiduboxapp".to_string(),
                x_t5_auth: "914964676".to_string(),
            },
            obfuscation: ObfuscationConfig {
                host: "gw.alicdn.com".to_string(),
                port: 80,
            },
        };
        let test_config_path = "test_config.toml";
        let config_str = toml::to_string(&config).unwrap();
        std::fs::write(test_config_path, config_str).unwrap();

        // 测试从文件读取
        let loaded_config = Config::from_file(test_config_path).unwrap();
        assert_eq!(loaded_config.local_address, config.local_address);

        // 清理测试文件
        std::fs::remove_file(test_config_path).unwrap();
    }
}
