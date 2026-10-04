//! v1.6.3 行为调整（Policy Adjustment）。
//!
//! 学习必须**真的改变行为**，所以本模块只做一件事：把反馈统计与学习信号变成
//! 三个策略参数的下一个取值，并把「为什么动」写进 `reasons`。
//!
//! | 策略 | 参数 | 更新规则（全整数、全基点） |
//! |---|---|---|
//! | 定价策略 | `price_bp` | 带惯性的收益爬坡：上一轮平均收益变好 → 沿原方向继续；变差 → 反向；冷启动方向由接受率与目标的偏差给出；有界步长 |
//! | 任务选择策略 | `task_bias_bp[task_type]` | 类型的综合评分（完成质量 2/3 + 收益 1/3）与整体评分之差，除以 20 后按步长封顶 |
//! | 协作对象选择策略 | `peer_bias_bp[did]` | 协作者质量与整体质量之差（同样封顶），并叠加违规惩罚 |
//!
//! 三条不变式：
//!
//! 1. **没有证据就不动**：`confidence_bp < min_confidence_bp` 时返回未改动的参数，
//!    理由写明 `evidence-below-threshold`。「没数据也学一点」是把噪声当信号。
//! 2. **每次只动一步**：单轮任一参数的移动量不超过 `*_step_bp`，且始终落在 `PolicyBounds` 内。
//! 3. **可解释**：`PolicyAdjustment::reasons` 里的每一条都由输入数据决定，没有「因为学习过了」。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, CoreError, CoreResult, Credits, Did, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::feedback::FeedbackReport;
use crate::violation::ViolationLog;

/// 策略参数：Agent 自己的行为参数，全部是整数基点（10000 = 1.0 倍 / 100%）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyParams {
    /// 报价倍率（基点），10000 = 参考价。**定价策略**。
    pub price_bp: i64,
    /// 任务类型偏好增量（基点）。**任务选择策略**；缺省 0 表示不偏爱。
    pub task_bias_bp: BTreeMap<String, i64>,
    /// 协作者偏好增量（基点）。**协作对象选择策略**；缺省 0 表示不偏爱。
    pub peer_bias_bp: BTreeMap<Did, i64>,
}

impl PolicyParams {
    /// 未学习的初始策略：参考价上浮 20%（`price_bp = 12000`）、对所有任务类型与协作者一视同仁。
    ///
    /// 这个取值就是对照组的定义：一个**没有从经验里学过任何东西**的保守默认策略。
    pub fn baseline() -> Self {
        Self {
            price_bp: 12_000,
            task_bias_bp: BTreeMap::new(),
            peer_bias_bp: BTreeMap::new(),
        }
    }

    pub fn bias_of_task(&self, task_type: &str) -> i64 {
        self.task_bias_bp.get(task_type).copied().unwrap_or(0)
    }

    pub fn bias_of_peer(&self, peer: &Did) -> i64 {
        self.peer_bias_bp.get(peer).copied().unwrap_or(0)
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 公开投影：任务类型偏好可以发布（技能名不是隐私），协作者只发布聚合量。
    pub fn public_json(&self) -> CoreResult<Value> {
        let biases: Vec<i64> = self.peer_bias_bp.values().copied().collect();
        let count = biases.len() as i64;
        let sum: i64 = biases.iter().copied().sum();
        let mean = if count > 0 { sum / count } else { 0 };
        Ok(serde_json::json!({
            "price_bp": self.price_bp,
            "task_bias_bp": &self.task_bias_bp,
            "peers_tracked": count,
            "peer_bias_min_bp": biases.iter().copied().min(),
            "peer_bias_max_bp": biases.iter().copied().max(),
            "peer_bias_mean_bp": mean,
        }))
    }
}

/// 参数边界：Agent 自己设定的安全护栏，防止一轮反馈把策略推到荒谬位置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyBounds {
    pub price_min_bp: i64,
    pub price_max_bp: i64,
    pub bias_min_bp: i64,
    pub bias_max_bp: i64,
    /// 单轮定价调整步长。
    pub price_step_bp: i64,
    /// 单轮偏好调整步长。
    pub bias_step_bp: i64,
}

impl Default for PolicyBounds {
    fn default() -> Self {
        Self {
            price_min_bp: 5_000,
            price_max_bp: 20_000,
            bias_min_bp: -3_000,
            bias_max_bp: 3_000,
            price_step_bp: 500,
            bias_step_bp: 250,
        }
    }
}

/// 目标与阈值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyTargets {
    /// 目标接受率（基点）：冷启动时决定定价方向。
    pub accept_rate_target_bp: i64,
    /// 接受率死区（基点）：与目标差距小于它时不因接受率而动价。
    pub accept_deadband_bp: i64,
    /// 学习门槛：置信度低于它就不改动任何参数。
    pub min_confidence_bp: i64,
    /// 违规惩罚（基点）：有过违规记录的协作者一次性下调多少偏好。
    pub violation_penalty_bp: i64,
    /// 收益归一化参考（微积分）：把平均收益折算成万分比时的分母。
    pub reward_ref: Credits,
    /// 收益趋势死区（基点）：平滑收益的跌幅小于它时视为噪声，不改变定价方向。
    pub reward_noise_floor_bp: i64,
}

impl Default for PolicyTargets {
    fn default() -> Self {
        Self {
            accept_rate_target_bp: 8_500,
            accept_deadband_bp: 500,
            min_confidence_bp: 10_000,
            violation_penalty_bp: 2_000,
            reward_ref: Credits(100),
            reward_noise_floor_bp: 300,
        }
    }
}

/// 学习信号（本轮观测到的标量）。
///
/// `mean_reward` / `prev_mean_reward` 建议传**平滑后**的每任务平均收益
/// （调用方用自己的指数移动平均），因为单轮收益的方差远大于一步定价的影响：
/// 用两轮原始值比较会让定价在噪声里来回跳，而不是走向最优点。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signals {
    /// 本轮样本数。
    pub sample: usize,
    /// 置信度（万分比）。
    pub confidence_bp: i64,
    /// 本轮接受率；`None` 表示本 Agent 还没有报价历史（例如第一次评估）。
    pub accept_rate_bp: Option<i64>,
    /// 本轮成功率（万分比）。
    pub success_bp: i64,
    /// 本轮（平滑后）每任务平均收益（微积分，整数除法）。
    pub mean_reward: Credits,
    /// 上一轮（平滑后）每任务平均收益（`ZERO` 表示没有上一轮）。
    pub prev_mean_reward: Credits,
    /// 上一轮定价移动方向（-1 / 0 / +1）。
    pub prev_price_dir: i64,
    /// 本轮违规条数。
    pub violations: u32,
    /// 本轮信誉变化（v1.6.5 起由本地信誉台账提供；本版固定 0）。
    pub reputation_delta: i64,
}

impl Signals {
    /// 最小可用信号：只有样本数与平均收益，没有任何报价历史（`accept_rate_bp = None`）。
    pub fn cold_start(sample: usize, quality_bp: i64, mean_reward: Credits) -> Self {
        Self {
            sample,
            confidence_bp: crate::feedback::confidence_of(sample),
            accept_rate_bp: None,
            success_bp: quality_bp,
            mean_reward,
            prev_mean_reward: Credits::ZERO,
            prev_price_dir: 0,
            violations: 0,
            reputation_delta: 0,
        }
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

/// 一次行为调整的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyAdjustment {
    /// 调整后的参数（无证据时与输入相同）。
    pub next: PolicyParams,
    /// 参数是否真的变了。
    pub changed: bool,
    /// 定价移动量（基点，带符号）。
    pub price_moved_bp: i64,
    /// 任务偏好移动量。
    pub task_bias_moved_bp: BTreeMap<String, i64>,
    /// 协作者偏好移动量（本地视图，含 DID）。
    pub peer_bias_moved_bp: BTreeMap<Did, i64>,
    /// 每一步的理由（由输入数据决定）。
    pub reasons: Vec<String>,
}

impl PolicyAdjustment {
    fn unchanged(current: &PolicyParams, reason: &str) -> Self {
        Self {
            next: current.clone(),
            changed: false,
            price_moved_bp: 0,
            task_bias_moved_bp: BTreeMap::new(),
            peer_bias_moved_bp: BTreeMap::new(),
            reasons: vec![reason.to_string()],
        }
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 公开投影：理由可以发布（不含 DID 的策略自我解释），协作者移动量只发布聚合。
    pub fn public_json(&self) -> CoreResult<Value> {
        Ok(serde_json::json!({
            "changed": self.changed,
            "price_moved_bp": self.price_moved_bp,
            "task_bias_moved_bp": &self.task_bias_moved_bp,
            "peers_moved": self.peer_bias_moved_bp.len(),
            "peer_bias_moved_min_bp": self.peer_bias_moved_bp.values().copied().min(),
            "peer_bias_moved_max_bp": self.peer_bias_moved_bp.values().copied().max(),
            "reasons": &self.reasons,
        }))
    }
}

/// 任务类型的综合评分（万分比）：完成质量 2/3 + 收益 1/3。
///
/// 只用两个可观测维度，避免引入「权重调参」这种不可证伪的东西：
/// 质量来自 `Outcome`，收益来自经验里的 `reward`（经 `reward_ref` 归一化后封顶 10000）。
pub fn task_score_bp(feedback: &crate::feedback::Feedback, reward_ref: Credits) -> i64 {
    let reward_bp = if reward_ref.get() > 0 {
        feedback
            .mean_reward
            .get()
            .saturating_mul(10_000)
            .checked_div(reward_ref.get())
            .unwrap_or(0)
            .min(10_000)
    } else {
        0
    };
    (2 * feedback.quality_bp + reward_bp) / 3
}

/// 核心入口：根据反馈与信号计算下一个策略参数。
pub fn adjust(
    current: &PolicyParams,
    report: &FeedbackReport,
    violations: &ViolationLog,
    signals: &Signals,
    bounds: &PolicyBounds,
    targets: &PolicyTargets,
) -> CoreResult<PolicyAdjustment> {
    validate_bounds(bounds, targets)?;

    // 不变式 1：没有足够证据就不动。
    if signals.confidence_bp < targets.min_confidence_bp {
        return Ok(PolicyAdjustment::unchanged(
            current,
            &format!(
                "evidence-below-threshold: confidence={}bp < {}bp，本轮不调整任何策略参数",
                signals.confidence_bp, targets.min_confidence_bp
            ),
        ));
    }

    let mut next = current.clone();
    let mut reasons = Vec::new();

    // ---- 定价策略 ----
    let mut dir = price_direction(signals, targets);
    // 信誉变化也是一类学习信号：信誉在下降时冻结涨价（不为了一点收益去冒失去协作对象的险）。
    // 这一条让「信誉变化」这个信号真的参与决策，而不是被记录后丢掉。
    if dir > 0 && signals.reputation_delta < 0 {
        reasons.push(format!(
            "pricing: reputation_delta={} < 0 → 冻结涨价（信誉下降时不提价）",
            signals.reputation_delta
        ));
        dir = 0;
    }
    let price_moved = move_price(&mut next, dir, bounds);
    if price_moved != 0 {
        reasons.push(format!(
            "pricing: accept={}bp target={}bp prev_reward={} now_reward={} → price {}{}{}bp",
            signals.accept_rate_bp.map(|v| v.to_string()).unwrap_or_else(|| "n/a".to_string()),
            targets.accept_rate_target_bp,
            signals.prev_mean_reward,
            signals.mean_reward,
            current.price_bp,
            if price_moved > 0 { "+" } else { "-" },
            price_moved.abs()
        ));
    } else {
        reasons.push(format!(
            "pricing: hold at {}bp (accept={}bp target={}bp deadband=±{}bp)",
            next.price_bp,
            signals.accept_rate_bp.map(|v| v.to_string()).unwrap_or_else(|| "n/a".to_string()),
            targets.accept_rate_target_bp,
            targets.accept_deadband_bp
        ));
    }

    // ---- 任务选择策略 ----
    let overall_score = task_score_bp(&report.overall, targets.reward_ref);
    let mut task_moved: BTreeMap<String, i64> = BTreeMap::new();
    for f in &report.per_type {
        if !f.sufficient {
            continue;
        }
        let score = task_score_bp(f, targets.reward_ref);
        let delta = (score - overall_score)
            .checked_div(20)
            .unwrap_or(0)
            .clamp(-bounds.bias_step_bp, bounds.bias_step_bp);
        let before = next.bias_of_task(&f.scope);
        let after = (before + delta).clamp(bounds.bias_min_bp, bounds.bias_max_bp);
        if after != before {
            task_moved.insert(f.scope.clone(), after - before);
        }
        next.task_bias_bp.insert(f.scope.clone(), after);
        reasons.push(format!(
            "task-select: {} score={}bp overall={}bp → bias {}→{}bp",
            f.scope, score, overall_score, before, after
        ));
    }

    // ---- 协作对象选择策略 ----
    let mut peer_moved: BTreeMap<Did, i64> = BTreeMap::new();
    let mut peers: Vec<Did> = report.per_peer.iter().map(|p| p.peer.clone()).collect();
    for p in violations.peers() {
        if !peers.contains(&p) {
            peers.push(p);
        }
    }
    for peer in peers {
        let before = next.bias_of_peer(&peer);
        let mut after = before;
        let mut why = String::new();
        if let Some(stats) = report.peer_stats(&peer) {
            if stats.sufficient || stats.sample > 0 {
                let delta = (stats.quality_bp - report.overall.quality_bp)
                    .checked_div(20)
                    .unwrap_or(0)
                    .clamp(-bounds.bias_step_bp, bounds.bias_step_bp);
                after = (after + delta).clamp(bounds.bias_min_bp, bounds.bias_max_bp);
                why.push_str(&format!(
                    "quality={}bp overall_quality={}bp",
                    stats.quality_bp, report.overall.quality_bp
                ));
            }
        }
        let violations_of_peer = violations.count_of(&peer);
        if violations_of_peer > 0 {
            after = (after - targets.violation_penalty_bp).clamp(bounds.bias_min_bp, bounds.bias_max_bp);
            if !why.is_empty() {
                why.push_str("; ");
            }
            why.push_str(&format!("violations={violations_of_peer} penalty={}bp", targets.violation_penalty_bp));
        }
        if after != before {
            peer_moved.insert(peer.clone(), after - before);
        }
        next.peer_bias_bp.insert(peer.clone(), after);
        if !why.is_empty() {
            reasons.push(format!(
                "peer-select: {} bias {}→{}bp ({why})",
                peer_tag(&peer),
                before,
                after
            ));
        }
    }

    let changed = next != *current;
    if !changed {
        reasons.push("no-change: 反馈与当前策略一致，保持参数不变".to_string());
    }
    Ok(PolicyAdjustment {
        next,
        changed,
        price_moved_bp: price_moved,
        task_bias_moved_bp: task_moved,
        peer_bias_moved_bp: peer_moved,
        reasons,
    })
}

/// 定价方向。
///
/// 主驱动是**接受率与目标区间的偏差**（离目标越远，方向越确定），
/// 收益趋势只作安全阀：平滑收益显著下滑（超过死区）时反向，说明当前方向在把 Agent 带离最优点。
///
/// 为什么不用「收益变好就继续」当主驱动：单轮收益的方差远大于一步定价的影响（8–12 个任务里
/// 成败比例波动很大），拿它当方向会让定价在噪声里随机游走——本轨道 v1.6.3 的第一版实现
/// 就是这样，实测学习组比对照组还差；改成「接受率驱动 + 收益安全阀」后收敛到目标区间。
fn price_direction(signals: &Signals, targets: &PolicyTargets) -> i64 {
    let base = match signals.accept_rate_bp {
        None => 0,
        Some(accept) => {
            let gap = accept - targets.accept_rate_target_bp;
            if gap.abs() <= targets.accept_deadband_bp {
                0
            } else if gap > 0 {
                1
            } else {
                -1
            }
        }
    };
    if base == 0 {
        // 落在死区内 = 收敛点：停手，不因为噪声继续推动价格。
        return 0;
    }
    if signals.prev_mean_reward > Credits::ZERO {
        let floor = targets
            .reward_noise_floor_bp
            .saturating_mul(signals.prev_mean_reward.get())
            / 10_000;
        let delta = signals
            .mean_reward
            .get()
            .saturating_sub(signals.prev_mean_reward.get());
        if delta < -floor {
            return -base;
        }
    }
    base
}

/// 移动定价一步并返回带符号的移动量（越界则不动）。
fn move_price(params: &mut PolicyParams, dir: i64, bounds: &PolicyBounds) -> i64 {
    if dir == 0 {
        return 0;
    }
    let step = bounds.price_step_bp * dir.signum();
    let after = (params.price_bp + step).clamp(bounds.price_min_bp, bounds.price_max_bp);
    let moved = after - params.price_bp;
    params.price_bp = after;
    moved
}

/// 协作者的**可发布标签**：DID 的内容摘要前 16 位。
///
/// 为什么不用 `short_id(did)`：DID 形如 `did:au4a:<hex>`，直接截断会带上 `did:au4a:` 前缀，
/// 于是「公开投影脱敏」这条不变式会被自己的解释文本破坏。用摘要前缀既不泄露 DID 形式，
/// 又能在本地把同一协作者的多条理由对上号。
fn peer_tag(peer: &Did) -> String {
    au4a_core::short_id(&au4a_core::content_hash(peer.as_str().as_bytes()))
}

fn validate_bounds(bounds: &PolicyBounds, targets: &PolicyTargets) -> CoreResult<()> {
    if bounds.price_min_bp <= 0
        || bounds.price_max_bp < bounds.price_min_bp
        || bounds.bias_min_bp > bounds.bias_max_bp
    {
        return Err(CoreError::NegativeAmount);
    }
    if bounds.price_step_bp < 0 || bounds.bias_step_bp < 0 {
        return Err(CoreError::NegativeAmount);
    }
    if targets.accept_rate_target_bp < 0 || targets.accept_deadband_bp < 0 {
        return Err(CoreError::NegativeAmount);
    }
    if targets.min_confidence_bp < 0 || targets.min_confidence_bp > 10_000 {
        return Err(CoreError::InvalidKind);
    }
    Ok(())
}

/// 真实断言：无证据不动、有证据必动、越界被钳制、理由可追溯。
pub fn self_check() -> Vec<SelfCheck> {
    use crate::experience::{Experience, ExperienceStore, Outcome};
    use crate::feedback::FeedbackAnalyser;

    let mut checks = Vec::new();
    let peers: Vec<Did> = (0..2u8)
        .map(|s| au4a_core::AgentKeys::from_seed(&[0x40 + s; 32]).did())
        .collect();
    let baseline = PolicyParams::baseline();
    let bounds = PolicyBounds::default();
    let targets = PolicyTargets::default();

    // 场景：类型 A 全成功高收益，类型 B 全失败零收益；peer0 成功、peer1 失败。
    let mut store = match ExperienceStore::new(32) {
        Ok(s) => s,
        Err(_) => return vec![crate::check("policy.evidence_gate", false, "经验库构造失败")],
    };
    let mut ok = true;
    for i in 0..8u64 {
        let (task_type, outcome, reward, peer) = if i % 2 == 0 {
            ("good.type", Outcome::Success, 80i64, peers[0].clone())
        } else {
            ("bad.type", Outcome::Failure, 0, peers[1].clone())
        };
        let context = format!("ctx-{i}");
        let built = Experience::new(
            &format!("t-{i}"),
            task_type,
            &context,
            "deliver",
            outcome,
            Credits(reward),
            i,
            &[peer],
        )
        .and_then(|e| store.record(e));
        if built.is_err() {
            ok = false;
        }
    }
    let report = match FeedbackAnalyser::analyse(&store) {
        Ok(r) => r,
        Err(_) => return vec![crate::check("policy.evidence_gate", false, "反馈分析失败")],
    };
    let violations = ViolationLog::new();

    // 1) 置信度不足 → 参数原封不动
    let weak = Signals::cold_start(1, 5_000, Credits(10));
    let held = adjust(&baseline, &report, &violations, &weak, &bounds, &targets);
    let hold_ok = held
        .map(|a| !a.changed && a.next == baseline && a.reasons[0].contains("evidence-below-threshold"))
        .unwrap_or(false);
    checks.push(crate::check(
        "policy.evidence_gate",
        ok && hold_ok,
        format!("样本 1 → confidence 2500bp < 10000bp 时参数不变（含理由）：{hold_ok}"),
    ));

    // 2) 证据充分 → 三个策略都动，且方向正确
    let signals = Signals {
        sample: 8,
        confidence_bp: 10_000,
        accept_rate_bp: Some(3_000),
        success_bp: 5_000,
        mean_reward: Credits(10),
        prev_mean_reward: Credits::ZERO,
        prev_price_dir: 0,
        violations: 0,
        reputation_delta: 0,
    };
    let moved = adjust(&baseline, &report, &violations, &signals, &bounds, &targets);
    let (moved_ok, detail) = match &moved {
        Ok(a) => {
            let price_down = a.price_moved_bp == -bounds.price_step_bp;
            let good_up = a.task_bias_moved_bp.get("good.type").copied().unwrap_or(0) > 0;
            let bad_down = a.task_bias_moved_bp.get("bad.type").copied().unwrap_or(0) < 0;
            let peer_up = a
                .peer_bias_moved_bp
                .get(&peers[0])
                .copied()
                .unwrap_or(0)
                > 0;
            let peer_down = a
                .peer_bias_moved_bp
                .get(&peers[1])
                .copied()
                .unwrap_or(0)
                < 0;
            (
                a.changed && price_down && good_up && bad_down && peer_up && peer_down,
                format!(
                    "price_moved={} good.type={:?} bad.type={:?} peer0={:?} peer1={:?}",
                    a.price_moved_bp,
                    a.task_bias_moved_bp.get("good.type"),
                    a.task_bias_moved_bp.get("bad.type"),
                    a.peer_bias_moved_bp.get(&peers[0]),
                    a.peer_bias_moved_bp.get(&peers[1])
                ),
            )
        }
        Err(e) => (false, format!("调整失败: {e:?}")),
    };
    checks.push(crate::check("policy.three_levers_move", moved_ok, detail));

    // 3) 违规惩罚：有违规记录的协作者偏好被额外下调
    let mut with_violation = ViolationLog::new();
    let penalty_ok = match crate::violation::Violation::new(&peers[0], "t-0", "withheld", 1) {
        Ok(v) => {
            let _ = with_violation.record(v);
            adjust(&baseline, &report, &with_violation, &signals, &bounds, &targets)
                .map(|a| {
                    a.next.bias_of_peer(&peers[0])
                        < moved.as_ref().map(|m| m.next.bias_of_peer(&peers[0])).unwrap_or(0)
                })
                .unwrap_or(false)
        }
        Err(_) => false,
    };
    checks.push(crate::check(
        "policy.violation_penalty",
        penalty_ok,
        "同一协作者：有违规记录时的偏好严格低于无违规记录时",
    ));

    // 4) 边界被钳制：极端参数不会越界
    let extreme = PolicyParams {
        price_bp: bounds.price_min_bp,
        task_bias_bp: BTreeMap::new(),
        peer_bias_bp: BTreeMap::new(),
    };
    let clamp_ok = adjust(&extreme, &report, &violations, &signals, &bounds, &targets)
        .map(|a| {
            a.next.price_bp >= bounds.price_min_bp
                && a.next.price_bp <= bounds.price_max_bp
                && a
                    .next
                    .task_bias_bp
                    .values()
                    .all(|v| *v >= bounds.bias_min_bp && *v <= bounds.bias_max_bp)
        })
        .unwrap_or(false);
    checks.push(crate::check(
        "policy.bounds_clamped",
        clamp_ok,
        "价格已在下界时不再下移；所有偏好落在 [bias_min, bias_max] 内",
    ));
    checks
}
