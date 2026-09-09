//! RUST CHAT
//!
//! 库目标：存放业务逻辑，供 `src/main.rs` 与 `tests/` 调用。

mod error;

pub use crate::error::{Error, Result};

/// 把两个数相加。
///
/// # Examples
///
/// ```
/// let sum = chat::add(1, 2);
/// assert_eq!(sum, 3);
/// ```
#[must_use]
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

/// 生成问候语。
///
/// # Errors
///
/// `name` 去掉首尾空白后为空时返回 [`Error::EmptyName`]。
///
/// # Examples
///
/// ```
/// # fn main() -> chat::Result<()> {
/// let msg = chat::greet("world")?;
/// assert_eq!(msg, "Hello, world!");
///
/// let blank = chat::greet("  ");
/// assert!(blank.is_err());
/// # Ok(())
/// # }
/// ```
pub fn greet(name: &str) -> Result<String> {
    if name.trim().is_empty() {
        return Err(Error::EmptyName);
    }
    Ok(format!("Hello, {name}!"))
}

#[cfg(test)]
mod tests {
    use super::{add, greet};

    #[test]
    fn it_works() {
        assert_eq!(add(2, 2), 4);
    }

    #[test]
    fn greet_formats_message() {
        assert_eq!(greet("world").unwrap(), "Hello, world!");
    }

    #[test]
    fn greet_rejects_blank_name() {
        assert!(greet("   ").is_err());
    }

    /// 异步测试示例。
    #[tokio::test]
    async fn async_test_works() {
        assert_eq!(add(1, 1), 2);
    }
}
