//! 端到端自有流程（`scenario`）：Agent 自主注册 → 收集自己的经验 → 返回可核对摘要。
//!
//! 这一版（v1.6.1）做的是「经验可记录、可寻址、可重放」：同样的种子跑两次，
//! 规范 JSON 必须逐字节相同。后续小版本在同一入口上继续叠加反馈、行为调整与对照评估。

use au4a_core::{canonicalize, AgentKeys, CoreError, CoreResult, Credits, Did, SelfCheck};
use au4a_kernel::{Kernel, KernelConfig};
use serde_json::{json, Value};

use crate::experience::{Experience, ExperienceStore, Outcome};
use crate::rng::hash64;
use crate::TRACK;

/// 场景种子：固定值，保证端到端演示与部署验证可复现。
pub const SCENARIO_SEED: u64 = 0x1616_1616;

/// 场景里的任务类型（与 spec 的「任务类型维度」对应）。
pub const TASK_TYPES: [&str; 3] = ["translate.en-zh", "summarize.zh", "classify.zh"];

/// 场景里收集的经验条数。
pub const SCENARIO_TASKS: u64 = 24;

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

/// 跑一遍本轨道的端到端流程，返回 JSON 摘要。
pub fn run(kernel: &mut Kernel) -> CoreResult<Value> {
    let learner_keys = AgentKeys::from_seed(&[0x16; 32]);
    let learner = ensure_agent(
        kernel,
        &learner_keys,
        "learner-16",
        &["learn.local", "learn.evaluate"],
    )?;
    let peer_dids: Vec<Did> = (0..3u8)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect();

    let mut store = ExperienceStore::new(64)?;
    let mut successes = 0u32;
    let mut settled = Credits::ZERO;
    for i in 0..SCENARIO_TASKS {
        let ty = TASK_TYPES[(hash64(SCENARIO_SEED, &[i]) % TASK_TYPES.len() as u64) as usize];
        let roll = hash64(SCENARIO_SEED, &[i, 7]) % 100;
        let outcome = if roll < 55 {
            Outcome::Success
        } else if roll < 80 {
            Outcome::Partial
        } else {
            Outcome::Failure
        };
        let partner_count = 1 + (hash64(SCENARIO_SEED, &[i, 11]) % 2) as usize;
        let peers = &peer_dids[..partner_count];
        let reward = match outcome {
            Outcome::Success => 40,
            Outcome::Partial => 20,
            Outcome::Failure => 0,
        };
        let exp = Experience::new(
            &format!("task-{i:04}"),
            ty,
            &format!("场景上下文 #{i}：{ty} 任务，参与者 {partner_count} 个"),
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
        settled = settled.checked_add(Credits(reward))?;
    }

    kernel.emit(
        &format!("{TRACK}.experience.collect"),
        format!(
            "经验收集：{} 条，成功 {successes} 条，账面收益 {settled} 微积分",
            store.len()
        ),
    );

    let stats = serde_json::to_value(store.stats()).map_err(|_| CoreError::Encoding)?;
    Ok(json!({
        "track": TRACK,
        "version": crate::VERSION,
        "learner": learner.as_str(),
        "agents": kernel.agent_count(),
        "experiences": store.len(),
        "successes": successes,
        "reward_total": settled.get(),
        "digest": store.digest()?,
        "store": stats,
    }))
}

/// 场景自检：两次独立运行必须得到**逐字节相同**的摘要，且账本守恒。
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
                "场景运行返回错误（注册/记录失败）",
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
    let registered = first_kernel.agent_count() == 1;
    let passed = same_bytes && experiences == SCENARIO_TASKS && conserved && registered;
    checks.push(crate::check(
        "scenario.end_to_end",
        passed,
        format!(
            "两次独立运行规范 JSON 逐字节相同={same_bytes}；经验 {experiences}/{SCENARIO_TASKS} 条；\
             注册 Agent={registered}；账本守恒={conserved}"
        ),
    ));
    checks
}
