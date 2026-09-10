//! 本 crate 的公开错误类型。
//!
//! 这里是库那一侧的具体错误，可供调用方 `match`；应用侧的 `src/main.rs`
//! 用 `anyhow::Result` 收口。

use std::path::PathBuf;

/// 本 crate 所有可恢复错误的统一入口。
///
/// 标了 `#[non_exhaustive]`，调用方的 `match` 必须保留 `_` 分支。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// 所有候选位置都没有找到配置文件
    #[error("未找到配置文件, 已以此尝试: {tried}")]
    ConfigNotFound {
        /// 已尝试过的路径，用 `, ` 连接，便于运维直接看出找了哪些位置
        tried: String,
    },

    /// 配置文件存在但读不出来（权限、坏掉的符号链接等）
    #[error("读取配置文件 {} 失败", path.display())]
    ConfigRead {
        /// 读取失败的路径
        path: PathBuf,
        /// 底层 IO 错误，`#[source]` 让 `{:#}` 能打出完整错误链
        #[source]
        source: std::io::Error,
    },

    /// 内容不是合法 TOML，或字段与 [`crate::AppConfig`] 对不上。
    #[error("解析配置失败（来源：{origin}）")]
    ConfigParse {
        /// 配置的来源描述：文件路径，或字符串解析时的占位说明
        origin: String,
        /// 底层解析错误，含出错的行列位置
        #[source]
        source: toml::de::Error,
    },

    /// 用于覆盖配置的环境变量存在，但值解析不了
    #[error("环境变量 {name} 的值 `{value}` 无效")]
    InvalidEnvVar {
        /// 环境变量名
        name: &'static str,
        /// 无法解析的原始值
        value: String,
    },

    /// 监听地址绑定失败，最常见的原因是端口被占用
    #[error("绑定监听地址 {addr} 失败")]
    Bind {
        /// 尝试绑定的地址
        addr: String,
        /// 底层 IO 错误
        source: std::io::Error,
    },

    /// HTTP 服务在运行期异常退出
    #[error("HTTP 服务异常退出")]
    Serve(#[source] std::io::Error),

    /// 数据库操作失败。
    #[error("数据库操作失败")]
    Database(#[from] sqlx::Error),

    /// 数据库操作失败。
    #[error("数据库操作失败")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// JWT 签发或校验失败
    #[error("JWT 处理失败")]
    Jwt(#[from] crate::jwt::JwtError),
}

/// 带默认错误类型的 `Result` 别名，公开 API 统一写 `Result<T>`。
pub type Result<T, E = Error> = core::result::Result<T, E>;
