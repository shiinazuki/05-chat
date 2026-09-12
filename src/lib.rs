//! RUST CHAT
//!
//! 库目标：存放业务逻辑，供 `src/main.rs` 与 `tests/` 调用。

use std::{ops::Deref, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::{fs, net::TcpListener};
use tracing::{error, info, warn};

mod config;
mod error;
mod handlers;
mod jwt;
mod middlewares;
mod models;

pub use crate::{
    config::{AppConfig, AuthConfig, DatabaseConfig, ServerConfig},
    error::{Error, ErrorOutput, Result},
    handlers::AuthOutput,
    jwt::{DecodingKey, EncodingKey, JwtError},
    models::{
        Chat, ChatSort, ChatType, CreateChat, CreateMessage, CreateUser, ListMessages, Message,
        SigninUser, SortOrder, User, ensure_member,
    },
};

pub async fn serve_on() -> Result<()> {
    let config = AppConfig::load().await?;
    let pool = connect_database(&config.database).await?;
    sqlx::migrate!().run(&pool).await?;
    info!("数据库迁移已应用");

    let addr = format!("0.0.0.0:{}", config.server.port);

    let encoding_pem = fs::read_to_string(&config.auth.encoding_key_path)
        .await
        .map_err(|source| Error::ConfigRead {
            path: config.auth.encoding_key_path.clone(),
            source,
        })?;
    let decoding_pem = fs::read_to_string(&config.auth.decoding_key_path)
        .await
        .map_err(|source| Error::ConfigRead {
            path: config.auth.decoding_key_path.clone(),
            source,
        })?;
    let encoding_key = EncodingKey::load_pem(&encoding_pem)?;
    let decoding_key = DecodingKey::load_pem(&decoding_pem)?;

    let state = AppState::new(config, pool, encoding_key, decoding_key);
    let app = get_router(state)?;
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|source| Error::Bind {
            addr: addr.clone(),
            source,
        })?;
    info!(%addr,  "服务已启动");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(Error::Serve)?;

    info!("服务已关闭");
    Ok(())
}

/// 所有 handler 共享的应用状态。
///
/// 克隆只复制一个 `Arc`——axum 每个请求都会克隆一次 state
#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

/// 状态的真身。
///
/// 必须是 `pub` 而不是 `pub(crate)`：下面给 `AppState` 实现了 `Deref` 指向它，
/// 外部能拿到它的值却写不出类型名的话，`unnameable_types` 会报警
#[derive(Debug)]
pub struct AppStateInner {
    /// 全局配置，启动后只读
    pub config: AppConfig,
    /// 数据库连接池。`PgPool` 内部已是 `Arc`，克隆很便宜。
    pub pool: PgPool,
    pub encoding_key: EncodingKey,
    pub decoding_key: DecodingKey,
}

impl Deref for AppState {
    type Target = AppStateInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AppState {
    /// 用加载好的配置和已建立的连接池构造状态。
    pub fn new(
        config: AppConfig,
        pool: PgPool,
        encoding_key: EncodingKey,
        decoding_key: DecodingKey,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                config,
                pool,
                encoding_key,
                decoding_key,
            }),
        }
    }
}

/// 按配置建立数据库连接池。
///
/// 三个超时参数都显式设置：默认值在生产上不合适——取不到连接时无限等待，
/// 会把上游的线程/任务全部堵死，故障从数据库扩散成全站不可用
pub async fn connect_database(config: &DatabaseConfig) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs))
        .idle_timeout(Duration::from_secs(config.idle_timeout_secs))
        .connect(&config.url)
        .await?;

    Ok(pool)
}

/// 组装路由表。
///
/// 只负责「路由 → handler」的映射，不做绑定端口、不启动服务——
/// 那是 `main.rs` 的职责。这样测试可以只拿路由表，不需要真的听端口
pub fn get_router(state: AppState) -> Result<Router> {
    let server = state.config.server.clone();

    // 受保护：先过认证中间件，User 已注入扩展
    let protected = Router::new()
        .route("/users/me", get(handlers::me))
        .route(
            "/chats",
            get(handlers::list_chats).post(handlers::create_chat),
        )
        .route(
            "/chats/{id}/messages",
            get(handlers::list_messages).post(handlers::send_message),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middlewares::verify_token,
        ));

    // 公开：注册与登录本身不能要求已登录
    let public = Router::new()
        .route("/signup", post(handlers::signup))
        .route("/signin", post(handlers::signin));

    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .nest("/api", public.merge(protected))
        .with_state(state);

    middlewares::with_layers(router, &server)
}

/// 存活探针：进程还活着就返回 200。
///
/// 刻意不查任何外部依赖——这个端点失败会导致容器被杀掉重启，
/// 数据库抖一下就滚动重启全部实例是典型的自伤
async fn healthz() -> &'static str {
    "ok"
}

/// 就绪探针：能和数据库完成一次往返才算就绪。
///
/// 失败只会被负载均衡摘掉、不会重启，所以依赖探测放在这里而不是 `/healthz`。
async fn readyz(State(state): State<AppState>) -> Result<&'static str, StatusCode> {
    match sqlx::query_scalar!(r#"SELECT 1 AS "ok!""#)
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => Ok("ready"),
        Err(err) => {
            warn!(%err, "就绪探测失败：数据库不可用");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

/// 等待关闭信号：Ctrl-C，或 unix 上的 SIGTERM。
///
/// 只接 Ctrl-C 在容器里等于没接——`docker stop` 和 k8s 发的都是 SIGTERM，
/// 收不到就只能等宽限期结束被 SIGKILL，正在处理的请求会被硬切断。
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            error!(%err, "监听 Ctrl-C 失败");
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
