//! 集成测试：以外部使用者的视角调用 crate 的公开 API。
//!
//! 它是独立 crate，只能访问 `src/lib.rs` 导出的 `pub` 项，`main.rs` 里的内容在这里
//! 访问不到。
//!
//! ⚠️ 下面用到的 `add` / `greet` 是 `src/lib.rs` 里的骨架函数，改写 lib.rs 时要一并
//!    替换成对自己公开 API 的调用，否则 `cargo test` / `just ci` 会编译失败。

use chat::{Error, add, greet};

#[test]
fn add_works_from_outside() {
    assert_eq!(add(40, 2), 42);
}

#[test]
fn greet_error_is_public_and_matchable() {
    // 断言错误类型是公开且可 match 的
    let err = greet("").unwrap_err();
    assert!(matches!(err, Error::EmptyName));
}
