//! 消息相关的 handler

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    AppState, Result, User,
    models::{CreateMessage, ListMessages, Message, ensure_member},
};

/// 拉取会话消息（游标分页）
pub(crate) async fn list_messages(
    Extension(user): Extension<User>,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    Query(params): Query<ListMessages>,
) -> Result<impl IntoResponse> {
    ensure_member(chat_id, user.id, &state.pool).await?;
    let messages = Message::list(chat_id, params, &state.pool).await?;
    Ok(Json(messages))
}

/// 在会话里发消息
pub(crate) async fn send_message(
    Extension(user): Extension<User>,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    Json(input): Json<CreateMessage>,
) -> Result<impl IntoResponse> {
    ensure_member(chat_id, user.id, &state.pool).await?;
    let message = Message::create(&input, chat_id, user.id, &state.pool).await?;
    Ok((StatusCode::CREATED, Json(message)))
}
