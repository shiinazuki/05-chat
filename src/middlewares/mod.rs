//! 通用中间件栈

mod auth;

use std::time::Duration;

use axum::{
    Router,
    extract::Request,
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::{Level, info_span};

pub(crate) use crate::middlewares::auth::verify_token;
use crate::{Error, Result, ServerConfig};

/// 请求 id 所用的头名
const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// 给路由套上通用中间件栈。
///
/// `ServiceBuilder` 里写在前面的是外层，顺序不能随意调整，理由见模块文档。
///
/// # Errors
///
/// 配置里的跨域来源不是合法 HTTP 头值时返回 [`Error::InvalidOrigin`]
pub(crate) fn with_layers(router: Router, config: &ServerConfig) -> Result<Router> {
    let origins = config
        .allowed_origins
        .iter()
        .map(|origin| {
            origin
                .parse::<HeaderValue>()
                .map_err(|_| Error::InvalidOrigin(origin.clone()))
        })
        .collect::<Result<Vec<_>>>()?;

    let router = router.layer(
        ServiceBuilder::new()
            // ① 最外层：没有它，后面所有层都拿不到 request id
            .layer(SetRequestIdLayer::new(X_REQUEST_ID, MakeRequestUuid))
        // ② 靠外：被内层拒掉的请求（超时、超限、跨域）也要能进日志
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(|req: &Request| {
                        // 默认的 span 里没有 request_id，必须自己挂进去，
                                               // 否则同一请求的多条日志无法串联
                        let request_id = req.headers()
                            .get(X_REQUEST_ID)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("_");
                        info_span!(
                            "http",
                            method = %req.method(),
                            uri = %req.uri(),
                            request_id = %request_id,
                        )
                    })
                    .on_response(
                        DefaultOnResponse::new()
                            .level(Level::INFO)
                            .latency_unit(tower_http::LatencyUnit::Millis),
                    ),
            )
         // ③ 把 request id 写回响应头，客户端报障时能直接给出这个 id
            .layer(PropagateRequestIdLayer::new(X_REQUEST_ID))
            .layer(CompressionLayer::new())
        // ④ 必须在认证之外：预检请求不带 Authorization 头
            .layer(
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(origins))
                    .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
                    .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                    .allow_credentials(true),
            )
        // ⑤ 防慢连接与超大 body
            .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT,
                Duration::from_secs(config.request_timeout_secs), ))
            .layer(RequestBodyLimitLayer::new(config.body_limit_bytes)),
    );

    Ok(router)
}
