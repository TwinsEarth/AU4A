//! v1.2.10 — 示例：两条端到端路径，各返回一份 JSON 摘要。
//!
//! ```text
//! 成功链路：报价 120 → 还价 100 →（拒绝）→ 还价 95 → 接受 → 双方签合约 → 执行 → 结算 95
//! 违约链路：报价 80 → 还价 75 → 接受 → 双方签合约 → 执行 → 违约申诉
//!           → 立案（两名第三方仲裁员）→ 双签裁决 → 罚没 + 赔付 → 结案
//! ```
//!
//! 两条链路都**真的跑**：消息经 PMB 投递并逐条验签、每次相位转换双方签名（或合约条款授权）、
//! 合约哈希锚定、归档规范 JSON 逐字节重放、账本守恒。没有 mock，也没有「演示专用分支」。

use au4a_core::{CoreError, CoreResult, Credits, Did, EvidenceGrade};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

use crate::journal::Journal;
use crate::msg::{BreachKind, NegotiationMsg, Terms};
use crate::rounds::{Negotiation, DEFAULT_MAX_ROUNDS};

/// 成功链路沿用 `scenario_agents` 的那一对确定性 Agent（种子在 `lib.rs` 里统一定义）。
/// 违约链路用自己的四个 Agent（两名当事人 + 两名仲裁员）。
const BREACH_SEEDS: ([u8; 32], [u8; 32], [u8; 32], [u8; 32]) =
    ([0x56; 32], [0x78; 32], [0x9A; 32], [0xBC; 32]);

fn agent(seed: &[u8; 32]) -> au4a_core::AgentKeys {
    au4a_core::AgentKeys::from_seed(seed)
}

fn terms(price: i64, deadline: u64) -> CoreResult<Terms> {
    Terms::new(
        "summarize.zh",
        Credits(price),
        deadline,
        EvidenceGrade::Verified,
    )
}

/// 把内核队列里的消息收成一份可断言的转录。
fn transcript(kernel: &mut Kernel) -> CoreResult<Vec<Value>> {
    let mut rows = Vec::new();
    for env in kernel.drain() {
        let parsed = NegotiationMsg::from_env(&env)?;
        rows.push(json!({
            "kind": parsed.kind(),
            "id": env.id,
            "from": env.from.as_str(),
            "ts": env.ts,
            "in_reply_to": env.in_reply_to,
        }));
    }
    Ok(rows)
}

/// 成功链路：多轮还价 → 达成 → 双方签合约 → 执行 → 结算。
pub fn success_path(kernel: &mut Kernel) -> CoreResult<Value> {
    let (proposer, responder) = crate::scenario_agents(kernel)?;

    let mut negotiation = Negotiation::open(
        kernel,
        &proposer,
        &responder,
        terms(120, 40)?,
        DEFAULT_MAX_ROUNDS,
    )?;
    negotiation.counter(kernel, &responder, &proposer, terms(100, 40)?)?;
    negotiation.reject(kernel, &proposer, &responder, "deadline too tight")?;
    negotiation.counter(kernel, &proposer, &responder, terms(95, 42)?)?;
    negotiation.accept(kernel, &responder, &proposer)?;
    let contract = negotiation.sign_contract(kernel, &proposer, &responder)?;
    negotiation.execute(kernel, &proposer, &responder)?;
    let paid = negotiation.settle(kernel, &proposer, &responder)?;

    let archived = negotiation.archive()?;
    let replayed = Journal::decode(&archived)?;
    let transcript = transcript(kernel)?;
    let summary = negotiation.summary()?;
    kernel.emit(
        "1.2.example.success",
        format!(
            "成功链路：{} 条消息，{} → {}，成交 {}，双签合约 {} 已锚定",
            transcript.len(),
            crate::state::Phase::Idle.as_str(),
            negotiation.phase().as_str(),
            paid.0,
            au4a_core::short_id(&contract.hash)
        ),
    );

    Ok(json!({
        "path": "success",
        "session": negotiation.session(),
        "phase": negotiation.phase().as_str(),
        "steps": 8,
        "delivered": transcript.len(),
        "transcript": transcript,
        "rounds_used": negotiation.rounds_used(),
        "max_rounds": negotiation.max_rounds(),
        "offers": negotiation.offers().len(),
        "rejections": negotiation.rejections().len(),
        "price_trail": negotiation.price_trail(),
        "transitions": negotiation.machine().seq(),
        "history_tip": summary["history_tip"],
        "contract": contract.summary(),
        "contract_anchored": contract.verify_anchor().is_ok(),
        "settled_amount": paid.0,
        "journal_bytes": archived.len(),
        "replay_byte_exact": replayed.encode()? == archived,
        "replay_digest": negotiation.journal().replay_digest()?,
        "conservation_ok": kernel.ledger().check_conservation().is_ok(),
    }))
}

/// 违约链路：达成并执行后违约 → 仲裁 → 双签裁决 → 罚没 + 赔付 → 结案。
pub fn breach_path(kernel: &mut Kernel) -> CoreResult<Value> {
    let client = agent(&BREACH_SEEDS.0);
    let provider = agent(&BREACH_SEEDS.1);
    let arbiter1 = agent(&BREACH_SEEDS.2);
    let arbiter2 = agent(&BREACH_SEEDS.3);
    crate::ensure_agent(kernel, &client, "negotiate.client", &["summarize.zh"])?;
    crate::ensure_agent(kernel, &provider, "negotiate.provider", &["summarize.zh"])?;
    crate::ensure_agent(kernel, &arbiter1, "negotiate.arbiter.1", &[])?;
    crate::ensure_agent(kernel, &arbiter2, "negotiate.arbiter.2", &[])?;

    let mut negotiation = Negotiation::open(
        kernel,
        &client,
        &provider,
        terms(80, 40)?,
        DEFAULT_MAX_ROUNDS,
    )?;
    negotiation.counter(kernel, &provider, &client, terms(75, 40)?)?;
    negotiation.accept(kernel, &client, &provider)?;
    negotiation.sign_contract(kernel, &client, &provider)?;
    negotiation.execute(kernel, &client, &provider)?;
    let claim = negotiation.report_breach(
        kernel,
        &client,
        BreachKind::NonDelivery,
        EvidenceGrade::Verified,
        "nothing was delivered after the deadline",
    )?;
    if negotiation.phase() != crate::state::Phase::Arbitration {
        return Err(CoreError::InvalidKind);
    }

    let case = negotiation.open_case(&[arbiter1.did(), arbiter2.did()], kernel.tick())?;
    let price = negotiation
        .contract()
        .ok_or(CoreError::NotSealed)?
        .terms
        .price;
    let ruling = {
        let case = negotiation.case_mut().ok_or(CoreError::NotSealed)?;
        case.rule(
            &crate::arbitration::ArbitrationPolicy::default(),
            &[&arbiter1, &arbiter2],
            price,
            "breach claim verified by both arbiters",
            kernel.tick(),
        )?
    };
    let at = kernel.tick();
    let enforcement = negotiation
        .case_mut()
        .ok_or(CoreError::NotSealed)?
        .enforce(kernel, at)?;
    negotiation.resolve(kernel, &provider, &client)?;

    let archived = negotiation.archive()?;
    let replayed = Journal::decode(&archived)?;
    let transcript = transcript(kernel)?;
    let summary = negotiation.summary()?;
    kernel.emit(
        "1.2.example.breach",
        format!(
            "违约链路：案件 {} 裁决={} 罚没={} 赔付={}，最终相位 {}",
            case.short_id(),
            ruling.verdict.as_str(),
            enforcement.slashed.0,
            enforcement.compensated.0,
            negotiation.phase().as_str()
        ),
    );

    Ok(json!({
        "path": "breach",
        "session": negotiation.session(),
        "phase": negotiation.phase().as_str(),
        "steps": 8,
        "delivered": transcript.len(),
        "transcript": transcript,
        "rounds_used": negotiation.rounds_used(),
        "offers": negotiation.offers().len(),
        "price_trail": negotiation.price_trail(),
        "transitions": negotiation.machine().seq(),
        "history_tip": summary["history_tip"],
        "contract": negotiation.contract().map(|c| c.summary()),
        "claim": claim.summary(),
        "case_id": case.case_id(),
        "case_short_id": case.short_id(),
        "arbiters": case.arbiters().len(),
        "ruling": ruling.summary(),
        "enforcement": enforcement.summary(),
        "conservation_ok": enforcement.conservation_ok && kernel.ledger().check_conservation().is_ok(),
        "journal_bytes": archived.len(),
        "replay_byte_exact": replayed.encode()? == archived,
        "replay_digest": negotiation.journal().replay_digest()?,
    }))
}

/// 两条链路依次跑一遍，返回两份 JSON 摘要。
///
/// 顺序固定（成功在前），因此同一个内核上重复调用只有逻辑时钟推进带来的 id/ts 变化，
/// 结构与数值完全一致——这正是 `scenario` 可重复性的来源。
pub fn run(kernel: &mut Kernel) -> CoreResult<Value> {
    let success = success_path(kernel)?;
    let breach = breach_path(kernel)?;
    Ok(json!({
        "success": success,
        "breach": breach,
        "conservation_ok": kernel.ledger().check_conservation().is_ok(),
    }))
}

/// 供 `scenario` 与测试复用：两条链路的确定性 DID。
pub fn example_dids() -> [Did; 4] {
    [
        agent(&BREACH_SEEDS.0).did(),
        agent(&BREACH_SEEDS.1).did(),
        agent(&BREACH_SEEDS.2).did(),
        agent(&BREACH_SEEDS.3).did(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_kernel::KernelConfig;

    fn kernel() -> Kernel {
        Kernel::new(KernelConfig::default())
    }

    #[test]
    fn success_path_settles_and_replays_byte_exactly() {
        let mut k = kernel();
        let summary = success_path(&mut k).unwrap();
        assert_eq!(summary["phase"], "settled");
        assert_eq!(summary["settled_amount"], 95);
        assert_eq!(summary["price_trail"], json!([120, 100, 95]));
        assert_eq!(summary["rounds_used"], 2);
        assert_eq!(summary["transitions"], 8);
        assert_eq!(summary["contract"]["dual_signed"], true);
        assert_eq!(summary["contract_anchored"], true);
        assert_eq!(summary["replay_byte_exact"], true);
        assert_eq!(summary["conservation_ok"], true);
        assert_eq!(summary["delivered"], 7);
    }

    #[test]
    fn breach_path_goes_through_arbitration_and_closes_settled() {
        let mut k = kernel();
        let summary = breach_path(&mut k).unwrap();
        assert_eq!(summary["phase"], "settled", "裁决执行后由双方联署结案");
        assert_eq!(summary["claim"]["kind"], "non_delivery");
        assert_eq!(summary["claim"]["evidence"], "verified");
        assert_eq!(summary["arbiters"], 2);
        assert_eq!(summary["ruling"]["verdict"], "upheld");
        assert_eq!(summary["ruling"]["signatures"], 2);
        assert_eq!(summary["ruling"]["slash"], 15, "75 的 2000bp");
        assert_eq!(
            summary["ruling"]["compensate"], 37,
            "75 的 5000bp（向下取整）"
        );
        assert_eq!(summary["enforcement"]["slashed"], 15);
        assert_eq!(summary["enforcement"]["compensated"], 37);
        assert_eq!(summary["enforcement"]["conservation_ok"], true);
        assert_eq!(summary["case_id"].as_str().unwrap().len(), 64);
        assert_eq!(summary["replay_byte_exact"], true);
    }

    #[test]
    fn the_two_paths_use_different_agents_and_sessions() {
        let mut k = kernel();
        let both = run(&mut k).unwrap();
        assert_ne!(both["success"]["session"], both["breach"]["session"]);
        let dids = example_dids();
        assert_eq!(dids.len(), 4);
        assert_ne!(dids[0], dids[1]);
        assert_ne!(dids[2], dids[3]);
        assert_eq!(both["conservation_ok"], true);
    }

    #[test]
    fn running_twice_on_fresh_kernels_is_identical() {
        let mut k1 = kernel();
        let a = run(&mut k1).unwrap();
        let mut k2 = kernel();
        let b = run(&mut k2).unwrap();
        assert_eq!(a, b, "同样的种子给同样的两条链路");
    }
}
