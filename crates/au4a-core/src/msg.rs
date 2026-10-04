//! PMB 信封与分帧：Agent-to-Agent 通信是第一优先级。
//!
//! * **规范 JSON，不是 bincode/CBOR**：可读、可 diff、可被任何语言复现，且签名建立在字节上。
//! * **4 字节大端长度前缀**，上限 1 MiB：超限在**分配之前**用前缀拒绝，不会先分配再报错。
//! * **先签名后发送**：`sig` 覆盖除自身以外的全部字段，包括内容寻址得到的 `id`。
//! * `to: null` 表示广播（能力图通告走这条路径），人类观察层是旁路订阅，不参与投递。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::canon::{canonical_hash, canonicalize};
use crate::did::{AgentKeys, Did};
use crate::error::{CoreError, CoreResult};

/// 单帧上限：1 MiB。
pub const MAX_FRAME: usize = 1024 * 1024;

/// 消息类型名。刻意做成受校验的字符串而不是封闭枚举：
/// 后续轨道（协商、安全、治理）可以引入新的消息类型而不必修改冻结基元。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MsgKind(String);

impl MsgKind {
    pub fn new(s: &str) -> CoreResult<Self> {
        let bytes = s.as_bytes();
        let ok_len = !bytes.is_empty() && bytes.len() <= 64;
        let ok_head = bytes
            .first()
            .map(|b| b.is_ascii_lowercase())
            .unwrap_or(false);
        let ok_rest = bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'.' || *b == b'_');
        if ok_len && ok_head && ok_rest {
            Ok(MsgKind(s.to_string()))
        } else {
            Err(CoreError::InvalidKind)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 基元层已知的消息类型（后续轨道可扩展，不在此列也不违规）。
pub mod kinds {
    pub const AGENT_REGISTER: &str = "agent.register";
    pub const AGENT_CARD: &str = "agent.card";
    pub const AGENT_LEAVE: &str = "agent.leave";
    pub const PROGRESS_EVENT: &str = "progress.event";
    pub const SAFETY_REPORT: &str = "safety.report";
}

/// 一个 PMB 信封。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// 内容寻址 id：对未签名部分做规范哈希。
    pub id: String,
    pub from: Did,
    /// `None` 表示广播。
    pub to: Option<Did>,
    pub kind: MsgKind,
    /// 逻辑时钟读数（基元层不读墙钟）。
    pub ts: u64,
    pub in_reply_to: Option<String>,
    pub body: Value,
    /// hex 编码的 Ed25519 签名；空串表示未签名。
    pub sig: String,
}

impl Envelope {
    /// 构造未签名信封（`id` 与 `sig` 由 [`Envelope::seal`] 填充）。
    pub fn new(
        from: Did,
        to: Option<Did>,
        kind: &str,
        ts: u64,
        in_reply_to: Option<String>,
        body: Value,
    ) -> CoreResult<Self> {
        Ok(Self {
            id: String::new(),
            from,
            to,
            kind: MsgKind::new(kind)?,
            ts,
            in_reply_to,
            body,
            sig: String::new(),
        })
    }

    /// 被签名的载荷：除 `sig` 外的全部字段（含 `id`）。
    pub fn signing_payload(&self) -> CoreResult<Value> {
        Ok(json!({
            "id": self.id,
            "from": self.from,
            "to": self.to,
            "kind": self.kind,
            "ts": self.ts,
            "in_reply_to": self.in_reply_to,
            "body": self.body,
        }))
    }

    /// 计算内容寻址 id（除 `id`/`sig` 外全部字段）。
    pub fn compute_id(&self) -> CoreResult<String> {
        let payload = json!({
            "from": self.from,
            "to": self.to,
            "kind": self.kind,
            "ts": self.ts,
            "in_reply_to": self.in_reply_to,
            "body": self.body,
        });
        canonical_hash(&payload)
    }

    /// 签名并封口。id 在签名前确定，因此签名覆盖 id。
    pub fn seal(mut self, keys: &AgentKeys) -> CoreResult<Self> {
        if self.from != keys.did() {
            return Err(CoreError::InvalidSignature);
        }
        self.id = self.compute_id()?;
        let payload = self.signing_payload()?;
        self.sig = keys.sign_json(&payload)?;
        Ok(self)
    }

    /// 校验：id 必须自洽，签名必须由 `from` 出具。
    pub fn verify(&self) -> CoreResult<()> {
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        if self.compute_id()? != self.id {
            return Err(CoreError::InvalidSignature);
        }
        let payload = self.signing_payload()?;
        let bytes = canonicalize(&payload)?;
        self.from.verify(bytes.as_bytes(), &self.sig)
    }

    pub fn to_canonical(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self).map_err(|_| CoreError::Encoding)?;
        canonicalize(&value)
    }

    pub fn from_canonical(s: &str) -> CoreResult<Self> {
        serde_json::from_str(s).map_err(|_| CoreError::Encoding)
    }
}

/// 编码为线格式：4 字节大端长度前缀 + 规范 JSON。
pub fn encode_frame(env: &Envelope) -> CoreResult<Vec<u8>> {
    let payload = env.to_canonical()?;
    let bytes = payload.as_bytes();
    if bytes.len() > MAX_FRAME {
        return Err(CoreError::FrameTooLarge);
    }
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(out)
}

/// 解码：先读前缀，超限立刻拒绝（不分配负载），再校验长度一致。
pub fn decode_frame(buf: &[u8]) -> CoreResult<Envelope> {
    if buf.len() < 4 {
        return Err(CoreError::FrameTruncated);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > MAX_FRAME {
        return Err(CoreError::FrameTooLarge);
    }
    if buf.len() != 4 + len {
        return Err(CoreError::FrameTruncated);
    }
    let text = std::str::from_utf8(&buf[4..]).map_err(|_| CoreError::Encoding)?;
    Envelope::from_canonical(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    #[test]
    fn sealed_envelope_verifies_and_is_content_addressed() {
        let a = agent(1);
        let b = agent(2);
        let env = Envelope::new(
            a.did(),
            Some(b.did()),
            kinds::AGENT_REGISTER,
            1,
            None,
            json!({"skills": ["translate"]}),
        )
        .unwrap()
        .seal(&a)
        .unwrap();
        env.verify().unwrap();
        let again = env.clone().seal(&a).unwrap();
        assert_eq!(again.id, env.id);
    }

    #[test]
    fn body_tampering_is_detected() {
        let a = agent(3);
        let mut env = Envelope::new(a.did(), None, kinds::PROGRESS_EVENT, 1, None, json!({"p": 1}))
            .unwrap()
            .seal(&a)
            .unwrap();
        env.body = json!({"p": 2});
        assert!(env.verify().is_err());
    }

    #[test]
    fn someone_elses_signature_is_refused() {
        let a = agent(4);
        let b = agent(5);
        let mut env = Envelope::new(a.did(), None, kinds::PROGRESS_EVENT, 1, None, json!({}))
            .unwrap()
            .seal(&a)
            .unwrap();
        env.sig = b.sign(b"whatever");
        assert_eq!(env.verify(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn frame_roundtrip() {
        let a = agent(6);
        let env = Envelope::new(a.did(), Some(agent(7).did()), kinds::AGENT_CARD, 9, None, json!({"x": 1}))
            .unwrap()
            .seal(&a)
            .unwrap();
        let frame = encode_frame(&env).unwrap();
        assert_eq!(decode_frame(&frame).unwrap(), env);
    }

    #[test]
    fn oversized_frame_is_refused_from_the_prefix_before_allocation() {
        let mut header = Vec::from((MAX_FRAME as u32 + 1).to_be_bytes());
        header.extend_from_slice(b"{}");
        assert_eq!(decode_frame(&header), Err(CoreError::FrameTooLarge));
    }

    #[test]
    fn truncated_frames_are_refused() {
        assert_eq!(decode_frame(&[0, 0]), Err(CoreError::FrameTruncated));
        assert_eq!(decode_frame(&[0, 0, 0, 5, b'{']), Err(CoreError::FrameTruncated));
    }

    #[test]
    fn kind_names_are_validated() {
        assert!(MsgKind::new("negotiate.propose").is_ok());
        assert!(MsgKind::new("").is_err());
        assert!(MsgKind::new("Upper").is_err());
        assert!(MsgKind::new("has space").is_err());
        assert!(MsgKind::new(&"a".repeat(65)).is_err());
    }

    #[test]
    fn unsigned_envelope_is_refused() {
        let a = agent(9);
        let env = Envelope::new(a.did(), None, kinds::AGENT_CARD, 1, None, json!({})).unwrap();
        assert_eq!(env.verify(), Err(CoreError::NotSealed));
    }
}
