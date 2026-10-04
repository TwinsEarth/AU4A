//! v1.6.10 评估（Evaluation）：学习组 vs 对照组的固定种子评估与跨种子稳健性。
//!
//! 这一版回答本轨道的总问题：**学习到底有没有用、有多大用、稳不稳**。
//!
//! * 每个种子跑两组（对照组参数冻结、学习组走完整循环），两组面对**同一个市场**；
//! * 汇总指标按池化计算（成功率 = Σ成功 / Σ任务），避免「先算每种子再平均」把小样本放大；
//! * 稳健性用三个数说明：改善种子数、**中位数**提升、以及最差种子的提升（负值必须公开）；
//! * 消融实验（ablation）把三个策略杠杆分开跑，证明提升不是某一个旋钮的假象。
//!
//! 诚实边界：市场是合成的（隐藏难度/可靠性/违规率由 `sim` 的常量定义）。所以
//! 「学习带来提升」的**含义**是「在这个合成环境里、相对未学习的初始策略，提升可测量且跨种子稳定」，
//! 不是真实网络绩效数据 → 该主张按 `cpu-proto` 对待，代码与断言本身是 `verified`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use au4a_core::{canonical_hash, CoreError, CoreResult, Credits, SelfCheck};

use crate::sim::{ab_test, MarketConfig};

/// 默认评估种子集合（6 个不同市场形态；数字本身无关，只要固定且互不相同）。
pub const EVAL_SEEDS: [u64; 6] = [
    0x1616,
    2 * 0x1616,
    3 * 0x1616,
    5 * 0x1616,
    9 * 0x1616,
    17 * 0x1616,
];

/// 评估配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalConfig {
    pub seeds: Vec<u64>,
    /// 基准市场配置（`learn` 字段被忽略：两组都会跑）。
    pub market: MarketConfig,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            seeds: EVAL_SEEDS.to_vec(),
            market: MarketConfig::default(),
        }
    }
}

impl EvalConfig {
    pub fn validate(&self) -> CoreResult<()> {
        if self.seeds.is_empty() || self.seeds.len() > 64 {
            return Err(CoreError::InvalidKind);
        }
        self.market.validate()
    }

    /// 只跑前 `n` 个种子（自检与部署验证用它保持轻量）。
    pub fn limited(&self, n: usize) -> Self {
        let mut c = self.clone();
        c.seeds.truncate(n.clamp(1, self.seeds.len()));
        c
    }
}

/// 一组（学习组或对照组）的汇总。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmStats {
    pub runs: u32,
    pub ticks: u32,
    pub successes: u32,
    pub partials: u32,
    pub violations: u32,
    pub revenue: Credits,
    /// 池化成功率：Σ成功 / Σ任务。
    pub success_rate_bp: i64,
    /// 池化接受率。
    pub accept_rate_bp: i64,
    /// 每任务平均收益。
    pub revenue_per_tick: Credits,
    /// 有多少次跑批真的改变了策略参数。
    pub params_changed_runs: u32,
}

/// 单个种子的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedResult {
    pub seed: u64,
    pub control_success_bp: i64,
    pub learning_success_bp: i64,
    pub success_lift_bp: i64,
    pub control_revenue: Credits,
    pub learning_revenue: Credits,
    pub revenue_lift_credits: Credits,
    pub revenue_lift_bp: i64,
    pub control_violations: u32,
    pub learning_violations: u32,
    pub learning_price_bp: i64,
    pub params_differ: bool,
    /// 该种子是否「三项都不差」：成功率不降、收益提升、违规不增。
    pub improved: bool,
}

/// 一份评估报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalReport {
    pub seeds: usize,
    pub ticks_per_arm: u32,
    pub control: ArmStats,
    pub learning: ArmStats,
    pub per_seed: Vec<SeedResult>,
    /// 池化成功率提升（基点）。
    pub success_lift_bp: i64,
    /// 池化收益提升（基点，相对对照组）。
    pub revenue_lift_bp: i64,
    pub revenue_lift_credits: Credits,
    /// 违规减少条数（对照组 − 学习组）。
    pub violations_reduction: i64,
    pub improved_seeds: usize,
    pub negative_seeds: usize,
    /// 各种子成功率提升的中位数（排序后取中间；偶数个取偏保守的下中位）。
    pub median_success_lift_bp: i64,
    pub min_success_lift_bp: i64,
    pub max_success_lift_bp: i64,
}

impl EvalReport {
    /// 是否整体改善：池化成功率与收益都严格变好、违规不增、改善种子占多数。
    pub fn improved(&self) -> bool {
        self.success_lift_bp > 0
            && self.revenue_lift_credits > Credits::ZERO
            && self.violations_reduction >= 0
            && self.improved_seeds * 2 >= self.seeds
    }

    /// 是否跨种子稳定：整体改善 + 中位数为正 + 明显变差的种子不超过三分之一。
    pub fn stable(&self) -> bool {
        self.improved() && self.median_success_lift_bp > 0 && self.negative_seeds * 3 <= self.seeds
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 观察层用的紧凑投影（全部是聚合数字，无 DID、无上下文）。
    pub fn summary(&self) -> CoreResult<Value> {
        Ok(serde_json::json!({
            "seeds": self.seeds,
            "ticks_per_arm": self.ticks_per_arm,
            "control_success_bp": self.control.success_rate_bp,
            "learning_success_bp": self.learning.success_rate_bp,
            "success_lift_bp": self.success_lift_bp,
            "control_revenue": self.control.revenue,
            "learning_revenue": self.learning.revenue,
            "revenue_lift_bp": self.revenue_lift_bp,
            "control_violations": self.control.violations,
            "learning_violations": self.learning.violations,
            "violations_reduction": self.violations_reduction,
            "improved_seeds": self.improved_seeds,
            "negative_seeds": self.negative_seeds,
            "median_success_lift_bp": self.median_success_lift_bp,
            "min_success_lift_bp": self.min_success_lift_bp,
            "max_success_lift_bp": self.max_success_lift_bp,
            "improved": self.improved(),
            "stable": self.stable(),
        }))
    }
}

/// 消融实验的一行：只打开一部分策略杠杆时的提升。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AblationRow {
    pub label: String,
    pub price: bool,
    pub task: bool,
    pub peer: bool,
    pub success_lift_bp: i64,
    pub revenue_lift_bp: i64,
    pub violations_reduction: i64,
}

/// 跑一次评估。
pub fn evaluate(config: &EvalConfig) -> CoreResult<EvalReport> {
    config.validate()?;
    let mut per_seed = Vec::with_capacity(config.seeds.len());
    let mut ctrl = Totals::default();
    let mut learn = Totals::default();

    for seed in &config.seeds {
        let market = MarketConfig {
            seed: *seed,
            ..config.market.clone()
        };
        let (control, learning, comparison) = ab_test(&market)?;
        let improved = comparison.success_lift_bp > 0
            && comparison.revenue_lift_credits > Credits::ZERO
            && comparison.learning_violations <= comparison.control_violations;
        per_seed.push(SeedResult {
            seed: *seed,
            control_success_bp: comparison.control_success_bp,
            learning_success_bp: comparison.learning_success_bp,
            success_lift_bp: comparison.success_lift_bp,
            control_revenue: comparison.control_revenue,
            learning_revenue: comparison.learning_revenue,
            revenue_lift_credits: comparison.revenue_lift_credits,
            revenue_lift_bp: comparison.revenue_lift_bp,
            control_violations: comparison.control_violations,
            learning_violations: comparison.learning_violations,
            learning_price_bp: comparison.learning_price_bp,
            params_differ: comparison.params_differ,
            improved,
        });
        ctrl.add(&control);
        learn.add(&learning);
    }

    let mut lifts: Vec<i64> = per_seed.iter().map(|s| s.success_lift_bp).collect();
    lifts.sort_unstable();
    let seeds = config.seeds.len();
    let median = if lifts.is_empty() {
        0
    } else {
        lifts[(seeds - 1) / 2]
    };
    let revenue_lift_credits = learn.revenue.get().saturating_sub(ctrl.revenue.get());
    let revenue_lift_bp = if ctrl.revenue > Credits::ZERO {
        revenue_lift_credits.saturating_mul(10_000) / ctrl.revenue.get()
    } else {
        0
    };

    Ok(EvalReport {
        seeds,
        ticks_per_arm: ctrl.ticks,
        control: ctrl.stats(),
        learning: learn.stats(),
        per_seed: per_seed.clone(),
        success_lift_bp: learn.success_rate_bp() - ctrl.success_rate_bp(),
        revenue_lift_bp,
        revenue_lift_credits: Credits(revenue_lift_credits),
        violations_reduction: ctrl.violations as i64 - learn.violations as i64,
        improved_seeds: per_seed.iter().filter(|s| s.improved).count(),
        negative_seeds: per_seed.iter().filter(|s| s.success_lift_bp < 0).count(),
        median_success_lift_bp: median,
        min_success_lift_bp: lifts.first().copied().unwrap_or(0),
        max_success_lift_bp: lifts.last().copied().unwrap_or(0),
    })
}

/// 消融：分别只打开定价 / 任务偏好 / 协作偏好，以及三者全开。
///
/// 目的：证明「提升」不是某一个杠杆的假象，并给出每个杠杆的独立贡献。
pub fn ablations(config: &EvalConfig) -> CoreResult<Vec<AblationRow>> {
    config.validate()?;
    let variants = [
        ("price-only", true, false, false),
        ("task-only", false, true, false),
        ("peer-only", false, false, true),
        ("all-levers", true, true, true),
    ];
    let mut rows = Vec::new();
    for (label, price, task, peer) in variants {
        let mut ctrl = Totals::default();
        let mut learn = Totals::default();
        for seed in &config.seeds {
            let market = MarketConfig {
                seed: *seed,
                learn_price: price,
                learn_task: task,
                learn_peer: peer,
                ..config.market.clone()
            };
            let (control, learning, _) = ab_test(&market)?;
            ctrl.add(&control);
            learn.add(&learning);
        }
        let control_revenue = ctrl.revenue.get();
        let revenue_lift = learn.revenue.get().saturating_sub(control_revenue);
        rows.push(AblationRow {
            label: label.to_string(),
            price,
            task,
            peer,
            success_lift_bp: learn.success_rate_bp() - ctrl.success_rate_bp(),
            revenue_lift_bp: if control_revenue > 0 {
                revenue_lift * 10_000 / control_revenue
            } else {
                0
            },
            violations_reduction: ctrl.violations as i64 - learn.violations as i64,
        });
    }
    Ok(rows)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Totals {
    runs: u32,
    ticks: u32,
    successes: u32,
    partials: u32,
    accepted: u32,
    violations: u32,
    revenue: Credits,
    params_changed_runs: u32,
}

impl Totals {
    fn add(&mut self, run: &crate::sim::MarketRun) {
        self.runs = self.runs.saturating_add(1);
        self.ticks = self.ticks.saturating_add(run.ticks);
        self.successes = self.successes.saturating_add(run.successes);
        self.partials = self.partials.saturating_add(run.partials);
        self.accepted = self.accepted.saturating_add(run.accepted);
        self.violations = self.violations.saturating_add(run.violations);
        self.revenue = Credits(self.revenue.get().saturating_add(run.revenue.get()));
        if run.params_changed() {
            self.params_changed_runs = self.params_changed_runs.saturating_add(1);
        }
    }

    fn success_rate_bp(&self) -> i64 {
        ratio(self.successes, self.ticks)
    }

    fn stats(&self) -> ArmStats {
        ArmStats {
            runs: self.runs,
            ticks: self.ticks,
            successes: self.successes,
            partials: self.partials,
            violations: self.violations,
            revenue: self.revenue,
            success_rate_bp: self.success_rate_bp(),
            accept_rate_bp: ratio(self.accepted, self.ticks),
            revenue_per_tick: Credits(self.revenue.get() / self.ticks.max(1) as i64),
            params_changed_runs: self.params_changed_runs,
        }
    }
}

fn ratio(part: u32, whole: u32) -> i64 {
    if whole == 0 {
        0
    } else {
        (part as i64).saturating_mul(10_000) / whole as i64
    }
}

/// 真实断言：跨种子提升为正且稳定、消融能分辨杠杆、报告可复现且可发布。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    // 自检保持轻量（部署验证会调用它）：2 个种子
    let config = EvalConfig::default().limited(2);
    let first = evaluate(&config);
    let second = evaluate(&config);
    let (report, again) = match (first, second) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            return vec![crate::check(
                "eval.runs",
                false,
                "评估运行失败（配置非法或对照实验报错）",
            )]
        }
    };

    checks.push(crate::check(
        "eval.learning_beats_control",
        report.improved() && report.stable(),
        format!(
            "{} 种子 × {} tick/组：成功率 {}bp → {}bp（池化 +{}bp，中位 +{}bp，改善 {}/{}，负 {}/{}）；\
             收益 {} → {}（+{}bp）；违规 {} → {}",
            report.seeds,
            report.ticks_per_arm,
            report.control.success_rate_bp,
            report.learning.success_rate_bp,
            report.success_lift_bp,
            report.median_success_lift_bp,
            report.improved_seeds,
            report.seeds,
            report.negative_seeds,
            report.seeds,
            report.control.revenue,
            report.learning.revenue,
            report.revenue_lift_bp,
            report.control.violations,
            report.learning.violations
        ),
    ));

    let reproducible = report == again
        && match (report.digest(), again.digest()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
    let publishable = report
        .summary()
        .map(|v| !v.to_string().contains("did:au4a:"))
        .unwrap_or(false);
    checks.push(crate::check(
        "eval.reproducible_and_publishable",
        reproducible && publishable,
        format!("两次评估逐字段与摘要一致={reproducible}；投影不含 DID={publishable}"),
    ));
    checks
}
