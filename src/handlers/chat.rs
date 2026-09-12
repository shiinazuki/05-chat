use axum::{
    Extension, Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{
    AppState, ChatSort, Result, SortOrder, User,
    models::{Chat, CreateChat},
};

/// 会话列表的查询参数
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub(crate) struct ListChatsQuery {
    #[serde(default)]
    pub sort: ChatSort,
    #[serde(default)]
    pub order: SortOrder,
}

/// 列出当前用户参与的会话
pub(crate) async fn list_chats(
    Extension(user): Extension<User>,
    State(state): State<AppState>,
    Query(params): Query<ListChatsQuery>,
) -> Result<impl IntoResponse> {
    let chats =
        Chat::list_for_user(user.id, user.ws_id, params.sort, params.order, &state.pool).await?;
    Ok(Json(chats))
}

/// 创建会话。
///
/// 工作区与创建者都取自 token 里的 `User`，**不接受客户端传入**——
/// 否则任何人都能往别人的工作区里塞会话
pub(crate) async fn create_chat(
    Extension(user): Extension<User>,
    State(state): State<AppState>,
    Json(input): Json<CreateChat>,
) -> Result<impl IntoResponse> {
    let chat = Chat::create(&input, user.ws_id, user.id, &state.pool).await?;
    Ok((StatusCode::CREATED, Json(chat)))
}
