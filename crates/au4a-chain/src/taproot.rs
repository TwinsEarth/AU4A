//! v1.8.2 Taproot Assets：资产锚定与 Merkle 证明（**确定性测试网原型**）。
//!
//! Taproot Assets 的思路是：资产承诺被塞进一个 BTC 输出的 Taproot 承诺里，链上只看到
//! 一个输出键（`output_key`），资产细节由持有者用 Merkle 证明自己验证。
//! 这里把该语义做成纯 CPU 状态机：
//!
//! * **锚定**：`taproot.anchor` 把若干资产叶子（`asset_id + amount + seal`）做成一棵
//!   Merkle 树，树根与内部键一起派生出 `output_key = H(internal_key ‖ merkle_root)`；
//! * **证明**：`taproot.proof` 为第 i 个叶子生成认证路径；
//! * **验证**：`taproot.verify` 重算根并检查 —— 叶子被篡改、路径被换、根不符都会失败；
//!   另外**锚定交易必须已达最终性**，否则拒绝（`Timeout`）——fail-closed；
//! * **具名拒绝**：脚本路径花费 / Schnorr 多签 / 资产增发 / 链下互换等**按名字**拒绝。
//!
//! 证据等级恒为 `cpu-proto`：这里没有 BIP341 的真实 tweak、没有真实签名、没有 BTC 节点。

use std::collections::BTreeMap;

use au4a_core::{CoreError, CoreResult, Credits, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::rgb::Seal;
use crate::testnet::{ChainRefusal, ChainTx, Receipt, RefusalSpec, Testnet, ONCHAIN_GRADE};

/// 本适配器支持的操作。
pub const TAPROOT_SUPPORTED: [&str; 3] = ["taproot.anchor", "taproot.proof", "taproot.verify"];

/// 具名拒绝清单。
pub const TAPROOT_REFUSALS: [RefusalSpec; 4] = [
    RefusalSpec {
        op: "taproot.script_path_spend",
        code: RefusalCode::Unsupported,
        reason: "脚本路径花费需要真实 Taproot 脚本树与见证，本原型只做键路径语义",
    },
    RefusalSpec {
        op: "taproot.schnorr_multisig",
        code: RefusalCode::Unsupported,
        reason: "Schnorr 多签需要真实签名聚合，本轨道没有签名设备与网络",
    },
    RefusalSpec {
        op: "taproot.asset_inflation",
        code: RefusalCode::PolicyDenied,
        reason: "增发会破坏总量守恒，策略上永久拒绝",
    },
    RefusalSpec {
        op: "taproot.offchain_swap",
        code: RefusalCode::Unsupported,
        reason: "链下互换需要真实对手方与超时脚本，本轨道只做锚定语义",
    },
];

/// Merkle 认证路径的一步。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofStep {
    pub hash: String,
    /// 兄弟节点是否在右侧。
    pub right: bool,
}

/// 一个资产叶子（承诺）。
pub fn leaf_hash(asset_id: &str, amount: Credits, seal: &Seal) -> CoreResult<String> {
    au4a_core::canonical_hash(&json!({
        "asset_id": asset_id,
        "amount": amount,
        "seal": seal,
    }))
}

fn hash_pair(left: &str, right: &str) -> CoreResult<String> {
    au4a_core::canonical_hash(&json!({ "l": left, "r": right }))
}

/// 比特币式 Merkle 根：奇数个节点时复制最后一个；空树返回 64 个 0。
pub fn merkle_root(leaves: &[String]) -> CoreResult<String> {
    if leaves.is_empty() {
        return Ok("0".repeat(64));
    }
    let mut level: Vec<String> = leaves.to_vec();
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            let last = level.last().cloned().unwrap_or_default();
            level.push(last);
        }
        let mut next = Vec::with_capacity(level.len() / 2);
        let mut i = 0;
        while i + 1 < level.len() {
            next.push(hash_pair(&level[i], &level[i + 1])?);
            i += 2;
        }
        level = next;
    }
    Ok(level.remove(0))
}

/// 第 `index` 个叶子的认证路径。
pub fn merkle_proof(leaves: &[String], index: usize) -> CoreResult<Vec<ProofStep>> {
    if index >= leaves.len() {
        return Err(CoreError::InvalidKind);
    }
    let mut level: Vec<String> = leaves.to_vec();
    let mut idx = index;
    let mut proof = Vec::new();
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            let last = level.last().cloned().unwrap_or_default();
            level.push(last);
        }
        let sibling = if idx.is_multiple_of(2) {
            idx + 1
        } else {
            idx - 1
        };
        proof.push(ProofStep {
            hash: level.get(sibling).cloned().unwrap_or_default(),
            right: idx.is_multiple_of(2),
        });
        let mut next = Vec::with_capacity(level.len() / 2);
        let mut i = 0;
        while i + 1 < level.len() {
            next.push(hash_pair(&level[i], &level[i + 1])?);
            i += 2;
        }
        level = next;
        idx /= 2;
    }
    Ok(proof)
}

/// 用认证路径重算根，判断是否等于给定根。
pub fn verify_merkle(leaf: &str, proof: &[ProofStep], root: &str) -> CoreResult<bool> {
    let mut acc = leaf.to_string();
    for step in proof {
        acc = if step.right {
            hash_pair(&acc, &step.hash)?
        } else {
            hash_pair(&step.hash, &acc)?
        };
    }
    Ok(acc == root)
}

/// 一次锚定：内部键 + Merkle 根 → 输出键。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaprootAnchor {
    pub asset_id: String,
    pub internal_key: String,
    pub merkle_root: String,
    pub output_key: String,
    pub amount_total: Credits,
    pub leaves: Vec<String>,
    /// 锚定交易 id 与高度（测试网语义）。
    pub tx_id: String,
    pub height: u64,
}

impl TaprootAnchor {
    /// 派生输出键：`H(internal_key ‖ merkle_root)`（简化版 BIP341 tweak）。
    pub fn output_key_for(internal_key: &str, merkle_root: &str) -> CoreResult<String> {
        if internal_key.is_empty() || internal_key.len() != 64 {
            return Err(CoreError::InvalidKind);
        }
        au4a_core::canonical_hash(
            &json!({ "internal_key": internal_key, "merkle_root": merkle_root }),
        )
    }

    /// 结构自洽性：输出键必须真的是由内部键与根派生出来的。
    pub fn verify(&self) -> CoreResult<()> {
        if self.output_key != Self::output_key_for(&self.internal_key, &self.merkle_root)? {
            return Err(CoreError::InvalidSignature);
        }
        if self.leaves.is_empty() {
            return Err(CoreError::ZeroAmount);
        }
        Ok(())
    }

    pub fn proof_for(&self, index: usize) -> CoreResult<Vec<ProofStep>> {
        merkle_proof(&self.leaves, index)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "asset_id": self.asset_id,
            "internal_key": self.internal_key,
            "merkle_root": self.merkle_root,
            "output_key": self.output_key,
            "amount_total": self.amount_total,
            "leaves": self.leaves.len(),
            "tx_id": self.tx_id,
            "height": self.height,
            "grade": ONCHAIN_GRADE.as_str(),
        })
    }
}

/// 适配器执行结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaprootOutcome {
    pub op: String,
    pub detail: String,
    pub receipt: Option<Receipt>,
    pub proof: Vec<ProofStep>,
    pub grade: String,
}

/// Taproot Assets 适配器。
#[derive(Clone, Debug, Default)]
pub struct TaprootAdapter {
    anchors: BTreeMap<String, TaprootAnchor>,
    verified: Vec<String>,
}

impl TaprootAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn anchor_of(&self, asset_id: &str) -> Option<&TaprootAnchor> {
        self.anchors.get(asset_id)
    }

    pub fn anchor_count(&self) -> usize {
        self.anchors.len()
    }

    pub fn verified_count(&self) -> usize {
        self.verified.len()
    }

    /// 已锚定的资产总额（双轨对账用）。
    pub fn anchored_total(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for anchor in self.anchors.values() {
            sum = sum.checked_add(anchor.amount_total)?;
        }
        Ok(sum)
    }

    /// 执行：已知但拒绝的操作按其具名码拒绝；未知操作按名字拒绝。
    pub fn execute(
        &mut self,
        net: &mut Testnet,
        tx: &ChainTx,
    ) -> Result<TaprootOutcome, ChainRefusal> {
        match tx.op.as_str() {
            "taproot.anchor" => self.anchor(net, tx),
            "taproot.proof" => self.proof(net, tx),
            "taproot.verify" => self.verify(net, tx),
            _ => {
                let refusal = match TAPROOT_REFUSALS.iter().find(|spec| spec.op == tx.op) {
                    Some(spec) => ChainRefusal::new(&tx.op, spec.code, spec.reason),
                    None => ChainRefusal::unsupported(&tx.op, &TAPROOT_REFUSALS),
                };
                net.record_refusal(refusal.clone());
                Err(refusal)
            }
        }
    }

    fn anchor(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<TaprootOutcome, ChainRefusal> {
        let malformed =
            |detail: &str| ChainRefusal::new("taproot.anchor", RefusalCode::Malformed, detail);
        let asset_id = tx
            .payload
            .get("asset_id")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("缺少 asset_id"))?
            .to_string();
        let internal_key = tx
            .payload
            .get("internal_key")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("缺少 internal_key"))?
            .to_string();
        let amounts: Vec<i64> = tx
            .payload
            .get("amounts")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_i64).collect())
            .ok_or_else(|| malformed("缺少 amounts"))?;
        if amounts.is_empty() || amounts.iter().any(|a| *a <= 0) {
            let refusal = malformed("金额列表为空或含非正数");
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        if self.anchors.contains_key(&asset_id) {
            let refusal = ChainRefusal::new(
                "taproot.anchor",
                RefusalCode::Conflict,
                format!("资产 {asset_id} 已经锚定过（重复锚定被拒）"),
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let anchor_tx = Seal::new(tx.id.clone(), 0);
        let mut leaves = Vec::with_capacity(amounts.len());
        let mut total = Credits::ZERO;
        for amount in &amounts {
            let amount = Credits::new(*amount).map_err(|_| malformed("金额为负"))?;
            total = total
                .checked_add(amount)
                .map_err(|_| malformed("金额溢出"))?;
            leaves.push(
                leaf_hash(&asset_id, amount, &anchor_tx).map_err(|_| malformed("叶子哈希失败"))?,
            );
        }
        let merkle = merkle_root(&leaves).map_err(|_| malformed("Merkle 根计算失败"))?;
        let output_key = TaprootAnchor::output_key_for(&internal_key, &merkle)
            .map_err(|_| malformed("输出键派生失败"))?;
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "taproot");
        let height = receipt.height;
        let anchor = TaprootAnchor {
            asset_id: asset_id.clone(),
            internal_key,
            merkle_root: merkle,
            output_key,
            amount_total: total,
            leaves,
            tx_id: tx.id.clone(),
            height,
        };
        self.anchors.insert(asset_id.clone(), anchor.clone());
        Ok(TaprootOutcome {
            op: "taproot.anchor".to_string(),
            detail: format!(
                "锚定 {asset_id}：{} 个叶子、总额 {total}，输出键 {}",
                anchor.leaves.len(),
                au4a_core::short_id(&anchor.output_key)
            ),
            receipt: Some(receipt),
            proof: Vec::new(),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    fn proof(&self, net: &mut Testnet, tx: &ChainTx) -> Result<TaprootOutcome, ChainRefusal> {
        let asset_id = tx
            .payload
            .get("asset_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainRefusal::new("taproot.proof", RefusalCode::Malformed, "缺少 asset_id")
            })?;
        let index = tx.payload.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let anchor = self.anchors.get(asset_id).ok_or_else(|| {
            ChainRefusal::new(
                "taproot.proof",
                RefusalCode::Conflict,
                format!("资产 {asset_id} 未锚定"),
            )
        })?;
        let proof = anchor.proof_for(index).map_err(|_| {
            ChainRefusal::new("taproot.proof", RefusalCode::Malformed, "叶子下标越界")
        })?;
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "taproot");
        Ok(TaprootOutcome {
            op: "taproot.proof".to_string(),
            detail: format!(
                "为 {} 的第 {index} 个叶子生成认证路径（{} 步）",
                au4a_core::short_id(asset_id),
                proof.len()
            ),
            receipt: Some(receipt),
            proof,
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    fn verify(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<TaprootOutcome, ChainRefusal> {
        let asset_id = tx
            .payload
            .get("asset_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainRefusal::new("taproot.verify", RefusalCode::Malformed, "缺少 asset_id")
            })?
            .to_string();
        let leaf = tx
            .payload
            .get("leaf")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainRefusal::new("taproot.verify", RefusalCode::Malformed, "缺少 leaf")
            })?
            .to_string();
        let proof: Vec<ProofStep> = tx
            .payload
            .get("proof")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| {
                ChainRefusal::new("taproot.verify", RefusalCode::Malformed, "认证路径不可解析")
            })?
            .unwrap_or_default();
        let anchor = self.anchors.get(&asset_id).cloned().ok_or_else(|| {
            let refusal = ChainRefusal::new(
                "taproot.verify",
                RefusalCode::Conflict,
                format!("资产 {asset_id} 未锚定"),
            );
            net.record_refusal(refusal.clone());
            refusal
        })?;
        anchor.verify().map_err(|_| {
            let refusal = ChainRefusal::new(
                "taproot.verify",
                RefusalCode::Malformed,
                "锚定结构自洽性失败（输出键与内部键/根不符）",
            );
            net.record_refusal(refusal.clone());
            refusal
        })?;
        // fail-closed：锚定交易未达最终性就不认。
        if !net.is_final(anchor.height) {
            let refusal = ChainRefusal::new(
                "taproot.verify",
                RefusalCode::Timeout,
                format!(
                    "锚定高度 {} 尚未最终（当前 {}，最终化到 {}）",
                    anchor.height,
                    net.height(),
                    net.finalized_height()
                ),
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let ok = verify_merkle(&leaf, &proof, &anchor.merkle_root).map_err(|_| {
            ChainRefusal::new("taproot.verify", RefusalCode::Malformed, "认证路径计算失败")
        })?;
        if !ok {
            let refusal = ChainRefusal::new(
                "taproot.verify",
                RefusalCode::Malformed,
                "Merkle 证明不成立（叶子或路径被篡改）",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "taproot");
        self.verified.push(asset_id.clone());
        Ok(TaprootOutcome {
            op: "taproot.verify".to_string(),
            detail: format!(
                "Merkle 证明成立：根 {}（锚定于高度 {}）",
                au4a_core::short_id(&anchor.merkle_root),
                anchor.height
            ),
            receipt: Some(receipt),
            proof,
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::BridgeBook;
    use crate::testnet::ChainId;
    use au4a_core::{AgentKeys, Did, Ledger};

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn key() -> String {
        au4a_core::content_hash(b"internal-key-au4a").to_string()
    }

    fn anchor_tx(net: &Testnet, asset: &str, amounts: &[i64], nonce: u64) -> ChainTx {
        ChainTx::new(
            net.chain(),
            "taproot.anchor",
            &did(1),
            nonce,
            json!({ "asset_id": asset, "internal_key": key(), "amounts": amounts }),
        )
        .unwrap()
    }

    #[test]
    fn anchoring_derives_a_deterministic_output_key_and_conserves_the_total() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = TaprootAdapter::new();
        let tx = anchor_tx(&net, "tap:USDT", &[250, 150], 1);
        let outcome = adapter.execute(&mut net, &tx).unwrap();
        assert!(outcome.grade == "cpu-proto");
        let anchor = adapter.anchor_of("tap:USDT").unwrap();
        assert_eq!(anchor.amount_total, Credits(400));
        assert_eq!(anchor.leaves.len(), 2);
        assert_eq!(
            anchor.output_key,
            TaprootAnchor::output_key_for(&anchor.internal_key, &anchor.merkle_root).unwrap()
        );
        assert_eq!(adapter.anchored_total().unwrap(), Credits(400));
        // 同样的输入 → 同样的根与输出键（确定性）。
        let merkle = merkle_root(&anchor.leaves).unwrap();
        assert_eq!(merkle, anchor.merkle_root);
    }

    #[test]
    fn merkle_proofs_verify_and_tampering_breaks_them() {
        let leaves: Vec<String> = (1..=5)
            .map(|i| {
                leaf_hash("tap:USDT", Credits(i * 10), &Seal::new(format!("s{i}"), 0)).unwrap()
            })
            .collect();
        let root = merkle_root(&leaves).unwrap();
        for index in 0..leaves.len() {
            let proof = merkle_proof(&leaves, index).unwrap();
            assert!(
                verify_merkle(&leaves[index], &proof, &root).unwrap(),
                "index {index}"
            );
        }
        // 换一个叶子 → 不成立。
        let proof = merkle_proof(&leaves, 0).unwrap();
        let wrong = leaf_hash("tap:USDT", Credits(999), &Seal::new("s1", 0)).unwrap();
        assert!(!verify_merkle(&wrong, &proof, &root).unwrap());
        // 篡改路径 → 不成立。
        let mut bad = proof.clone();
        if let Some(step) = bad.first_mut() {
            step.hash = "f".repeat(64);
        }
        assert!(!verify_merkle(&leaves[0], &bad, &root).unwrap());
        // 越界下标被拒。
        assert_eq!(merkle_proof(&leaves, 99), Err(CoreError::InvalidKind));
    }

    #[test]
    fn a_proof_verifies_end_to_end_only_after_finality() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = TaprootAdapter::new();
        let tx = anchor_tx(&net, "tap:USDT", &[100, 100, 200], 1);
        adapter.execute(&mut net, &tx).unwrap();
        net.mine();
        let anchor = adapter.anchor_of("tap:USDT").unwrap().clone();
        let leaf = anchor.leaves[2].clone();
        let proof = anchor.proof_for(2).unwrap();

        let verify_tx = ChainTx::new(
            ChainId::BtcRegtest,
            "taproot.verify",
            &did(2),
            1,
            json!({ "asset_id": "tap:USDT", "leaf": leaf, "proof": proof }),
        )
        .unwrap();
        // 未达最终性 → Timeout。
        let err = adapter.execute(&mut net, &verify_tx).unwrap_err();
        assert_eq!(err.code, RefusalCode::Timeout);
        // 叠够最终性 → 证明成立。
        net.mine_to(net.height() + net.finality_depth());
        let outcome = adapter.execute(&mut net, &verify_tx).unwrap();
        assert!(outcome.detail.contains("Merkle 证明成立"));
        assert_eq!(adapter.verified_count(), 1);
    }

    #[test]
    fn a_tampered_leaf_is_refused_by_the_verifier() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 1);
        let mut adapter = TaprootAdapter::new();
        let tx = anchor_tx(&net, "tap:USDT", &[100, 200], 1);
        adapter.execute(&mut net, &tx).unwrap();
        net.mine_to(3);
        let anchor = adapter.anchor_of("tap:USDT").unwrap().clone();
        let proof = anchor.proof_for(0).unwrap();
        let fake_leaf =
            leaf_hash("tap:USDT", Credits(9_999), &Seal::new(tx.id.clone(), 0)).unwrap();
        let verify_tx = ChainTx::new(
            ChainId::BtcRegtest,
            "taproot.verify",
            &did(3),
            1,
            json!({ "asset_id": "tap:USDT", "leaf": fake_leaf, "proof": proof }),
        )
        .unwrap();
        let err = adapter.execute(&mut net, &verify_tx).unwrap_err();
        assert_eq!(err.code, RefusalCode::Malformed);
        assert!(err.detail.contains("证明不成立"));
        assert_eq!(net.refusals().len(), 1);
    }

    #[test]
    fn anchoring_the_same_asset_twice_is_refused() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 1);
        let mut adapter = TaprootAdapter::new();
        let first = anchor_tx(&net, "tap:USDT", &[100], 1);
        adapter.execute(&mut net, &first).unwrap();
        let second = anchor_tx(&net, "tap:USDT", &[100], 2);
        let err = adapter.execute(&mut net, &second).unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
        assert_eq!(adapter.anchor_count(), 1);
    }

    #[test]
    fn degenerate_anchors_are_refused() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 1);
        let mut adapter = TaprootAdapter::new();
        // 空金额列表。
        let empty = ChainTx::new(
            ChainId::BtcRegtest,
            "taproot.anchor",
            &did(1),
            1,
            json!({ "asset_id": "tap:X", "internal_key": key(), "amounts": [] }),
        )
        .unwrap();
        assert_eq!(
            adapter.execute(&mut net, &empty).unwrap_err().code,
            RefusalCode::Malformed
        );
        // 内部键长度不对。
        let bad_key = ChainTx::new(
            ChainId::BtcRegtest,
            "taproot.anchor",
            &did(1),
            2,
            json!({ "asset_id": "tap:X", "internal_key": "short", "amounts": [10] }),
        )
        .unwrap();
        assert_eq!(
            adapter.execute(&mut net, &bad_key).unwrap_err().code,
            RefusalCode::Malformed
        );
        assert_eq!(adapter.anchor_count(), 0);
    }

    #[test]
    fn named_refusals_cover_the_taproot_unsupported_set() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 1);
        let mut adapter = TaprootAdapter::new();
        for (op, code) in [
            ("taproot.script_path_spend", RefusalCode::Unsupported),
            ("taproot.schnorr_multisig", RefusalCode::Unsupported),
            ("taproot.asset_inflation", RefusalCode::PolicyDenied),
            ("taproot.offchain_swap", RefusalCode::Unsupported),
            ("taproot.unknown_op", RefusalCode::Unsupported),
        ] {
            let tx = ChainTx::new(ChainId::BtcRegtest, op, &did(1), 9, json!({})).unwrap();
            let err = adapter.execute(&mut net, &tx).unwrap_err();
            assert_eq!(err.op, op);
            assert_eq!(err.code, code);
        }
        assert_eq!(net.refusals().len(), 5);
        assert_eq!(TAPROOT_REFUSALS.len(), 4);
        assert_eq!(TAPROOT_SUPPORTED.len(), 3);
    }

    #[test]
    fn the_taproot_rail_matches_the_local_escrow() {
        let who = did(9);
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(1_000)).unwrap();
        let mut book = BridgeBook::new();
        book.bridge_out(&mut ledger, &who, Credits(400), "tap:USDT", "taproot")
            .unwrap();
        let mut net = Testnet::new(ChainId::BtcRegtest, 1);
        let mut adapter = TaprootAdapter::new();
        let tx = anchor_tx(&net, "tap:USDT", &[250, 150], 1);
        adapter.execute(&mut net, &tx).unwrap();
        assert_eq!(book.escrowed(), Credits(400));
        assert_eq!(adapter.anchored_total().unwrap(), Credits(400));
        book.require_consistent(&ledger).unwrap();
        ledger.check_conservation().unwrap();
    }
}
