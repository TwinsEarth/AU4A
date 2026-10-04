//! v1.8.1 RGB 集成：客户端验证的密封转移包（**确定性测试网原型**）。
//!
//! RGB 的核心思想是**客户端验证**：链上只放承诺（commitment），资产状态由持有者自己验证。
//! 这里把这个语义做成一个纯 CPU 状态机：
//!
//! * **契约（genesis）**：`rgb.issue` 生成资产 id（内容寻址）、总量与初始分配（封印 → 数量）；
//! * **密封转移**：`rgb.transfer` 提交一个 [`TransferBundle`]——输入封印、输出封印与致盲因子，
//!   包体用规范哈希封口（[`seal_bundle`]）；验证规则：输入未花、输出封印唯一、
//!   `Σ输入 == Σ输出`、承诺与内容一致；
//! * **最终化**：`rgb.finalize` 只认**已达最终性**的收据；未达最终性一律拒绝（fail-closed），
//!   这样 [`Testnet::reorg`] 回滚未最终化的转移时不会留下「已最终化」的假象；
//! * **具名拒绝清单**：原子交换 / 闪电路由 / 无锚增发 / 共识桥等**按名字**拒绝
//!   （[`RefusalCode::Unsupported`]），未知操作同样按名字拒绝，绝不静默通过。
//!
//! 证据等级恒为 `cpu-proto`：这是**语义原型**，不是真实 RGB 实现（没有真实的 tapret/opret
//! 承诺、没有真实的见证、没有共识层）。

use std::collections::{BTreeMap, BTreeSet};

use au4a_core::{CoreError, CoreResult, Credits, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::testnet::{ChainRefusal, ChainTx, Receipt, RefusalSpec, Testnet, ONCHAIN_GRADE};

/// 本适配器支持的四个操作（其余一律按名字拒绝）。
pub const RGB_SUPPORTED: [&str; 4] = ["rgb.issue", "rgb.transfer", "rgb.verify", "rgb.finalize"];

/// 具名拒绝清单：这些操作我们知道是什么，但**本原型不做**，原因写在 reason 里。
pub const RGB_REFUSALS: [RefusalSpec; 5] = [
    RefusalSpec {
        op: "rgb.atomic_swap",
        code: RefusalCode::Unsupported,
        reason: "原子交换需要 HTLC/哈希时间锁语义，本原型未实现",
    },
    RefusalSpec {
        op: "rgb.lightning_route",
        code: RefusalCode::Unsupported,
        reason: "闪电网络路由需要真实 P2P 与通道状态，本轨道不联网",
    },
    RefusalSpec {
        op: "rgb.unanchored_issuance",
        code: RefusalCode::PolicyDenied,
        reason: "无锚增发会破坏总量守恒，策略上永久拒绝",
    },
    RefusalSpec {
        op: "rgb.consensus_bridge",
        code: RefusalCode::Unsupported,
        reason: "共识层桥接需要真实 BTC 节点与 SPV 证明，本轨道只做语义原型",
    },
    RefusalSpec {
        op: "rgb.blind_unsealed",
        code: RefusalCode::Malformed,
        reason: "未带致盲因子的转移包不可验证，拒绝",
    },
];

/// 单次使用封印（对应一个未花费输出）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Seal {
    pub txid: String,
    pub vout: u32,
}

impl Seal {
    pub fn new(txid: impl Into<String>, vout: u32) -> Self {
        Self {
            txid: txid.into(),
            vout,
        }
    }

    pub fn key(&self) -> String {
        format!("{}:{}", self.txid, self.vout)
    }
}

/// RGB 契约的客户端验证状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbContract {
    pub asset_id: String,
    pub ticker: String,
    /// 发行总量（之后任何转移都不得改变它）。
    pub supply: Credits,
    /// 封印 → 余额。
    pub allocations: BTreeMap<String, Credits>,
    /// 已花费的封印。
    pub spent: BTreeSet<String>,
    /// 已最终化的转移包 id。
    pub finalized: Vec<String>,
}

impl RgbContract {
    /// 创世：生成资产 id（内容寻址）与初始分配。
    pub fn issue(
        ticker: &str,
        supply: Credits,
        genesis_seal: Seal,
        nonce: u64,
    ) -> CoreResult<Self> {
        if ticker.is_empty() {
            return Err(CoreError::InvalidKind);
        }
        if supply == Credits::ZERO {
            return Err(CoreError::ZeroAmount);
        }
        let asset_id = au4a_core::canonical_hash(&json!({
            "ticker": ticker,
            "supply": supply,
            "seal": genesis_seal,
            "nonce": nonce,
        }))?;
        let mut allocations = BTreeMap::new();
        allocations.insert(genesis_seal.key(), supply);
        Ok(Self {
            asset_id,
            ticker: ticker.to_string(),
            supply,
            allocations,
            spent: BTreeSet::new(),
            finalized: Vec::new(),
        })
    }

    /// 链上表示总量（未花费封印上的余额之和）。
    pub fn circulating(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for (seal, amount) in &self.allocations {
            if !self.spent.contains(seal) {
                sum = sum.checked_add(*amount)?;
            }
        }
        Ok(sum)
    }

    /// 总量守恒：流通量必须等于发行量（未花费余额总和恒定）。
    pub fn check_supply_conservation(&self) -> CoreResult<()> {
        if self.circulating()? != self.supply {
            return Err(CoreError::Overflow);
        }
        Ok(())
    }

    pub fn balance_of(&self, seal: &Seal) -> Credits {
        self.allocations
            .get(&seal.key())
            .copied()
            .unwrap_or_default()
    }

    pub fn is_spent(&self, seal: &Seal) -> bool {
        self.spent.contains(&seal.key())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "asset_id": self.asset_id,
            "ticker": self.ticker,
            "supply": self.supply,
            "circulating": self.circulating().unwrap_or_default(),
            "seals": self.allocations.len(),
            "spent": self.spent.len(),
            "finalized": self.finalized.len(),
            "grade": ONCHAIN_GRADE.as_str(),
        })
    }
}

/// 密封转移包：输入封印 → 输出封印，带致盲因子与承诺。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferBundle {
    pub asset_id: String,
    pub inputs: Vec<Seal>,
    pub outputs: Vec<(Seal, Credits)>,
    /// 致盲因子（原型里是一个内容哈希串；真实 RGB 是盲化承诺）。
    pub blinding: String,
    /// 承诺：对除本字段外的全部内容做规范哈希。
    pub commitment: String,
}

impl TransferBundle {
    pub fn new(
        asset_id: &str,
        inputs: Vec<Seal>,
        outputs: Vec<(Seal, Credits)>,
        blinding: &str,
    ) -> CoreResult<Self> {
        let mut bundle = Self {
            asset_id: asset_id.to_string(),
            inputs,
            outputs,
            blinding: blinding.to_string(),
            commitment: String::new(),
        };
        bundle.commitment = bundle.compute_commitment()?;
        Ok(bundle)
    }

    fn compute_commitment(&self) -> CoreResult<String> {
        au4a_core::canonical_hash(&json!({
            "asset_id": self.asset_id,
            "inputs": self.inputs,
            "outputs": self.outputs.iter().map(|(s, a)| json!([s, a])).collect::<Vec<_>>(),
            "blinding": self.blinding,
        }))
    }

    pub fn id(&self) -> &str {
        &self.commitment
    }

    /// 校验密封包：承诺自洽、输入输出非空、封印不重复、`Σ输入 == Σ输出`。
    pub fn verify(&self) -> Result<(), ChainRefusal> {
        if self.commitment
            != self.compute_commitment().map_err(|_| {
                ChainRefusal::new("rgb.transfer", RefusalCode::Malformed, "承诺计算失败")
            })?
        {
            return Err(ChainRefusal::new(
                "rgb.transfer",
                RefusalCode::Malformed,
                "承诺与包体内容不一致（疑似篡改）",
            ));
        }
        if self.blinding.is_empty() {
            return Err(ChainRefusal::new(
                "rgb.blind_unsealed",
                RefusalCode::Malformed,
                RGB_REFUSALS[4].reason,
            ));
        }
        if self.inputs.is_empty() || self.outputs.is_empty() {
            return Err(ChainRefusal::new(
                "rgb.transfer",
                RefusalCode::Malformed,
                "输入或输出为空",
            ));
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for seal in &self.inputs {
            if !seen.insert(seal.key()) {
                return Err(ChainRefusal::new(
                    "rgb.transfer",
                    RefusalCode::Malformed,
                    format!("输入封印重复：{}", seal.key()),
                ));
            }
        }
        let mut out_seen: BTreeSet<String> = BTreeSet::new();
        for (seal, amount) in &self.outputs {
            if *amount == Credits::ZERO {
                return Err(ChainRefusal::new(
                    "rgb.transfer",
                    RefusalCode::Malformed,
                    "输出金额为零",
                ));
            }
            if !out_seen.insert(seal.key()) {
                return Err(ChainRefusal::new(
                    "rgb.transfer",
                    RefusalCode::Malformed,
                    format!("输出封印重复：{}", seal.key()),
                ));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "asset_id": self.asset_id,
            "inputs": self.inputs,
            "outputs": self.outputs,
            "blinding": self.blinding,
            "commitment": self.commitment,
        })
    }
}

/// 适配器执行结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RgbOutcome {
    pub op: String,
    pub accepted: bool,
    pub detail: String,
    pub receipt: Option<Receipt>,
    pub grade: String,
}

/// RGB 适配器：所有状态都在本地，所有操作都要先过测试网（防重放/nonce），再到客户端验证。
#[derive(Clone, Debug, Default)]
pub struct RgbAdapter {
    pub contract: Option<RgbContract>,
}

impl RgbAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contract(&self) -> CoreResult<&RgbContract> {
        self.contract.as_ref().ok_or(CoreError::UnknownAgent)
    }

    /// 执行一笔交易。**已知但拒绝**的操作按其具名拒绝码拒绝；不认识的操作按名字拒绝
    /// （`Unsupported`，并在 detail 里列出已知清单）。两种情况都会记进测试网。
    pub fn execute(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<RgbOutcome, ChainRefusal> {
        match tx.op.as_str() {
            "rgb.issue" => self.issue(net, tx),
            "rgb.transfer" => self.transfer(net, tx),
            "rgb.verify" => self.verify_bundle(net, tx),
            "rgb.finalize" => self.finalize(net, tx),
            _ => {
                let refusal = match RGB_REFUSALS.iter().find(|spec| spec.op == tx.op) {
                    Some(spec) => ChainRefusal::new(&tx.op, spec.code, spec.reason),
                    None => ChainRefusal::unsupported(&tx.op, &RGB_REFUSALS),
                };
                net.record_refusal(refusal.clone());
                Err(refusal)
            }
        }
    }

    fn issue(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<RgbOutcome, ChainRefusal> {
        if self.contract.is_some() {
            let refusal = ChainRefusal::new(
                "rgb.issue",
                RefusalCode::Conflict,
                "本适配器只允许一次创世发行",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let ticker = tx
            .payload
            .get("ticker")
            .and_then(Value::as_str)
            .ok_or_else(|| ChainRefusal::new("rgb.issue", RefusalCode::Malformed, "缺少 ticker"))?;
        let supply = tx
            .payload
            .get("supply")
            .and_then(Value::as_i64)
            .and_then(|v| Credits::new(v).ok())
            .ok_or_else(|| ChainRefusal::new("rgb.issue", RefusalCode::Malformed, "缺少 supply"))?;
        let seal = Seal::new(tx.id.clone(), 0);
        let contract = RgbContract::issue(ticker, supply, seal, tx.nonce).map_err(|err| {
            ChainRefusal::new("rgb.issue", RefusalCode::Malformed, err.to_string())
        })?;
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "rgb");
        self.contract = Some(contract);
        Ok(RgbOutcome {
            op: "rgb.issue".to_string(),
            accepted: true,
            detail: format!("发行 {ticker} 总量 {supply}（封印 {}:0）", tx.id),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    fn transfer(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<RgbOutcome, ChainRefusal> {
        let bundle: TransferBundle = serde_json::from_value(tx.payload.clone()).map_err(|_| {
            ChainRefusal::new("rgb.transfer", RefusalCode::Malformed, "转移包不可解析")
        })?;
        if let Err(refusal) = bundle.verify() {
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let contract = self.contract.as_mut().ok_or_else(|| {
            ChainRefusal::new(
                "rgb.transfer",
                RefusalCode::Conflict,
                "尚未发行，无契约状态",
            )
        })?;
        if bundle.asset_id != contract.asset_id {
            let refusal = ChainRefusal::new(
                "rgb.transfer",
                RefusalCode::Malformed,
                "转移包的资产 id 与本契约不符",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        // 输入必须存在且未花：双花在这里被拒。
        let mut total_in = Credits::ZERO;
        for seal in &bundle.inputs {
            if contract.is_spent(seal) {
                let refusal = ChainRefusal::new(
                    "rgb.transfer",
                    RefusalCode::Conflict,
                    format!("封印 {} 已被花费（双花）", seal.key()),
                );
                net.record_refusal(refusal.clone());
                return Err(refusal);
            }
            let amount = contract.balance_of(seal);
            if amount == Credits::ZERO {
                let refusal = ChainRefusal::new(
                    "rgb.transfer",
                    RefusalCode::Malformed,
                    format!("封印 {} 不存在或余额为零", seal.key()),
                );
                net.record_refusal(refusal.clone());
                return Err(refusal);
            }
            total_in = total_in.checked_add(amount).map_err(|_| {
                ChainRefusal::new("rgb.transfer", RefusalCode::Malformed, "输入金额溢出")
            })?;
        }
        let mut total_out = Credits::ZERO;
        for (_, amount) in &bundle.outputs {
            total_out = total_out.checked_add(*amount).map_err(|_| {
                ChainRefusal::new("rgb.transfer", RefusalCode::Malformed, "输出金额溢出")
            })?;
        }
        if total_in != total_out {
            let refusal = ChainRefusal::new(
                "rgb.transfer",
                RefusalCode::Malformed,
                format!("客户端验证失败：Σ输入 {total_in} != Σ输出 {total_out}"),
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "rgb");
        // 客户端状态更新（先花输入，再记输出）。
        for seal in &bundle.inputs {
            contract.spent.insert(seal.key());
        }
        for (seal, amount) in &bundle.outputs {
            contract.allocations.insert(seal.key(), *amount);
        }
        contract.check_supply_conservation().map_err(|_| {
            ChainRefusal::new("rgb.transfer", RefusalCode::Malformed, "总量守恒被破坏")
        })?;
        Ok(RgbOutcome {
            op: "rgb.transfer".to_string(),
            accepted: true,
            detail: format!(
                "密封转移 {} → {} 个输出，承诺 {}",
                total_in,
                bundle.outputs.len(),
                au4a_core::short_id(&bundle.commitment)
            ),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    fn verify_bundle(&self, net: &mut Testnet, tx: &ChainTx) -> Result<RgbOutcome, ChainRefusal> {
        let bundle: TransferBundle = serde_json::from_value(tx.payload.clone()).map_err(|_| {
            ChainRefusal::new("rgb.verify", RefusalCode::Malformed, "转移包不可解析")
        })?;
        if let Err(refusal) = bundle.verify() {
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let contract = self
            .contract
            .as_ref()
            .ok_or_else(|| ChainRefusal::new("rgb.verify", RefusalCode::Malformed, "尚未发行"))?;
        contract.check_supply_conservation().map_err(|_| {
            ChainRefusal::new("rgb.verify", RefusalCode::Conflict, "总量守恒被破坏")
        })?;
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "rgb");
        Ok(RgbOutcome {
            op: "rgb.verify".to_string(),
            accepted: true,
            detail: format!(
                "客户端验证通过：承诺 {}，流通量 {} / 发行量 {}",
                au4a_core::short_id(&bundle.commitment),
                contract.circulating().unwrap_or_default(),
                contract.supply
            ),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    /// 最终化：只认**已达最终性**的收据；否则拒绝（fail-closed）。
    fn finalize(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<RgbOutcome, ChainRefusal> {
        let bundle_id = tx
            .payload
            .get("bundle_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainRefusal::new("rgb.finalize", RefusalCode::Malformed, "缺少 bundle_id")
            })?
            .to_string();
        let height = tx
            .payload
            .get("height")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ChainRefusal::new("rgb.finalize", RefusalCode::Malformed, "缺少 height")
            })?;
        if !net.is_final(height) {
            let refusal = ChainRefusal::new(
                "rgb.finalize",
                RefusalCode::Timeout,
                format!(
                    "高度 {height} 尚未最终（当前高度 {}，最终化到 {}）：等够了再最终化",
                    net.height(),
                    net.finalized_height()
                ),
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let contract = self.contract.as_mut().ok_or_else(|| {
            ChainRefusal::new(
                "rgb.finalize",
                RefusalCode::Conflict,
                "尚未发行，无契约状态",
            )
        })?;
        if contract.finalized.contains(&bundle_id) {
            let refusal =
                ChainRefusal::new("rgb.finalize", RefusalCode::Conflict, "该转移包已经最终化");
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "rgb");
        contract.finalized.push(bundle_id.clone());
        Ok(RgbOutcome {
            op: "rgb.finalize".to_string(),
            accepted: true,
            detail: format!(
                "转移包 {} 在高度 {height} 最终化（测试网语义，非真实链）",
                au4a_core::short_id(&bundle_id)
            ),
            receipt: Some(receipt),
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

    fn issue(net: &mut Testnet, adapter: &mut RgbAdapter, ticker: &str, supply: i64) -> ChainTx {
        let tx = ChainTx::new(
            net.chain(),
            "rgb.issue",
            &did(1),
            1,
            json!({ "ticker": ticker, "supply": supply }),
        )
        .unwrap();
        adapter.execute(net, &tx).unwrap();
        tx
    }

    fn transfer(
        net: &mut Testnet,
        adapter: &mut RgbAdapter,
        inputs: Vec<Seal>,
        outputs: Vec<(Seal, i64)>,
        nonce: u64,
    ) -> ChainTx {
        let bundle = TransferBundle::new(
            &adapter.contract().unwrap().asset_id,
            inputs,
            outputs.into_iter().map(|(s, a)| (s, Credits(a))).collect(),
            "blind-au4a",
        )
        .unwrap();
        let tx = ChainTx::new(
            net.chain(),
            "rgb.transfer",
            &did(1),
            nonce,
            serde_json::to_value(&bundle).unwrap(),
        )
        .unwrap();
        adapter.execute(net, &tx).unwrap();
        tx
    }

    #[test]
    fn issuing_creates_a_content_addressed_asset_with_conserved_supply() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = RgbAdapter::new();
        let genesis = issue(&mut net, &mut adapter, "USDT-RGB", 1_000);
        let contract = adapter.contract().unwrap();
        assert_eq!(contract.ticker, "USDT-RGB");
        assert_eq!(contract.supply, Credits(1_000));
        assert_eq!(contract.circulating().unwrap(), Credits(1_000));
        assert_eq!(contract.asset_id.len(), 64);
        contract.check_supply_conservation().unwrap();
        // 创世封印挂在发行交易自己的 id 上。
        assert_eq!(
            contract.balance_of(&Seal::new(genesis.id.clone(), 0)),
            Credits(1_000)
        );
        // 第二次发行被拒绝。
        let again = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.issue",
            &did(1),
            2,
            json!({ "ticker": "OTHER", "supply": 1 }),
        )
        .unwrap();
        let err = adapter.execute(&mut net, &again).unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
        assert_eq!(net.refusals().len(), 1);
    }

    #[test]
    fn a_sealed_transfer_moves_value_and_conserves_the_supply() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = RgbAdapter::new();
        let genesis = issue(&mut net, &mut adapter, "USDT-RGB", 1_000);
        net.mine();
        let tx = transfer(
            &mut net,
            &mut adapter,
            vec![Seal::new(genesis.id.clone(), 0)],
            vec![
                (Seal::new("alice-out", 0), 600),
                (Seal::new("bob-out", 1), 400),
            ],
            2,
        );
        net.mine();
        let contract = adapter.contract().unwrap();
        assert!(contract.is_spent(&Seal::new(genesis.id.clone(), 0)));
        assert_eq!(
            contract.balance_of(&Seal::new("alice-out", 0)),
            Credits(600)
        );
        assert_eq!(contract.balance_of(&Seal::new("bob-out", 1)), Credits(400));
        assert_eq!(contract.circulating().unwrap(), Credits(1_000));
        contract.check_supply_conservation().unwrap();
        assert_eq!(tx.op, "rgb.transfer");
        assert_eq!(net.refusals().len(), 0);
    }

    #[test]
    fn a_tampered_bundle_is_refused() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = RgbAdapter::new();
        let genesis = issue(&mut net, &mut adapter, "USDT-RGB", 1_000);
        let mut bundle = TransferBundle::new(
            &adapter.contract().unwrap().asset_id,
            vec![Seal::new(genesis.id.clone(), 0)],
            vec![(Seal::new("out", 0), Credits(1_000))],
            "blind",
        )
        .unwrap();
        // 篡改输出金额但不改承诺 → 必须被拒。
        bundle.outputs = vec![(Seal::new("out", 0), Credits(2_000))];
        let tx = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.transfer",
            &did(1),
            2,
            serde_json::to_value(&bundle).unwrap(),
        )
        .unwrap();
        let err = adapter.execute(&mut net, &tx).unwrap_err();
        assert_eq!(err.code, RefusalCode::Malformed);
        assert!(err.detail.contains("承诺"));
        assert_eq!(net.refusals().len(), 1);
    }

    #[test]
    fn double_spending_a_seal_is_refused() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = RgbAdapter::new();
        let genesis = issue(&mut net, &mut adapter, "USDT-RGB", 1_000);
        net.mine();
        transfer(
            &mut net,
            &mut adapter,
            vec![Seal::new(genesis.id.clone(), 0)],
            vec![(Seal::new("a", 0), 1_000)],
            2,
        );
        net.mine();
        // 再用同一个（已花费）封印去转移。
        let bundle = TransferBundle::new(
            &adapter.contract().unwrap().asset_id,
            vec![Seal::new(genesis.id.clone(), 0)],
            vec![(Seal::new("b", 0), Credits(1_000))],
            "blind",
        )
        .unwrap();
        let tx = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.transfer",
            &did(1),
            3,
            serde_json::to_value(&bundle).unwrap(),
        )
        .unwrap();
        let err = adapter.execute(&mut net, &tx).unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
        assert!(err.detail.contains("双花"));
        assert_eq!(
            adapter.contract().unwrap().circulating().unwrap(),
            Credits(1_000)
        );
    }

    #[test]
    fn a_bundle_whose_inputs_do_not_match_its_outputs_is_refused() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = RgbAdapter::new();
        let genesis = issue(&mut net, &mut adapter, "USDT-RGB", 1_000);
        net.mine();
        // Σ输入 1000 != Σ输出 999：客户端验证必须拦住。
        let bundle = TransferBundle::new(
            &adapter.contract().unwrap().asset_id,
            vec![Seal::new(genesis.id.clone(), 0)],
            vec![(Seal::new("x", 0), Credits(999))],
            "blind",
        )
        .unwrap();
        let tx = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.transfer",
            &did(1),
            2,
            serde_json::to_value(&bundle).unwrap(),
        )
        .unwrap();
        let err = adapter.execute(&mut net, &tx).unwrap_err();
        assert_eq!(err.code, RefusalCode::Malformed);
        assert!(err.detail.contains("Σ输入"));
    }

    #[test]
    fn finalizing_before_finality_is_refused_and_succeeds_afterwards() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 3);
        let mut adapter = RgbAdapter::new();
        let genesis = issue(&mut net, &mut adapter, "USDT-RGB", 1_000);
        net.mine();
        transfer(
            &mut net,
            &mut adapter,
            vec![Seal::new(genesis.id.clone(), 0)],
            vec![(Seal::new("a", 0), 1_000)],
            2,
        );
        net.mine();
        let bundle_id = adapter.contract().unwrap().asset_id.clone();
        let early = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.finalize",
            &did(1),
            3,
            json!({ "bundle_id": bundle_id, "height": 2 }),
        )
        .unwrap();
        let err = adapter.execute(&mut net, &early).unwrap_err();
        assert_eq!(err.code, RefusalCode::Timeout);
        assert!(err.detail.contains("尚未最终"));
        // 叠够最终性深度（高度 2 + 3 = 5）。
        net.mine_to(5);
        assert!(net.is_final(2));
        let late = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.finalize",
            &did(1),
            4,
            json!({ "bundle_id": adapter.contract().unwrap().asset_id.clone(), "height": 2 }),
        )
        .unwrap();
        let outcome = adapter.execute(&mut net, &late).unwrap();
        assert!(outcome.accepted);
        assert_eq!(outcome.grade, "cpu-proto");
        assert_eq!(adapter.contract().unwrap().finalized.len(), 1);
        // 重复最终化被拒。
        let again = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.finalize",
            &did(1),
            5,
            json!({ "bundle_id": adapter.contract().unwrap().asset_id.clone(), "height": 2 }),
        )
        .unwrap();
        assert_eq!(
            adapter.execute(&mut net, &again).unwrap_err().code,
            RefusalCode::Conflict
        );
    }

    #[test]
    fn unsupported_operations_are_refused_by_name() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = RgbAdapter::new();
        issue(&mut net, &mut adapter, "USDT-RGB", 1_000);
        for (op, code) in [
            ("rgb.atomic_swap", RefusalCode::Unsupported),
            ("rgb.lightning_route", RefusalCode::Unsupported),
            ("rgb.unanchored_issuance", RefusalCode::PolicyDenied),
            ("rgb.consensus_bridge", RefusalCode::Unsupported),
            ("rgb.totally_unknown", RefusalCode::Unsupported),
        ] {
            let tx = ChainTx::new(ChainId::BtcRegtest, op, &did(1), 9, json!({})).unwrap();
            let err = adapter.execute(&mut net, &tx).unwrap_err();
            assert_eq!(err.op, op, "拒绝必须带上操作名");
            assert_eq!(err.code, code);
        }
        // 所有具名拒绝都被记进测试网，没有静默通过。
        assert_eq!(net.refusals().len(), 5);
        assert_eq!(RGB_REFUSALS.len(), 5);
        assert_eq!(RGB_SUPPORTED.len(), 4);
    }

    #[test]
    fn replaying_the_same_transfer_transaction_is_refused() {
        let mut net = Testnet::new(ChainId::BtcRegtest, 2);
        let mut adapter = RgbAdapter::new();
        let genesis = issue(&mut net, &mut adapter, "USDT-RGB", 1_000);
        net.mine();
        let bundle = TransferBundle::new(
            &adapter.contract().unwrap().asset_id,
            vec![Seal::new(genesis.id.clone(), 0)],
            vec![(Seal::new("a", 0), Credits(1_000))],
            "blind",
        )
        .unwrap();
        let tx = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.transfer",
            &did(1),
            2,
            serde_json::to_value(&bundle).unwrap(),
        )
        .unwrap();
        adapter.execute(&mut net, &tx).unwrap();
        net.mine();
        // 同一笔交易再提交一次 → 测试网按重放拒绝（封印已花时也可能先被双花检查拦住）。
        let err = adapter.execute(&mut net, &tx).unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
        assert!(
            err.detail.contains("重放") || err.detail.contains("双花"),
            "拒绝原因应说明重放或双花，实际：{}",
            err.detail
        );
    }

    #[test]
    fn the_local_escrow_and_the_rgb_supply_are_the_same_number() {
        let who = did(9);
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(1_000)).unwrap();
        let mut book = BridgeBook::new();
        book.bridge_out(&mut ledger, &who, Credits(400), "rgb:USDT-RGB", "rgb")
            .unwrap();

        let mut net = Testnet::new(ChainId::BtcRegtest, 1);
        let mut adapter = RgbAdapter::new();
        issue(&mut net, &mut adapter, "USDT-RGB", 400);
        // 双轨守恒：本地托管 == 链上表示 == RGB 流通量。
        assert_eq!(book.escrowed(), Credits(400));
        assert_eq!(book.chain_supply(), Credits(400));
        assert_eq!(
            adapter.contract().unwrap().circulating().unwrap(),
            Credits(400)
        );
        book.require_consistent(&ledger).unwrap();
        adapter
            .contract()
            .unwrap()
            .check_supply_conservation()
            .unwrap();
        ledger.check_conservation().unwrap();
    }
}
