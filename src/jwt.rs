//! Ed25519（EdDSA）JWT 的签发与校验。
//!
//! 只支持 `EdDSA` 一种算法：JWT 最经典的「`alg` 混淆攻击」来自实现按 header 里
//! 声明的算法去挑验证方式，攻击者把 `alg` 改成 `none` 或对称算法就能伪造。
//! 这里的验证路径写死了 Ed25519，header 里的 `alg` 只被拿来**核对**，不参与选择。

use std::{fmt, time::Duration};

use base64ct::{Base64UrlUnpadded, Encoding};
use chrono::Utc;
use ed25519_dalek::{
    Signature, Signer, SigningKey, VerifyingKey,
    pkcs8::{DecodePrivateKey as _, DecodePublicKey},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const ALG: &str = "EdDSA";
const TYP: &str = "JWT";
const ISSUER: &str = "chat_server";
const AUDITENCE: &str = "chat_web";

/// JWT 层的错误。刻意不合并进 [`crate::Error`]，让这一层可以独立测试
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JwtError {
    /// 私钥 PEM 无法解析
    #[error("密钥 PEM 无法解析")]
    Pem(#[from] ed25519_dalek::pkcs8::Error),

    /// 公钥 PEM 无法解析（公钥走 SPKI，错误类型和私钥不同)
    #[error("公钥 PEM 无法解析")]
    SpkiPem(#[from] ed25519_dalek::pkcs8::spki::Error),

    /// token 不是三段式结构
    #[error("token 结构不合法：应为三段以 . 分隔")]
    Malformed,

    /// 某一段不是合法的 base64url
    #[error("token 的 base64 段无法解码")]
    Base64,

    /// header 或 payload 不是合法 JSON，或字段对不上
    #[error("token 的 JSON 段无法解析")]
    Json(#[from] serde_json::Error),

    /// 签名校验不通过：被篡改，或不是这把私钥签的
    #[error("签名校验失败")]
    BadSignature,

    /// header 里声明了别的算法
    #[error("不支持的算法 {0}，只接受 EdDSA")]
    UnsupportedAlg(String),

    /// 已过期。
    #[error("token 已过期")]
    Expired,

    /// 签发者不是本服务。
    #[error("签发者不匹配")]
    BadIssuer,

    /// 受众不是本服务的客户端
    #[error("受众不匹配")]
    BadAudience,

    /// 传入的有效期换算成秒之后溢出了。
    #[error("过期时间超出可表示范围")]
    TtlOutOfRange,
}

type Result<T> = core::result::Result<T, JwtError>;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    alg: String,
    typ: String,
}

/// 标准 claims + 业务自定义 claims，序列化后平铺成同一层 JSON
#[derive(Debug, Serialize, Deserialize)]
struct Payload<T> {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    #[serde(flatten)]
    custom: T,
}

/// 签发用的私钥
pub struct EncodingKey(SigningKey);

// 不 derive Debug：私钥绝不能因为某处 `{:?}` 就进了日志
impl fmt::Debug for EncodingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EncodingKey(<redacted>)")
    }
}

impl EncodingKey {
    /// 从 PKCS#8 PEM 加载私钥。
    ///
    /// # Errors
    ///
    /// PEM 格式不对或不是 Ed25519 私钥时返回 [`JwtError::Pem`]。
    pub fn load_pem(pem: &str) -> Result<Self> {
        Ok(Self(SigningKey::from_pkcs8_pem(pem)?))
    }

    /// 签发一个有效期为 `ttl` 的 token。
    ///
    /// # Errors
    ///
    /// 自定义 claims 无法序列化时返回 [`JwtError::Json`]；
    /// `ttl` 换算成秒后溢出 `i64` 时返回 [`JwtError::TtlOutOfRange`]
    pub fn sign<T: Serialize>(&self, custom: &T, ttl: Duration) -> Result<String> {
        let now = Utc::now().timestamp();
        let ttl = i64::try_from(ttl.as_secs()).map_err(|_| JwtError::TtlOutOfRange)?;
        let exp = now.checked_add(ttl).ok_or(JwtError::TtlOutOfRange)?;

        let header = Header {
            alg: ALG.to_owned(),
            typ: TYP.to_owned(),
        };
        let payload = Payload {
            iss: ISSUER.to_owned(),
            aud: AUDITENCE.to_owned(),
            iat: now,
            exp,
            custom,
        };

        let signing_input = format!(
            "{}.{}",
            Base64UrlUnpadded::encode_string(&serde_json::to_vec(&header)?),
            Base64UrlUnpadded::encode_string(&serde_json::to_vec(&payload)?),
        );
        let signature = self.0.sign(signing_input.as_bytes());
        Ok(format!(
            "{signing_input}.{}",
            Base64UrlUnpadded::encode_string(&signature.to_bytes())
        ))
    }
}

/// 校验用的公钥。
///
/// 公钥就是 32 字节，`Copy` 是合理的；私钥那边刻意不给 `Copy`，
/// 免得密钥材料被无意复制得到处都是
#[derive(Debug, Clone, Copy)]
pub struct DecodingKey(VerifyingKey);

impl DecodingKey {
    /// 从 SPKI PEM 加载公钥。
    ///
    /// # Errors
    /// PEM 格式不对或不是 Ed25519 公钥时返回 [`JwtError::SpkiPem`]
    pub fn load_pem(pem: &str) -> Result<Self> {
        Ok(Self(VerifyingKey::from_public_key_pem(pem)?))
    }

    /// 校验 token 并取出自定义 claims。
    ///
    /// # Errors
    ///
    /// 结构、签名、算法、有效期、签发者、受众任一不通过都会返回对应的
    /// [`JwtError`] 变体
    pub fn verify<T: DeserializeOwned>(&self, token: &str) -> Result<T> {
        let mut parts = token.split('.');
        let (Some(h), Some(p), Some(s), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(JwtError::Malformed);
        };

        // 先验签名，再碰内容：没验过的 JSON 不解析、不进业务逻辑
        let signing_input = &token[..h.len() + 1 + p.len()];
        let sig_bytes = Base64UrlUnpadded::decode_vec(s).map_err(|_| JwtError::Base64)?;
        let signature = Signature::from_slice(&sig_bytes).map_err(|_| JwtError::BadSignature)?;
        self.0
            .verify_strict(signing_input.as_bytes(), &signature)
            .map_err(|_| JwtError::BadSignature)?;

        let header: Header = serde_json::from_slice(
            &Base64UrlUnpadded::decode_vec(h).map_err(|_| JwtError::Base64)?,
        )?;
        if header.alg != ALG {
            return Err(JwtError::UnsupportedAlg(header.alg));
        }

        let payload: Payload<T> = serde_json::from_slice(
            &Base64UrlUnpadded::decode_vec(p).map_err(|_| JwtError::Base64)?,
        )?;
        let now = Utc::now().timestamp();
        if payload.exp <= now {
            return Err(JwtError::Expired);
        }
        if payload.iss != ISSUER {
            return Err(JwtError::BadIssuer);
        }
        if payload.aud != AUDITENCE {
            return Err(JwtError::BadAudience);
        }

        Ok(payload.custom)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use base64ct::{Base64UrlUnpadded, Encoding as _};
    use ed25519_dalek::{
        SigningKey,
        pkcs8::{EncodePrivateKey as _, EncodePublicKey as _, spki::der::pem::LineEnding},
    };
    use serde::{Deserialize, Serialize};

    use super::{DecodingKey, EncodingKey, JwtError};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Claim {
        id: i64,
        email: String,
    }

    /// 用固定字节在内存里造一对测试密钥。
    ///
    /// 不落盘：pre-commit 钩子会拦下任何含 `BEGIN ... PRIVATE KEY` 的新增行。
    /// 固定种子还带来一个好处——测试完全可复现。
    fn test_keys(seed: u8) -> (EncodingKey, DecodingKey) {
        let sk = SigningKey::from_bytes(&[seed; 32]);
        let sk_pem = sk.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let pk_pem = sk
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        (
            EncodingKey::load_pem(&sk_pem).unwrap(),
            DecodingKey::load_pem(&pk_pem).unwrap(),
        )
    }

    fn claim() -> Claim {
        Claim {
            id: 7,
            email: "z@s.com".to_owned(),
        }
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let (enc, dec) = test_keys(7);
        let token = enc.sign(&claim(), Duration::from_secs(90)).unwrap();
        assert_eq!(token.split('.').count(), 3);
        assert_eq!(dec.verify::<Claim>(&token).unwrap(), claim());
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let (enc, dec) = test_keys(7);
        let token = enc.sign(&claim(), Duration::from_secs(90)).unwrap();
        let parts: Vec<_> = token.split('.').collect();

        let mut payload: serde_json::Value =
            serde_json::from_slice(&Base64UrlUnpadded::decode_vec(parts[1]).unwrap()).unwrap();
        payload["id"] = serde_json::json!(999);
        let forged = format!(
            "{}.{}.{}",
            parts[0],
            Base64UrlUnpadded::encode_string(&serde_json::to_vec(&payload).unwrap()),
            parts[2],
        );

        assert!(matches!(
            dec.verify::<Claim>(&forged),
            Err(JwtError::BadSignature)
        ));
    }

    #[test]
    fn expired_token_is_rejected() {
        let (enc, dec) = test_keys(7);
        let token = enc.sign(&claim(), Duration::ZERO).unwrap();
        assert!(matches!(
            dec.verify::<Claim>(&token),
            Err(JwtError::Expired)
        ));
    }

    #[test]
    fn token_from_another_key_is_rejected() {
        let (enc, _) = test_keys(7);
        let (_, dec) = test_keys(9);
        let token = enc.sign(&claim(), Duration::from_secs(90)).unwrap();
        assert!(matches!(
            dec.verify::<Claim>(&token),
            Err(JwtError::BadSignature)
        ));
    }

    #[test]
    fn malformed_token_is_rejected() {
        let (_, dec) = test_keys(7);
        assert!(matches!(
            dec.verify::<Claim>("only.two"),
            Err(JwtError::Malformed)
        ));
    }
}
