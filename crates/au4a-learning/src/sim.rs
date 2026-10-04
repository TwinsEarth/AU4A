//! v1.6.3 合成任务市场（用于证明「学习真的改变行为」）。
//!
//! 这是本轨道唯一的实验台：一个**确定性**的单 Agent 市场，用来做学习组 vs 对照组的对照实验。
//!
//! 市场规则（全部整数、全部由种子散列决定，与调用顺序无关）：
//!
//! * 每个 tick 提供 3 个「(任务类型, 协作者)」组合；Agent 选一个（这就是任务选择 + 协作对象选择）。
//! * 报价 = `类型参考价 × price_bp / 10000`；接受率由需求曲线 `accept_rate_curve_bp(price_bp)` 决定。
//! * 成交后按「类型难度 + 协作者可靠性」决定成败；失败里有一部分是部分成功（半价结算）。
//! * 某些协作者是**恶意**的：会记录违规、不给收益。Agent 只能从结果里学出来。
//!
//! 两个arm 用**同一个种子**：市场（报价是否被接受、成败、违规）对两组完全一致，
//! 唯一差别是学习组会更新策略参数、对照组保持初始参数。所以两组的差值来自策略本身。
//!
//! 诚实边界（证据等级 `cpu-proto`）：市场是合成的，隐藏参数（难度、可靠性、违规率）
//! 由本文件的常量定义。因此「学习带来提升」这句话的含义是：
//! **在这个合成环境里、相对未学习的初始策略，提升是可测量的**——不是真实网络的绩效数据。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Credits, Did, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::experience::{Experience, ExperienceStore, Outcome};
use crate::feedback::{FeedbackAnalyser, MIN_SAMPLES};
use crate::model::{LearningModel, UpdateRecord};
use crate::policy::{adjust, PolicyAdjustment, PolicyBounds, PolicyParams, PolicyTargets};
use crate::rng::{hash_below, hash_bp};
use crate::signal::{LearningSignal, SignalWeights};
use crate::violation::{Violation, ViolationLog};

/// 任务类型画像。Agent **观察不到** `difficulty_bp`，只能从结果里估计。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskProfile {
    pub name: &'static str,
    /// 基础成功率（万分比）。
    pub difficulty_bp: i64,
    /// 参考价（微积分）。
    pub base_price: Credits,
}

/// 市场里的任务类型（与 `scenario::TASK_TYPES` 一致）。
pub const TASK_PROFILES: [TaskProfile; 3] = [
    TaskProfile {
        name: "translate.en-zh",
        difficulty_bp: 6_200,
        base_price: Credits(60),
    },
    TaskProfile {
        name: "summarize.zh",
        difficulty_bp: 4_800,
        base_price: Credits(40),
    },
    TaskProfile {
        name: "classify.zh",
        difficulty_bp: 7_600,
        base_price: Credits(90),
    },
];

/// 协作者画像（隐藏的真实属性）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerProfile {
    pub did: Did,
    pub reliability_bp: i64,
    /// 每次成交时发生违规的概率（万分比）。
    pub violation_bp: i64,
}

/// 由种子确定性地生成协作者画像：可靠性 3000–9000bp，约 1/3 是恶意协作者。
pub fn peer_profiles(seed: u64, count: u32) -> CoreResult<Vec<PeerProfile>> {
    if count == 0 || count > 16 {
        return Err(CoreError::InvalidKind);
    }
    let mut out = Vec::new();
    for i in 0..count {
        let did = AgentKeys::from_seed(&[0x40u8 + i as u8; 32]).did();
        let reliability_bp = 3_000 + hash_below(seed, &[i as u64, 1], 6_000) as i64;
        let malicious = hash_below(seed, &[i as u64, 2], 100) < 33;
        let violation_bp = if malicious { 4_000 } else { 100 };
        out.push(PeerProfile {
            did,
            reliability_bp,
            violation_bp,
        });
    }
    Ok(out)
}

/// 需求曲线：参考价（10000bp）下接受率 6500bp；价格每上浮 1000bp，接受率下降 1200bp。
pub fn accept_rate_curve_bp(price_bp: i64) -> i64 {
    let raw = 6_500 - (price_bp - 10_000) * 12 / 10;
    raw.clamp(500, 9_900)
}

/// 实际成功率：类型难度 + 协作者可靠性溢价（`(reliability - 5000) / 2`）。
pub fn effective_success_bp(task: &TaskProfile, peer: &PeerProfile) -> i64 {
    let peer_bonus = (peer.reliability_bp - 5_000) / 2;
    (task.difficulty_bp + peer_bonus).clamp(300, 9_900)
}

/// 市场配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketConfig {
    pub seed: u64,
    /// 学习轮数（每轮结束做一次反馈分析与行为调整）。
    pub rounds: u32,
    /// 每轮任务数。
    pub ticks_per_round: u32,
    /// 协作者数量。
    pub peers: u32,
    /// 选择噪声幅度（基点）：保证有探索，不靠「上帝视角」。
    pub noise_span_bp: i64,
    /// `true` = 学习组；`false` = 对照组（参数冻结在初始策略）。
    pub learn: bool,
    /// 是否允许学习**定价**（消融实验用）。
    pub learn_price: bool,
    /// 是否允许学习**任务选择偏好**（消融实验用）。
    pub learn_task: bool,
    /// 是否允许学习**协作对象偏好**（消融实验用）。
    pub learn_peer: bool,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            seed: crate::scenario::SCENARIO_SEED,
            rounds: 16,
            ticks_per_round: 12,
            peers: 8,
            noise_span_bp: 1_200,
            learn: true,
            learn_price: true,
            learn_task: true,
            learn_peer: true,
        }
    }
}

impl MarketConfig {
    pub fn validate(&self) -> CoreResult<()> {
        if self.rounds == 0
            || (self.ticks_per_round as usize) < MIN_SAMPLES
            || self.peers < 2
            || self.peers > 16
            || self.noise_span_bp < 0
        {
            return Err(CoreError::InvalidKind);
        }
        if self.learn && !(self.learn_price || self.learn_task || self.learn_peer) {
            // 学习打开但三个杠杆全被屏蔽 = 配置自相矛盾（会得到与对照组相同的轨迹）
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }

    /// 与配置相同、只切换学习开关的对照配置。
    pub fn arm(&self, learn: bool) -> Self {
        let mut c = self.clone();
        c.learn = learn;
        c
    }
}

/// 单轮统计。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundStats {
    pub round: u32,
    pub ticks: u32,
    pub accepted: u32,
    pub successes: u32,
    pub partials: u32,
    pub failures: u32,
    pub violations: u32,
    pub revenue: Credits,
    pub mean_reward: Credits,
    pub price_bp: i64,
    /// 参数摘要（内容寻址，便于断言「参数真的变了」而不泄露协作者 DID）。
    pub params_digest: String,
    /// 本轮参数是否改变。
    pub changed: bool,
    /// 本轮四类学习信号的综合值（万分比）。
    pub signal_composite_bp: i64,
    /// 本轮价格实际位移（模型更新之后，基点）。
    pub price_drift_bp: i64,
    /// 本轮信誉变化合计（本地台账）。
    pub reputation_delta: i64,
    /// 本轮的模型代际。
    pub generation: u32,
}

/// 一次市场跑批的完整结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketRun {
    pub learn: bool,
    pub seed: u64,
    pub ticks: u32,
    pub accepted: u32,
    pub successes: u32,
    pub partials: u32,
    pub failures: u32,
    pub violations: u32,
    pub revenue: Credits,
    pub initial_params: PolicyParams,
    pub final_params: PolicyParams,
    /// 结束时的模型代际（学习组应等于更新次数）。
    pub final_generation: u32,
    /// 结束时本地信誉台账的公开投影（无 DID）。
    pub final_reputation: Value,
    pub rounds: Vec<RoundStats>,
    /// 经验窗口里保留的条数（滚动窗口 = 3 轮）。
    pub window_experiences: usize,
    /// 全流程记录过的经验总条数。
    pub experiences_recorded: u32,
}

impl MarketRun {
    /// 成功率（万分比，分母是所有 tick）。被拒报价的任务计入未成功。
    pub fn success_rate_bp(&self) -> i64 {
        ratio_bp(self.successes as i64, self.ticks as i64)
    }

    /// 部分成功率（万分比）。
    pub fn partial_rate_bp(&self) -> i64 {
        ratio_bp(self.partials as i64, self.ticks as i64)
    }

    pub fn accept_rate_bp(&self) -> i64 {
        ratio_bp(self.accepted as i64, self.ticks as i64)
    }

    /// 每 tick 平均收益（整数除法；被拒/失败/违规的 tick 收益为 0）。
    pub fn revenue_per_tick(&self) -> Credits {
        Credits(self.revenue.get() / self.ticks.max(1) as i64)
    }

    /// 策略参数是否真的变了（学习是否发生）。
    pub fn params_changed(&self) -> bool {
        self.final_params != self.initial_params
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 公开投影：不含协作者 DID、不含本地经验库内容。
    pub fn public_json(&self) -> CoreResult<Value> {
        Ok(serde_json::json!({
            "learn": self.learn,
            "seed": self.seed,
            "ticks": self.ticks,
            "accepted": self.accepted,
            "successes": self.successes,
            "partials": self.partials,
            "failures": self.failures,
            "violations": self.violations,
            "revenue": self.revenue,
            "revenue_per_tick": self.revenue_per_tick(),
            "success_rate_bp": self.success_rate_bp(),
            "accept_rate_bp": self.accept_rate_bp(),
            "params_changed": self.params_changed(),
            "initial_params": self.initial_params.public_json()?,
            "final_params": self.final_params.public_json()?,
            "final_generation": self.final_generation,
            "final_reputation": self.final_reputation,
            "rounds": &self.rounds,
            "window_experiences": self.window_experiences,
            "experiences_recorded": self.experiences_recorded,
        }))
    }
}

/// 学习组 vs 对照组的对比报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    pub seed: u64,
    pub ticks: u32,
    pub control_success_bp: i64,
    pub learning_success_bp: i64,
    pub success_lift_bp: i64,
    pub control_revenue: Credits,
    pub learning_revenue: Credits,
    pub revenue_lift_credits: Credits,
    pub revenue_lift_bp: i64,
    pub control_violations: u32,
    pub learning_violations: u32,
    pub control_price_bp: i64,
    pub learning_price_bp: i64,
    pub params_differ: bool,
}

impl Comparison {
    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 改善是否可量化：策略不同 + 成功率不降 + 收益严格上升 + 违规不增。
    pub fn improved(&self) -> bool {
        self.params_differ
            && self.success_lift_bp > 0
            && self.revenue_lift_credits > Credits::ZERO
            && self.learning_violations <= self.control_violations
    }
}

/// 对照实验结果：`(对照组, 学习组, 对比报告)`。
pub fn ab_test(config: &MarketConfig) -> CoreResult<(MarketRun, MarketRun, Comparison)> {
    config.validate()?;
    let control = run(&config.arm(false))?;
    let learning = run(&config.arm(true))?;
    let comparison = compare(&control, &learning)?;
    Ok((control, learning, comparison))
}

/// 对比两次跑批。
pub fn compare(control: &MarketRun, learning: &MarketRun) -> CoreResult<Comparison> {
    if control.seed != learning.seed || control.ticks != learning.ticks {
        // 不同市场不可比：宁可报错，也不要给出一个看起来像结论的数字。
        return Err(CoreError::InvalidKind);
    }
    let revenue_lift = learning.revenue.checked_sub(control.revenue)?;
    let revenue_lift_bp = if control.revenue > Credits::ZERO {
        learning
            .revenue
            .get()
            .saturating_sub(control.revenue.get())
            .saturating_mul(10_000)
            / control.revenue.get()
    } else {
        0
    };
    Ok(Comparison {
        seed: control.seed,
        ticks: control.ticks,
        control_success_bp: control.success_rate_bp(),
        learning_success_bp: learning.success_rate_bp(),
        success_lift_bp: learning.success_rate_bp() - control.success_rate_bp(),
        control_revenue: control.revenue,
        learning_revenue: learning.revenue,
        revenue_lift_credits: revenue_lift,
        revenue_lift_bp,
        control_violations: control.violations,
        learning_violations: learning.violations,
        control_price_bp: control.final_params.price_bp,
        learning_price_bp: learning.final_params.price_bp,
        params_differ: learning.final_params != control.final_params,
    })
}

/// 跑一遍市场。
pub fn run(config: &MarketConfig) -> CoreResult<MarketRun> {
    config.validate()?;
    let profiles = peer_profiles(config.seed, config.peers)?;
    let bounds = PolicyBounds::default();
    let targets = PolicyTargets::default();
    let window = (config.ticks_per_round as usize)
        .saturating_mul(3)
        .max(MIN_SAMPLES);
    let mut store = ExperienceStore::new(window)?;
    let mut violations = ViolationLog::new();
    // v1.6.5：行为调整的意图由模型更新落地（学习率/动量/阻尼/遗忘/漂移钳制），信誉台账记录观测到的结果。
    let mut model = LearningModel::new(bounds.clone());
    let initial_params = model.params().clone();

    let mut rounds = Vec::with_capacity(config.rounds as usize);
    let mut totals = Totals::default();
    let mut prev_reward = Credits::ZERO;
    let mut prev_dir = 0i64;
    let mut experiences_recorded = 0u32;

    for round in 0..config.rounds {
        let mut acc = Totals::default();
        for tick in 0..config.ticks_per_round {
            let global = (round as u64) * (config.ticks_per_round as u64) + (tick as u64);
            let Some((task_index, peer_index)) =
                pick_option(config, &profiles, model.params(), global)
            else {
                continue;
            };
            acc.ticks = acc.ticks.saturating_add(1);
            let task = &TASK_PROFILES[task_index];
            let peer = &profiles[peer_index];
            let task_id = format!("r{round}-t{tick}");
            let price = task.base_price.scaled_bp(model.params().price_bp)?;
            let accepted = hash_bp(config.seed, &[global, task_index as u64, 6])
                < accept_rate_curve_bp(model.params().price_bp);
            let mut revenue = Credits::ZERO;
            let (outcome, action);
            if !accepted {
                outcome = Outcome::Failure;
                action = "quote-rejected";
            } else {
                acc.accepted = acc.accepted.saturating_add(1);
                if hash_bp(config.seed, &[global, task_index as u64, 9]) < peer.violation_bp {
                    // 恶意协作者：收了活不交付 → 记违规、零收益。
                    outcome = Outcome::Failure;
                    action = "deliver-withheld";
                    acc.violations = acc.violations.saturating_add(1);
                    violations.record(Violation::new(
                        &peer.did,
                        &task_id,
                        "peer-withheld-deliverable",
                        global,
                    )?)?;
                } else if hash_bp(config.seed, &[global, task_index as u64, 7])
                    < effective_success_bp(task, peer)
                {
                    outcome = Outcome::Success;
                    action = "deliver";
                    revenue = price;
                    acc.successes = acc.successes.saturating_add(1);
                } else if hash_bp(config.seed, &[global, task_index as u64, 8]) < 3_000 {
                    outcome = Outcome::Partial;
                    action = "deliver-partial";
                    revenue = Credits(price.get() / 2);
                    acc.partials = acc.partials.saturating_add(1);
                } else {
                    outcome = Outcome::Failure;
                    action = "deliver-failed";
                }
            }
            let context = format!("market r{round}t{tick}: {} 任务，单一协作者", task.name);
            let exp = Experience::new(
                &task_id,
                task.name,
                &context,
                action,
                outcome,
                revenue,
                global,
                std::slice::from_ref(&peer.did),
            )?;
            store.record(exp)?;
            acc.revenue = acc.revenue.checked_add(revenue)?;
            // v1.6.5：本地信誉台账只按**观测到的结果**累计（违规单独一条更强的负向更新）。
            acc.reputation_delta = acc
                .reputation_delta
                .saturating_add(model.reputation_mut().apply_outcome(&peer.did, outcome));
            if action == "deliver-withheld" {
                acc.reputation_delta = acc
                    .reputation_delta
                    .saturating_add(model.reputation_mut().apply_violation(&peer.did));
            }
        }

        // ---- 学习阶段：反馈分析 → 学习信号 → 行为调整 ----
        let report = FeedbackAnalyser::analyse(&store)?;
        let ticks = acc.ticks.max(1) as i64;
        let round_mean = Credits(acc.revenue.get() / ticks);
        // 收益趋势用指数移动平均（α = 1/4）：单轮收益的方差远大于一步定价的影响，
        // 不平滑就会把噪声当趋势（v1.6.3 第一版的实测教训）。
        let reward_ema = if round == 0 {
            round_mean
        } else {
            Credits((3 * prev_reward.get() + round_mean.get()) / 4)
        };
        // v1.6.4/v1.6.5：四类学习信号（质量/结算/信誉/违规）从经验窗口、信誉台账与违规台账折算。
        let signal = LearningSignal::from_store(
            &store,
            &violations,
            acc.reputation_delta,
            &SignalWeights::default(),
        )?;
        let signals = signal.to_signals(
            acc.ticks as usize,
            Some(ratio_bp(acc.accepted as i64, acc.ticks as i64)),
            reward_ema,
            prev_reward,
            prev_dir,
        );
        let adjustment = adjust(
            model.params(),
            &report,
            &violations,
            &signals,
            &bounds,
            &targets,
        )?;
        let mut record: Option<UpdateRecord> = None;
        if config.learn && adjustment.changed {
            // 消融：按配置屏蔽某些策略杠杆（掩码后的意图才交给模型更新）
            let masked = mask_adjustment(&adjustment, config);
            let any_lever = masked.price_moved_bp != 0
                || !masked.task_bias_moved_bp.is_empty()
                || !masked.peer_bias_moved_bp.is_empty();
            if any_lever {
                // 意图由模型更新落地：学习率缩放 → 阻尼 → 动量 → 遗忘 → 漂移钳制。
                let applied = model.apply(&masked)?;
                prev_dir = if applied.price_drift_bp == 0 {
                    prev_dir
                } else {
                    applied.price_drift_bp.signum()
                };
                record = Some(applied);
            }
        }
        prev_reward = reward_ema;

        rounds.push(RoundStats {
            round,
            ticks: acc.ticks,
            accepted: acc.accepted,
            successes: acc.successes,
            partials: acc.partials,
            failures: acc
                .ticks
                .saturating_sub(acc.successes)
                .saturating_sub(acc.partials),
            violations: acc.violations,
            revenue: acc.revenue,
            mean_reward: round_mean,
            price_bp: model.params().price_bp,
            params_digest: model.params().digest()?,
            changed: record.is_some(),
            signal_composite_bp: signal.composite_bp,
            price_drift_bp: record.as_ref().map(|r| r.price_drift_bp).unwrap_or(0),
            reputation_delta: acc.reputation_delta,
            generation: model.generation(),
        });
        totals.add(&acc);
        experiences_recorded = experiences_recorded.saturating_add(acc.ticks);
    }

    Ok(MarketRun {
        learn: config.learn,
        seed: config.seed,
        ticks: totals.ticks,
        accepted: totals.accepted,
        successes: totals.successes,
        partials: totals.partials,
        failures: totals
            .ticks
            .saturating_sub(totals.successes)
            .saturating_sub(totals.partials),
        violations: totals.violations,
        revenue: totals.revenue,
        initial_params,
        final_params: model.params().clone(),
        final_generation: model.generation(),
        final_reputation: model.reputation().public_json()?,
        rounds,
        window_experiences: store.len(),
        experiences_recorded,
    })
}

/// 在市场提供的 3 个「(任务类型, 协作者)」组合里选一个：
/// `score = 任务偏好 + 协作者偏好 + 噪声`，取最大（并列取小下标，确定性）。
fn pick_option(
    config: &MarketConfig,
    profiles: &[PeerProfile],
    params: &PolicyParams,
    global: u64,
) -> Option<(usize, usize)> {
    let mut best: Option<(i64, usize, usize)> = None;
    for (task_index, task) in TASK_PROFILES.iter().enumerate() {
        let peer_index = hash_below(
            config.seed,
            &[global, task_index as u64, 3],
            profiles.len() as u64,
        ) as usize;
        let Some(peer) = profiles.get(peer_index) else {
            continue;
        };
        let span = (config.noise_span_bp as u64).saturating_mul(2) + 1;
        let noise = hash_below(config.seed, &[global, task_index as u64, 4], span) as i64
            - config.noise_span_bp;
        let score = params.bias_of_task(task.name) + params.bias_of_peer(&peer.did) + noise;
        let replace = match best {
            None => true,
            Some((best_score, _, _)) => score > best_score,
        };
        if replace {
            best = Some((score, task_index, peer_index));
        }
    }
    best.map(|(_, t, p)| (t, p))
}

fn ratio_bp(part: i64, whole: i64) -> i64 {
    if whole <= 0 {
        0
    } else {
        part.saturating_mul(10_000) / whole
    }
}

/// 按 `MarketConfig` 的掩码裁掉某些策略杠杆（消融实验用）。`reasons` 原样保留，便于核对。
fn mask_adjustment(adjustment: &PolicyAdjustment, config: &MarketConfig) -> PolicyAdjustment {
    PolicyAdjustment {
        next: adjustment.next.clone(),
        changed: adjustment.changed,
        price_moved_bp: if config.learn_price {
            adjustment.price_moved_bp
        } else {
            0
        },
        task_bias_moved_bp: if config.learn_task {
            adjustment.task_bias_moved_bp.clone()
        } else {
            BTreeMap::new()
        },
        peer_bias_moved_bp: if config.learn_peer {
            adjustment.peer_bias_moved_bp.clone()
        } else {
            BTreeMap::new()
        },
        reasons: adjustment.reasons.clone(),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Totals {
    ticks: u32,
    accepted: u32,
    successes: u32,
    partials: u32,
    violations: u32,
    revenue: Credits,
    /// 本轮信誉变化合计（本地台账实际施加量）。
    reputation_delta: i64,
}

impl Totals {
    fn add(&mut self, other: &Totals) {
        self.ticks = self.ticks.saturating_add(other.ticks);
        self.accepted = self.accepted.saturating_add(other.accepted);
        self.successes = self.successes.saturating_add(other.successes);
        self.partials = self.partials.saturating_add(other.partials);
        self.violations = self.violations.saturating_add(other.violations);
        self.revenue = Credits(self.revenue.get().saturating_add(other.revenue.get()));
        self.reputation_delta = self.reputation_delta.saturating_add(other.reputation_delta);
    }
}

/// 真实断言：学习组参数确实变了、对照组没变、提升可量化、两次跑批逐字节一致。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    let config = MarketConfig::default();
    let mut runs = Vec::new();
    for _ in 0..2 {
        match ab_test(&config) {
            Ok(triple) => runs.push(triple),
            Err(_) => {
                return vec![crate::check(
                    "market.ab_test_runs",
                    false,
                    "对照实验运行失败（配置非法或散列失败）",
                )]
            }
        }
    }
    let (control, learning, cmp_first) = match runs.first() {
        Some((c, l, cmp)) => (c.clone(), l.clone(), cmp.clone()),
        None => return checks,
    };
    let cmp_second = match runs.get(1) {
        Some((_, _, cmp)) => cmp.clone(),
        None => return checks,
    };
    let control_second = match runs.get(1) {
        Some((c, _, _)) => c.clone(),
        None => return checks,
    };

    let learning_moved = learning.params_changed()
        && learning.final_params.price_bp != learning.initial_params.price_bp
        && !learning.final_params.task_bias_bp.is_empty();
    let control_frozen = !control.params_changed();
    checks.push(crate::check(
        "market.learning_changes_behaviour",
        learning_moved && control_frozen,
        format!(
            "学习组 price {}→{}bp、任务偏好 {} 项、协作者偏好 {} 项；对照组参数未变={control_frozen}",
            learning.initial_params.price_bp,
            learning.final_params.price_bp,
            learning.final_params.task_bias_bp.len(),
            learning.final_params.peer_bias_bp.len()
        ),
    ));

    checks.push(crate::check(
        "market.quantified_improvement",
        cmp_first.improved(),
        format!(
            "成功率 {}bp → {}bp（+{}bp）；收益 {}→{} 微积分（+{}bp）；违规 {}→{}",
            cmp_first.control_success_bp,
            cmp_first.learning_success_bp,
            cmp_first.success_lift_bp,
            cmp_first.control_revenue,
            cmp_first.learning_revenue,
            cmp_first.revenue_lift_bp,
            cmp_first.control_violations,
            cmp_first.learning_violations
        ),
    ));

    let digests_equal = match (control.digest(), control_second.digest()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    checks.push(crate::check(
        "market.reproducible",
        cmp_first == cmp_second && digests_equal,
        format!(
            "两次独立对照实验的对比报告相同={}；对照组跑批摘要逐字节相同={digests_equal}",
            cmp_first == cmp_second
        ),
    ));
    checks
}
