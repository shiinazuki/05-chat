//! 集成测试：以外部使用者的视角调用 crate 的公开 API。
//!
//! 它是独立 crate，只能访问 `src/lib.rs` 导出的 `pub` 项，`main.rs` 里的内容在这里
//! 访问不到。

use chat::{AppConfig, Error};

const SAMPLE: &str = r#"
[server]
port = 6688

[database]
url = "postgres://localhost/chat_test"
max_connections = 5
acquire_timeout_secs = 3
idle_timeout_secs = 600
"#;

#[test]
fn parses_full_config() {
    let config = AppConfig::from_toml_str(SAMPLE).expect("配置应能解析");
    assert_eq!(config.server.port, 6688);
    assert_eq!(config.database.max_connections, 5);
}

#[test]
fn rejects_config_missing_database_table() {
    // 只有 [server]，缺整个 [database] 段
    let err = AppConfig::from_toml_str("[server]\nport = 6688\n").unwrap_err();
    assert!(matches!(err, Error::ConfigParse { .. }));
}

#[test]
fn rejects_out_of_range_port() {
    // 70000 超出 u16，serde 在反序列化阶段就会拒掉
    let toml = SAMPLE.replace("port = 6688", "port = 70000");
    let err = AppConfig::from_toml_str(&toml).unwrap_err();
    assert!(matches!(err, Error::ConfigParse { .. }));
}

/// 验证迁移能在一个全新的库上跑通，且连接池可用。
///
/// `#[sqlx::test]` 会为这个测试单独建一个库、跑完 `migrations/` 下的全部迁移、
/// 把 pool 注入进来，测试通过后自动删库；**失败则保留**，方便进去查现场。
#[sqlx::test]
async fn migrations_apply_on_a_fresh_database(pool: sqlx::PgPool) {
    let ok: i32 = sqlx::query_scalar!(r#"SELECT 1 AS "ok!""#)
        .fetch_one(&pool)
        .await
        .expect("应能查询新建的测试库");
    assert_eq!(ok, 1);

    // 迁移里的哨兵行应该在
    let sentinel: i64 = sqlx::query_scalar!(r#"SELECT id AS "id!" FROM users WHERE id = 0"#)
        .fetch_one(&pool)
        .await
        .expect("哨兵用户应存在");
    assert_eq!(sentinel, 0);
}
