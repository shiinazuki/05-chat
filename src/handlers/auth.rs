use axum::{Extension, Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

use crate::{
    AppState, Error, Result,
    models::{CreateUser, SigninUser, User},
};

/// 认证成功后返回的凭证
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthOutput {
    /// 签发的 JWT，客户端后续放进 `Authorization: Bearer <token>`
    pub token: String,
}

/// 注册新用户，成功返回 201 与一枚 token
pub(crate) async fn signup(
    State(state): State<AppState>,
    Json(input): Json<CreateUser>,
) -> Result<impl IntoResponse> {
    let user = User::create(&input, &state.pool).await?;
    let token = state
        .encoding_key
        .sign(&user, state.config.auth.token_ttl())?;

    Ok((StatusCode::CREATED, Json(AuthOutput { token })))
}

/// 登录，成功返回 200 与一枚 token
pub(crate) async fn signin(
    State(state): State<AppState>,
    Json(input): Json<SigninUser>,
) -> Result<impl IntoResponse> {
    // 邮箱不存在与密码错误必须走同一个出口
    let Some(user) = User::verify(&input, &state.pool).await? else {
        return Err(Error::InvalidCredentials);
    };

    let token = state
        .encoding_key
        .sign(&user, state.config.auth.token_ttl())?;

    Ok((StatusCode::OK, Json(AuthOutput { token })))
}

/// 返回当前登录用户，用来验证认证链路是否打通
pub(crate) async fn me(Extension(user): Extension<User>) -> impl IntoResponse {
    Json(user)
}
