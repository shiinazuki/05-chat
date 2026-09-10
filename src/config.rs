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

/// 覆盖 `database.url` 的环境变量。生产环境靠它注入带密码的连接串
const ENV_DATABASE_URL: &str = "CHAT_DATABASE_URL";

/// 覆盖密钥路径的环境变量。生产环境把密钥挂载成文件，用这两个变量指过去
const ENV_ENCODING_KEY_PATH: &str = "CHAT_AUTH_ENCODING_KEY_PATH";
const ENV_DECODING_KEY_PATH: &str = "CHAT_AUTH_DECODING_KEY_PATH";

///  覆盖 `server.allowed_origins` 的环境变量
const ENV_ALLOWED_ORIGINS: &str = "CHAT_SERVER_ALLOWED_ORIGINS";

/// 应用的全部配置。
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// HTTP 服务相关配置
    pub server: ServerConfig,

    /// 数据库相关配置
    pub database: DatabaseConfig,

    /// 认证相关配置
    pub auth: AuthConfig,
}

/// 认证配置
#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Ed25519 私钥（PKCS#8 PEM）的路径
    pub encoding_key_path: PathBuf,

    /// Ed25519 公钥（SPKI PEM）的路径
    pub decoding_key_path: PathBuf,

    /// 签发的 token 有效期，单位秒
    pub token_ttl_secs: u64,
}

impl AuthConfig {
    /// 签发 token 时使用的有效期
    #[must_use]
    pub fn token_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.token_ttl_secs)
    }
}

/// 数据库连接与连接池配置
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    /// Postgres 连接串
    pub url: String,

    /// 连接池上限。超过这个数的请求会排队，而不是压垮数据库
    pub max_connections: u32,

    /// 从池里取连接的等待上限，超时就快速失败而不是无限挂着
    pub acquire_timeout_secs: u64,

    /// 空闲连接保留多久后回收
    pub idle_timeout_secs: u64,
}

/// HTTP 服务配置
// 含 Vec<String> 后不再满足 Copy，derive 里的 Copy 要去掉。
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// 监听端口
    pub port: u16,

    /// 允许跨域访问的前端来源白名单。空列表表示不开放跨域
    pub allowed_origins: Vec<String>,

    /// 单个请求的处理超时，单位秒
    pub request_timeout_secs: u64,

    /// 请求体大小上限，单位字节
    pub body_limit_bytes: usize,
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
    /// let toml = r#"
    /// [server]
    /// port = 6688
    /// allowed_origins = ["http://localhost:5173"]
    /// request_timeout_secs = 30
    /// body_limit_bytes = 1048576
    ///
    /// [database]
    /// url = "postgres://localhost/chat"
    /// max_connections = 10
    /// acquire_timeout_secs = 3
    /// idle_timeout_secs = 600
    ///
    /// [auth]
    /// encoding_key_path = "fixtures/encoding.pem"
    /// decoding_key_path = "fixtures/decoding.pem"
    /// token_ttl_secs = 604800
    /// "#;
    ///
    /// let config = chat::AppConfig::from_toml_str(toml)?;
    /// assert_eq!(config.server.port, 6688);
    /// assert_eq!(config.database.max_connections, 10);
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
        if let Some(raw) = Self::non_empty(&lookup, ENV_SERVER_PORT) {
            self.server.port = raw.parse().map_err(|_| Error::InvalidEnvVar {
                name: ENV_SERVER_PORT,
                value: raw,
            })?;
        }

        if let Some(url) = Self::non_empty(&lookup, ENV_DATABASE_URL) {
            self.database.url = url;
        }

        if let Some(ek) = Self::non_empty(&lookup, ENV_ENCODING_KEY_PATH) {
            self.auth.encoding_key_path = ek.into();
        }

        if let Some(dk) = Self::non_empty(&lookup, ENV_DECODING_KEY_PATH) {
            self.auth.decoding_key_path = dk.into();
        }

        if let Some(allow_origins) = Self::non_empty(&lookup, ENV_ALLOWED_ORIGINS) {
            self.server.allowed_origins = allow_origins
                .split(',')
                .map(|origin| origin.trim().to_owned())
                .filter(|origin| !origin.is_empty())
                .collect();
        }

        Ok(())
    }

    /// 取环境变量的值并去掉首尾空白，空字符串按「未设置」处理
    fn non_empty(lookup: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
        lookup(name)
            .map(|raw| raw.trim().to_owned())
            .filter(|value| !value.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, ENV_SERVER_PORT};
    use crate::Error;

    const SAMPLE: &str = r#"
    [server]
    port = 6688
    allowed_origins = ["http://localhost:5173"]
    request_timeout_secs = 30
    body_limit_bytes = 1048576

    [database]
    url = "postgres://localhost/chat_test"
    max_connections = 5
    acquire_timeout_secs = 3
    idle_timeout_secs = 600

    [auth]
    encoding_key_path = "fixtures/encoding.pem"
    decoding_key_path = "fixtures/decoding.pem"
    token_ttl_secs = 604800
    "#;

    fn base() -> AppConfig {
        AppConfig::from_toml_str(SAMPLE).unwrap()
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
