//! 会话模型与相关的数据库操作

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::{Error, Result};

/// 对应 Postgres 的 `chat_type` 枚举。
///
/// `type_name` 必须与迁移里 `CREATE TYPE` 的名字逐字一致；`rename_all` 负责把
/// Rust 的 `PrivateChannel` 对上 SQL 的 `private_channel`。名字对不上时
/// sqlx 会在**编译期**报错，不会等到运行时
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "chat_type", rename_all = "snake_case")]
#[serde(rename_all = "camelCase")]
pub enum ChatType {
    /// 两人单聊，没有名字
    Single,
    /// 多人群聊，没有名字
    Group,
    /// 命名的私有频道
    PrivateChannel,
    /// 命名的公开频道
    PublicChannel,
}

/// 会话
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    pub id: i64,
    pub ws_id: i64,
    /// 单聊与小群没有名字，schema 里这一列可空
    pub name: Option<String>,
    pub r#type: ChatType,
    /// 成员的用户 id
    pub members: Vec<i64>,
    pub created_at: DateTime<Utc>,
}

/// 创建会话的请求体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChat {
    /// 频道名。单聊与小群可不填
    pub name: Option<String>,
    /// 成员的用户 id，必须包含创建者自己
    pub members: Vec<i64>,
    /// 命名频道是否公开。未命名的会话忽略此项
    pub public: bool,
}

impl CreateChat {
    /// 按「是否命名 + 成员数 + 是否公开」推导会话类型。
    ///
    /// 类型不由客户端指定，而是从事实推导出来——让客户端自己声明 `type`，
    /// 就等于允许它造出「两人的公开频道」这类自相矛盾的数据
    fn chat_type(&self) -> Result<ChatType> {
        let len = self.members.len();
        if len < 2 {
            return Err(Error::Validation("会话至少需要两名成员".to_owned()));
        }
        if len > 8 && self.name.is_none() {
            return Err(Error::Validation("超过 8 人的会话必须命名".to_owned()));
        }

        Ok(match (self.name.as_ref(), len, self.public) {
            (None, 2, _) => ChatType::Single,
            (None, _, _) => ChatType::Group,
            (Some(_), _, true) => ChatType::PublicChannel,
            (Some(_), _, false) => ChatType::PrivateChannel,
        })
    }
}

impl Chat {
    /// 创建会话。
    ///
    /// # Errors
    ///
    /// 成员数不合法、创建者不在成员里、成员含非本工作区用户时返回
    /// [`Error::Validation`]；数据库失败返回 [`Error::Database`]
    pub async fn create(
        input: &CreateChat,
        ws_id: i64,
        creator_id: i64,
        pool: &PgPool,
    ) -> Result<Self> {
        if !input.members.contains(&creator_id) {
            return Err(Error::Validation("创建者必须是会话成员之一".to_owned()));
        }
        let chat_type = input.chat_type()?;

        // members 是 bigint[]，而 Postgres 不支持给数组元素建外键——
        // 「成员必须是真实且同工作区的用户」这条约束只能在应用层查。
        // 这就是用数组存关系（而不是建关联表）要付的代价
        let expected = i64::try_from(input.members.len())
            .map_err(|_| Error::Validation("成员数量过多".to_owned()))?;

        let actual = sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM users WHERE id = ANY($1) AND ws_id = $2"#,
            &input.members,
            ws_id,
        )
        .fetch_one(pool)
        .await?;

        if actual != expected {
            return Err(Error::Validation(
                "成员列表里存在不存在或不属于本工作区的用户".to_owned(),
            ));
        }

        sqlx::query_as!(
            Self,
            r#"
            INSERT INTO chats(ws_id, name, type, members)
            VALUES ($1, $2, $3, $4)
            RETURNING id, ws_id, name, type AS "type: ChatType", members, created_at
            "#,
            ws_id,
            input.name,
            chat_type as ChatType,
            &input.members,
        )
        .fetch_one(pool)
        .await
        .map_err(Error::Database)
    }

    /// 列出用户参与的所有会话。
    ///
    /// 用 `members @> ARRAY[$2]::bigint[]` 而不是 `$2 = ANY(members)`——
    /// 只有前者能走 `chat_members_index` 那个 GIN 索引。
    ///
    /// # Errors
    ///
    /// 数据库查询失败时返回 [`Error::Database`]
    pub async fn list_for_user(user_id: i64, ws_id: i64, pool: &PgPool) -> Result<Vec<Self>> {
        sqlx::query_as!(
            Self,
            r#"
               SELECT id, ws_id, name, type AS "type: ChatType", members, created_at
               FROM chats
               WHERE ws_id = $1 AND members @> ARRAY[$2]::bigint[]
               ORDER BY created_at DESC
               "#,
            ws_id,
            user_id,
        )
        .fetch_all(pool)
        .await
        .map_err(Error::Database)
    }

    /// 按 id 取会话，不存在返回 `Ok(None)`。
    ///
    /// # Errors
    ///
    /// 数据库查询失败时返回 [`Error::Database`]
    pub async fn get_by_id(id: i64, pool: &PgPool) -> Result<Option<Self>> {
        sqlx::query_as!(
            Self,
            r#"
              SELECT id, ws_id, name, type AS "type: ChatType", members, created_at
              FROM chats
              WHERE id = $1
              "#,
            id,
        )
        .fetch_optional(pool)
        .await
        .map_err(Error::Database)
    }

    /// 判断用户是否为该会话成员。阶段 5B 的消息鉴权会用到
    #[must_use]
    pub fn has_member(&self, user_id: i64) -> bool {
        self.members.contains(&user_id)
    }
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::{Chat, ChatType, CreateChat};
    use crate::{
        Error,
        models::{CreateUser, User},
    };

    async fn seed_users(pool: &PgPool, count: usize) -> Vec<i64> {
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let input = CreateUser {
                fullname: format!("用户{i}"),
                email: format!("u{i}@b.com"),
                password: "hunter2".to_owned(),
            };
            ids.push(User::create(&input, pool).await.unwrap().id);
        }
        ids
    }

    fn input(name: Option<&str>, members: &[i64], public: bool) -> CreateChat {
        CreateChat {
            name: name.map(ToOwned::to_owned),
            members: members.to_vec(),
            public,
        }
    }

    #[sqlx::test]
    async fn two_unnamed_members_make_a_single_chat(pool: PgPool) {
        let ids = seed_users(&pool, 2).await;
        let chat = Chat::create(&input(None, &ids, false), 0, ids[0], &pool)
            .await
            .unwrap();
        assert_eq!(chat.r#type, ChatType::Single);
        assert_eq!(chat.name, None);
    }

    #[sqlx::test]
    async fn named_public_makes_a_public_channel(pool: PgPool) {
        let ids = seed_users(&pool, 3).await;
        let chat = Chat::create(&input(Some("公告"), &ids, true), 0, ids[0], &pool)
            .await
            .unwrap();
        assert_eq!(chat.r#type, ChatType::PublicChannel);
    }

    #[sqlx::test]
    async fn creator_must_be_a_member(pool: PgPool) {
        let ids = seed_users(&pool, 3).await;
        let err = Chat::create(&input(None, &ids[1..], false), 0, ids[0], &pool)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[sqlx::test]
    async fn unknown_member_is_rejected(pool: PgPool) {
        let ids = seed_users(&pool, 2).await;
        let bogus = vec![ids[0], 999_999];
        let err = Chat::create(&input(None, &bogus, false), 0, ids[0], &pool)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[sqlx::test]
    async fn list_only_returns_chats_i_belong_to(pool: PgPool) {
        let ids = seed_users(&pool, 3).await;
        Chat::create(&input(None, &ids[..2], false), 0, ids[0], &pool)
            .await
            .unwrap();

        let mine = Chat::list_for_user(ids[0], 0, &pool).await.unwrap();
        assert_eq!(mine.len(), 1);

        let others = Chat::list_for_user(ids[2], 0, &pool).await.unwrap();
        assert!(others.is_empty(), "不该看到自己没参与的会话");
    }
}
