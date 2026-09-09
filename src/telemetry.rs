//! 日志与追踪的初始化。

use std::io::IsTerminal as _;

use tracing_subscriber::{EnvFilter, filter::LevelFilter};

/// 安装全局 subscriber，在 `main` 的最开头调用一次。
///
/// 过滤规则优先取环境变量 `RUST_LOG`，其次取 `default_level`；日志写到 stderr。
pub(crate) fn init(default_level: &str) {
    // `default_level` 解析不了时退回 info
    let default = default_level.parse().unwrap_or_else(|_| {
        eprintln!("无法解析日志级别 `{default_level}`，退回 info");
        tracing::Level::INFO.into()
    });

    warn_if_env_looks_like_a_typo();

    // 以 default 为默认指令构建过滤器：RUST_LOG 为空或非法时仍按 default 放行
    let filter = EnvFilter::builder()
        .with_default_directive(default)
        .with_env_var(EnvFilter::DEFAULT_ENV)
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(ansi_enabled())
        .with_target(true)
        .init();
}

/// 判断日志是否上色：stderr 是终端、且未设置非空的 `NO_COLOR` 时返回 `true`。
fn ansi_enabled() -> bool {
    let disabled_by_env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    !disabled_by_env && std::io::stderr().is_terminal()
}

/// `RUST_LOG` 被写成单个裸词、且它不是日志级别时，打印一行提示。
///
/// 只提示，不改变过滤行为。
fn warn_if_env_looks_like_a_typo() {
    let Ok(raw) = std::env::var(EnvFilter::DEFAULT_ENV) else {
        return;
    };
    let raw = raw.trim();

    // 空值和带 `=` / `,` 的按模块细分写法交给 EnvFilter 自己判断
    if raw.is_empty() || raw.contains('=') || raw.contains(',') {
        return;
    }
    // 能解析成级别（名字、`off`、0-5 的数字）即为正常用法
    if raw.parse::<LevelFilter>().is_ok() {
        return;
    }

    eprintln!("提示：RUST_LOG=`{raw}` 不是日志级别，按 EnvFilter 的语法它是一个「目标名」，");
    eprintln!("      即只放行名为 {raw} 的模块。想调级别请写 error / warn / info / debug / trace");
}
