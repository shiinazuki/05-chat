//! 本 crate 的公开错误类型。
//!
//! 这里是库那一侧的具体错误，可供调用方 `match`；应用侧的 `src/main.rs`
//! 用 `anyhow::Result` 收口。

/// 本 crate 所有可恢复错误的统一入口。
///
/// 标了 `#[non_exhaustive]`，调用方的 `match` 必须保留 `_` 分支。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// 传入的名字为空，或者只有空白字符。
    #[error("名字不能为空")]
    EmptyName,

    /// 底层 IO 失败，`#[from]` 让 `?` 能从 [`std::io::Error`] 直接转换。
    #[error("IO 操作失败")]
    Io(#[from] std::io::Error),
}

/// 带默认错误类型的 `Result` 别名，公开 API 统一写 `Result<T>`。
pub type Result<T, E = Error> = core::result::Result<T, E>;
