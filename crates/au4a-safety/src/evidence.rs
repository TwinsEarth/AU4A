//! v1.5.2 证据承诺：举报与申诉必须携带**可复算**的证据哈希。
//!
//! 「带证据哈希」如果只是把一个字符串塞进消息里，那它证明不了任何事。AU4A 的做法是
//! **承诺 + 复算**：证据本体用规范 JSON（或原始字节）算出 SHA-256，摘要随举报提交，
//! 安全服务收到后重新计算一次。对不上就是伪造，摘要不成形就是缺失——两者都必须拒绝。
//!
//! 摘要不是自由文本，也不参与经济计算：它只回答「这份证据是不是你说的那一份」。

use au4a_core::{canonical_hash, content_hash, CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 证据种类。新增种类是接口扩展。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// PMB 信封（或其线格式字节）。
    Envelope,
    /// 交互记录 / 交付记录。
    Transcript,
    /// 账本状态证明（守恒式快照）。
    LedgerProof,
    /// 第三方见证签名材料。
    Witness,
}

impl EvidenceKind {
    pub const ALL: [EvidenceKind; 4] = [
        EvidenceKind::Envelope,
        EvidenceKind::Transcript,
        EvidenceKind::LedgerProof,
        EvidenceKind::Witness,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::Envelope => "envelope",
            EvidenceKind::Transcript => "transcript",
            EvidenceKind::LedgerProof => "ledger_proof",
            EvidenceKind::Witness => "witness",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        EvidenceKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// 证据引用：种类 + 内容摘要（小写 hex 的 SHA-256）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub digest: String,
}

impl EvidenceRef {
    /// 对规范 JSON 做承诺。浮点会被基元层拒绝（不参与任何度量）。
    pub fn commit(kind: EvidenceKind, payload: &Value) -> CoreResult<Self> {
        Ok(Self {
            kind,
            digest: canonical_hash(payload)?,
        })
    }

    /// 对原始字节做承诺（例如 PMB 线格式帧）。
    pub fn commit_bytes(kind: EvidenceKind, bytes: &[u8]) -> Self {
        Self {
            kind,
            digest: content_hash(bytes),
        }
    }

    /// 复算：摘要必须成形，且与载荷一致。
    pub fn verify(&self, payload: &Value) -> CoreResult<()> {
        if !self.is_well_formed() {
            return Err(CoreError::InvalidSignature);
        }
        if canonical_hash(payload)? == self.digest {
            Ok(())
        } else {
            Err(CoreError::InvalidSignature)
        }
    }

    pub fn verify_bytes(&self, bytes: &[u8]) -> CoreResult<()> {
        if self.is_well_formed() && content_hash(bytes) == self.digest {
            Ok(())
        } else {
            Err(CoreError::InvalidSignature)
        }
    }

    pub fn is_well_formed(&self) -> bool {
        is_lower_hex64(&self.digest)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 64 位小写 hex。大小写混写不接受：规范形式只有一种。
pub fn is_lower_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 一组证据的准入检查：非空，且每一个摘要都成形。
pub fn require_well_formed(refs: &[EvidenceRef]) -> CoreResult<()> {
    if refs.is_empty() {
        return Err(CoreError::InvalidSignature);
    }
    if refs.iter().all(|r| r.is_well_formed()) {
        Ok(())
    } else {
        Err(CoreError::InvalidSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commitment_roundtrips_and_detects_tampering() {
        let payload = json!({"delivered": false, "task": "t-1", "amount": 250});
        let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload).unwrap();
        assert!(reference.is_well_formed());
        reference.verify(&payload).unwrap();

        // 改一个字节就复算失败：伪造的证据进不来。
        let tampered = json!({"delivered": false, "task": "t-1", "amount": 251});
        assert_eq!(
            reference.verify(&tampered),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn malformed_or_empty_digests_are_refused() {
        let payload = json!({"a": 1});
        for bad in [
            "".to_string(),
            "deadbeef".to_string(),
            "A".repeat(64),
            "g".repeat(64),
            format!("{}0", "a".repeat(64)),
        ] {
            let reference = EvidenceRef {
                kind: EvidenceKind::Witness,
                digest: bad.clone(),
            };
            assert!(!reference.is_well_formed(), "{bad} 不应成形");
            assert_eq!(
                reference.verify(&payload),
                Err(CoreError::InvalidSignature),
                "{bad}"
            );
        }
        // 大小写混写同样不被接受（规范形式唯一）。
        let mixed = format!("{}{}", "a".repeat(63), "F");
        assert!(!is_lower_hex64(&mixed));
    }

    #[test]
    fn byte_commitment_matches_the_frozen_frame_hash() {
        let frame = b"\x00\x00\x00\x02{}";
        let reference = EvidenceRef::commit_bytes(EvidenceKind::Envelope, frame);
        reference.verify_bytes(frame).unwrap();
        assert_eq!(
            reference.verify_bytes(b"\x00\x00\x00\x02{\"}\""),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn require_well_formed_rejects_empty_and_partial_bundles() {
        let good = EvidenceRef::commit(EvidenceKind::Transcript, &json!({"x": 1})).unwrap();
        assert_eq!(require_well_formed(&[]), Err(CoreError::InvalidSignature));
        require_well_formed(&[good.clone()]).unwrap();
        let bad = EvidenceRef {
            kind: EvidenceKind::Transcript,
            digest: "not-a-digest".to_string(),
        };
        assert_eq!(
            require_well_formed(&[good, bad]),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn kinds_roundtrip_and_are_unique() {
        assert_eq!(EvidenceKind::ALL.len(), 4);
        for kind in EvidenceKind::ALL {
            assert_eq!(EvidenceKind::parse(kind.as_str()), Some(kind));
            let reference = EvidenceRef {
                kind,
                digest: "0".repeat(64),
            };
            let round = EvidenceRef::from_json(&reference.to_json().unwrap()).unwrap();
            assert_eq!(round, reference);
        }
        assert_eq!(EvidenceKind::parse("vibes"), None);
    }

    #[test]
    fn floats_are_refused_at_commitment_time() {
        let floaty = json!({"rate": 1.5});
        assert_eq!(
            EvidenceRef::commit(EvidenceKind::Transcript, &floaty),
            Err(CoreError::FloatForbidden)
        );
    }
}
