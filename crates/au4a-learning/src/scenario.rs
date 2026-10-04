//! 端到端自有流程（`scenario`）：自主注册 → 收集经验 → 反馈分析 → 真实结算 → 返回可核对摘要。
//!
//! 已实现的能力每一版都会叠加在这里，而不是只多打印一行字：
//!
//! * v1.6.1：经验可记录、可寻址、可重放（同种子两次运行规范 JSON 逐字节相同）。
//! * v1.6.2：经验聚合成按任务类型 / 按协作者的反馈统计；成功经验用**共享内核的账本**
//!   真实结算（`EvidenceGrade::CpuProto`，金额受 `cpu_proto_settle_cap` 限制）。
//!   结算失败不会被静默吞掉：计入 `settlement_refused` 并保留第一条原因。

use au4a_core::{
    canonicalize, AgentKeys, CoreError, CoreResult, Credits, Did, EvidenceGrade, SelfCheck,
};
use au4a_kernel::{Kernel, KernelConfig};
use serde_json::{json, Value};

use crate::experience::{Experience, ExperienceStore, Outcome};
use crate::feedback::FeedbackAnalyser;
use crate::policy::{adjust, PolicyBounds, PolicyParams, PolicyTargets, Signals};
use crate::privacy::{publish, seal, PrivacyPolicy};
use crate::rng::hash64;
use crate::signal::{LearningSignal, SignalWeights};
use crate::sim::{ab_test, MarketConfig};
use crate::violation::ViolationLog;
use crate::TRACK;

/// 场景种子：固定值，保证端到端演示与部署验证可复现。
pub const SCENARIO_SEED: u64 = 0x1616_1616;

/// 场景里的任务类型（与 spec 的「任务类型维度」对应）。
pub const TASK_TYPES: [&str; 3] = ["translate.en-zh", "summarize.zh", "classify.zh"];

/// 场景里收集的经验条数。
pub const SCENARIO_TASKS: u64 = 24;

/// 场景里的协作者数量。
pub const SCENARIO_PEERS: u8 = 3;

/// 幂等注册：已注册则直接复用（`scenario` 可能在同一内核上被演示与验证各调一次）。
pub(crate) fn ensure_agent(
    kernel: &mut Kernel,
    keys: &AgentKeys,
    display: &str,
    skills: &[&str],
) -> CoreResult<Did> {
    let did = keys.did();
    if kernel.card(&did).is_some() {
        return Ok(did);
    }
    let min_stake = kernel.config().min_stake;
    let stake = if min_stake > Credits::ZERO {
        min_stake
    } else {
        Credits(1)
    };
    match kernel.register(keys, display, skills, stake) {
        Ok(card) => Ok(card.did),
        Err(CoreError::DuplicateAgent) => Ok(did),
        Err(e) => Err(e),
    }
}

/// 学习者密钥（固定种子 → 固定 DID）。
pub fn learner_keys() -> AgentKeys {
    AgentKeys::from_seed(&[0x16; 32])
}

/// 协作者 DID（固定种子集合）。
pub fn peer_dids() -> Vec<Did> {
    (0..SCENARIO_PEERS)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect()
}

/// 场景里第 `i` 个任务的结局（无状态散列 → 与调用顺序无关，可复现）。
fn outcome_of(i: u64) -> Outcome {
    let roll = hash64(SCENARIO_SEED, &[i, 7]) % 100;
    if roll < 55 {
        Outcome::Success
    } else if roll < 80 {
        Outcome::Partial
    } else {
        Outcome::Failure
    }
}

/// 跑一遍本轨道的端到端流程，返回 JSON 摘要。
pub fn run(kernel: &mut Kernel) -> CoreResult<Value> {
    let learner = ensure_agent(
        kernel,
        &learner_keys(),
        "learner-16",
        &["learn.local", "learn.evaluate"],
    )?;
    let peer_dids = peer_dids();
    for (i, peer_keys) in (0..SCENARIO_PEERS)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]))
        .enumerate()
    {
        ensure_agent(
            kernel,
            &peer_keys,
            &format!("peer-16-{i}"),
            &["translate.en-zh", "summarize.zh", "classify.zh"],
        )?;
    }

    let mut store = ExperienceStore::new(64)?;
    let mut successes = 0u32;
    let mut reward_total = Credits::ZERO;
    for i in 0..SCENARIO_TASKS {
        let task_type = TASK_TYPES[(hash64(SCENARIO_SEED, &[i]) % TASK_TYPES.len() as u64) as usize];
        let outcome = outcome_of(i);
        let partner_count = 1 + (hash64(SCENARIO_SEED, &[i, 11]) % 2) as usize;
        let peers = &peer_dids[..partner_count];
        let reward = match outcome {
            Outcome::Success => 40,
            Outcome::Partial => 20,
            Outcome::Failure => 0,
        };
        let context = format!("场景上下文 #{i}：{task_type} 任务，参与者 {partner_count} 个");
        let exp = Experience::new(
            &format!("task-{i:04}"),
            task_type,
            &context,
            "quote-then-deliver",
            outcome,
            Credits(reward),
            i,
            peers,
        )?;
        store.record(exp)?;
        if outcome.is_success() {
            successes += 1;
        }
        reward_total = reward_total.checked_add(Credits(reward))?;
    }

    kernel.emit(
        &format!("{TRACK}.experience.collect"),
        format!(
            "经验收集：{} 条，成功 {successes} 条，账面收益 {reward_total} 微积分",
            store.len()
        ),
    );

    // v1.6.2：反馈分析（按任务类型 / 按协作者）
    let report = FeedbackAnalyser::analyse(&store)?;
    kernel.emit(
        &format!("{TRACK}.feedback.analyse"),
        format!(
            "反馈分析：overall 成功率 {}bp、平均质量 {}bp、平均收益 {}，样本充分={}；任务类型 {} 个、协作者 {} 个",
            report.overall.success_bp,
            report.overall.quality_bp,
            report.overall.mean_reward,
            report.overall.sufficient,
            report.per_type.len(),
            report.per_peer.len()
        ),
    );

    // v1.6.2：成功经验走共享内核账本真实结算。
    // 金额**不在这里预钳制**：证据闸门（`EvidenceGrade::CpuProto` 的 settle_cap）是内核的职责，
    // 学习轨道只负责如实记录「有几笔被拒、第一笔为什么被拒」。
    let cap = kernel.config().cpu_proto_settle_cap;
    let mut settled_total = Credits::ZERO;
    let mut settlement_refused = 0u32;
    let mut first_refusal: Option<String> = None;
    for exp in store.entries() {
        if !exp.is_success() {
            continue;
        }
        // 没有参与者的成功经验没有结算对象；跳过而不是当作失败。
        let Some(to) = exp.peer_agents.first() else {
            continue;
        };
        if exp.reward == Credits::ZERO {
            continue;
        }
        match kernel.settle(&learner, to, exp.reward, EvidenceGrade::CpuProto) {
            Ok(()) => settled_total = settled_total.checked_add(exp.reward)?,
            Err(e) => {
                settlement_refused = settlement_refused.saturating_add(1);
                if first_refusal.is_none() {
                    first_refusal = Some(format!("{e:?}"));
                }
            }
        }
    }
    if settled_total > Credits::ZERO {
        kernel.emit(
            &format!("{TRACK}.settlement"),
            format!(
                "结算：{settled_total} 微积分（grade=cpu-proto，cap={cap}），被拒 {settlement_refused} 笔"
            ),
        );
    }

    let stats = serde_json::to_value(store.stats()).map_err(|_| CoreError::Encoding)?;

    // v1.6.6：本地加密视图 + 对外公开视图（脱敏、小样本抑制）
    let privacy_policy = PrivacyPolicy::default();
    let view = publish(&store, &privacy_policy)?;
    let sealed = seal(&store, &AgentKeys::from_seed(&[0x16; 32]).seed())?;
    kernel.emit(
        &format!("{TRACK}.privacy.publish"),
        format!(
            "隐私：公开视图 {} 个聚合（抑制 {} 个）、本地加密视图 {} 字节；协作者只以计数出现",
            view.aggregates.len(),
            view.suppressed_groups,
            sealed.plaintext_len
        ),
    );

    // v1.6.4：四类学习信号（完成质量 / 结算金额 / 信誉变化 / 违规记录）折算成统一向量
    let violations = ViolationLog::new();
    let signal = LearningSignal::from_store(&store, &violations, 0, &SignalWeights::default())?;
    kernel.emit(
        &format!("{TRACK}.signal.aggregate"),
        format!(
            "学习信号：质量 {}bp、结算 {} 微积分、信誉 {}、违规 {} → 综合 {}bp",
            signal.quality_bp,
            signal.settled,
            signal.reputation_delta,
            signal.violations,
            signal.composite_bp
        ),
    );

    // v1.6.3：用这 24 条成功/失败经验做一次真实的行为调整（策略参数必须真的变）
    let baseline = PolicyParams::baseline();
    let signals = Signals::cold_start(
        store.len(),
        report.overall.quality_bp,
        report.overall.mean_reward,
    );
    let adjustment = adjust(
        &baseline,
        &report,
        &ViolationLog::new(),
        &signals,
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )?;
    if adjustment.changed {
        kernel.emit(
            &format!("{TRACK}.policy.adjust"),
            format!(
                "行为调整：定价移动 {:?}bp 至 {}bp、任务偏好 {} 项变动、协作者偏好 {} 项变动",
                adjustment.price_moved_bp,
                adjustment.next.price_bp,
                adjustment.task_bias_moved_bp.len(),
                adjustment.peer_bias_moved_bp.len()
            ),
        );
    }

    // v1.6.3：学习组 vs 对照组（同一市场、同一种子，唯一差别是是否更新策略参数）
    let (control, learning, comparison) = ab_test(&MarketConfig::default())?;
    kernel.emit(
        &format!("{TRACK}.market.ab_test"),
        format!(
            "对照实验：成功率 {}bp→{}bp（+{}bp），收益 {}→{} 微积分（+{}bp），违规 {}→{}",
            comparison.control_success_bp,
            comparison.learning_success_bp,
            comparison.success_lift_bp,
            comparison.control_revenue,
            comparison.learning_revenue,
            comparison.revenue_lift_bp,
            comparison.control_violations,
            comparison.learning_violations
        ),
    );

    Ok(json!({
        "track": TRACK,
        "version": crate::VERSION,
        "learner": learner.as_str(),
        "agents": kernel.agent_count(),
        "experiences": store.len(),
        "successes": successes,
        "reward_total": reward_total.get(),
        "settled_total": settled_total.get(),
        "settlement_refused": settlement_refused,
        "settlement_first_refusal": first_refusal,
        "digest": store.digest()?,
        "store": stats,
        "feedback": report.public_json()?,
        "learning_signal": signal.to_value()?,
        "privacy": {
            "policy_tag": view.policy_tag,
            "view": view.to_value()?,
            "sealed_bytes": sealed.plaintext_len,
        },
        "ledger_conserved": kernel.ledger().check_conservation().is_ok(),
        "policy_adjustment": adjustment.public_json()?,
        "policy_before": baseline.public_json()?,
        "policy_after": adjustment.next.public_json()?,
        "market": {
            "control": control.public_json()?,
            "learning": learning.public_json()?,
            "comparison": comparison.to_value()?,
            "improved": comparison.improved(),
        },
    }))
}

/// 场景自检：两次独立运行必须得到**逐字节相同**的摘要，账本守恒，反馈统计与经验一致。
pub fn self_check() -> Vec<SelfCheck> {
    let mut first_kernel = Kernel::new(KernelConfig::default());
    let mut second_kernel = Kernel::new(KernelConfig::default());
    let first = run(&mut first_kernel);
    let second = run(&mut second_kernel);
    let mut checks = Vec::new();

    let (a, b) = match (first, second) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            checks.push(crate::check(
                "scenario.end_to_end",
                false,
                "场景运行返回错误（注册/记录/结算失败）",
            ));
            return checks;
        }
    };
    let same_bytes = match (canonicalize(&a), canonicalize(&b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    };
    let experiences = a.get("experiences").and_then(Value::as_u64).unwrap_or(0);
    let conserved = first_kernel.ledger().check_conservation().is_ok();
    let agents = first_kernel.agent_count();
    let expected_agents = 1 + SCENARIO_PEERS as usize;
    let passed =
        same_bytes && experiences == SCENARIO_TASKS && conserved && agents == expected_agents;
    checks.push(crate::check(
        "scenario.end_to_end",
        passed,
        format!(
            "两次独立运行规范 JSON 逐字节相同={same_bytes}；经验 {experiences}/{SCENARIO_TASKS} 条；\
             Agent {agents}/{expected_agents}（学习者+协作者）；账本守恒={conserved}"
        ),
    ));

    // 反馈统计必须与经验条数一致（不是「跑通了就算」）
    let feedback_total = a
        .get("feedback")
        .and_then(|f| f.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let settled = a
        .get("settled_total")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let successes = a.get("successes").and_then(Value::as_u64).unwrap_or(0);
    let honest = feedback_total == experiences
        && settled >= 0
        && settled <= (successes as i64) * 40
        && a.get("settlement_first_refusal").is_some();
    checks.push(crate::check(
        "scenario.feedback_matches_experience",
        honest,
        format!(
            "反馈样本 {feedback_total} == 经验 {experiences}；结算合计 {settled} 微积分 ≤ 成功数×单价；\
             结算被拒计数存在={}",
            a.get("settlement_refused").is_some()
        ),
    ));

    // v1.6.3：学习必须真的改变行为，且改善可量化（不是打印「已学习」）
    let policy_changed = a
        .get("policy_adjustment")
        .and_then(|p| p.get("changed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let market_improved = a
        .get("market")
        .and_then(|m| m.get("improved"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let lift = a
        .get("market")
        .and_then(|m| m.get("comparison"))
        .and_then(|c| c.get("success_lift_bp"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let revenue_lift = a
        .get("market")
        .and_then(|m| m.get("comparison"))
        .and_then(|c| c.get("revenue_lift_credits"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    checks.push(crate::check(
        "scenario.learning_changes_behaviour",
        policy_changed && market_improved && lift > 0 && revenue_lift > 0,
        format!(
            "策略参数改变={policy_changed}；学习组成功率提升 +{lift}bp、收益提升 +{revenue_lift} 微积分；\
             对照实验判定改善={market_improved}"
        ),
    ));
    checks
}
