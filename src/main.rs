//! RUST CHAT
//!
//! 可执行入口：初始化日志、收口错误、把结果写到 stdout，业务逻辑在 `src/lib.rs`。

mod telemetry;

use anyhow::Context as _;
use chat::serve_on;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("info");
    serve_on().await.context("启动 chat 服务失败")?;

    Ok(())
}
