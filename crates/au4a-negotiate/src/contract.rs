//! v1.2.5 — 合约签订。
//!
//! 合约是协商的产物，也是后续执行、结算、违约、仲裁的唯一依据。因此它必须满足三件事：
//!
//! 1. **内容寻址**：`hash = SHA-256(规范 JSON(合约条款))`（用冻结基元的 [`canonical_hash`]）。
//!    哈希写进合约本身，并在 [`Contract::verify`] 里**重算比对**——改条款、改哈希都会露馅。
//! 2. **双方签名才成立**：只有提议方或应答方的签名有效，且必须**恰好两方各一个**；
//!    单方签名、重复签名、第三方签名一律 `Err`。
//! 3. **可锚定**：任一方可以把 `(contract_id, contract_hash)` 锚定到共享内核的只读进度流
//!    （`contract.anchor`），锚点自带签名，任何人可以离线复核「这份合约在什么时刻被承认过」。
//!
//! 合约文本本身也是规范 JSON：`encode()` 的输出可以逐字节比对，`decode()` 会重验哈希与双方签名。

use au4a_core::{canonical_hash, canonicalize, AgentKeys, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::msg::Terms;
use crate::state::{DualSigned, PartySignature};

/// 锚定事件类型：写进 `Kernel` 的只读进度流，人类观察层只读订阅。
pub const ANCHOR_EVENT: &str = "contract.anchor";

/// 一次锚定：谁在什么逻辑时刻承认了哪个合约哈希。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub contract_id: String,
    pub contract_hash: String,
    pub by: Did,
    /// 逻辑时钟读数。
    pub at: u64,
    /// 锚定方对 `(contract_id, contract_hash, by, at)` 的签名。
    pub sig: String,
}

impl Anchor {
    /// 被签名的载荷。
    pub fn payload(&self) -> Value {
        json!({
            "contract_id": self.contract_id,
            "contract_hash": self.contract_hash,
            "by": self.by,
            "at": self.at,
        })
    }

    pub fn verify(&self) -> CoreResult<()> {
        let bytes = canonicalize(&self.payload())?;
        self.by.verify(bytes.as_bytes(), &self.sig)
    }
}

/// 合约。
///
/// 字段全部公开：这样「篡改」在测试里是**真实可做**的动作，而不是靠想象。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    /// 合约编号：条款哈希的前 16 位（`short_id`），人类可读，不参与验签。
    pub id: String,
    /// 条款的规范 JSON SHA-256（64 位小写 hex）。
    pub hash: String,
    pub proposer: Did,
    pub responder: Did,
    pub terms: Terms,
    /// 产生它的协商会话 id（内容寻址）。
    pub negotiation: String,
    pub created_at: u64,
    /// 双方签名，按 DID 字节序排序。
    pub signatures: Vec<PartySignature>,
    pub anchor: Option<Anchor>,
}

impl Contract {
    /// 草拟一份**未签名**合约。
    pub fn draft(
        proposer: &AgentKeys,
        responder: &Did,
        terms: &Terms,
        negotiation: &str,
        at: u64,
    ) -> CoreResult<Self> {
        terms.validate()?;
        if !crate::msg::is_label(negotiation) {
            return Err(CoreError::InvalidKind);
        }
        if proposer.did() == *responder {
            // 跟自己签合约不是协商，是自欺。
            return Err(CoreError::InvalidKind);
        }
        let mut contract = Self {
            id: String::new(),
            hash: String::new(),
            proposer: proposer.did(),
            responder: responder.clone(),
            terms: terms.clone(),
            negotiation: negotiation.to_string(),
            created_at: at,
            signatures: Vec::new(),
            anchor: None,
        };
        contract.hash = contract.compute_hash()?;
        contract.id = au4a_core::short_id(&contract.hash);
        Ok(contract)
    }

    /// 被签名/被哈希的条款载荷（不含签名与锚点：签名覆盖条款，不覆盖签名的容器）。
    pub fn payload(&self) -> Value {
        json!({
            "proposer": self.proposer,
            "responder": self.responder,
            "terms": self.terms,
            "negotiation": self.negotiation,
            "created_at": self.created_at,
        })
    }

    /// 重算条款哈希。
    pub fn compute_hash(&self) -> CoreResult<String> {
        canonical_hash(&self.payload())
    }

    /// 规范 JSON 文本。可以逐字节比对，也可以离线归档。
    pub fn encode(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self).map_err(|_| CoreError::Encoding)?;
        canonicalize(&value)
    }

    /// 从规范 JSON 恢复并**重新校验**（哈希 + 双方签名 + 锚点）。
    pub fn decode(text: &str) -> CoreResult<Self> {
        let contract: Self = serde_json::from_str(text).map_err(|_| CoreError::Encoding)?;
        contract.verify()?;
        if let Some(anchor) = &contract.anchor {
            anchor.verify()?;
        }
        Ok(contract)
    }

    pub fn parties(&self) -> [&Did; 2] {
        [&self.proposer, &self.responder]
    }

    pub fn signed_by(&self, did: &Did) -> bool {
        self.signatures.iter().any(|s| &s.did == did)
    }

    /// 一方签署。只接受当事人，且不接受同一方重复签署。
    pub fn sign(&mut self, keys: &AgentKeys) -> CoreResult<()> {
        let did = keys.did();
        if did != self.proposer && did != self.responder {
            return Err(CoreError::UnknownAgent);
        }
        if self.signed_by(&did) {
            return Err(CoreError::InvalidSignature);
        }
        let sig = keys.sign_json(&self.payload())?;
        self.signatures.push(PartySignature { did, sig });
        self.signatures.sort_by(|a, b| a.did.cmp(&b.did));
        Ok(())
    }

    /// 校验：哈希自洽 + **恰好双方各一个**有效签名。
    pub fn verify(&self) -> CoreResult<()> {
        if self.proposer == self.responder {
            return Err(CoreError::InvalidKind);
        }
        if self.hash != self.compute_hash()? {
            // 条款被改过，或者哈希被改过——两种都拒绝。
            return Err(CoreError::InvalidSignature);
        }
        if self.id != au4a_core::short_id(&self.hash) {
            return Err(CoreError::InvalidSignature);
        }
        if self.signatures.len() != 2 {
            return Err(CoreError::NotSealed);
        }
        let bytes = canonicalize(&self.payload())?;
        for signature in &self.signatures {
            if signature.did != self.proposer && signature.did != self.responder {
                return Err(CoreError::UnknownAgent);
            }
            signature.did.verify(bytes.as_bytes(), &signature.sig)?;
        }
        if !self.signed_by(&self.proposer) || !self.signed_by(&self.responder) {
            return Err(CoreError::NotSealed);
        }
        Ok(())
    }

    /// 双签是否成立。
    pub fn is_dual_signed(&self) -> bool {
        self.verify().is_ok()
    }

    /// 锚定：由某一方把 `(id, hash)` 写进可被任何人复核的锚点。
    ///
    /// 只有**已经签署**的当事人可以锚定——否则锚点会变成「我承认一份我没签的合约」。
    pub fn anchor(&mut self, keys: &AgentKeys, at: u64) -> CoreResult<Anchor> {
        let did = keys.did();
        if !self.signed_by(&did) {
            // `CoreError` 是冻结枚举（没有 `Unauthorized` 变体）：非当事人锚定映射为
            // 「不是这条合约的参与方」，与 `sign()` 对第三方的判定保持一致。
            return Err(CoreError::UnknownAgent);
        }
        let mut anchor = Anchor {
            contract_id: self.id.clone(),
            contract_hash: self.hash.clone(),
            by: did,
            at,
            sig: String::new(),
        };
        anchor.sig = keys.sign_json(&anchor.payload())?;
        anchor.verify()?;
        self.anchor = Some(anchor.clone());
        Ok(anchor)
    }

    /// 复核锚点（离线也可做）。
    pub fn verify_anchor(&self) -> CoreResult<()> {
        let anchor = self.anchor.as_ref().ok_or(CoreError::NotSealed)?;
        if anchor.contract_id != self.id || anchor.contract_hash != self.hash {
            return Err(CoreError::InvalidSignature);
        }
        if anchor.by != self.proposer && anchor.by != self.responder {
            return Err(CoreError::UnknownAgent);
        }
        anchor.verify()
    }

    /// 供观察层展示的只读摘要。
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "hash": self.hash,
            "proposer": self.proposer.as_str(),
            "responder": self.responder.as_str(),
            "price": self.terms.price.0,
            "task": self.terms.task,
            "evidence": self.terms.evidence.as_str(),
            "signatures": self.signatures.len(),
            "dual_signed": self.is_dual_signed(),
            "anchored": self.anchor.is_some(),
        })
    }
}

/// 合约天然满足「双签合约」见证接口：违约进仲裁（v1.2.6 / v1.2.7）直接用它。
impl DualSigned for Contract {
    fn contract_id(&self) -> &str {
        &self.id
    }

    fn contract_hash(&self) -> &str {
        &self.hash
    }

    fn is_dual_signed(&self) -> bool {
        Contract::is_dual_signed(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::{Credits, EvidenceGrade};

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn terms(price: i64) -> Terms {
        Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
    }

    /// 草拟 + 双签，返回合约与双方密钥。
    fn signed_contract() -> (Contract, AgentKeys, AgentKeys) {
        let a = agent(1);
        let b = agent(2);
        let mut c = Contract::draft(&a, &b.did(), &terms(100), "session-1", 7).unwrap();
        c.sign(&a).unwrap();
        c.sign(&b).unwrap();
        (c, a, b)
    }

    #[test]
    fn hash_is_the_canonical_json_sha256_and_anchors_the_id() {
        let (c, _, _) = signed_contract();
        assert_eq!(c.hash.len(), 64);
        assert!(crate::msg::is_hash(&c.hash));
        assert_eq!(c.id, au4a_core::short_id(&c.hash));
        assert_eq!(c.hash, c.compute_hash().unwrap());

        // 同一份条款：不同次数、不同实例得到同一个哈希。
        let a = agent(1);
        let b = agent(2);
        let again = Contract::draft(&a, &b.did(), &terms(100), "session-1", 7).unwrap();
        assert_eq!(again.hash, c.hash);
        // 价格变一点，哈希全变。
        let dearer = Contract::draft(&a, &b.did(), &terms(101), "session-1", 7).unwrap();
        assert_ne!(dearer.hash, c.hash);
        // 会话、时间任何一个变了，哈希都变。
        assert_ne!(
            Contract::draft(&a, &b.did(), &terms(100), "session-2", 7)
                .unwrap()
                .hash,
            c.hash
        );
        assert_ne!(
            Contract::draft(&a, &b.did(), &terms(100), "session-1", 8)
                .unwrap()
                .hash,
            c.hash
        );
    }

    #[test]
    fn both_signatures_are_required() {
        let a = agent(3);
        let b = agent(4);
        let mut c = Contract::draft(&a, &b.did(), &terms(100), "s", 1).unwrap();
        assert_eq!(c.verify(), Err(CoreError::NotSealed));
        assert!(!c.is_dual_signed());
        c.sign(&a).unwrap();
        assert_eq!(c.verify(), Err(CoreError::NotSealed), "单方签名不成立");
        c.sign(&b).unwrap();
        c.verify().unwrap();
        assert!(c.is_dual_signed());
        assert_eq!(c.signatures.len(), 2);
        // 重复签名被拒。
        assert_eq!(c.sign(&a), Err(CoreError::InvalidSignature));
        // 第三方签名被拒。
        let outsider = agent(5);
        assert_eq!(c.sign(&outsider), Err(CoreError::UnknownAgent));
    }

    #[test]
    fn tampering_after_signing_is_detected() {
        let (c, _, _) = signed_contract();

        let mut price_changed = c.clone();
        price_changed.terms.price = Credits(1);
        assert_eq!(price_changed.verify(), Err(CoreError::InvalidSignature));

        let mut deadline_changed = c.clone();
        deadline_changed.terms.deadline = 999;
        assert_eq!(deadline_changed.verify(), Err(CoreError::InvalidSignature));

        let mut hash_changed = c.clone();
        hash_changed.hash = "0".repeat(64);
        assert_eq!(hash_changed.verify(), Err(CoreError::InvalidSignature));

        let mut id_changed = c.clone();
        id_changed.id = "deadbeefdeadbeef".to_string();
        assert_eq!(id_changed.verify(), Err(CoreError::InvalidSignature));

        let mut sig_changed = c.clone();
        sig_changed.signatures[0].sig = "00".repeat(64);
        assert_eq!(sig_changed.verify(), Err(CoreError::InvalidSignature));

        let mut party_swapped = c.clone();
        party_swapped.responder = agent(9).did();
        assert_eq!(party_swapped.verify(), Err(CoreError::InvalidSignature));

        // 原合约仍然成立。
        c.verify().unwrap();
    }

    #[test]
    fn encode_decode_is_byte_exact_and_revalidates() {
        let (c, _, _) = signed_contract();
        let text = c.encode().unwrap();
        let restored = Contract::decode(&text).unwrap();
        assert_eq!(restored, c);
        assert_eq!(restored.encode().unwrap(), text);

        assert_eq!(Contract::decode(""), Err(CoreError::Encoding));
        assert_eq!(Contract::decode("{}"), Err(CoreError::Encoding));

        let tampered = text.replace("\"price\":100", "\"price\":7");
        assert_ne!(tampered, text);
        assert_eq!(
            Contract::decode(&tampered),
            Err(CoreError::InvalidSignature)
        );

        // 破坏签名后不能通过 decode（decode 会 verify）。
        let broken = text.replace(&c.signatures[0].sig, &"00".repeat(64));
        assert_eq!(Contract::decode(&broken), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn only_a_signer_can_anchor_and_anchors_are_verifiable() {
        let (mut c, a, b) = signed_contract();
        let outsider = agent(7);
        assert_eq!(c.anchor(&outsider, 9), Err(CoreError::UnknownAgent));

        let anchor = c.anchor(&a, 9).unwrap();
        assert_eq!(anchor.contract_id, c.id);
        assert_eq!(anchor.contract_hash, c.hash);
        assert_eq!(anchor.by, a.did());
        c.verify_anchor().unwrap();
        assert_eq!(c.anchor.as_ref().unwrap().payload()["at"], json!(9));

        // 应答方同样可以锚定同一份哈希。
        let by_responder = c.anchor(&b, 10).unwrap();
        assert_eq!(by_responder.by, b.did());
        assert_eq!(by_responder.contract_hash, c.hash);
        assert_eq!(c.anchor.as_ref().unwrap().at, 10);

        // 锚点被篡改 → 复核失败。
        let mut moved = c.clone();
        if let Some(target) = moved.anchor.as_mut() {
            target.at = 99;
        }
        assert_eq!(moved.verify_anchor(), Err(CoreError::InvalidSignature));

        // 换一份合约的哈希进锚点 → 失败。
        let mut swapped = c.clone();
        if let Some(target) = swapped.anchor.as_mut() {
            target.contract_hash = "1".repeat(64);
        }
        assert_eq!(swapped.verify_anchor(), Err(CoreError::InvalidSignature));

        // 没锚定的合约：明确说没有，而不是假装通过。
        let (fresh, _, _) = signed_contract();
        assert_eq!(fresh.verify_anchor(), Err(CoreError::NotSealed));
    }

    #[test]
    fn draft_refuses_self_dealing_and_bad_input() {
        let a = agent(11);
        assert_eq!(
            Contract::draft(&a, &a.did(), &terms(100), "s", 1),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            Contract::draft(&a, &agent(12).did(), &terms(100), "", 1),
            Err(CoreError::InvalidKind)
        );
        // 零价条款连 `Terms::new` 都造不出来，所以直接构造再接进 draft。
        let zero = Terms {
            task: "summarize.zh".to_string(),
            price: Credits(0),
            deadline: 40,
            evidence: EvidenceGrade::Verified,
        };
        assert_eq!(
            Contract::draft(&a, &agent(12).did(), &zero, "s", 1),
            Err(CoreError::ZeroAmount)
        );
    }

    #[test]
    fn the_dual_signed_witness_interface_matches_the_contract() {
        let (mut c, _, _) = signed_contract();
        assert!(DualSigned::is_dual_signed(&c));
        assert_eq!(DualSigned::contract_id(&c), c.id.as_str());
        assert_eq!(DualSigned::contract_hash(&c), c.hash.as_str());

        // 破坏双签后，见证接口必须说「不成立」，否则违约仲裁会被单方合约骗过。
        c.signatures.pop();
        assert!(!DualSigned::is_dual_signed(&c));

        let summary = signed_contract().0.summary();
        assert_eq!(summary["dual_signed"], true);
        assert_eq!(summary["price"], 100);
        assert_eq!(summary["signatures"], 2);
    }
}
