//! v1.1.6 —— 路径规划（图搜索，不是逐步贪心）。
//!
//! 需求是「先英译中、再情感分析」，而能力分散在不同 Agent 上，格式还各不相同。
//! 这一层要回答的是：**存不存在一条能跑通的流水线？如果存在，哪一条代价最小？**
//!
//! 为什么必须是搜索而不是每步取最优（贪心）：
//! 贪心分不清「本步最便宜」和「整条链能跑通」。演示数据集里 `dave` 的翻译最便宜
//! （1 微积分）但产出 `text/html`，而所有情感分析能力都只接受 `application/json`
//! 或 `text/plain` —— 贪心选 `dave` 直接走进死路，正确答案是 `bob → carol`（3 + 2 = 5）。
//! `tests/planner.rs` 对这个反例有显式断言（断言第一步**没有**选最便宜的那个）。
//!
//! 算法：**多准则标签设定搜索**（label-setting shortest path）。
//!
//! * 节点 = `(第 i 步, 某个候选能力)`；边 = 「上一步产出 ∩ 本步接受 ≠ ∅」。
//! * 标签 = `(代价, 价格, 延迟)`；代价是整数：价格 + 延迟折算 + 换手成本。
//! * 用 `BinaryHeap` 按代价最优优先展开；每个节点保留**互不支配**的标签
//!   （A 支配 B ⟺ A 三个分量都不大于 B），这样「最便宜」与「够得着预算/期限」
//!   这两个目标不会被单一标量合并掉——那正是贪心会犯的错。
//! * 预算/期限在展开时剪枝，因此「无路径」是**搜出来的结论**，并且会给出具体到某一步的原因。
//!
//! 找不到路径不是错误，而是一种有类型的结局：[`PlanOutcome::NoPath`] + [`NoPathReason`]，
//! 它会映射到 `au4a-core` 的拒绝码，从而可以进内核的拒绝记录。

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use au4a_core::{Credits, Did, RefusalCode};
use serde_json::{json, Value};

use crate::capability::{Capability, FormatId, SkillId};
use crate::graph::AgentCapabilityGraph;
use crate::index::{CapabilityMatch, CapabilityQuery};

/// 每个节点最多保留多少个互不支配的标签。真实图里远达不到；
/// 达到上限时**丢弃新标签**并计数（而不是截断已有标签，那会让堆里的下标失效）。
const MAX_LABELS_PER_NODE: usize = 64;

/// 流水线的一步：需要哪个技能，以及这一步的额外要求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineStep {
    pub skill: SkillId,
    /// 这一步期望的产出格式（可选）。给定时只有产出该格式的能力入选。
    pub output_format: Option<FormatId>,
    /// 这一步的单价上限（可选）。
    pub max_price_per_unit: Option<Credits>,
}

impl PipelineStep {
    pub fn new(skill: SkillId) -> Self {
        Self {
            skill,
            output_format: None,
            max_price_per_unit: None,
        }
    }

    pub fn expecting(mut self, format: FormatId) -> Self {
        self.output_format = Some(format);
        self
    }

    pub fn with_max_price(mut self, price: Credits) -> Self {
        self.max_price_per_unit = Some(price);
        self
    }

    pub fn to_value(&self) -> Value {
        json!({
            "skill": self.skill.as_str(),
            "output_format": self.output_format.as_ref().map(FormatId::as_str),
            "max_price_per_unit": self.max_price_per_unit.map(|c| c.get()),
        })
    }
}

/// 一次规划请求：从什么格式进、要哪些技能、到什么时候为止、最多花多少。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineRequest {
    pub input_format: FormatId,
    pub steps: Vec<PipelineStep>,
    /// 最终产出格式（可选）。
    pub final_output_format: Option<FormatId>,
    /// 整条流水线的价格上限（微积分）。
    pub max_total_price: Option<Credits>,
    /// 整条流水线的期限（毫秒，按加权延迟求和）。
    pub deadline_ms: Option<u32>,
    /// 每一步的可靠度下限（万分比）。
    pub min_reliability_bp: Option<u16>,
    /// 载荷大小（字节）：超过某能力 `max_input_bytes` 的候选直接剔除。
    pub payload_bytes: Option<u64>,
    /// 区域要求。
    pub region: Option<String>,
}

impl PipelineRequest {
    pub fn new(input_format: FormatId, steps: Vec<PipelineStep>) -> Self {
        Self {
            input_format,
            steps,
            final_output_format: None,
            max_total_price: None,
            deadline_ms: None,
            min_reliability_bp: None,
            payload_bytes: None,
            region: None,
        }
    }

    pub fn with_final_output(mut self, format: FormatId) -> Self {
        self.final_output_format = Some(format);
        self
    }

    pub fn with_budget(mut self, price: Credits) -> Self {
        self.max_total_price = Some(price);
        self
    }

    pub fn with_deadline_ms(mut self, ms: u32) -> Self {
        self.deadline_ms = Some(ms);
        self
    }

    pub fn with_min_reliability_bp(mut self, bp: u16) -> Self {
        self.min_reliability_bp = Some(bp);
        self
    }

    pub fn with_payload_bytes(mut self, bytes: u64) -> Self {
        self.payload_bytes = Some(bytes);
        self
    }

    pub fn with_region(mut self, region: &str) -> Self {
        self.region = Some(region.to_string());
        self
    }

    pub fn to_value(&self) -> Value {
        json!({
            "input_format": self.input_format.as_str(),
            "steps": self.steps.iter().map(PipelineStep::to_value).collect::<Vec<_>>(),
            "final_output_format": self.final_output_format.as_ref().map(FormatId::as_str),
            "max_total_price": self.max_total_price.map(|c| c.get()),
            "deadline_ms": self.deadline_ms,
            "min_reliability_bp": self.min_reliability_bp,
            "payload_bytes": self.payload_bytes,
            "region": self.region,
        })
    }
}

/// 代价口径。价格是微积分，延迟按 `latency_price_per_ms` 折算，换手有固定成本。
///
/// 这三个数**不是**市场参数，而是调用方自己的偏好（例如「延迟敏感」就把
/// `latency_price_per_ms` 调大）。它们必须是整数，否则规范 JSON 会拒签。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanCost {
    pub latency_price_per_ms: i64,
    pub handoff_price: i64,
}

impl Default for PlanCost {
    fn default() -> Self {
        Self {
            latency_price_per_ms: 1,
            handoff_price: 1,
        }
    }
}

impl PlanCost {
    pub fn new(latency_price_per_ms: i64, handoff_price: i64) -> Self {
        Self {
            latency_price_per_ms,
            handoff_price,
        }
    }

    fn latency_price(&self, latency_ms: u64) -> i64 {
        let capped = latency_ms.min(i64::MAX as u64) as i64;
        capped.saturating_mul(self.latency_price_per_ms)
    }

    pub fn to_value(self) -> Value {
        json!({
            "latency_price_per_ms": self.latency_price_per_ms,
            "handoff_price": self.handoff_price,
        })
    }
}

/// 搜索过程的操作计数。**这是「真的搜了」的证据**（本 crate 禁止读墙钟）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchStats {
    /// 被展开的标签数。
    pub settled: usize,
    /// 因被支配而丢弃的标签数。
    pub dominance_pruned: usize,
    /// 因预算/期限而丢弃的扩展数。
    pub constraint_pruned: usize,
    /// 因节点标签数达到上限而丢弃的标签数（正常图里应为 0）。
    pub labels_capped: usize,
    /// 参与搜索的候选总数。
    pub candidates: usize,
}

impl SearchStats {
    pub fn to_value(self) -> Value {
        json!({
            "settled": self.settled,
            "dominance_pruned": self.dominance_pruned,
            "constraint_pruned": self.constraint_pruned,
            "labels_capped": self.labels_capped,
            "candidates": self.candidates,
        })
    }
}

/// 流水线里的一跳。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineNode {
    pub step: usize,
    pub did: Did,
    pub slot: usize,
    pub skill: SkillId,
    /// 这一步实际消费的格式（由兼容性决定，不是猜的）。
    pub input_format: FormatId,
    /// 这一步实际产出的格式。
    pub output_format: FormatId,
    pub price_per_unit: Credits,
    pub latency_ms: u64,
    /// 这一跳的代价（价格 + 延迟折算 + 换手）。
    pub cost: i64,
}

impl PipelineNode {
    pub fn to_value(&self) -> Value {
        json!({
            "step": self.step,
            "agent": self.did.as_str(),
            "slot": self.slot,
            "skill": self.skill.as_str(),
            "input_format": self.input_format.as_str(),
            "output_format": self.output_format.as_str(),
            "price_per_unit": self.price_per_unit.get(),
            "latency_ms": self.latency_ms,
            "cost": self.cost,
        })
    }
}

/// 一条流水线：谁做第几步、格式怎么接、总共多少钱多少延迟。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipeline {
    pub nodes: Vec<PipelineNode>,
    pub total_price: Credits,
    pub total_latency_ms: u64,
    pub total_cost: i64,
    pub search: SearchStats,
}

impl Pipeline {
    pub fn agents(&self) -> Vec<&Did> {
        self.nodes.iter().map(|n| &n.did).collect()
    }

    pub fn skills(&self) -> Vec<&SkillId> {
        self.nodes.iter().map(|n| &n.skill).collect()
    }

    /// 「换手次数」：相邻两步由不同 Agent 承担的次数。
    pub fn handoffs(&self) -> usize {
        self.nodes
            .windows(2)
            .filter(|pair| pair[0].did != pair[1].did)
            .count()
    }

    pub fn to_value(&self) -> Value {
        json!({
            "nodes": self.nodes.iter().map(PipelineNode::to_value).collect::<Vec<_>>(),
            "hops": self.nodes.len(),
            "handoffs": self.handoffs(),
            "total_price": self.total_price.get(),
            "total_latency_ms": self.total_latency_ms,
            "total_cost": self.total_cost,
            "search": self.search.to_value(),
        })
    }
}

/// 找不到路径的**具体**原因。含糊的失败没有价值：每一条都能指到某一步。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoPathReason {
    /// 请求里没有任何步骤。
    EmptyRequest,
    /// 该技能在视图里没有任何提供者。
    NoProvider { step: usize, skill: String },
    /// 有提供者，但格式接不上（上一步产出与这一步接受没有交集）。
    NoCompatibleFormat {
        step: usize,
        required: Vec<String>,
        produced: Vec<String>,
    },
    /// 有提供者，但被硬约束剔除（可靠度、区域、载荷、单步价格）。
    ConstraintRejected { step: usize, skill: String },
    /// 任何可行链的总价都超过预算。
    BudgetExceeded { allowed: i64 },
    /// 任何可行链的总延迟都超过期限。
    DeadlineExceeded { allowed_ms: u64 },
}

impl NoPathReason {
    pub fn refusal_code(&self) -> RefusalCode {
        match self {
            Self::EmptyRequest => RefusalCode::Malformed,
            Self::NoProvider { .. } => RefusalCode::Unsupported,
            Self::NoCompatibleFormat { .. } => RefusalCode::Unsupported,
            Self::ConstraintRejected { .. } => RefusalCode::PolicyDenied,
            Self::BudgetExceeded { .. } => RefusalCode::PolicyDenied,
            Self::DeadlineExceeded { .. } => RefusalCode::Timeout,
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::EmptyRequest => "request has no steps".to_string(),
            Self::NoProvider { step, skill } => format!("step {step}: no provider for skill {skill}"),
            Self::NoCompatibleFormat {
                step,
                required,
                produced,
            } => format!(
                "step {step}: no format handoff (required {required:?}, produced {produced:?})"
            ),
            Self::ConstraintRejected { step, skill } => {
                format!("step {step}: every provider of {skill} was rejected by constraints")
            }
            Self::BudgetExceeded { allowed } => format!("cheapest pipeline costs more than {allowed}"),
            Self::DeadlineExceeded { allowed_ms } => {
                format!("fastest pipeline is slower than {allowed_ms} ms")
            }
        }
    }
}

/// 一条「没有路径」的结论，连同搜索证据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoPath {
    pub reason: NoPathReason,
    pub search: SearchStats,
}

impl NoPath {
    pub fn refusal_code(&self) -> RefusalCode {
        self.reason.refusal_code()
    }

    pub fn to_value(&self) -> Value {
        json!({
            "reason": format!("{:?}", self.reason),
            "detail": self.reason.detail(),
            "refusal_code": self.refusal_code().as_str(),
            "search": self.search.to_value(),
        })
    }
}

/// 规划结局：要么给出最小代价流水线，要么给出**明确的**无路径理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanOutcome {
    Path(Pipeline),
    NoPath(NoPath),
}

impl PlanOutcome {
    pub fn is_path(&self) -> bool {
        matches!(self, Self::Path(_))
    }

    pub fn pipeline(&self) -> Option<&Pipeline> {
        match self {
            Self::Path(pipeline) => Some(pipeline),
            Self::NoPath(_) => None,
        }
    }

    pub fn refusal_code(&self) -> Option<RefusalCode> {
        match self {
            Self::Path(_) => None,
            Self::NoPath(no_path) => Some(no_path.refusal_code()),
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Self::Path(pipeline) => json!({"outcome": "path", "pipeline": pipeline.to_value()}),
            Self::NoPath(no_path) => json!({"outcome": "no_path", "no_path": no_path.to_value()}),
        }
    }
}

/// 每一步的候选清单（来自 v1.1.5 的索引查询）。
type StepCandidates = Vec<CapabilityMatch>;

/// 搜索标签：三个分量互不折算，避免「便宜」与「够快」被压成一个标量后丢失可行解。
///
/// 被支配的标签只打 `dead` 标记而**不删除**：这样数组下标稳定，
/// 堆里未展开的条目不至于因为重排而指到别的标签上。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Label {
    cost: i64,
    price: i64,
    latency: u64,
    prev: Option<(usize, usize, usize)>,
    dead: bool,
}

/// A 支配 B：A 在三个分量上都不差。
fn dominates(a: &Label, b: &Label) -> bool {
    a.cost <= b.cost && a.price <= b.price && a.latency <= b.latency
}

fn node_cost(capability: &Capability, cost: &PlanCost, handoff: bool) -> i64 {
    let base = capability
        .price_per_unit
        .get()
        .saturating_add(cost.latency_price(capability.effective_latency_ms()));
    if handoff {
        base.saturating_add(cost.handoff_price)
    } else {
        base
    }
}

fn first_format(set: &std::collections::BTreeSet<FormatId>) -> FormatId {
    set.iter()
        .next()
        .cloned()
        .unwrap_or_else(crate::capability::text_plain)
}

/// 规划主入口。返回 [`PlanOutcome`]，不返回 `Err`：
/// 「没有可行流水线」是业务结论，不是程序错误。
pub fn plan(
    graph: &mut AgentCapabilityGraph,
    request: &PipelineRequest,
    cost: &PlanCost,
    now: u64,
) -> PlanOutcome {
    if request.steps.is_empty() {
        return PlanOutcome::NoPath(NoPath {
            reason: NoPathReason::EmptyRequest,
            search: SearchStats::default(),
        });
    }
    // 过期邻居不参与规划。
    let _ = graph.expire_neighbors(now);

    let candidates = match collect_candidates(graph, request, now) {
        Ok(candidates) => candidates,
        Err(no_path) => return PlanOutcome::NoPath(no_path),
    };

    let (search, path) = search_labels(&candidates, request, cost);
    match path {
        Some(path) => PlanOutcome::Path(build_pipeline(&candidates, &path, request, cost, search)),
        None => {
            let reason = if request.max_total_price.is_some() || request.deadline_ms.is_some() {
                // 放宽预算与期限再搜一次：能搜到说明约束是原因，搜不到说明格式本身不通。
                let mut relaxed = request.clone();
                relaxed.max_total_price = None;
                relaxed.deadline_ms = None;
                match plan(graph, &relaxed, cost, now) {
                    PlanOutcome::Path(feasible) => {
                        match request.max_total_price {
                            Some(budget) if feasible.total_price.get() > budget.get() => {
                                NoPathReason::BudgetExceeded {
                                    allowed: budget.get(),
                                }
                            }
                            _ => NoPathReason::DeadlineExceeded {
                                allowed_ms: request.deadline_ms.map(u64::from).unwrap_or(0),
                            },
                        }
                    }
                    PlanOutcome::NoPath(inner) => inner.reason,
                }
            } else {
                classify_failure(&candidates, request)
            };
            PlanOutcome::NoPath(NoPath { reason, search })
        }
    }
}

/// 收集每一步的候选，并区分「没有提供者」「有提供者但被硬约束剔除」。
fn collect_candidates(
    graph: &mut AgentCapabilityGraph,
    request: &PipelineRequest,
    now: u64,
) -> Result<Vec<StepCandidates>, NoPath> {
    let mut candidates: Vec<StepCandidates> = Vec::with_capacity(request.steps.len());
    let last_index = request.steps.len() - 1;
    for (index, step) in request.steps.iter().enumerate() {
        let mut query = CapabilityQuery::new(step.skill.clone()).with_limit(0);
        let output = if index == last_index {
            request.final_output_format.clone().or(step.output_format.clone())
        } else {
            step.output_format.clone()
        };
        if let Some(format) = &output {
            query = query.with_output_format(format.clone());
        }
        if let Some(price) = step.max_price_per_unit {
            query = query.with_max_price(price);
        }
        if let Some(bp) = request.min_reliability_bp {
            query = query.with_min_reliability_bp(bp);
        }
        if let Some(region) = &request.region {
            query = query.with_region(region);
        }
        let mut found = graph.query(&query, now).matches;
        if let Some(bytes) = request.payload_bytes {
            found.retain(|m| m.capability.constraints.accepts_payload(bytes));
        }
        if found.is_empty() {
            let unfiltered = graph
                .query(&CapabilityQuery::new(step.skill.clone()).with_limit(0), now)
                .matches;
            let reason = if unfiltered.is_empty() {
                NoPathReason::NoProvider {
                    step: index,
                    skill: step.skill.as_str().to_string(),
                }
            } else {
                NoPathReason::ConstraintRejected {
                    step: index,
                    skill: step.skill.as_str().to_string(),
                }
            };
            return Err(NoPath {
                reason,
                search: SearchStats::default(),
            });
        }
        candidates.push(found);
    }
    Ok(candidates)
}

/// 多准则标签设定搜索。返回（统计，最优路径的 `(step, slot, label)` 链）。
fn search_labels(
    candidates: &[StepCandidates],
    request: &PipelineRequest,
    cost: &PlanCost,
) -> (SearchStats, Option<Vec<(usize, usize, usize)>>) {
    let steps = candidates.len();
    let mut stats = SearchStats {
        candidates: candidates.iter().map(Vec::len).sum(),
        ..SearchStats::default()
    };
    let mut labels: Vec<Vec<Vec<Label>>> = candidates
        .iter()
        .map(|row| vec![Vec::new(); row.len()])
        .collect();
    // 堆序：代价 → 延迟 → 价格 → 步号 → DID → 槽位 → 标签下标（全序，确定性）。
    let mut heap: BinaryHeap<Reverse<(i64, u64, i64, usize, Did, usize, usize)>> = BinaryHeap::new();

    for (slot, candidate) in candidates[0].iter().enumerate() {
        if !candidate.capability.accepts_format(&request.input_format) {
            continue;
        }
        let label = Label {
            cost: node_cost(&candidate.capability, cost, false),
            price: candidate.capability.price_per_unit.get(),
            latency: candidate.capability.effective_latency_ms(),
            prev: None,
            dead: false,
        };
        labels[0][slot].push(label);
        heap.push(Reverse((
            label.cost,
            label.latency,
            label.price,
            0,
            candidate.did.clone(),
            slot,
            0,
        )));
    }

    while let Some(Reverse((popped_cost, _, _, step, _, slot, index))) = heap.pop() {
        let Some(label) = labels[step][slot].get(index).copied() else {
            continue;
        };
        // 过期条目：下标可能对应被标记死亡或已被替换的标签。
        if label.dead || label.cost != popped_cost {
            continue;
        }
        stats.settled += 1;
        if step + 1 == steps {
            continue;
        }
        let capability_here = candidates[step][slot].capability.clone();
        let agent_here = candidates[step][slot].did.clone();
        for (next_slot, next) in candidates[step + 1].iter().enumerate() {
            if capability_here.handoff_format(&next.capability).is_none() {
                continue;
            }
            // 换手成本：相邻两步由不同 Agent 承担时加一次。
            let handoff = agent_here != next.did;
            let extra = node_cost(&next.capability, cost, handoff);
            let candidate_label = Label {
                cost: label.cost.saturating_add(extra),
                price: label
                    .price
                    .saturating_add(next.capability.price_per_unit.get()),
                latency: label
                    .latency
                    .saturating_add(next.capability.effective_latency_ms()),
                prev: Some((step, slot, index)),
                dead: false,
            };
            let over_budget = request
                .max_total_price
                .map_or(false, |budget| candidate_label.price > budget.get());
            let over_deadline = request
                .deadline_ms
                .map_or(false, |deadline| candidate_label.latency > u64::from(deadline));
            if over_budget || over_deadline {
                stats.constraint_pruned += 1;
                continue;
            }
            let bucket = &mut labels[step + 1][next_slot];
            if bucket
                .iter()
                .any(|existing| !existing.dead && dominates(existing, &candidate_label))
            {
                stats.dominance_pruned += 1;
                continue;
            }
            if bucket.len() >= MAX_LABELS_PER_NODE {
                stats.labels_capped += 1;
                continue;
            }
            for existing in bucket.iter_mut() {
                if !existing.dead && dominates(&candidate_label, existing) {
                    existing.dead = true;
                }
            }
            bucket.push(candidate_label);
            let new_index = bucket.len() - 1;
            heap.push(Reverse((
                candidate_label.cost,
                candidate_label.latency,
                candidate_label.price,
                step + 1,
                next.did.clone(),
                next_slot,
                new_index,
            )));
        }
    }

    // 终点：最后一步里 (代价, 延迟, 价格) 最小的**存活**标签；平手时取先出现的（确定性）。
    let mut best: Option<(usize, usize)> = None;
    let mut best_key: Option<(i64, u64, i64)> = None;
    for (slot, bucket) in labels[steps - 1].iter().enumerate() {
        for (index, label) in bucket.iter().enumerate() {
            if label.dead {
                continue;
            }
            let key = (label.cost, label.latency, label.price);
            if best_key.map_or(true, |known| key < known) {
                best_key = Some(key);
                best = Some((slot, index));
            }
        }
    }

    let Some((goal_slot, goal_index)) = best else {
        return (stats, None);
    };
    let mut chain: Vec<(usize, usize, usize)> = Vec::new();
    let mut cursor = Some((steps - 1, goal_slot, goal_index));
    while let Some((step, slot, index)) = cursor {
        chain.push((step, slot, index));
        cursor = labels[step][slot][index].prev;
    }
    chain.reverse();
    (stats, Some(chain))
}

/// 把标签链翻译成流水线（格式在相邻两跳之间由交集确定）。
fn build_pipeline(
    candidates: &[StepCandidates],
    chain: &[(usize, usize, usize)],
    request: &PipelineRequest,
    cost: &PlanCost,
    search: SearchStats,
) -> Pipeline {
    let mut nodes: Vec<PipelineNode> = Vec::with_capacity(chain.len());
    for (position, (step, slot, _)) in chain.iter().enumerate() {
        let candidate = &candidates[*step][*slot];
        let capability = &candidate.capability;
        let input_format = if position == 0 {
            request.input_format.clone()
        } else {
            let previous = &candidates[chain[position - 1].0][chain[position - 1].1].capability;
            previous
                .handoff_format(capability)
                .unwrap_or_else(|| first_format(&capability.supported_formats))
        };
        let output_format = if position + 1 < chain.len() {
            let next = &candidates[chain[position + 1].0][chain[position + 1].1].capability;
            capability
                .handoff_format(next)
                .unwrap_or_else(|| first_format(&capability.produced_formats))
        } else {
            request
                .final_output_format
                .clone()
                .unwrap_or_else(|| first_format(&capability.produced_formats))
        };
        let handoff = position > 0
            && candidates[chain[position - 1].0][chain[position - 1].1].did != candidate.did;
        nodes.push(PipelineNode {
            step: *step,
            did: candidate.did.clone(),
            slot: candidate.slot,
            skill: capability.skill.clone(),
            input_format,
            output_format,
            price_per_unit: capability.price_per_unit,
            latency_ms: capability.effective_latency_ms(),
            cost: node_cost(capability, cost, handoff),
        });
    }
    let total_price = nodes
        .iter()
        .fold(0i64, |acc, node| acc.saturating_add(node.price_per_unit.get()));
    let total_latency_ms = nodes
        .iter()
        .fold(0u64, |acc, node| acc.saturating_add(node.latency_ms));
    let total_cost = nodes.iter().fold(0i64, |acc, node| acc.saturating_add(node.cost));
    Pipeline {
        nodes,
        total_price: Credits(total_price),
        total_latency_ms,
        total_cost,
        search,
    }
}

/// 搜索失败时，用候选分布把原因分类到**具体某一步**。
fn classify_failure(candidates: &[StepCandidates], request: &PipelineRequest) -> NoPathReason {
    let start_ok = candidates[0]
        .iter()
        .any(|m| m.capability.accepts_format(&request.input_format));
    if !start_ok {
        return NoPathReason::NoCompatibleFormat {
            step: 0,
            required: vec![request.input_format.as_str().to_string()],
            produced: candidates[0]
                .iter()
                .flat_map(|m| m.capability.produced_formats.iter())
                .map(|f| f.as_str().to_string())
                .collect(),
        };
    }
    for step in 0..candidates.len().saturating_sub(1) {
        let reachable = candidates[step].iter().any(|previous| {
            candidates[step + 1]
                .iter()
                .any(|next| previous.capability.handoff_format(&next.capability).is_some())
        });
        if !reachable {
            return NoPathReason::NoCompatibleFormat {
                step: step + 1,
                required: candidates[step + 1]
                    .iter()
                    .flat_map(|m| m.capability.supported_formats.iter())
                    .map(|f| f.as_str().to_string())
                    .collect(),
                produced: candidates[step]
                    .iter()
                    .flat_map(|m| m.capability.produced_formats.iter())
                    .map(|f| f.as_str().to_string())
                    .collect(),
            };
        }
    }
    // 理论不可达：所有边都存在、且没有预算/期限约束时，搜索一定能找到路径。
    NoPathReason::ConstraintRejected {
        step: candidates.len() - 1,
        skill: request
            .steps
            .last()
            .map(|s| s.skill.as_str().to_string())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::Declaration;
    use crate::graph::CapGraphConfig;
    use au4a_core::AgentKeys;

    fn skill(name: &str) -> SkillId {
        SkillId::new(name).expect("valid skill")
    }

    fn format(name: &str) -> FormatId {
        FormatId::new(name).expect("valid format")
    }

    /// 与 `demo_agents()` 同构的图，但由测试自行构造，避免依赖 lib.rs 的演示数据。
    fn demo_graph() -> (AgentCapabilityGraph, Vec<Did>) {
        let alice = AgentKeys::from_seed(&[11; 32]);
        let bob = AgentKeys::from_seed(&[12; 32]);
        let carol = AgentKeys::from_seed(&[13; 32]);
        let dave = AgentKeys::from_seed(&[14; 32]);
        let erin = AgentKeys::from_seed(&[15; 32]);
        let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());

        let declarations = [
            (
                &bob,
                Capability::new(skill("translate.en-zh"), Credits(3))
                    .with_formats(&["text/plain"], &["application/json"])
                    .expect("formats")
                    .with_latency(90, 240),
            ),
            (
                &carol,
                Capability::new(skill("sentiment.analyze"), Credits(2))
                    .with_formats(&["application/json"], &["application/json"])
                    .expect("formats")
                    .with_latency(110, 300),
            ),
            (
                &dave,
                Capability::new(skill("translate.en-zh"), Credits(1))
                    .with_formats(&["text/plain"], &["text/html"])
                    .expect("formats")
                    .with_latency(60, 150),
            ),
            (
                &erin,
                Capability::new(skill("sentiment.analyze"), Credits(1))
                    .with_formats(&["text/plain"], &["application/json"])
                    .expect("formats")
                    .with_latency(50, 120),
            ),
        ];
        for (keys, capability) in declarations {
            let signed = Declaration::new(keys.did(), 1, 0, vec![capability])
                .expect("coherent")
                .sign(keys)
                .expect("signed");
            assert!(graph.apply(&signed, 0).is_applied());
        }
        (graph, vec![bob.did(), carol.did(), dave.did(), erin.did()])
    }

    fn two_step_request() -> PipelineRequest {
        PipelineRequest::new(
            format("text/plain"),
            vec![
                PipelineStep::new(skill("translate.en-zh")),
                PipelineStep::new(skill("sentiment.analyze")),
            ],
        )
    }

    #[test]
    fn the_format_compatible_pipeline_is_found() {
        let (mut graph, dids) = demo_graph();
        let outcome = plan(&mut graph, &two_step_request(), &PlanCost::default(), 0);
        let pipeline = outcome.pipeline().expect("a path exists");
        assert_eq!(pipeline.skills().len(), 2);
        assert_eq!(pipeline.nodes[0].did, dids[0], "第一步必须是 bob（唯一产出 json 的翻译）");
        assert_eq!(pipeline.nodes[1].did, dids[1], "第二步必须是 carol（唯一接受 json 的情感分析）");
        assert_eq!(pipeline.nodes[0].output_format.as_str(), "application/json");
        assert_eq!(pipeline.nodes[1].input_format.as_str(), "application/json");
        assert_eq!(pipeline.total_price, Credits(5));
        assert!(pipeline.search.settled > 0, "搜索确实展开过标签");
    }

    #[test]
    fn greedy_would_pick_the_cheaper_incompatible_provider_and_fail() {
        let (mut graph, dids) = demo_graph();
        // 贪心的第一选择：该技能下最便宜的能力（dave，1 微积分，产出 text/html）。
        let cheapest = graph
            .query(&CapabilityQuery::new(skill("translate.en-zh")).with_limit(0), 0)
            .matches
            .into_iter()
            .min_by_key(|m| m.capability.price_per_unit.get())
            .expect("some translate provider");
        assert_eq!(cheapest.did, dids[2], "最便宜的是 dave");
        let dave_capability = cheapest.capability.clone();

        // 从 dave 出发没有任何可接续的情感分析能力 → 贪心必然失败。
        let continues = graph
            .query(&CapabilityQuery::new(skill("sentiment.analyze")).with_limit(0), 0)
            .matches
            .iter()
            .filter(|m| dave_capability.handoff_format(&m.capability).is_some())
            .count();
        assert_eq!(continues, 0, "dave 的 text/html 接不上任何情感分析");

        // 但规划必须给出可行解，且第一步不是最便宜的那个。
        let pipeline = plan(&mut graph, &two_step_request(), &PlanCost::default(), 0)
            .pipeline()
            .expect("search finds a path greedy cannot")
            .clone();
        assert_ne!(pipeline.nodes[0].did, dids[2], "真搜索不能选贪心的死路");
        assert_eq!(pipeline.total_price, Credits(5), "3 + 2，而不是 1 + ?");
    }

    #[test]
    fn an_impossible_chain_returns_a_typed_no_path() {
        let (mut graph, _) = demo_graph();
        // 先情感分析再翻译：情感分析接受 json 或 text/plain，产出 json；
        // 而两个翻译都只接受 text/plain → 接不上。
        let request = PipelineRequest::new(
            format("text/plain"),
            vec![
                PipelineStep::new(skill("sentiment.analyze")),
                PipelineStep::new(skill("translate.en-zh")),
            ],
        );
        let outcome = plan(&mut graph, &request, &PlanCost::default(), 0);
        assert!(!outcome.is_path());
        match outcome {
            PlanOutcome::NoPath(no_path) => {
                assert!(matches!(
                    no_path.reason,
                    NoPathReason::NoCompatibleFormat { step: 1, .. }
                ));
                assert_eq!(no_path.refusal_code(), RefusalCode::Unsupported);
                assert!(no_path.reason.detail().contains("step 1"));
            }
            PlanOutcome::Path(pipeline) => panic!("不该有路径：{:?}", pipeline.to_value()),
        }
    }

    #[test]
    fn an_unknown_skill_is_named_as_the_missing_provider() {
        let (mut graph, _) = demo_graph();
        let request = PipelineRequest::new(
            format("text/plain"),
            vec![PipelineStep::new(skill("speech.transcribe"))],
        );
        match plan(&mut graph, &request, &PlanCost::default(), 0) {
            PlanOutcome::NoPath(no_path) => {
                assert!(matches!(no_path.reason, NoPathReason::NoProvider { step: 0, .. }));
                assert_eq!(no_path.refusal_code(), RefusalCode::Unsupported);
            }
            PlanOutcome::Path(pipeline) => panic!("不该有路径：{:?}", pipeline.to_value()),
        }
    }

    #[test]
    fn budget_and_deadline_prune_with_distinguishable_reasons() {
        let (mut graph, _) = demo_graph();
        // bob(3) + carol(2) = 5；预算 4 剪掉。
        let too_poor = two_step_request().with_budget(Credits(4));
        match plan(&mut graph, &too_poor, &PlanCost::default(), 0) {
            PlanOutcome::NoPath(no_path) => {
                assert_eq!(no_path.reason, NoPathReason::BudgetExceeded { allowed: 4 });
                assert_eq!(no_path.refusal_code(), RefusalCode::PolicyDenied);
            }
            PlanOutcome::Path(p) => panic!("预算 4 不该可行：{:?}", p.to_value()),
        }
        // 延迟 90 + 110 = 200ms；期限 150ms 剪掉。
        let too_slow = two_step_request().with_deadline_ms(150);
        match plan(&mut graph, &too_slow, &PlanCost::default(), 0) {
            PlanOutcome::NoPath(no_path) => {
                assert_eq!(no_path.reason, NoPathReason::DeadlineExceeded { allowed_ms: 150 });
                assert_eq!(no_path.refusal_code(), RefusalCode::Timeout);
            }
            PlanOutcome::Path(p) => panic!("150ms 不该可行：{:?}", p.to_value()),
        }
        // 预算 5、期限 200ms 恰好可行。
        let exact = two_step_request().with_budget(Credits(5)).with_deadline_ms(200);
        let pipeline = plan(&mut graph, &exact, &PlanCost::default(), 0)
            .pipeline()
            .expect("恰好可行")
            .clone();
        assert_eq!(pipeline.total_price, Credits(5));
        assert_eq!(pipeline.total_latency_ms, 200);
        assert_eq!(pipeline.handoffs(), 1, "两个不同 Agent → 一次换手");
    }

    #[test]
    fn constraints_on_payload_and_reliability_filter_candidates() {
        let (mut graph, _) = demo_graph();
        let too_big = two_step_request().with_payload_bytes(u64::MAX);
        match plan(&mut graph, &too_big, &PlanCost::default(), 0) {
            PlanOutcome::NoPath(no_path) => {
                assert!(matches!(
                    no_path.reason,
                    NoPathReason::ConstraintRejected { step: 0, .. }
                ));
                assert_eq!(no_path.refusal_code(), RefusalCode::PolicyDenied);
            }
            PlanOutcome::Path(p) => panic!("1MiB 上限下 u64::MAX 载荷不该可行：{:?}", p.to_value()),
        }
        let picky = two_step_request().with_min_reliability_bp(9_999);
        assert!(!plan(&mut graph, &picky, &PlanCost::default(), 0).is_path());
        let ok = two_step_request().with_min_reliability_bp(9_000);
        assert!(plan(&mut graph, &ok, &PlanCost::default(), 0).is_path());
    }

    #[test]
    fn planning_is_deterministic_and_reports_its_search_size() {
        let (mut first_graph, _) = demo_graph();
        let (mut second_graph, _) = demo_graph();
        let first = plan(&mut first_graph, &two_step_request(), &PlanCost::default(), 0);
        let second = plan(&mut second_graph, &two_step_request(), &PlanCost::default(), 0);
        assert_eq!(first.to_value(), second.to_value(), "同样输入 → 同样流水线");
        let pipeline = first.pipeline().expect("path");
        assert_eq!(pipeline.search.candidates, 4, "两个技能各 2 个候选");
        assert_eq!(pipeline.search.labels_capped, 0);
    }

    #[test]
    fn cost_weights_change_the_reported_cost_but_not_the_price() {
        let (mut graph, _) = demo_graph();
        let cheap_latency = plan(&mut graph, &two_step_request(), &PlanCost::new(1, 1), 0)
            .pipeline()
            .expect("path")
            .clone();
        let dear_latency = plan(&mut graph, &two_step_request(), &PlanCost::new(10, 1), 0)
            .pipeline()
            .expect("path")
            .clone();
        assert!(dear_latency.total_cost > cheap_latency.total_cost);
        assert_eq!(
            dear_latency.total_price, cheap_latency.total_price,
            "价格不随延迟权重变化"
        );
    }

    #[test]
    fn an_empty_request_is_malformed_not_unsupported() {
        let (mut graph, _) = demo_graph();
        let request = PipelineRequest::new(format("text/plain"), Vec::new());
        match plan(&mut graph, &request, &PlanCost::default(), 0) {
            PlanOutcome::NoPath(no_path) => {
                assert_eq!(no_path.reason, NoPathReason::EmptyRequest);
                assert_eq!(no_path.refusal_code(), RefusalCode::Malformed);
            }
            PlanOutcome::Path(p) => panic!("空请求不该有路径：{:?}", p.to_value()),
        }
    }

    #[test]
    fn a_single_step_pipeline_honours_the_final_output_format() {
        let (mut graph, _) = demo_graph();
        let wants_json = PipelineRequest::new(
            format("text/plain"),
            vec![PipelineStep::new(skill("translate.en-zh"))],
        )
        .with_final_output(format("application/json"));
        let pipeline = plan(&mut graph, &wants_json, &PlanCost::default(), 0)
            .pipeline()
            .expect("bob produces json")
            .clone();
        assert_eq!(pipeline.nodes.len(), 1);
        assert_eq!(pipeline.nodes[0].output_format.as_str(), "application/json");

        let wants_html = PipelineRequest::new(
            format("text/plain"),
            vec![PipelineStep::new(skill("translate.en-zh"))],
        )
        .with_final_output(format("text/html"));
        let pipeline = plan(&mut graph, &wants_html, &PlanCost::default(), 0)
            .pipeline()
            .expect("dave produces html")
            .clone();
        assert_eq!(pipeline.nodes[0].price_per_unit, Credits(1), "此时最便宜的就是对的");
    }
}
