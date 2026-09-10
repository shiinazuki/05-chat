//! Bearer token 认证

use axum::{
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tracing::warn;

use crate::{AppState, Error, User};

/// 校验 `Authorization: Bearer <token>`，通过则把 [`User`] 放进请求扩展。
///
/// 失败一律返回 401 且**不区分原因**——「没带 token」「已过期」「签名不对」
/// 对客户端来说都是「请重新登录」；区分开只会告诉攻击者他猜到了哪一步。
/// 真正的原因记在日志里，用 request id 就能定位
pub(crate) async fn verify_token(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    let Some(token) = token else {
        warn!("请求缺少 Authorization: Bearer 头");
        return Error::Unauthenticated.into_response();
    };

    match state.decoding_key.verify::<User>(token) {
        Ok(user) => {
            // 放进扩展，下游 handler 用 Extension<User> 取
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        Err(err) => {
            warn!(%err, "token 校验失败");
            Error::Unauthenticated.into_response()
        }
    }
}
