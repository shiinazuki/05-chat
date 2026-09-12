use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::{Error, Result};

/// 未指定 limit 时返回多少条
const DEFAULT_LIMIT: i64 = 20;

/// limit 的硬上限。客户端给多大都按这个截断
const MAX_LIMIT: i64 = 100;

/// 消息
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub chat_id: i64,
    pub sender_id: i64,
    pub content: String,
    pub files: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// 发消息的请求体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMessage {
    pub content: String,

    /// 附件路径，阶段 5C 的文件上传会填这里
    #[serde(default)]
    pub files: Vec<String>,
}

/// 拉取消息的分页参数
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ListMessages {
    /// 游标：只返回 id **严格小于**它的消息。首次拉取不传
    pub before: Option<i64>,
    /// 本次最多返回多少条，会被截断到 [1, 100]
    pub limit: Option<i64>,
}

impl Message {
    /// 在会话里发一条消息。
    ///
    /// # Errors
    ///
    /// 内容与附件同时为空时返回 [`Error::Validation`]
    pub async fn create(
        input: &CreateMessage,
        chat_id: i64,
        sender_id: i64,
        pool: &PgPool,
    ) -> Result<Self> {
        if input.content.trim().is_empty() && input.files.is_empty() {
            return Err(Error::Validation("消息内容与附件不能同时为空".to_owned()));
        }

        sqlx::query_as!(
            Self,
            r#"
                INSERT INTO messages (chat_id, sender_id, content, files)
                VALUES ($1, $2, $3, $4)
                RETURNING id, chat_id, sender_id, content, files, created_at
                "#,
            chat_id,
            sender_id,
            input.content,
            &input.files,
        )
        .fetch_one(pool)
        .await
        .map_err(Error::Database)
    }

    /// 按游标分页拉取会话消息，按 id 倒序（最新的在前）。
    ///
    /// 用 keyset 而不是 OFFSET：OFFSET 的代价随页码线性增长，且并发插入时
    /// 偏移量会错位导致漏读或重复。
    ///
    /// # Errors
    ///
    /// 数据库查询失败时返回 [`Error::Database`]
    pub async fn list(chat_id: i64, params: ListMessages, pool: &PgPool) -> Result<Vec<Self>> {
        // 永远不要信任客户端给的 limit：?limit=100000000 就是一次免费的 DoS
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        // 首次拉取没有游标，用 i64::MAX 表示「从最新开始」
        let before = params.before.unwrap_or(i64::MAX);

        sqlx::query_as!(
            Self,
            r#"
            SELECT id, chat_id, sender_id, content, files, created_at
            FROM messages
            WHERE chat_id = $1 AND id < $2
            ORDER BY id DESC
            LIMIT $3
            "#,
            chat_id,
            before,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(Error::Database)
    }
}

/// 确认用户是该会话的成员。
///
/// **会话不存在与非成员返回同一个错误**——若区分成 404 与 403，攻击者就能
/// 用这个差异探测出哪些会话 id 真实存在，以及自己被排除在哪些会话之外。
///
/// # Errors
///
/// 会话不存在或用户不是成员时返回 [`Error::ChatNotFound`]
pub async fn ensure_member(chat_id: i64, user_id: i64, pool: &PgPool) -> Result<()> {
    let is_member = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM chats
            WHERE id = $1 AND members @> ARRAY[$2]::bigint[]
            ) AS "exists!"
        "#,
        chat_id,
        user_id,
    )
    .fetch_one(pool)
    .await?;

    if is_member {
        Ok(())
    } else {
        Err(Error::ChatNotFound)
    }
}
