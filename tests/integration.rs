//! 集成测试：以外部使用者的视角调用 crate 的公开 API。
//!
//! 它是独立 crate，只能访问 `src/lib.rs` 导出的 `pub` 项，`main.rs` 里的内容在这里
//! 访问不到。

use chat::{AppConfig, Error};

#[test]
fn parses_minimal_config() {
    let config = AppConfig::from_toml_str("[server]\nport = 6688\n").expect("配置应能解析");
    assert_eq!(config.server.port, 6688);
}

#[test]
fn rejects_config_missing_server_table() {
    let err = AppConfig::from_toml_str("port = 6688\n").unwrap_err();
    assert!(matches!(err, Error::ConfigParse { .. }));
}

#[test]
fn rejects_out_of_range_port() {
    // 70000 超出 u16，serde 会在反序列化阶段拒掉
    let err = AppConfig::from_toml_str("[server]\nport = 70000\n").unwrap_err();
    assert!(matches!(err, Error::ConfigParse { .. }));
}
