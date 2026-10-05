//! 自证身份：`did:au4a:<32 字节 Ed25519 公钥的 64 位小写 hex>`。
//!
//! 没有注册中心、没有解析器、没有人类账户：DID 本身就是公钥，验签就是鉴权。
//! 这是「Agent 自主身份」在代码里的最小实现——Agent 可以自己生成密钥、自己加入网络，
//! 不需要任何人类批准。

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

pub const DID_PREFIX: &str = "did:au4a:";

/// 自证 DID。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Did(String);

/// v2.2.0 修复（P1）：**serde 路径必须经过 `parse()`**。
///
/// 修复前 `Did` 只是 `derive(Deserialize)` 的新类型，反序列化不走校验，于是信封体、注册请求、
/// 状态导入这些**全部走 serde 的入口**都能造出 `Did("garbage")`：类型成立、校验不成立。
/// 它不能伪造签名（`public_key()` 会失败），但会让非法身份先被写进状态、更晚才炸，
/// 并污染任何以 `Did` 为键的去重/计费。
impl TryFrom<String> for Did {
    type Error = CoreError;

    fn try_from(s: String) -> CoreResult<Self> {
        Did::parse(&s)
    }
}

impl From<Did> for String {
    fn from(d: Did) -> String {
        d.0
    }
}

impl Did {
    /// 解析并校验 `did:au4a:<64 hex>`。
    pub fn parse(s: &str) -> CoreResult<Self> {
        let hex_part = s.strip_prefix(DID_PREFIX).ok_or(CoreError::InvalidDid)?;
        if hex_part.len() != 64
            || !hex_part
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(CoreError::InvalidDid);
        }
        Ok(Did(s.to_string()))
    }

    /// 由公钥导出 DID。
    pub fn from_key(key: &VerifyingKey) -> Self {
        Did(format!("{DID_PREFIX}{}", hex::encode(key.to_bytes())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 取回公钥。DID 与公钥是同一件事的两种表示。
    pub fn public_key(&self) -> CoreResult<VerifyingKey> {
        let hex_part = self
            .0
            .strip_prefix(DID_PREFIX)
            .ok_or(CoreError::InvalidDid)?;
        let bytes = hex::decode(hex_part).map_err(|_| CoreError::InvalidDid)?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::InvalidDid)?;
        VerifyingKey::from_bytes(&arr).map_err(|_| CoreError::InvalidDid)
    }

    /// 验证一段字节上的签名（hex 编码的 64 字节）。
    pub fn verify(&self, msg: &[u8], sig_hex: &str) -> CoreResult<()> {
        let bytes = hex::decode(sig_hex).map_err(|_| CoreError::InvalidSignature)?;
        let arr: [u8; 64] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::InvalidSignature)?;
        let sig = Signature::from_bytes(&arr);
        self.public_key()?
            .verify(msg, &sig)
            .map_err(|_| CoreError::InvalidSignature)
    }
}

impl std::fmt::Display for Did {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Agent 的私钥持有者。私钥不出结构体，签名是唯一出口。
pub struct AgentKeys {
    signing: SigningKey,
}

impl AgentKeys {
    /// 自主生成：Agent 自己决定何时加入网络。
    pub fn generate() -> Self {
        Self {
            signing: SigningKey::generate(&mut OsRng),
        }
    }

    /// 从 32 字节种子确定性生成——测试与重放需要可复现的身份。
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(seed),
        }
    }

    pub fn did(&self) -> Did {
        Did::from_key(&self.signing.verifying_key())
    }

    /// 对字节签名，返回 hex。
    pub fn sign(&self, msg: &[u8]) -> String {
        hex::encode(self.signing.sign(msg).to_bytes())
    }

    /// 对 JSON 值的规范字节签名。
    pub fn sign_json(&self, value: &serde_json::Value) -> CoreResult<String> {
        let bytes = crate::canon::canonicalize(value)?;
        Ok(self.sign(bytes.as_bytes()))
    }

    /// 导出种子（仅用于本地持久化；调用方负责保护）。
    pub fn seed(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }
}

impl std::fmt::Debug for AgentKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AgentKeys({})", self.did())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_is_self_certifying() {
        let keys = AgentKeys::generate();
        let did = keys.did();
        assert!(did.as_str().starts_with(DID_PREFIX));
        assert_eq!(Did::parse(did.as_str()).unwrap(), did);
        assert_eq!(did.public_key().unwrap(), keys.signing.verifying_key());
    }

    #[test]
    fn seed_is_deterministic() {
        let a = AgentKeys::from_seed(&[7u8; 32]);
        let b = AgentKeys::from_seed(&[7u8; 32]);
        assert_eq!(a.did(), b.did());
    }

    #[test]
    fn signature_roundtrip_and_tamper_detection() {
        let keys = AgentKeys::from_seed(&[3u8; 32]);
        let did = keys.did();
        let sig = keys.sign(b"au4a");
        assert!(did.verify(b"au4a", &sig).is_ok());
        assert_eq!(did.verify(b"au4b", &sig), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn malformed_dids_are_refused() {
        for bad in [
            "did:au4a:",
            "did:au4a:zz",
            "did:au4a:00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF",
            "did:key:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        ] {
            assert_eq!(Did::parse(bad), Err(CoreError::InvalidDid), "{bad}");
        }
    }

    // ── v2.2.0 回归测试 ──────────────────────────────────────────────────────

    #[test]
    fn unvalidated_did_is_refused_by_serde() {
        // P1 回归：修复前 `Did` 只有 derive(Deserialize)，可反序列化出未校验的 DID。
        for bad in [
            "\"garbage\"",
            "\"did:au4a:\"",
            "\"did:au4a:zz\"",
            "\"did:au4a:00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF\"",
        ] {
            assert!(
                serde_json::from_str::<Did>(bad).is_err(),
                "serde 必须拒绝未校验的 DID: {bad}"
            );
        }
        let keys = AgentKeys::from_seed(&[9u8; 32]);
        let json = serde_json::to_string(&keys.did()).unwrap();
        assert_eq!(serde_json::from_str::<Did>(&json).unwrap(), keys.did());
    }

    #[test]
    fn did_parse_and_serde_agree() {
        // 两条入口必须同判：`parse()` 与 serde 路径对同一字符串给出同一结论。
        let good = AgentKeys::from_seed(&[11u8; 32]).did();
        let cases = [
            good.as_str().to_string(),
            "did:au4a:".to_string(),
            "did:au4a:zz".to_string(),
            "did:key:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff".to_string(),
        ];
        for s in cases {
            let parsed = Did::parse(&s).is_ok();
            let quoted = serde_json::to_string(&s).unwrap();
            let via_serde = serde_json::from_str::<Did>(&quoted).is_ok();
            assert_eq!(parsed, via_serde, "parse 与 serde 判定分歧: {s}");
        }
    }
}
