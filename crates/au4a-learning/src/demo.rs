//! v1.6.9 示例（Demo）：一个 Agent 跑完整学习循环，输出**可核对**的 JSON 报告。
//!
//! 报告逐段对应学习循环的五个阶段，每段都带真实数字（不是「已学习」这种自述）：
//!
//! ```text
//! 1 经验收集 → 2 反馈分析 → 3 学习信号 → 4 行为调整 → 5 效果评估
//!                                                    ↘ 6 模型更新（代际落地）
//! ```
//!
//! 本模块是**纯逻辑**（不读环境变量、不打印、不读文件）：命令行的参数解析与 stdout 都在
//! `examples/learning_loop.rs` 里，`scenario`/`self_check` 也复用同一个函数，
//! 于是「示例跑出来的东西」与「测试断言的东西」是同一份实现。

use au4a_core::{AgentKeys, CoreResult, Credits, Did};
use serde_json::{json, Value};

use crate::experience::{Experience, ExperienceStore, Outcome};
use crate::explain::explain;
use crate::feedback::FeedbackAnalyser;
use crate::model::LearningModel;
use crate::policy::{adjust, PolicyBounds, PolicyParams, PolicyTargets};
use crate::rng::hash64;
use crate::signal::{LearningSignal, SignalWeights};
use crate::sim::{ab_test, MarketConfig, TASK_PROFILES};
use crate::violation::ViolationLog;

/// 示例默认种子（`examples/learning_loop.rs` 不传参时用它）。
pub const DEMO_SEED: u64 = 0x16_0909;

/// 示例里收集的经验条数。
pub const DEMO_EXPERIENCES: u64 = 18;

/// 示例里把同一次调整落地多少代（展示动量与遗忘的时间尺度）。
pub const DEMO_GENERATIONS: u32 = 3;

/// 示例里生成的报价历史条数（定价学习的最小输入）。
pub const DEMO_QUOTES: u64 = 24;

/// 跑一遍完整学习循环，返回结构化报告。
pub fn demo_report(seed: u64) -> CoreResult<Value> {
    // ---- 1) 经验收集：Agent 自己记下 18 次任务经历 ----
    let peers: Vec<Did> = (0..3u8)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect();
    let mut store = ExperienceStore::new(32)?;
    for i in 0..DEMO_EXPERIENCES {
        let profile = &TASK_PROFILES[(hash64(seed, &[i]) % TASK_PROFILES.len() as u64) as usize];
        let roll = hash64(seed, &[i, 7]) % 100;
        let outcome = if roll < 50 {
            Outcome::Success
        } else if roll < 75 {
            Outcome::Partial
        } else {
            Outcome::Failure
        };
        let reward = match outcome {
            Outcome::Success => profile.base_price,
            Outcome::Partial => Credits(profile.base_price.get() / 2),
            Outcome::Failure => Credits::ZERO,
        };
        let partner = 1 + (hash64(seed, &[i, 11]) % 2) as usize;
        let context = format!("示例上下文 #{i}：{} 任务", profile.name);
        store.record(Experience::new(
            &format!("demo-{i:03}"),
            profile.name,
            &context,
            if outcome.is_success() {
                "deliver"
            } else {
                "retry-planned"
            },
            outcome,
            reward,
            i,
            &peers[..partner],
        )?)?;
    }

    // ---- 2) 反馈分析 ----
    let report = FeedbackAnalyser::analyse(&store)?;

    // ---- 3) 学习信号（质量 / 结算 / 信誉 / 违规）----
    let violations = ViolationLog::new();
    let signal = LearningSignal::from_store(&store, &violations, 0, &SignalWeights::default())?;

    // ---- 4) 行为调整 ----
    let bounds = PolicyBounds::default();
    let targets = PolicyTargets::default();
    let baseline = PolicyParams::baseline();
    // 报价历史：示例用市场公开的需求曲线 + 种子散列生成 24 次报价结果（可复现的真实数据，
    // 不是「假定接受率」）。没有报价历史时定价会停手，所以这一步是必要的输入。
    let (quoted, accepted) = quote_history(seed, baseline.price_bp);
    let accept_rate_bp = accepted * 10_000 / quoted.max(1);
    let signals = signal.to_signals(
        store.len(),
        Some(accept_rate_bp),
        report.overall.mean_reward,
        Credits::ZERO,
        0,
    );
    let adjustment = adjust(&baseline, &report, &violations, &signals, &bounds, &targets)?;

    // ---- 5) 模型更新：把同一次调整按代际落地（动量/遗忘/漂移钳制）----
    let mut model = LearningModel::new(bounds.clone());
    let mut timeline = Vec::new();
    for generation in 0..DEMO_GENERATIONS {
        let record = model.apply(&adjustment)?;
        timeline.push(json!({
            "generation": record.generation,
            "price_before_bp": record.price_before_bp,
            "price_after_bp": record.price_after_bp,
            "price_drift_bp": record.price_drift_bp,
            "momentum_bp": record.momentum_bp,
            "damped": record.damped,
            "clamped": record.clamped,
            "forgotten_entries": record.forgotten_entries,
            "note": if generation == 0 { "第一步被动量压小" } else { "逐步逼近满步长" },
        }));
    }

    // ---- 6) 效果评估：同种子的学习组 vs 对照组 ----
    let config = MarketConfig {
        seed,
        ..MarketConfig::default()
    };
    let (control, learning, comparison) = ab_test(&config)?;

    // ---- 可解释性：为什么参数是这个值（可直接对外发布）----
    let explanation = explain(
        model.params(),
        &report,
        &violations,
        &signals,
        &bounds,
        &targets,
    )?;

    Ok(json!({
        "demo": "track-1.6 individual learning loop",
        "seed": seed,
        "steps": [
            {
                "step": "1 经验收集",
                "evidence": format!("记录 {} 条经验（成功 {}），内容摘要 {}", store.len(), report.overall.success_bp, store.digest()?),
                "experiences": store.len(),
                "success_bp": report.overall.success_bp,
            },
            {
                "step": "2 反馈分析",
                "evidence": format!("整体质量 {}bp、平均收益 {}；任务类型 {} 个、协作者 {} 个（对外只发布计数）",
                    report.overall.quality_bp, report.overall.mean_reward, report.per_type.len(), report.per_peer.len()),
                "by_type": report.per_type.len(),
                "by_peer": report.per_peer.len(),
            },
            {
                "step": "3 学习信号",
                "evidence": format!("质量 {}bp、结算 {}、信誉 {}、违规 {} → 综合 {}bp",
                    signal.quality_bp, signal.settled, signal.reputation_delta, signal.violations, signal.composite_bp),
                "composite_bp": signal.composite_bp,
            },
            {
                "step": "4 行为调整",
                "evidence": format!("报价历史 {quoted} 次、接受率 {accept_rate_bp}bp；定价位移 {}bp、任务偏好变动 {} 项、协作者偏好变动 {} 项；理由 {} 条",
                    adjustment.price_moved_bp, adjustment.task_bias_moved_bp.len(),
                    adjustment.peer_bias_moved_bp.len(), adjustment.reasons.len()),
                "changed": adjustment.changed,
                "price_moved_bp": adjustment.price_moved_bp,
                "quote_history": { "quoted": quoted, "accepted": accepted, "accept_rate_bp": accept_rate_bp },
                "reasons": adjustment.public_json()?,
            },
            {
                "step": "5 模型更新",
                "evidence": format!("{} 代：定价 {}bp → {}bp", model.generation(),
                    baseline.price_bp, model.params().price_bp),
                "timeline": timeline,
            },
            {
                "step": "6 效果评估",
                "evidence": format!("成功率 {}bp → {}bp（+{}bp）；收益 {} → {}（+{}bp）；违规 {} → {}",
                    comparison.control_success_bp, comparison.learning_success_bp, comparison.success_lift_bp,
                    comparison.control_revenue, comparison.learning_revenue, comparison.revenue_lift_bp,
                    comparison.control_violations, comparison.learning_violations),
                "improved": comparison.improved(),
                "comparison": comparison.to_value()?,
            },
        ],
        "policy_before": baseline.public_json()?,
        "policy_after": model.params().public_json()?,
        "explanation": explanation.to_value()?,
        "market": {
            "control": control.public_json()?,
            "learning": learning.public_json()?,
        },
    }))
}

/// 报价历史：用公开的需求曲线 `accept_rate_curve_bp` 与种子散列生成 `DEMO_QUOTES` 次报价结果。
///
/// 这是一个**真实的小流程**（与市场里同一套规则），而不是「假设接受率是某个值」：
/// 同样的种子与价格永远得到同样的 (报价数, 接受数)。
fn quote_history(seed: u64, price_bp: i64) -> (i64, i64) {
    let curve = crate::sim::accept_rate_curve_bp(price_bp).max(0) as u64;
    let mut accepted = 0i64;
    for i in 0..DEMO_QUOTES {
        if hash64(seed, &[i, 0x99]) % 10_000 < curve {
            accepted += 1;
        }
    }
    (DEMO_QUOTES as i64, accepted)
}

/// 报告里是否所有对外发布部分都不含 DID（示例的自我审查，也在测试里断言）。
pub fn demo_report_is_publishable(report: &Value) -> bool {
    !report.to_string().contains("did:au4a:")
}
