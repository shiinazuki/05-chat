//! RUST CHAT
//!
//! 库目标：存放业务逻辑，供 `src/main.rs` 与 `tests/` 调用。
mod config;
mod error;

use std::sync::Arc;

use axum::{Router, routing::get};
pub use config::{AppConfig, ServerConfig};
pub use error::{Error, Result};
use tokio::net::TcpListener;

pub async fn serve_on() -> Result<()> {
    let config = AppConfig::load().await?;
    let addr = format!("0.0.0.0:{}", config.server.port);

    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|source| Error::Bind {
            addr: addr.clone(),
            source,
        })?;
    tracing::info!(%addr,  "服务已启动");

    let state = AppState::new(config);
    axum::serve(listener, get_router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(Error::Serve)?;

    tracing::info!("服务已关闭");
    Ok(())
}

/// 所有 handler 共享的应用状态。
///
/// axum 每处理一个请求就会 `clone` 一次 state，所以克隆代价必须足够低——
/// 这里只克隆一个 `Arc`。
#[derive(Debug, Clone)]
pub struct AppState {
    /// 全局配置，启动后只读
    pub config: Arc<AppConfig>,
}

impl AppState {
    /// 用加载好的配置构造状态
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

/// 组装路由表。
///
/// 只负责「路由 → handler」的映射，不做绑定端口、不启动服务——
/// 那是 `main.rs` 的职责。这样测试可以只拿路由表，不需要真的听端口
pub fn get_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}

/// 存活探针：进程还活着就返回 200。
///
/// 刻意不查任何外部依赖——这个端点失败会导致容器被杀掉重启，
/// 数据库抖一下就滚动重启全部实例是典型的自伤
async fn healthz() -> &'static str {
    "ok"
}

/// 就绪探针：外部依赖可用时返回 200。
///
/// 失败只会被负载均衡摘掉，不会重启，所以这里适合放依赖探测。
/// 阶段 2 接上数据库后，这里要加 `pool.acquire()`
async fn readyz() -> &'static str {
    "ready"
}

/// 等待关闭信号：Ctrl-C，或 unix 上的 SIGTERM。
///
/// 只接 Ctrl-C 在容器里等于没接——`docker stop` 和 k8s 发的都是 SIGTERM，
/// 收不到就只能等宽限期结束被 SIGKILL，正在处理的请求会被硬切断。
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(%err, "监听 Ctrl-C 失败");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};

        match signal(SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => tracing::error!(%err, "监听 SIGTERM 失败"),
        }
    };

    // 非 unix 平台没有 SIGTERM，用一个永不完成的 future 占位
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("收到 Ctrl-C，开始优雅关闭"),
        () = terminate => tracing::info!("收到 SIGTERM，开始优雅关闭"),
    }
}
