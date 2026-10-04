//! 确定性测试网：**所有**「链上」语义都只在这里，而且只在本地 CPU 上。
//!
//! 这个 crate 里没有网络、没有真实节点、没有真实资产。所谓「链上」是一个**纯 CPU 状态机**：
//! 交易内容寻址 → 校验（防篡改 / 防重放 / 防乱序 nonce）→ 入待打包区 → 出块（逻辑高度）
//! → 确认深度 → 最终性。哈希与签名来自 [`au4a_core`]，时间来自调用方给的逻辑高度。
//!
//! 三条硬纪律：
//!
//! 1. **证据等级恒为 [`ONCHAIN_GRADE`]（`cpu-proto`）**：收据上写死这个等级，任何地方都
//!    不可能把测试网结果冒充成真实链上成功；
//! 2. **具名拒绝**：不认识的操作**按名字**拒绝（[`ChainRefusal`] 带上 op 名与
//!    [`RefusalCode`]），并且记进 `refusals`；绝不允许静默通过；
//! 3. **可回滚的最终性**：[`Testnet::reorg`] 显式模拟重组——**未达最终性的收据不算数**，
//!    这是 fail-closed 的基础（对账与结算只认最终收据）。

use std::collections::{BTreeMap, BTreeSet};

use au4a_core::{CoreError, CoreResult, Did, EvidenceGrade, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 「链上」证据等级：**永远是 `cpu-proto`**（确定性测试网，不是真实链）。
pub const ONCHAIN_GRADE: EvidenceGrade = EvidenceGrade::CpuProto;

/// 两条确定性测试网：BTC 侧与 ETH 侧各一条本地实例。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainId {
    /// BTC 侧测试网（RGB / Taproot Assets 用）。
    BtcRegtest,
    /// ETH 侧本地链（ERC-8004 / x402 用）。
    EthLocal,
}

impl ChainId {
    pub const ALL: [ChainId; 2] = [ChainId::BtcRegtest, ChainId::EthLocal];

    pub fn as_str(self) -> &'static str {
        match self {
            ChainId::BtcRegtest => "btc-regtest",
            ChainId::EthLocal => "eth-local",
        }
    }
}

/// 一个「已知但不支持」的操作：适配器必须能按名字说清楚为什么拒绝。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalSpec {
    /// 操作名（形如 `rgb.atomic_swap`）。
    pub op: &'static str,
    /// 用哪个拒绝码。
    pub code: RefusalCode,
    /// 为什么不做（写给人看，但结论由代码给出）。
    pub reason: &'static str,
}

/// 一条链上拒绝：**一定带上操作名**，绝不静默。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainRefusal {
    pub op: String,
    pub code: RefusalCode,
    pub detail: String,
}

impl ChainRefusal {
    pub fn new(op: impl Into<String>, code: RefusalCode, detail: impl Into<String>) -> Self {
        Self {
            op: op.into(),
            code,
            detail: detail.into(),
        }
    }

    /// 不认识的操作：按名字拒绝，并在 detail 里列出本适配器支持/已知的拒绝清单。
    pub fn unsupported(op: &str, known: &[RefusalSpec]) -> Self {
        let names: Vec<&str> = known.iter().map(|r| r.op).collect();
        Self::new(
            op,
            RefusalCode::Unsupported,
            format!("unsupported operation `{op}`；已知的拒绝清单 = {names:?}"),
        )
    }

    pub fn to_json(&self) -> Value {
        json!({ "op": self.op, "code": self.code.as_str(), "detail": self.detail })
    }
}

/// 一笔提交给测试网的交易。`id` 是内容的规范哈希。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainTx {
    pub id: String,
    pub chain: ChainId,
    pub op: String,
    pub payload: Value,
    pub submitter: Did,
    pub nonce: u64,
}

impl ChainTx {
    /// 构造并计算 `id`（对除 id 外的全部字段做规范哈希）。
    pub fn new(
        chain: ChainId,
        op: &str,
        submitter: &Did,
        nonce: u64,
        payload: Value,
    ) -> CoreResult<Self> {
        let mut tx = Self {
            id: String::new(),
            chain,
            op: op.to_string(),
            payload,
            submitter: submitter.clone(),
            nonce,
        };
        tx.id = tx.compute_id()?;
        Ok(tx)
    }

    pub fn compute_id(&self) -> CoreResult<String> {
        au4a_core::canonical_hash(&json!({
            "chain": self.chain,
            "op": self.op,
            "payload": self.payload,
            "submitter": self.submitter,
            "nonce": self.nonce,
        }))
    }

    /// 校验内容寻址一致性；篡改任何字段都会失败。
    pub fn verify(&self) -> CoreResult<()> {
        if self.id != self.compute_id()? {
            return Err(CoreError::InvalidSignature);
        }
        Ok(())
    }
}

/// 出块记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub height: u64,
    pub tx_ids: Vec<String>,
}

/// 收据：测试网产出的唯一「成功」凭证，等级写死 `cpu-proto`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub tx_id: String,
    pub chain: ChainId,
    pub op: String,
    pub adapter: String,
    /// 入块高度（未出块时是「下一个高度」）。
    pub height: u64,
    pub confirmations: u64,
    /// 是否已达最终性（最终性之下的块可能被 [`Testnet::reorg`] 回滚）。
    pub finalized: bool,
    pub grade: EvidenceGrade,
}

/// 确定性测试网状态机。
#[derive(Clone, Debug)]
pub struct Testnet {
    chain: ChainId,
    height: u64,
    finality_depth: u64,
    pending: Vec<ChainTx>,
    blocks: Vec<Block>,
    /// 所有被接受过的交易（按 id），重组时用来重建 nonce 占用。
    tx_log: BTreeMap<String, ChainTx>,
    applied: BTreeSet<String>,
    nonces: BTreeMap<String, u64>,
    refusals: Vec<ChainRefusal>,
}

impl Testnet {
    /// `finality_depth`：一个块之上再叠这么多块才算最终。
    pub fn new(chain: ChainId, finality_depth: u64) -> Self {
        Self {
            chain,
            height: 0,
            finality_depth,
            pending: Vec::new(),
            blocks: Vec::new(),
            tx_log: BTreeMap::new(),
            applied: BTreeSet::new(),
            nonces: BTreeMap::new(),
            refusals: Vec::new(),
        }
    }

    /// 默认测试网：BTC 侧 6 个确认、ETH 侧 12 个确认（典型值，纯本地语义）。
    pub fn default_for(chain: ChainId) -> Self {
        match chain {
            ChainId::BtcRegtest => Self::new(chain, 6),
            ChainId::EthLocal => Self::new(chain, 12),
        }
    }

    pub fn chain(&self) -> ChainId {
        self.chain
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn finality_depth(&self) -> u64 {
        self.finality_depth
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn applied_len(&self) -> usize {
        self.applied.len()
    }

    pub fn refusals(&self) -> &[ChainRefusal] {
        &self.refusals
    }

    pub fn last_refusal(&self) -> Option<&ChainRefusal> {
        self.refusals.last()
    }

    /// 记录一条拒绝（适配器自己判定的拒绝也走这里，保证不静默）。
    pub fn record_refusal(&mut self, refusal: ChainRefusal) {
        self.refusals.push(refusal);
    }

    fn deny(&mut self, refusal: ChainRefusal) -> ChainRefusal {
        self.refusals.push(refusal.clone());
        refusal
    }

    /// 接受一笔交易：防篡改 → 防跨链 → 防重放 → 防乱序 nonce → 入待打包区。
    ///
    /// 任何一步失败都返回**带操作名**的 [`ChainRefusal`]（不 panic、不静默）。
    pub fn accept(&mut self, tx: &ChainTx) -> Result<Receipt, ChainRefusal> {
        if tx.verify().is_err() {
            return Err(self.deny(ChainRefusal::new(
                &tx.op,
                RefusalCode::Malformed,
                "tx id 与内容不一致（疑似篡改）",
            )));
        }
        if tx.chain != self.chain {
            return Err(self.deny(ChainRefusal::new(
                &tx.op,
                RefusalCode::Malformed,
                format!("交易链 {} 与本测试网 {} 不符", tx.chain.as_str(), self.chain.as_str()),
            )));
        }
        if self.applied.contains(&tx.id) || self.pending.iter().any(|p| p.id == tx.id) {
            return Err(self.deny(ChainRefusal::new(
                &tx.op,
                RefusalCode::Conflict,
                "重复交易（重放）被拒绝",
            )));
        }
        let key = tx.submitter.as_str().to_string();
        let last = self.nonces.get(&key).copied().unwrap_or(0);
        if tx.nonce <= last && last > 0 {
            return Err(self.deny(ChainRefusal::new(
                &tx.op,
                RefusalCode::StaleEpoch,
                format!("nonce {} 不大于已见 nonce {last}", tx.nonce),
            )));
        }
        self.nonces.insert(key, tx.nonce);
        self.tx_log.insert(tx.id.clone(), tx.clone());
        self.pending.push(tx.clone());
        let height = self.height + 1;
        Ok(Receipt {
            tx_id: tx.id.clone(),
            chain: self.chain,
            op: tx.op.clone(),
            adapter: String::new(),
            height,
            confirmations: 1,
            finalized: false,
            grade: ONCHAIN_GRADE,
        })
    }

    /// 给收据补上适配器名（收据由适配器返回给调用方）。
    pub fn stamp(receipt: &mut Receipt, adapter: &str) {
        receipt.adapter = adapter.to_string();
    }

    /// 出块：把当前待打包区封成一个块。空待打包区不出块。
    pub fn mine(&mut self) -> u64 {
        if self.pending.is_empty() {
            return self.height;
        }
        self.height += 1;
        let txs = std::mem::take(&mut self.pending);
        for tx in &txs {
            self.applied.insert(tx.id.clone());
        }
        self.blocks.push(Block {
            height: self.height,
            tx_ids: txs.iter().map(|t| t.id.clone()).collect(),
        });
        self.height
    }

    /// 出块直到某个高度（便捷方法）。
    pub fn mine_to(&mut self, height: u64) -> u64 {
        while self.height < height {
            let before = self.height;
            self.mine();
            if self.height == before {
                // 没有待打包交易：用空块推进高度（测试网允许空块）。
                self.height += 1;
                self.blocks.push(Block {
                    height: self.height,
                    tx_ids: Vec::new(),
                });
            }
        }
        self.height
    }

    /// 某高度的确认数（0 表示还没入块）。
    pub fn confirmations(&self, height: u64) -> u64 {
        if self.height >= height && height > 0 {
            self.height - height + 1
        } else {
            0
        }
    }

    /// 最终性：高度 `height` 的块之上是否已经叠够 `finality_depth` 个块。
    pub fn is_final(&self, height: u64) -> bool {
        height > 0 && self.height >= height.saturating_add(self.finality_depth)
    }

    /// 已最终化的高度上限。
    pub fn finalized_height(&self) -> u64 {
        self.height.saturating_sub(self.finality_depth)
    }

    /// 重组：回滚最近 `depth` 个块，返回被回滚的交易 id（可被重新打包）。
    ///
    /// 超过最终性的重组被拒绝（`Conflict`）——最终性在测试网里也是硬的。
    pub fn reorg(&mut self, depth: u64) -> Result<Vec<String>, ChainRefusal> {
        if depth == 0 {
            return Ok(Vec::new());
        }
        let safe = self.height.saturating_sub(self.finality_depth);
        if depth > self.height.saturating_sub(safe) {
            return Err(self.deny(ChainRefusal::new(
                "testnet.reorg",
                RefusalCode::Conflict,
                format!(
                    "重组深度 {depth} 超过最终性保护（高度 {}，最终化到 {}）",
                    self.height, safe
                ),
            )));
        }
        let mut dropped = Vec::new();
        for _ in 0..depth {
            if let Some(block) = self.blocks.pop() {
                for id in &block.tx_ids {
                    self.applied.remove(id);
                    dropped.push(id.clone());
                }
                self.height -= 1;
            }
        }
        // 被回滚的交易重新变得可打包：nonce 占用只由「链上 + 待打包」的交易决定。
        self.rebuild_nonces();
        Ok(dropped)
    }

    /// 重建 nonce 占用：只有「已在链上」或「正在待打包」的交易才算占用了 nonce。
    fn rebuild_nonces(&mut self) {
        self.nonces.clear();
        let live: Vec<ChainTx> = self
            .tx_log
            .values()
            .filter(|tx| self.applied.contains(&tx.id) || self.pending.iter().any(|p| p.id == tx.id))
            .cloned()
            .collect();
        for tx in live {
            let key = tx.submitter.as_str().to_string();
            let entry = self.nonces.entry(key).or_insert(0);
            if tx.nonce > *entry {
                *entry = tx.nonce;
            }
        }
    }

    /// 只读投影。
    pub fn to_json(&self) -> Value {
        json!({
            "chain": self.chain.as_str(),
            "height": self.height,
            "finality_depth": self.finality_depth,
            "finalized_height": self.finalized_height(),
            "pending": self.pending.len(),
            "applied": self.applied.len(),
            "refusals": self.refusals.iter().map(ChainRefusal::to_json).collect::<Vec<_>>(),
            "grade": ONCHAIN_GRADE.as_str(),
            "real_network": false,
            "note": "确定性测试网：纯 CPU 状态机，不联网；这不是真实链上执行",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn tx(op: &str, nonce: u64) -> ChainTx {
        ChainTx::new(
            ChainId::BtcRegtest,
            op,
            &did(1),
            nonce,
            json!({ "k": op }),
        )
        .unwrap()
    }

    #[test]
    fn a_tampered_tx_is_refused_by_name() {
        let mut net = Testnet::default_for(ChainId::BtcRegtest);
        let mut t = tx("rgb.transfer", 1);
        t.payload = json!({ "k": "tampered" });
        let err = net.accept(&t).unwrap_err();
        assert_eq!(err.op, "rgb.transfer");
        assert_eq!(err.code, RefusalCode::Malformed);
        assert_eq!(net.refusals().len(), 1);
        assert_eq!(net.pending_len(), 0);
    }

    #[test]
    fn the_wrong_chain_is_refused() {
        let mut net = Testnet::default_for(ChainId::EthLocal);
        let err = net.accept(&tx("erc8004.register", 1)).unwrap_err();
        assert_eq!(err.code, RefusalCode::Malformed);
        assert!(err.detail.contains("btc-regtest"));
    }

    #[test]
    fn replays_and_stale_nonces_are_refused() {
        let mut net = Testnet::default_for(ChainId::BtcRegtest);
        let first = tx("rgb.transfer", 1);
        net.accept(&first).unwrap();
        // 同一笔（同 id）再提交 → 重放。
        let replay = net.accept(&first).unwrap_err();
        assert_eq!(replay.code, RefusalCode::Conflict);
        // 新的 nonce 更小 → 乱序。
        let stale = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.transfer",
            &did(1),
            1,
            json!({ "k": "other" }),
        )
        .unwrap();
        assert_eq!(net.accept(&stale).unwrap_err().code, RefusalCode::StaleEpoch);
        assert_eq!(net.refusals().len(), 2);
    }

    #[test]
    fn mining_and_finality_follow_the_depth() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 3);
        let t = tx("rgb.transfer", 1);
        let receipt = net.accept(&t).unwrap();
        assert_eq!(receipt.height, 1);
        assert!(!receipt.finalized);
        assert_eq!(receipt.grade, EvidenceGrade::CpuProto);
        assert_eq!(net.mine(), 1);
        assert_eq!(net.confirmations(1), 1);
        assert!(!net.is_final(1));
        net.mine_to(4);
        assert_eq!(net.confirmations(1), 4);
        assert!(net.is_final(1));
        assert_eq!(net.finalized_height(), 1);
    }

    #[test]
    fn receipts_are_always_cpu_proto() {
        let mut net = Testnet::default_for(ChainId::EthLocal);
        let t = ChainTx::new(ChainId::EthLocal, "x402.pay", &did(2), 1, json!({})).unwrap();
        let mut receipt = net.accept(&t).unwrap();
        Testnet::stamp(&mut receipt, "x402");
        assert_eq!(receipt.grade, ONCHAIN_GRADE);
        assert_eq!(receipt.grade.as_str(), "cpu-proto");
        assert_eq!(receipt.adapter, "x402");
        let value = net.to_json();
        assert_eq!(value["real_network"], json!(false));
        assert_eq!(value["grade"], json!("cpu-proto"));
    }

    #[test]
    fn reorg_is_bounded_by_finality() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let t = tx("rgb.transfer", 1);
        net.accept(&t).unwrap();
        net.mine();
        net.mine_to(3);
        assert!(net.is_final(1));
        // 高度 3、最终化到 1：只能回滚 2 个块。
        let err = net.reorg(3).unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
        let dropped = net.reorg(2).unwrap();
        assert!(dropped.is_empty(), "回滚的是空块");
        assert_eq!(net.height(), 1);
        assert!(!net.is_final(1));
    }

    #[test]
    fn a_reorged_tx_can_be_replayed_but_not_before() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 5);
        let t = tx("rgb.transfer", 1);
        net.accept(&t).unwrap();
        net.mine();
        assert_eq!(net.accept(&t).unwrap_err().code, RefusalCode::Conflict);
        // 回滚 1 个块（未达最终性）→ 交易重新变得可打包。
        let dropped = net.reorg(1).unwrap();
        assert_eq!(dropped, vec![t.id.clone()]);
        assert!(net.accept(&t).is_ok());
    }

    #[test]
    fn unsupported_op_refusals_carry_the_name_and_the_known_list() {
        let known = [
            RefusalSpec {
                op: "rgb.atomic_swap",
                code: RefusalCode::Unsupported,
                reason: "需要 HTLC",
            },
            RefusalSpec {
                op: "rgb.lightning_route",
                code: RefusalCode::Unsupported,
                reason: "需要 Lightning 网络",
            },
        ];
        let refusal = ChainRefusal::unsupported("rgb.magic", &known);
        assert_eq!(refusal.op, "rgb.magic");
        assert_eq!(refusal.code, RefusalCode::Unsupported);
        assert!(refusal.detail.contains("rgb.atomic_swap"));
        assert!(refusal.to_json()["code"] == json!("unsupported"));
    }
}
