//! 快照签名与验证（v1.3.3）：内容寻址 + Ed25519，篡改或替换必须被拒。
//!
//! 内容寻址解决「这份字节有没有被改」，签名解决「这份字节是谁说的」。两者缺一不可：
//!
//! * 只有哈希 → 攻击者改完内容再改哈希，谁都能伪造一个自洽的快照。
//! * 只有签名 → 攻击者可以把**另一份**合法签名的快照塞进来（重放/替换），签名依然有效。
//!
//! 因此 AU4A 把「签名覆盖的内容根」和「调用方期望的内容根」分开：
//! [`SignedSnapshot::verify`] 证明签名有效；[`SignedSnapshot::verify_policy`] 证明
//! **这正是我要的那份**（agent + content_root + 最小 epoch）。只有后者返回
//! [`VerifiedSnapshot`]——那个类型的存在本身即证据。
//!
//! 签名主体是 **Agent 自己**（不是节点、不是运营方）：状态是 Agent 的主体延伸，
//! 只有它能对自己的状态签字。`sign` 因此拒绝「替别人签」。

use au4a_core::{canonicalize, AgentKeys, CoreError, CoreResult, Did};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use crate::snapshot::StateSnapshot;
use crate::transfer::{decode_frame, encode_frame};

/// 签名载荷的版本号。升级载荷结构 = 新版本号，老验证器会明确拒绝而不是误解。
pub const SIGNED_VERSION: u64 = 1;

/// 一份带签名的快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SignedSnapshot {
    version: u64,
    snapshot: StateSnapshot,
    signer: Did,
    sig: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedWire {
    version: u64,
    snapshot: StateSnapshot,
    signer: Did,
    sig: String,
}

impl SignedSnapshot {
    /// 用自己的密钥给自己的状态签字。替别人签 = `InvalidSignature`。
    pub fn sign(snapshot: StateSnapshot, keys: &AgentKeys) -> CoreResult<Self> {
        if snapshot.agent() != &keys.did() {
            return Err(CoreError::InvalidSignature);
        }
        let body = signing_body(&snapshot)?;
        let sig = keys.sign_json(&body)?;
        Ok(Self {
            version: SIGNED_VERSION,
            snapshot,
            signer: keys.did(),
            sig,
        })
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn snapshot(&self) -> &StateSnapshot {
        &self.snapshot
    }

    pub fn signer(&self) -> &Did {
        &self.signer
    }

    pub fn sig(&self) -> &str {
        &self.sig
    }

    /// 签名载荷：绑定 agent + 内容根 + 文档根 + 版本。
    /// 文档根已经覆盖 node/epoch 与全部块摘要，所以改任何一个字节都会让载荷变化。
    pub fn body(&self) -> CoreResult<Value> {
        signing_body(&self.snapshot)
    }

    /// 只验证「这份快照自洽且签名由 signer 出具」。
    ///
    /// 注意它**不能**证明「这就是我要的那份状态」——那是 `verify_policy` 的职责。
    pub fn verify(&self) -> CoreResult<()> {
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        if self.version != SIGNED_VERSION {
            return Err(CoreError::InvalidVersion);
        }
        // 声称的签名者必须就是快照的主人：签名者与主体分离即是伪造。
        if &self.signer != self.snapshot.agent() {
            return Err(CoreError::InvalidSignature);
        }
        Did::parse(self.signer.as_str())?;
        self.snapshot.verify()?;
        let bytes = canonicalize(&self.body()?)?;
        self.signer.verify(bytes.as_bytes(), &self.sig)
    }

    /// 按策略验证：签名有效 **且** 是我期望的 agent / 内容 / 不早于某时刻。
    /// 通过后返回 [`VerifiedSnapshot`]（只有验过的快照才有这个类型）。
    pub fn verify_policy(&self, policy: &SnapshotPolicy) -> CoreResult<VerifiedSnapshot> {
        self.verify()?;
        if self.snapshot.agent() != &policy.agent {
            // 换了一个 Agent 的状态：这不是「过期」，是冒名。
            return Err(CoreError::InvalidDid);
        }
        let content_root = self.snapshot.content_root()?;
        if let Some(expected) = &policy.content_root {
            if &content_root != expected {
                // 签名有效但内容不是我要的 → 被替换/被重放，必须拒绝。
                return Err(CoreError::InvalidSignature);
            }
        }
        if let Some(min_epoch) = policy.min_epoch {
            if self.snapshot.epoch() < min_epoch {
                // 合法但过期：竞争语义（StaleEpoch），不是恶意。
                return Err(CoreError::InvalidVersion);
            }
        }
        Ok(VerifiedSnapshot {
            snapshot: self.snapshot.clone(),
            signer: self.signer.clone(),
            content_root,
        })
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 反序列化即验证：**格式不对、结构被改、签名不符**都在这里被拒。
    /// 不做「先收下再想办法」——那正是把不可信状态放进系统的路径。
    pub fn from_value(value: &Value) -> CoreResult<Self> {
        let wire: SignedWire =
            serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)?;
        let signed = Self {
            version: wire.version,
            snapshot: wire.snapshot,
            signer: wire.signer,
            sig: wire.sig,
        };
        signed.verify()?;
        Ok(signed)
    }

    /// 线格式：与 v1.3.2 的分帧一致（4 字节大端长度前缀 + 规范 JSON，1 MiB 上限）。
    pub fn to_frame(&self) -> CoreResult<Vec<u8>> {
        encode_frame(&self.to_value()?)
    }

    pub fn from_frame(buf: &[u8]) -> CoreResult<Self> {
        Self::from_value(&decode_frame(buf)?)
    }
}

impl<'de> Deserialize<'de> for SignedSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error as DeError;
        let wire = SignedWire::deserialize(deserializer)?;
        let signed = SignedSnapshot {
            version: wire.version,
            snapshot: wire.snapshot,
            signer: wire.signer,
            sig: wire.sig,
        };
        signed.verify().map_err(D::Error::custom)?;
        Ok(signed)
    }
}

fn signing_body(snapshot: &StateSnapshot) -> CoreResult<Value> {
    Ok(json!({
        "agent": snapshot.agent(),
        "content_root": snapshot.content_root()?,
        "root": snapshot.root(),
        "v": SIGNED_VERSION,
    }))
}

/// 验证策略：**这是谁的状态、是不是我要的那份、够不够新**。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPolicy {
    pub agent: Did,
    /// 期望的状态内容根；`None` 表示只验签名与主体。
    pub content_root: Option<String>,
    /// 可接受的最小逻辑时刻。
    pub min_epoch: Option<u64>,
}

impl SnapshotPolicy {
    pub fn for_agent(agent: Did) -> Self {
        Self {
            agent,
            content_root: None,
            min_epoch: None,
        }
    }

    pub fn expecting_content_root(mut self, content_root: impl Into<String>) -> Self {
        self.content_root = Some(content_root.into());
        self
    }

    pub fn with_min_epoch(mut self, min_epoch: u64) -> Self {
        self.min_epoch = Some(min_epoch);
        self
    }
}

/// 验证通过的快照。类型即证据：构造它的唯一路径是 [`SignedSnapshot::verify_policy`]。
///
/// 刻意**不实现 `Deserialize`**：如果它能从 JSON 造出来，「验过的快照」就不再是证据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VerifiedSnapshot {
    snapshot: StateSnapshot,
    signer: Did,
    content_root: String,
}

impl VerifiedSnapshot {
    pub fn snapshot(&self) -> &StateSnapshot {
        &self.snapshot
    }

    pub fn signer(&self) -> &Did {
        &self.signer
    }

    pub fn content_root(&self) -> &str {
        &self.content_root
    }

    /// 取出内部快照（用于落到目标节点）。
    pub fn into_snapshot(self) -> StateSnapshot {
        self.snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{StateBlock, StateZone};
    use serde_json::json;

    fn keys(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn snap(keys: &AgentKeys, node: &str) -> StateSnapshot {
        let blocks = vec![
            StateBlock::new(StateZone::Fs, "/a", json!({"n": 1})).unwrap(),
            StateBlock::new(StateZone::Memory, "m", json!("v")).unwrap(),
            StateBlock::new(StateZone::Context, "goal", json!({"g": 1})).unwrap(),
        ];
        StateSnapshot::capture(&keys.did(), node, 3, blocks).unwrap()
    }

    #[test]
    fn a_signed_snapshot_verifies_and_roundtrips() {
        let k = keys(1);
        let signed = SignedSnapshot::sign(snap(&k, "node-a"), &k).unwrap();
        signed.verify().unwrap();
        let value = signed.to_value().unwrap();
        assert_eq!(SignedSnapshot::from_value(&value).unwrap(), signed);
        let frame = signed.to_frame().unwrap();
        assert_eq!(SignedSnapshot::from_frame(&frame).unwrap(), signed);
    }

    #[test]
    fn signing_someone_elses_state_is_refused() {
        let a = keys(2);
        let b = keys(3);
        assert_eq!(
            SignedSnapshot::sign(snap(&a, "node-a"), &b),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn a_replaced_snapshot_is_refused_even_with_a_valid_signature_of_its_own() {
        let k = keys(4);
        let older = SignedSnapshot::sign(snap(&k, "node-a"), &k).unwrap();
        // 攻击者用另一份「合法签名」的快照替换（重放/替换）。
        let mut other_blocks = vec![StateBlock::new(StateZone::Fs, "/a", json!({"n": 9})).unwrap()];
        other_blocks.push(StateBlock::new(StateZone::Memory, "m", json!("v2")).unwrap());
        let newer = SignedSnapshot::sign(
            StateSnapshot::capture(&k.did(), "node-a", 4, other_blocks).unwrap(),
            &k,
        )
        .unwrap();
        newer.verify().unwrap(); // 它自己的签名是对的
        let policy = SnapshotPolicy::for_agent(k.did())
            .expecting_content_root(older.snapshot().content_root().unwrap());
        assert_eq!(
            newer.verify_policy(&policy),
            Err(CoreError::InvalidSignature)
        );
        assert!(older.verify_policy(&policy).is_ok());
    }

    #[test]
    fn a_tampered_signature_or_body_is_refused_at_deserialization() {
        let k = keys(5);
        let signed = SignedSnapshot::sign(snap(&k, "node-a"), &k).unwrap();
        let mut value = signed.to_value().unwrap();
        value["sig"] = json!("00".repeat(64));
        assert_eq!(
            SignedSnapshot::from_value(&value),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn an_unsigned_or_wrong_version_payload_is_refused() {
        let k = keys(6);
        let signed = SignedSnapshot::sign(snap(&k, "node-a"), &k).unwrap();
        let mut unsigned = signed.to_value().unwrap();
        unsigned["sig"] = json!("");
        assert_eq!(
            SignedSnapshot::from_value(&unsigned),
            Err(CoreError::NotSealed)
        );

        let mut old = signed.to_value().unwrap();
        old["version"] = json!(99);
        assert_eq!(
            SignedSnapshot::from_value(&old),
            Err(CoreError::InvalidVersion)
        );
    }

    #[test]
    fn a_stale_but_authentic_snapshot_fails_the_min_epoch_policy() {
        let k = keys(7);
        let signed = SignedSnapshot::sign(snap(&k, "node-a"), &k).unwrap();
        let policy = SnapshotPolicy::for_agent(k.did()).with_min_epoch(10);
        assert_eq!(
            signed.verify_policy(&policy),
            Err(CoreError::InvalidVersion)
        );
        // 单验签名仍然通过：过期不是伪造，两者必须分开。
        signed.verify().unwrap();
    }

    #[test]
    fn another_agents_state_fails_the_agent_policy() {
        let a = keys(8);
        let b = keys(9);
        let signed = SignedSnapshot::sign(snap(&a, "node-a"), &a).unwrap();
        assert_eq!(
            signed.verify_policy(&SnapshotPolicy::for_agent(b.did())),
            Err(CoreError::InvalidDid)
        );
    }

    #[test]
    fn verified_snapshot_is_the_only_way_to_get_a_checked_value() {
        let k = keys(10);
        let signed = SignedSnapshot::sign(snap(&k, "node-a"), &k).unwrap();
        let policy = SnapshotPolicy::for_agent(k.did());
        let verified: VerifiedSnapshot = signed.verify_policy(&policy).unwrap();
        assert_eq!(verified.signer(), &k.did());
        assert_eq!(
            verified.content_root(),
            signed.snapshot().content_root().unwrap()
        );
        let inner = verified.into_snapshot();
        assert!(inner.verify().is_ok());
    }

    #[test]
    fn a_forged_signer_field_is_refused() {
        let a = keys(11);
        let b = keys(12);
        let signed = SignedSnapshot::sign(snap(&a, "node-a"), &a).unwrap();
        let mut value = signed.to_value().unwrap();
        value["signer"] = json!(b.did().as_str());
        assert_eq!(
            SignedSnapshot::from_value(&value),
            Err(CoreError::InvalidSignature)
        );
    }
}
