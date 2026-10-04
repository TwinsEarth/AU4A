//! v1.8.6 信誉桥接：把**链上声誉事件**映射进本地 4 维信誉（**确定性测试网原型**）。
//!
//! 链上的声誉是「事件流」：结算最终化、x402 支付结清、ERC-8004 反馈、验证背书、争议罚没。
//! 本地信誉是 **4 维**（全部整数基点）：
//!
//! | 维度 | 含义 | 主要来源事件 |
//! |---|---|---|
//! | `reliability` | 说到做到（按时结算） | `settlement_final`、`x402_settled`、`dispute_slashed`（负向） |
//! | `quality` | 交付质量 | `feedback`（ERC-8004 反馈） |
//! | `honesty` | 诚实（不伪造、不双花） | `validation`（验证背书）、`dispute_slashed`（负向） |
//! | `availability` | 可用性 | `x402_settled`、`settlement_final` |
//!
//! 三条规则：
//!
//! 1. **不可转让**：没有任何接口能把信誉从一个 DID 搬到另一个 DID；`reputation.transfer` /
//!    `reputation.buy` 等请求**按名字拒绝**（[`REPUTATION_REFUSALS`]）。
//! 2. **只认最终化事件**：未达最终性的链上事件一律拒绝（`Timeout`）——链上回滚不该污染信誉。
//! 3. **有界更新**：`new = old + (score − old) × weight_bp / 10000`，单步位移上限
//!    [`MAX_STEP_BP`]；重复事件向目标值**单调收敛且永不过冲**（有测试钉住）。
//!
//! 证据等级恒为 `cpu-proto`。

use std::collections::{BTreeMap, BTreeSet};

use au4a_core::{CoreError, CoreResult, Credits, Did, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::testnet::{ChainRefusal, ChainTx, RefusalSpec, Testnet, ONCHAIN_GRADE};

/// 单步最大位移（基点）。
pub const MAX_STEP_BP: i64 = 2_000;
/// 初始信誉（基点）：中性 5000。
pub const NEUTRAL_BP: i64 = 5_000;

/// 本适配器支持的操作。
pub const REPUTATION_SUPPORTED: [&str; 2] = ["reputation.apply_event", "reputation.read"];

/// 具名拒绝清单。
pub const REPUTATION_REFUSALS: [RefusalSpec; 4] = [
    RefusalSpec {
        op: "reputation.transfer",
        code: RefusalCode::PolicyDenied,
        reason: "信誉不可转让：能买卖的信誉就不再是信誉（参考项目的教训）",
    },
    RefusalSpec {
        op: "reputation.buy",
        code: RefusalCode::PolicyDenied,
        reason: "信誉不可购买：只由行为事件累积",
    },
    RefusalSpec {
        op: "reputation.bulk_import",
        code: RefusalCode::Unsupported,
        reason: "不支持批量导入外部信誉：没有可验证的来源",
    },
    RefusalSpec {
        op: "reputation.reset",
        code: RefusalCode::PolicyDenied,
        reason: "不支持重置信誉：历史事件是审计线索，不能被抹掉",
    },
];

/// 本地信誉的 4 个维度。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReputationDim {
    Reliability,
    Quality,
    Honesty,
    Availability,
}

impl ReputationDim {
    pub const ALL: [ReputationDim; 4] = [
        ReputationDim::Reliability,
        ReputationDim::Quality,
        ReputationDim::Honesty,
        ReputationDim::Availability,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReputationDim::Reliability => "reliability",
            ReputationDim::Quality => "quality",
            ReputationDim::Honesty => "honesty",
            ReputationDim::Availability => "availability",
        }
    }
}

/// 链上声誉事件的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReputationEventKind {
    /// 结算最终化（RGB / Taproot）。
    SettlementFinal,
    /// x402 支付结清。
    X402Settled,
    /// ERC-8004 反馈。
    Feedback,
    /// 验证背书。
    Validation,
    /// 争议罚没（负向事件）。
    DisputeSlashed,
}

impl ReputationEventKind {
    pub const ALL: [ReputationEventKind; 5] = [
        ReputationEventKind::SettlementFinal,
        ReputationEventKind::X402Settled,
        ReputationEventKind::Feedback,
        ReputationEventKind::Validation,
        ReputationEventKind::DisputeSlashed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReputationEventKind::SettlementFinal => "settlement_final",
            ReputationEventKind::X402Settled => "x402_settled",
            ReputationEventKind::Feedback => "feedback",
            ReputationEventKind::Validation => "validation",
            ReputationEventKind::DisputeSlashed => "dispute_slashed",
        }
    }

    /// 该事件影响哪些维度（**不可转让**：只影响事件主体的本地信誉）。
    pub fn dimensions(self) -> &'static [ReputationDim] {
        match self {
            ReputationEventKind::SettlementFinal => {
                &[ReputationDim::Reliability, ReputationDim::Availability]
            }
            ReputationEventKind::X402Settled => {
                &[ReputationDim::Reliability, ReputationDim::Availability]
            }
            ReputationEventKind::Feedback => &[ReputationDim::Quality],
            ReputationEventKind::Validation => &[ReputationDim::Honesty],
            ReputationEventKind::DisputeSlashed => {
                &[ReputationDim::Reliability, ReputationDim::Honesty]
            }
        }
    }

    /// 更新权重（基点）：验证背书与反馈最有信息量，罚没最重。
    pub fn weight_bp(self) -> i64 {
        match self {
            ReputationEventKind::SettlementFinal => 4_000,
            ReputationEventKind::X402Settled => 3_000,
            ReputationEventKind::Feedback => 6_000,
            ReputationEventKind::Validation => 5_000,
            ReputationEventKind::DisputeSlashed => 8_000,
        }
    }
}

/// 一条链上声誉事件（已经最终化）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainReputationEvent {
    pub id: String,
    pub agent: String,
    pub kind: ReputationEventKind,
    pub score_bp: i64,
    pub height: u64,
}

impl ChainReputationEvent {
    pub fn new(
        agent: &Did,
        kind: ReputationEventKind,
        score_bp: i64,
        height: u64,
    ) -> CoreResult<Self> {
        if !(0..=10_000).contains(&score_bp) {
            return Err(CoreError::InvalidKind);
        }
        let id = au4a_core::canonical_hash(&json!({
            "agent": agent,
            "kind": kind,
            "score_bp": score_bp,
            "height": height,
        }))?;
        Ok(Self {
            id,
            agent: agent.as_str().to_string(),
            kind,
            score_bp,
            height,
        })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "agent": self.agent,
            "kind": self.kind.as_str(),
            "score_bp": self.score_bp,
            "height": self.height,
            "grade": ONCHAIN_GRADE.as_str(),
        })
    }
}

/// 一个 DID 的本地 4 维信誉。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalReputation {
    pub did: String,
    pub reliability_bp: i64,
    pub quality_bp: i64,
    pub honesty_bp: i64,
    pub availability_bp: i64,
    pub events_applied: usize,
    pub last_height: u64,
}

impl LocalReputation {
    pub fn neutral(did: &Did) -> Self {
        Self {
            did: did.as_str().to_string(),
            reliability_bp: NEUTRAL_BP,
            quality_bp: NEUTRAL_BP,
            honesty_bp: NEUTRAL_BP,
            availability_bp: NEUTRAL_BP,
            events_applied: 0,
            last_height: 0,
        }
    }

    pub fn dim(&self, dim: ReputationDim) -> i64 {
        match dim {
            ReputationDim::Reliability => self.reliability_bp,
            ReputationDim::Quality => self.quality_bp,
            ReputationDim::Honesty => self.honesty_bp,
            ReputationDim::Availability => self.availability_bp,
        }
    }

    fn dim_mut(&mut self, dim: ReputationDim) -> &mut i64 {
        match dim {
            ReputationDim::Reliability => &mut self.reliability_bp,
            ReputationDim::Quality => &mut self.quality_bp,
            ReputationDim::Honesty => &mut self.honesty_bp,
            ReputationDim::Availability => &mut self.availability_bp,
        }
    }

    /// 综合分：4 维平均（整数除法）。
    pub fn overall_bp(&self) -> i64 {
        let sum = self.reliability_bp + self.quality_bp + self.honesty_bp + self.availability_bp;
        sum / 4
    }

    /// 有界更新：向目标值移动 `weight_bp` 比例，单步不超过 [`MAX_STEP_BP`]。
    fn apply(&mut self, dim: ReputationDim, score_bp: i64, weight_bp: i64) {
        let old = self.dim(dim);
        let delta = score_bp - old;
        let scaled = delta.saturating_mul(weight_bp) / 10_000;
        let step = scaled.clamp(-MAX_STEP_BP, MAX_STEP_BP);
        let next = (old + step).clamp(0, 10_000);
        *self.dim_mut(dim) = next;
    }

    pub fn to_json(&self) -> Value {
        json!({
            "did": self.did,
            "reliability_bp": self.reliability_bp,
            "quality_bp": self.quality_bp,
            "honesty_bp": self.honesty_bp,
            "availability_bp": self.availability_bp,
            "overall_bp": self.overall_bp(),
            "events_applied": self.events_applied,
            "last_height": self.last_height,
            "grade": ONCHAIN_GRADE.as_str(),
        })
    }
}

/// 信誉桥：链上事件 → 本地 4 维信誉。
#[derive(Clone, Debug, Default)]
pub struct ReputationBridge {
    agents: BTreeMap<String, LocalReputation>,
    applied: BTreeSet<String>,
    refusals: Vec<ChainRefusal>,
}

impl ReputationBridge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reputation_of(&self, who: &Did) -> LocalReputation {
        self.agents
            .get(who.as_str())
            .cloned()
            .unwrap_or_else(|| LocalReputation::neutral(who))
    }

    pub fn applied_events(&self) -> usize {
        self.applied.len()
    }

    pub fn refusals(&self) -> &[ChainRefusal] {
        &self.refusals
    }

    fn deny(&mut self, refusal: ChainRefusal) -> ChainRefusal {
        self.refusals.push(refusal.clone());
        refusal
    }

    /// 应用一条**已最终化**的链上事件。未最终化 → `Timeout`；重复 → `Conflict`。
    pub fn apply_event(
        &mut self,
        net: &Testnet,
        event: &ChainReputationEvent,
    ) -> Result<LocalReputation, ChainRefusal> {
        if event.id != self.event_id(event).unwrap_or_default() {
            let refusal = ChainRefusal::new(
                "reputation.apply_event",
                RefusalCode::Malformed,
                "事件 id 与内容不一致（疑似篡改）",
            );
            return Err(self.deny(refusal));
        }
        if !net.is_final(event.height) {
            let refusal = ChainRefusal::new(
                "reputation.apply_event",
                RefusalCode::Timeout,
                format!(
                    "事件高度 {} 尚未最终（当前 {}，最终化到 {}）：回滚不该污染信誉",
                    event.height,
                    net.height(),
                    net.finalized_height()
                ),
            );
            return Err(self.deny(refusal));
        }
        if self.applied.contains(&event.id) {
            let refusal = ChainRefusal::new(
                "reputation.apply_event",
                RefusalCode::Conflict,
                "同一事件不能重复计入（重放）",
            );
            return Err(self.deny(refusal));
        }
        let did = Did::parse(&event.agent).map_err(|_| {
            self.deny(ChainRefusal::new(
                "reputation.apply_event",
                RefusalCode::Malformed,
                "agent DID 非法",
            ))
        })?;
        let entry = self
            .agents
            .entry(did.as_str().to_string())
            .or_insert_with(|| LocalReputation::neutral(&did));
        for dim in event.kind.dimensions() {
            entry.apply(*dim, event.score_bp, event.kind.weight_bp());
        }
        entry.events_applied += 1;
        entry.last_height = entry.last_height.max(event.height);
        self.applied.insert(event.id.clone());
        Ok(entry.clone())
    }

    fn event_id(&self, event: &ChainReputationEvent) -> CoreResult<String> {
        let did = Did::parse(&event.agent)?;
        Ok(ChainReputationEvent::new(&did, event.kind, event.score_bp, event.height)?.id)
    }

    /// 执行一个链上请求（信誉操作都在本地，不走测试网交易，但拒绝要**按名字**留痕）。
    pub fn execute(&mut self, op: &str) -> Result<(), ChainRefusal> {
        let refusal = match REPUTATION_REFUSALS.iter().find(|spec| spec.op == op) {
            Some(spec) => ChainRefusal::new(op, spec.code, spec.reason),
            None => ChainRefusal::unsupported(op, &REPUTATION_REFUSALS),
        };
        Err(self.deny(refusal))
    }

    /// 只读投影。
    pub fn to_json(&self) -> Value {
        let mut rows: Vec<&LocalReputation> = self.agents.values().collect();
        rows.sort_by(|a, b| a.did.cmp(&b.did));
        json!({
            "agents": rows,
            "applied_events": self.applied.len(),
            "refusals": self.refusals.iter().map(ChainRefusal::to_json).collect::<Vec<_>>(),
            "transferable": false,
            "grade": ONCHAIN_GRADE.as_str(),
            "note": "信誉不可转让：事件只能计入事件主体自己的 4 维信誉",
        })
    }
}

/// 把本地 4 维信誉折算成「可信度金额」（只读投影用；不参与结算）。
pub fn credibility_credits(reputation: &LocalReputation) -> CoreResult<Credits> {
    Credits::new(reputation.overall_bp().clamp(0, 10_000))
}

/// 信誉相关的拒绝码说明：不可转让用 [`RefusalCode::PolicyDenied`]，未知操作用 `Unsupported`。
pub fn refusal_code_for(op: &str) -> RefusalCode {
    REPUTATION_REFUSALS
        .iter()
        .find(|spec| spec.op == op)
        .map(|spec| spec.code)
        .unwrap_or(RefusalCode::Unsupported)
}

/// 信誉桥接不产生测试网交易（纯本地映射），但保留 `ChainTx` 形态以便审计对账。
pub fn event_tx(height: u64, kind: ReputationEventKind, who: &Did) -> CoreResult<ChainTx> {
    ChainTx::new(
        crate::testnet::ChainId::EthLocal,
        "reputation.apply_event",
        who,
        height,
        json!({ "kind": kind.as_str() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testnet::{ChainId, Testnet};
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn net(finality: u64, height: u64) -> Testnet {
        let mut net = Testnet::new(ChainId::EthLocal, finality);
        net.mine_to(height);
        net
    }

    #[test]
    fn events_move_the_matching_dimensions_toward_the_score() {
        let who = did(1);
        let net = net(1, 10);
        let mut bridge = ReputationBridge::new();
        // 初始 5000；反馈 9000、权重 6000 → 位移 (9000-5000)×6000/10000 = 2400 → 但单步上限 2000。
        let event =
            ChainReputationEvent::new(&who, ReputationEventKind::Feedback, 9_000, 2).unwrap();
        let after = bridge.apply_event(&net, &event).unwrap();
        assert_eq!(after.quality_bp, 7_000); // 5000 + 2000（被 MAX_STEP_BP 截断）
        assert_eq!(after.reliability_bp, NEUTRAL_BP, "反馈不动可靠性");
        assert_eq!(after.events_applied, 1);
        assert_eq!(bridge.applied_events(), 1);
        // 结算最终化：可靠性 + 可用性各走 4000 权重的一半左右。
        let settle =
            ChainReputationEvent::new(&who, ReputationEventKind::SettlementFinal, 8_000, 3).unwrap();
        let after2 = bridge.apply_event(&net, &settle).unwrap();
        // (8000-5000)×4000/10000 = 1200
        assert_eq!(after2.reliability_bp, 6_200);
        assert_eq!(after2.availability_bp, 6_200);
        assert_eq!(after2.quality_bp, 7_000);
    }

    #[test]
    fn non_final_events_are_refused() {
        let who = did(2);
        let net = net(5, 3); // 最终化到 0
        let mut bridge = ReputationBridge::new();
        let event =
            ChainReputationEvent::new(&who, ReputationEventKind::Feedback, 9_000, 2).unwrap();
        let err = bridge.apply_event(&net, &event).unwrap_err();
        assert_eq!(err.code, RefusalCode::Timeout);
        assert_eq!(bridge.applied_events(), 0);
        assert_eq!(bridge.reputation_of(&who).quality_bp, NEUTRAL_BP);
        assert_eq!(bridge.refusals().len(), 1);
    }

    #[test]
    fn replayed_and_tampered_events_are_refused() {
        let who = did(3);
        let net = net(1, 10);
        let mut bridge = ReputationBridge::new();
        let event =
            ChainReputationEvent::new(&who, ReputationEventKind::Validation, 7_000, 2).unwrap();
        bridge.apply_event(&net, &event).unwrap();
        // 重放。
        assert_eq!(
            bridge.apply_event(&net, &event).unwrap_err().code,
            RefusalCode::Conflict
        );
        // 篡改（改分数但不改 id）。
        let mut tampered = event.clone();
        tampered.score_bp = 1;
        assert_eq!(
            bridge.apply_event(&net, &tampered).unwrap_err().code,
            RefusalCode::Malformed
        );
        // 越界分数在构造时就被拒。
        assert!(ChainReputationEvent::new(&who, ReputationEventKind::Feedback, 10_001, 2).is_err());
        assert!(ChainReputationEvent::new(&who, ReputationEventKind::Feedback, -1, 2).is_err());
    }

    #[test]
    fn repeated_events_converge_monotonically_without_overshoot() {
        let who = did(4);
        let net = net(1, 100);
        let mut bridge = ReputationBridge::new();
        let mut last = NEUTRAL_BP;
        for i in 0..30u64 {
            let event =
                ChainReputationEvent::new(&who, ReputationEventKind::Feedback, 10_000, 2 + i)
                    .unwrap();
            let after = bridge.apply_event(&net, &event).unwrap();
            assert!(
                after.quality_bp >= last,
                "第 {i} 步信誉下降：{last} → {}",
                after.quality_bp
            );
            assert!(after.quality_bp <= 10_000, "超过上限：{}", after.quality_bp);
            last = after.quality_bp;
        }
        assert!(
            last >= 9_999,
            "重复满分事件应收敛到 1bp 以内（整数除法不会正好落在 10000），实际 {last}"
        );
        // 反向也收敛且不过冲（整数除法同样收敛到 1bp 以内）。
        for i in 0..30u64 {
            let event =
                ChainReputationEvent::new(&who, ReputationEventKind::Feedback, 0, 50 + i).unwrap();
            bridge.apply_event(&net, &event).unwrap();
        }
        let floor = bridge.reputation_of(&who).quality_bp;
        assert!((0..=1).contains(&floor), "收敛下界应接近 0，实际 {floor}");
    }

    #[test]
    fn disputes_lower_reliability_and_honesty() {
        let who = did(5);
        let net = net(1, 10);
        let mut bridge = ReputationBridge::new();
        let event =
            ChainReputationEvent::new(&who, ReputationEventKind::DisputeSlashed, 0, 2).unwrap();
        let after = bridge.apply_event(&net, &event).unwrap();
        // (0-5000)×8000/10000 = -4000 → 被 MAX_STEP_BP 截断为 -2000。
        assert_eq!(after.reliability_bp, 3_000);
        assert_eq!(after.honesty_bp, 3_000);
        assert_eq!(after.quality_bp, NEUTRAL_BP);
        assert!(after.overall_bp() < NEUTRAL_BP);
    }

    #[test]
    fn reputation_cannot_be_transferred_or_bought() {
        let mut bridge = ReputationBridge::new();
        for op in [
            "reputation.transfer",
            "reputation.buy",
            "reputation.bulk_import",
            "reputation.reset",
            "reputation.unknown",
        ] {
            let err = bridge.execute(op).unwrap_err();
            assert_eq!(err.op, op);
            let expected = refusal_code_for(op);
            assert_eq!(err.code, expected);
        }
        assert_eq!(bridge.refusals().len(), 5);
        assert_eq!(REPUTATION_REFUSALS.len(), 4);
        assert_eq!(REPUTATION_SUPPORTED.len(), 2);
    }

    #[test]
    fn the_bridge_is_deterministic_and_float_free() {
        let who = did(6);
        let net = net(1, 20);
        let mut a = ReputationBridge::new();
        let mut b = ReputationBridge::new();
        for (kind, score, height) in [
            (ReputationEventKind::SettlementFinal, 9_000i64, 2u64),
            (ReputationEventKind::Feedback, 8_000, 3),
            (ReputationEventKind::Validation, 7_000, 4),
            (ReputationEventKind::X402Settled, 6_000, 5),
        ] {
            let event = ChainReputationEvent::new(&who, kind, score, height).unwrap();
            a.apply_event(&net, &event).unwrap();
            b.apply_event(&net, &event).unwrap();
        }
        assert_eq!(a.reputation_of(&who), b.reputation_of(&who));
        assert_eq!(
            au4a_core::canonicalize(&a.to_json()).unwrap(),
            au4a_core::canonicalize(&b.to_json()).unwrap()
        );
        assert_eq!(a.to_json()["transferable"], json!(false));
        assert_eq!(a.to_json()["grade"], json!("cpu-proto"));
        assert_eq!(credibility_credits(&a.reputation_of(&who)).unwrap(), Credits(a.reputation_of(&who).overall_bp()));
    }

    #[test]
    fn dimensions_are_complete_and_named() {
        let names: Vec<&str> = ReputationDim::ALL.iter().map(|d| d.as_str()).collect();
        assert_eq!(
            names,
            vec!["reliability", "quality", "honesty", "availability"]
        );
        // 每种事件都至少影响一个维度，且总分在 0..=10000 内。
        for kind in ReputationEventKind::ALL {
            assert!(!kind.dimensions().is_empty(), "{}", kind.as_str());
            assert!(kind.weight_bp() > 0 && kind.weight_bp() <= 10_000);
        }
        let neutral = LocalReputation::neutral(&did(7));
        assert_eq!(neutral.overall_bp(), NEUTRAL_BP);
        assert_eq!(neutral.events_applied, 0);
    }
}
