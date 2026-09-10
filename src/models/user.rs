//! 用户模型与相关的数据库操作。

use std::{fmt, sync::LazyLock};

use argon2::{
    Argon2, PasswordHasher as _, PasswordVerifier as _, password_hash::phc::PasswordHash,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::{Error, Result};

/// 一个固定口令的哈希，只用于登录失败时对齐耗时。
///
/// 邮箱不存在时若直接返回，请求会明显快于「邮箱存在但密码错」，攻击者据此
/// 就能枚举出哪些邮箱注册过。让两条路径都跑一次 argon2 可以抹平这个差异
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"a fixed password used only to equalize timing")
        .expect("对固定常量口令做哈希不会失败")
        .to_string()
});

/// 用户的公开视图。
///
/// 刻意不含 `password_hash`：在类型层面杜绝哈希被序列化给客户端或塞进 JWT
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub ws_id: i64,
    pub fullname: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
}

/// 注册请求体
#[derive(Clone, Serialize, Deserialize)]
pub struct CreateUser {
    pub fullname: String,
    pub email: String,
    pub password: String,
}

// 明文口令绝不能因为某处 `{:?}` 就进了日志，所以手写 Debug
impl fmt::Debug for CreateUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateUser")
            .field("fullname", &self.fullname)
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// 登录请求体
#[derive(Clone, Serialize, Deserialize)]
pub struct SigninUser {
    pub email: String,
    pub password: String,
}

impl fmt::Debug for SigninUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigninUser")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl User {
    /// 创建用户。
    ///
    /// # Errors
    ///
    /// 邮箱已存在时返回 [`Error::EmailTaken`]；哈希失败返回
    /// [`Error::PasswordHash`]，其余数据库错误返回 [`Error::Database`]
    pub async fn create(input: &CreateUser, pool: &PgPool) -> Result<Self> {
        let password_hash = hash_password(input.password.clone()).await?;

        sqlx::query_as!(
            Self,
            r#"
            INSERT INTO users (ws_id, fullname, email, password_hash)
            VALUES ($1, $2, $3, $4)
            RETURNING id, ws_id, fullname, email, created_at
            "#,
            0i64,
            input.fullname,
            input.email,
            password_hash
        )
        .fetch_one(pool)
        .await
        .map_err(|err| {
            if matches!(&err, sqlx::Error::Database(db) if db.is_unique_violation()) {
                Error::EmailTaken(input.email.clone())
            } else {
                Error::Database(err)
            }
        })
    }

    /// 按邮箱与口令校验用户。
    ///
    /// 校验不通过返回 `Ok(None)` 而不是错误——调用方必须把「邮箱不存在」和
    /// 「密码错误」合并成同一个响应，区分开等于告诉攻击者哪些邮箱注册过。
    ///
    /// # Errors
    ///
    /// 数据库查询失败或哈希校验过程本身出错时返回对应变体
    pub async fn verify(input: &SigninUser, pool: &PgPool) -> Result<Option<Self>> {
        let row = sqlx::query!(
            r#"
              SELECT id, ws_id, fullname, email, created_at, password_hash
              FROM users
              WHERE email = $1
              "#,
            &input.email
        )
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            // 邮箱不存在也走一次校验，把两条路径的耗时对齐
            verify_password(input.password.clone(), DUMMY_HASH.clone()).await?;
            return Ok(None);
        };

        if verify_password(input.password.clone(), row.password_hash).await? {
            Ok(Some(Self {
                id: row.id,
                ws_id: row.ws_id,
                fullname: row.fullname,
                email: row.email,
                created_at: row.created_at,
            }))
        } else {
            Ok(None)
        }
    }
}

/// 在阻塞线程池里做 argon2 哈希。
///
/// argon2 的默认参数（`m=19456, t=2`）在 release 下哈希约 31 ms、校验约 19 ms。
/// 这个慢是**故意**的——它抬高了离线爆破的成本。但直接在 async handler 里跑，
/// 就等于把一个 tokio 工作线程独占几十毫秒；并发一上来，同一线程上排队的
/// 其他请求会成片超时。放进 `spawn_blocking` 才不会拖累事件循环
async fn hash_password(password: String) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(Error::PasswordHash)
    })
    .await?
}

/// 在阻塞线程池里校验口令
async fn verify_password(password: String, hash: String) -> Result<bool> {
    tokio::task::spawn_blocking(move || {
        // 存储的哈希无法解析（例如迁移里的哨兵用户存的是空串）视为校验失败，
        // 而不是 500——否则 500 与 401 的差异会暴露账号是否存在
        let Ok(parsed) = PasswordHash::new(&hash) else {
            tracing::warn!("数据库中存在无法解析的密码哈希，该账号无法登录");
            return Ok(false);
        };

        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await?
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::{CreateUser, SigninUser, User};
    use crate::Error;

    fn create_input(email: &str) -> CreateUser {
        CreateUser {
            fullname: "张三".to_owned(),
            email: email.to_owned(),
            password: "hunter2".to_owned(),
        }
    }

    fn signin_input(email: &str, password: &str) -> SigninUser {
        SigninUser {
            email: email.to_owned(),
            password: password.to_owned(),
        }
    }

    #[test]
    fn debug_does_not_leak_password() {
        let rendered = format!("{:?}", create_input("a@b.com"));
        assert!(!rendered.contains("hunter2"), "Debug 输出泄漏了明文口令");
        assert!(rendered.contains("redacted"));
    }

    #[sqlx::test]
    async fn create_then_verify(pool: PgPool) {
        let created = User::create(&create_input("a@b.com"), &pool)
            .await
            .expect("应能创建用户");
        assert_eq!(created.email, "a@b.com");
        assert!(created.id > 0);

        let verified = User::verify(&signin_input("a@b.com", "hunter2"), &pool)
            .await
            .expect("校验过程不应出错")
            .expect("正确口令应能通过");
        assert_eq!(verified, created);
    }

    #[sqlx::test]
    async fn duplicate_email_is_rejected(pool: PgPool) {
        User::create(&create_input("a@b.com"), &pool).await.unwrap();
        let err = User::create(&create_input("a@b.com"), &pool)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::EmailTaken(email) if email == "a@b.com"));
    }

    #[sqlx::test]
    async fn wrong_password_is_rejected(pool: PgPool) {
        User::create(&create_input("a@b.com"), &pool).await.unwrap();
        let result = User::verify(&signin_input("a@b.com", "wrong"), &pool)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[sqlx::test]
    async fn unknown_email_is_rejected(pool: PgPool) {
        let result = User::verify(&signin_input("nobody@b.com", "hunter2"), &pool)
            .await
            .unwrap();
        assert!(result.is_none());
    }
}
