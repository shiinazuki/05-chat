//! 应用配置：从 TOML 文件加载，再用环境变量逐项覆盖。

use std::{
    env,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use tokio::fs;

use crate::{Error, Result};

/// 配置文件的候选位置，按顺序查找，取第一个存在的
const SEARCH_PATHS: [&str; 2] = ["chat.toml", "/etc/chat/chat.toml"];

/// 直接指定配置文件路径的环境变量；设置后跳过 [`SEARCH_PATHS`]
const ENV_CONFIG_PATH: &str = "CHAT_CONFIG";

/// 覆盖 `server.port` 的环境变量
const ENV_SERVER_PORT: &str = "CHAT_SERVER_PORT";

/// 应用的全部配置。
// TODO(阶段 2)：加上 `db_url: String` 之后本类型不再是 Copy，届时删掉 Copy derive。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct AppConfig {
    /// HTTP 服务相关配置
    pub server: ServerConfig,
}

/// HTTP 服务配置。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ServerConfig {
    /// 监听端口
    pub port: u16,
}

impl AppConfig {
    /// 加载配置。
    ///
    /// 查找顺序：环境变量 `CHAT_CONFIG` 指定的路径 → `./chat.toml` →
    /// `/etc/chat/chat.toml`，取第一个存在的文件；解析后再用环境变量逐项覆盖。
    ///
    /// # Errors
    ///
    /// - [`Error::ConfigNotFound`]：所有候选路径都不存在
    /// - [`Error::ConfigRead`]：文件存在但读取失败
    /// - [`Error::ConfigParse`]：内容不是合法 TOML，或字段对不上
    /// - [`Error::InvalidEnvVar`]：覆盖用的环境变量值无法解析
    pub async fn load() -> Result<Self> {
        let path = Self::locate().await?;
        let mut config = Self::from_file(&path).await?;
        config.apply_overrides(|name| env::var(name).ok())?;
        Ok(config)
    }

    /// 从 TOML 字符串解析配置，不读文件、不看环境变量。
    ///
    /// # Errors
    ///
    /// 内容不是合法 TOML 或字段对不上时返回 [`Error::ConfigParse`]。
    ///
    /// # Examples
    ///
    /// ```
    /// let config = chat::AppConfig::from_toml_str("[server]\nport = 6688\n")?;
    /// assert_eq!(config.server.port, 6688);
    /// # Ok::<(), chat::Error>(())
    /// ```
    pub fn from_toml_str(text: &str) -> Result<Self> {
        Self::parse(text, "<字符串>")
    }

    /// 定位配置文件：`CHAT_CONFIG` 优先，其次按 [`SEARCH_PATHS`] 依次探测
    async fn locate() -> Result<PathBuf> {
        // 显式指定就用它，不存在也不回退——静默回退会让运维以为配置生效了
        if let Some(raw) = env::var_os(ENV_CONFIG_PATH) {
            return Ok(PathBuf::from(raw));
        }

        for candidate in SEARCH_PATHS {
            if fs::try_exists(candidate).await.unwrap_or(false) {
                return Ok(PathBuf::from(candidate));
            }
        }

        Err(Error::ConfigNotFound {
            tried: SEARCH_PATHS.join(", "),
        })
    }

    /// 读取并解析指定路径的配置文件
    async fn from_file(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .await
            .map_err(|source| Error::ConfigRead {
                path: path.to_path_buf(),
                source,
            })?;

        Self::parse(&text, &path.display().to_string())
    }

    /// 解析 TOML 文本，`origin` 只用于出错时的定位信息
    fn parse(text: &str, origin: &str) -> Result<Self> {
        toml::from_str(text).map_err(|source| Error::ConfigParse {
            origin: origin.to_owned(),
            source,
        })
    }

    /// 用 `lookup` 查到的值逐项覆盖配置。
    ///
    /// 把「怎么查」抽成参数而不是直接读环境变量，是为了让这段逻辑可测：
    /// Rust 2024 起 `std::env::set_var` 是 `unsafe fn`，而本 workspace
    /// `unsafe_code = "forbid"`，测试里根本没法改环境变量。
    fn apply_overrides(&mut self, lookup: impl Fn(&str) -> Option<String>) -> Result<()> {
        if let Some(raw) = lookup(ENV_SERVER_PORT) {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.server.port = raw.parse().map_err(|_| Error::InvalidEnvVar {
                    name: ENV_SERVER_PORT,
                    value: raw.to_owned(),
                })?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, ENV_SERVER_PORT};
    use crate::Error;

    fn base() -> AppConfig {
        AppConfig::from_toml_str("[server]\nport = 6688\n").unwrap()
    }

    #[test]
    fn env_override_replaces_port() {
        let mut config = base();
        config
            .apply_overrides(|name| (name == ENV_SERVER_PORT).then(|| "9000".to_owned()))
            .unwrap();
        assert_eq!(config.server.port, 9000);
    }

    #[test]
    fn invalid_env_override_is_rejected() {
        let mut config = base();
        let err = config
            .apply_overrides(|_| Some("not-a-port".to_owned()))
            .unwrap_err();
        assert!(matches!(err, Error::InvalidEnvVar { .. }));
    }

    #[test]
    fn missing_env_keeps_file_value() {
        let mut config = base();
        config.apply_overrides(|_| None).unwrap();
        assert_eq!(config.server.port, 6688);
    }
}
