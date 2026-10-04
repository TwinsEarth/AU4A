//! AU4A 轨道 1.2 — Negotiation 协商协议（v1.2.1 → v1.2.10）。
//!
//! 协商协议：多轮报价/还价、合约签订、执行、结算、违约仲裁，状态机与协商记录可持久化与重放。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责，并留下自检与证据。
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）。
//!
//! 设计要点（贯穿全部小版本）：
//!
//! * **Agent 是主体**：任何一条协商消息都由 Agent 自己的私钥签名后经 PMB 投递；
//!   人类只能通过 `Kernel::observe()` 只读观察，本 crate 不提供任何人类写路径。
//! * **确定性**：不读墙钟、不碰文件、不开网络；时间一律来自 `Kernel::now()`/`LogicalClock`。
//! * **内容寻址**：会话 id、条款哈希、合约哈希全部是规范 JSON 的 SHA-256，
//!   所以「同一条款」在不同进程里有同一串字节，重放可以逐字节比对。
//! * **双方签名**：状态机每一次转换都必须由双方签署（或由双签合约的条款授权），
//!   单方签名不成立。

pub mod contract;
pub mod journal;
pub mod msg;
pub mod rounds;
pub mod state;

pub use contract::{Anchor, Contract, ANCHOR_EVENT};
pub use journal::{Journal, JOURNAL_VERSION};
pub use msg::{kinds, BreachKind, NegotiationMsg, Terms, ALL_KINDS};
pub use rounds::{Negotiation, Offer, Rejection, DEFAULT_MAX_ROUNDS};
pub use state::{
    transition, Basis, DualSigned, Event, PartySignature, Phase, StateMachine, TransitionRecord,
    LEGAL_TRANSITIONS,
};

use au4a_core::{CoreResult, Credits, Did, EvidenceGrade, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

/// 轨道号。
pub const TRACK: &str = "1.2";
/// 轨道标题。
pub const TITLE: &str = "Negotiation 协商协议";
/// 版本区间。
pub const RANGE: &str = "v1.2.1 → v1.2.10";
/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_negotiate";

/// 场景用 Agent 的确定性种子。种子固定 ⇒ 场景可重复（同样的输入给同样的输出）。
pub(crate) const PROPOSER_SEED: [u8; 32] = [0x12; 32];
pub(crate) const RESPONDER_SEED: [u8; 32] = [0x34; 32];

/// 场景里的两个协商方：提议方与应答方。
///
/// 注册是幂等的：内核里已有同名 Agent 就复用它的名片，
/// 这样 `scenario` 在同一个内核上被多次调用（端到端演示 + 10 次部署验证）不会因重复注册而失败。
pub(crate) fn scenario_agents(kernel: &mut Kernel) -> CoreResult<(au4a_core::AgentKeys, au4a_core::AgentKeys)> {
    let proposer = au4a_core::AgentKeys::from_seed(&PROPOSER_SEED);
    let responder = au4a_core::AgentKeys::from_seed(&RESPONDER_SEED);
    ensure_agent(kernel, &proposer, "negotiate.proposer", &["summarize.zh"])?;
    ensure_agent(kernel, &responder, "negotiate.responder", &["summarize.zh"])?;
    Ok((proposer, responder))
}

pub(crate) fn ensure_agent(
    kernel: &mut Kernel,
    keys: &au4a_core::AgentKeys,
    display: &str,
    skills: &[&str],
) -> CoreResult<()> {
    if kernel.card(&keys.did()).is_some() {
        return Ok(());
    }
    kernel.register(keys, display, skills, Credits(20))?;
    Ok(())
}

/// 把内部断言映射成一条可展示的自检项（`detail` 写清断言了什么）。
fn check(name: &str, outcome: CoreResult<String>) -> SelfCheck {
    match outcome {
        Ok(detail) => SelfCheck::pass(TRACK, name, detail),
        Err(err) => SelfCheck::fail(TRACK, name, err.to_string()),
    }
}

fn msg_kinds_check() -> CoreResult<String> {
    if ALL_KINDS.len() != 6 {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    let mut seen: Vec<&str> = Vec::new();
    for k in ALL_KINDS {
        au4a_core::MsgKind::new(k)?;
        if seen.contains(&k) {
            return Err(au4a_core::CoreError::InvalidKind);
        }
        seen.push(k);
    }
    Ok(format!("{ALL_KINDS:?} 六个类型名均为合法 MsgKind 且互不相同"))
}

fn msg_roundtrip_check() -> CoreResult<String> {
    let a = au4a_core::AgentKeys::from_seed(&[0xA1; 32]);
    let b = au4a_core::AgentKeys::from_seed(&[0xB2; 32]);
    let terms = Terms::new("summarize.zh", Credits(120), 40, EvidenceGrade::Verified)?;
    let h = terms.hash()?;
    let msgs = [
        NegotiationMsg::request("selfcheck", terms.clone())?,
        NegotiationMsg::counter("selfcheck", 1, Terms::new("summarize.zh", Credits(100), 40, EvidenceGrade::Verified)?)?,
        NegotiationMsg::accept("selfcheck", 1, &h)?,
        NegotiationMsg::reject("selfcheck", 1, "price")?,
        NegotiationMsg::sign_contract("c-selfcheck", &h)?,
        NegotiationMsg::breach("c-selfcheck", &h, BreachKind::NonDelivery, "none")?,
    ];
    let mut verified = 0usize;
    for msg in &msgs {
        let env = msg.signed(&a, &b.did(), 7, None)?;
        env.verify()?;
        if &NegotiationMsg::from_env(&env)? != msg {
            return Err(au4a_core::CoreError::Encoding);
        }
        verified += 1;
    }
    Ok(format!("{verified} 种消息 签名→投递→解析 逐字段相等"))
}

fn msg_rejection_check() -> CoreResult<String> {
    let a = au4a_core::AgentKeys::from_seed(&[0xC3; 32]);
    let b = au4a_core::AgentKeys::from_seed(&[0xD4; 32]);
    let env = NegotiationMsg::request(
        "selfcheck",
        Terms::new("summarize.zh", Credits(10), 1, EvidenceGrade::Verified)?,
    )?
    .signed(&a, &b.did(), 1, None)?;
    let mut tampered = env.clone();
    tampered.body = NegotiationMsg::counter(
        "selfcheck",
        1,
        Terms::new("summarize.zh", Credits(1), 1, EvidenceGrade::Verified)?,
    )?
    .body()?;
    if NegotiationMsg::from_env(&tampered) != Err(au4a_core::CoreError::InvalidSignature) {
        return Err(au4a_core::CoreError::InvalidSignature);
    }
    if NegotiationMsg::accept("selfcheck", 0, "nothex") != Err(au4a_core::CoreError::InvalidKind) {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    Ok("篡改报文拒绝=invalid_signature，非法载荷拒绝=invalid_kind".to_string())
}

fn state_legal_table_check() -> CoreResult<String> {
    let mut legal = 0usize;
    let mut refused = 0usize;
    for phase in Phase::ALL {
        for event in Event::ALL {
            let expected = LEGAL_TRANSITIONS
                .iter()
                .find(|(f, e, _)| *f == phase && *e == event)
                .map(|(_, _, t)| *t);
            match (transition(phase, event), expected) {
                (Ok(got), Some(want)) if got == want => legal += 1,
                (Err(au4a_core::CoreError::InvalidKind), None) => refused += 1,
                _ => {
                    return Err(au4a_core::CoreError::InvalidKind);
                }
            }
        }
    }
    if legal != LEGAL_TRANSITIONS.len() {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    Ok(format!(
        "穷举 {} 种 (相位,事件) 组合：{legal} 合法 / {refused} 非法且全部返回 Err",
        Phase::ALL.len() * Event::ALL.len()
    ))
}

fn state_dual_signature_check() -> CoreResult<String> {
    let a = au4a_core::AgentKeys::from_seed(&[0xE5; 32]);
    let b = au4a_core::AgentKeys::from_seed(&[0xF6; 32]);
    let parties = vec![a.did(), b.did()];
    let mut machine = StateMachine::open("selfcheck-state")?;
    let single = machine.stage(Event::Request, &a, 1)?;
    if machine.commit(single.clone(), &parties, None) != Err(au4a_core::CoreError::NotSealed) {
        return Err(au4a_core::CoreError::NotSealed);
    }
    if !machine.history().is_empty() {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    machine.commit(
        {
            let mut dual = single;
            StateMachine::co_sign(&mut dual, &b)?;
            dual
        },
        &parties,
        None,
    )?;
    if machine.phase() != Phase::Negotiating {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    machine.verify_history(&parties)?;
    Ok("单签 commit 被拒且相位不变；双方联署后才进入 NEGOTIATING".to_string())
}

fn state_round_quota_check() -> CoreResult<String> {
    let a = au4a_core::AgentKeys::from_seed(&[0x17; 32]);
    let b = au4a_core::AgentKeys::from_seed(&[0x28; 32]);
    let parties = vec![a.did(), b.did()];
    let mut machine = StateMachine::open("selfcheck-rounds")?;
    machine.transact(Event::Request, &a, &b, 1, &parties)?;
    for i in 0..3 {
        machine.transact(Event::Reject, &b, &a, 2 + i, &parties)?;
    }
    if machine.round() != 0 {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    machine.transact(Event::Counter, &a, &b, 9, &parties)?;
    if machine.round() != 1 {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    Ok("3 次 REJECT 后轮数仍为 0，1 次 COUNTER 后为 1".to_string())
}

fn rounds_cap_check() -> CoreResult<String> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let a = au4a_core::AgentKeys::from_seed(&[0x5B; 32]);
    let b = au4a_core::AgentKeys::from_seed(&[0x6C; 32]);
    ensure_agent(&mut kernel, &a, "selfcheck.a", &["summarize.zh"])?;
    ensure_agent(&mut kernel, &b, "selfcheck.b", &["summarize.zh"])?;
    let deal = |price: i64| Terms::new("summarize.zh", Credits(price), 10, EvidenceGrade::Verified);
    let mut negotiation = Negotiation::open(&mut kernel, &a, &b, deal(100)?, 1)?;
    negotiation.counter(&mut kernel, &b, &a, deal(90)?)?;
    if negotiation.counter(&mut kernel, &a, &b, deal(80)?) != Err(au4a_core::CoreError::Overflow) {
        return Err(au4a_core::CoreError::Overflow);
    }
    if kernel.refusals().last().map(|(_, r)| r.code) != Some(au4a_core::RefusalCode::PolicyDenied) {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    if negotiation.rounds_used() != 1 || negotiation.offers().len() != 2 {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    Ok("max_rounds=1：第 2 次还价 → Err(Overflow) + 内核记 policy_denied，轮数与报价数不变".to_string())
}

fn rounds_reject_check() -> CoreResult<String> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let a = au4a_core::AgentKeys::from_seed(&[0x7D; 32]);
    let b = au4a_core::AgentKeys::from_seed(&[0x8E; 32]);
    ensure_agent(&mut kernel, &a, "selfcheck.r1", &["summarize.zh"])?;
    ensure_agent(&mut kernel, &b, "selfcheck.r2", &["summarize.zh"])?;
    let terms = Terms::new("summarize.zh", Credits(100), 10, EvidenceGrade::Verified)?;
    let mut negotiation = Negotiation::open(&mut kernel, &a, &b, terms, 2)?;
    for _ in 0..5 {
        negotiation.reject(&mut kernel, &b, &a, "not yet")?;
    }
    if negotiation.rounds_used() != 0 || negotiation.rejections().len() != 5 {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    negotiation.counter(&mut kernel, &b, &a, Terms::new("summarize.zh", Credits(90), 10, EvidenceGrade::Verified)?)?;
    if negotiation.rounds_used() != 1 {
        return Err(au4a_core::CoreError::InvalidKind);
    }
    Ok("5 次 REJECT 后轮数仍为 0，随后 1 次 COUNTER 才消耗 1 轮".to_string())
}

fn contract_dual_signature_check() -> CoreResult<String> {
    let a = au4a_core::AgentKeys::from_seed(&[0x9F; 32]);
    let b = au4a_core::AgentKeys::from_seed(&[0xA0; 32]);
    let terms = Terms::new("summarize.zh", Credits(120), 40, EvidenceGrade::Verified)?;
    let mut contract = Contract::draft(&a, &b.did(), &terms, "selfcheck-contract", 1)?;
    if contract.verify() != Err(au4a_core::CoreError::NotSealed) {
        return Err(au4a_core::CoreError::NotSealed);
    }
    contract.sign(&a)?;
    if contract.verify() != Err(au4a_core::CoreError::NotSealed) {
        return Err(au4a_core::CoreError::NotSealed);
    }
    contract.sign(&b)?;
    contract.verify()?;
    if !contract.is_dual_signed() || contract.hash.len() != 64 {
        return Err(au4a_core::CoreError::InvalidSignature);
    }

    // 签署后改一个条款字段，哈希与签名双双失效。
    let mut tampered = contract.clone();
    tampered.terms.price = Credits(1);
    if tampered.verify() != Err(au4a_core::CoreError::InvalidSignature) {
        return Err(au4a_core::CoreError::InvalidSignature);
    }
    // 锚点可离线复核。
    let anchor = contract.anchor(&a, 2)?;
    contract.verify_anchor()?;
    if anchor.contract_hash != contract.hash {
        return Err(au4a_core::CoreError::InvalidSignature);
    }
    Ok(format!(
        "合约 {}：单签被拒，双签成立，改价即废，锚点可复核",
        au4a_core::short_id(&contract.hash)
    ))
}

fn journal_roundtrip_check() -> CoreResult<String> {
    let a = au4a_core::AgentKeys::from_seed(&[0x39; 32]);
    let b = au4a_core::AgentKeys::from_seed(&[0x4A; 32]);
    let parties = vec![a.did(), b.did()];
    let terms = Terms::new("summarize.zh", Credits(120), 40, EvidenceGrade::Verified)?;
    let session = msg::session_id(&a.did(), &b.did(), &terms)?;
    let mut journal = Journal::open(&session, &parties)?;
    let request = NegotiationMsg::request(&session, terms)?.signed(&a, &b.did(), 1, None)?;
    journal.append(&request)?;
    let mut machine = StateMachine::open(&session)?;
    let record = machine.transact(Event::Request, &a, &b, 1, &parties)?;
    journal.append_transition(&record, None)?;

    let text = journal.encode()?;
    let restored = Journal::decode(&text)?;
    let again = restored.encode()?;
    if again != text {
        return Err(au4a_core::CoreError::Encoding);
    }
    if restored.replay_digest()? != journal.replay_digest()? {
        return Err(au4a_core::CoreError::Encoding);
    }
    // 篡改版本必须被明确拒绝，而不是被猜着接受。
    let bumped = text.replace("\"version\":1", "\"version\":2");
    if Journal::decode(&bumped) != Err(au4a_core::CoreError::InvalidVersion) {
        return Err(au4a_core::CoreError::InvalidVersion);
    }
    Ok(format!(
        "{} 字节归档：encode→decode→encode 逐字节相同，重放摘要一致，版本篡改被拒",
        text.len()
    ))
}

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
pub fn self_check() -> Vec<SelfCheck> {
    vec![
        check("track.wired", Ok(format!("{TITLE} {RANGE} 已接入 au4a-node"))),
        check("msg.kinds", msg_kinds_check()),
        check("msg.roundtrip", msg_roundtrip_check()),
        check("msg.rejection", msg_rejection_check()),
        check("state.legal_table", state_legal_table_check()),
        check("state.dual_signature", state_dual_signature_check()),
        check("state.round_quota", state_round_quota_check()),
        check("journal.roundtrip", journal_roundtrip_check()),
        check("rounds.cap", rounds_cap_check()),
        check("rounds.reject_free", rounds_reject_check()),
        check("contract.dual_signature", contract_dual_signature_check()),
    ]
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "kinds": ALL_KINDS,
        "phases": Phase::ALL.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        "events": Event::ALL.iter().map(|e| e.as_str()).collect::<Vec<_>>(),
        "legal_transitions": LEGAL_TRANSITIONS.len(),
        "journal_version": JOURNAL_VERSION,
        "default_max_rounds": DEFAULT_MAX_ROUNDS,
        "checks": self_check().len(),
        "checks_passed": self_check().iter().filter(|c| c.passed).count(),
    }))
}

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
/// 每一版迭代都应该让这里多做一件真实的事，而不是多打印一行字。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    let (proposer, responder) = scenario_agents(kernel)?;
    let opening = Terms::new("summarize.zh", Credits(120), 40, EvidenceGrade::Verified)?;

    // 多轮协商引擎真跑：开局 → 还价 → 拒绝（不占额度）→ 再还价。
    let mut negotiation =
        Negotiation::open(kernel, &proposer, &responder, opening, DEFAULT_MAX_ROUNDS)?;
    negotiation.counter(
        kernel,
        &responder,
        &proposer,
        Terms::new("summarize.zh", Credits(100), 40, EvidenceGrade::Verified)?,
    )?;
    negotiation.reject(kernel, &proposer, &responder, "deadline too tight")?;
    // 还价必须由上一次报价的对端发出：上一次是应答方报的价，所以这次由提议方还价。
    negotiation.counter(
        kernel,
        &proposer,
        &responder,
        Terms::new("summarize.zh", Credits(95), 42, EvidenceGrade::Verified)?,
    )?;
    // 应答方接受提议方的 95，然后双方签订合约（双方签名 + 锚定哈希）。
    negotiation.accept(kernel, &responder, &proposer)?;
    let contract = negotiation.sign_contract(kernel, &proposer, &responder)?;

    let delivered = kernel.drain();
    let mut transcript: Vec<Value> = Vec::new();
    for env in &delivered {
        let parsed = NegotiationMsg::from_env(env)?;
        transcript.push(json!({
            "kind": parsed.kind(),
            "id": env.id,
            "from": env.from.as_str(),
            "ts": env.ts,
        }));
    }

    // 归档：把这次协商持久化成规范 JSON，再解回来做逐字节比对（纯内存，无文件 I/O）。
    let archived = negotiation.archive()?;
    let restored = Journal::decode(&archived)?;
    let byte_exact = restored.encode()? == archived;
    let replay_digest = restored.replay_digest()?;
    let summary = negotiation.summary()?;

    kernel.emit(
        format!("{TRACK}.scenario").as_str(),
        format!(
            "{TITLE}：{} 条协商消息经 PMB 投递并逐条验签；{} → {}（{} 条双签记录，{} 轮报价）；合约 {} 双签并锚定；归档 {} 字节，重放{}",
            transcript.len(),
            Phase::Idle.as_str(),
            negotiation.phase().as_str(),
            negotiation.machine().seq(),
            negotiation.rounds_used(),
            au4a_core::short_id(&contract.hash),
            archived.len(),
            if byte_exact { "逐字节一致" } else { "不一致" }
        ),
    );

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "session": negotiation.session(),
        "delivered": transcript.len(),
        "transcript": transcript,
        "phase": negotiation.phase().as_str(),
        "transitions": negotiation.machine().seq(),
        "rounds_used": negotiation.rounds_used(),
        "max_rounds": negotiation.max_rounds(),
        "offers": negotiation.offers().len(),
        "rejections": negotiation.rejections().len(),
        "price_trail": negotiation.price_trail(),
        "history_tip": summary["history_tip"],
        "journal_bytes": archived.len(),
        "replay_byte_exact": byte_exact,
        "replay_digest": replay_digest,
        "contract": contract.summary(),
        "contract_anchored": contract.verify_anchor().is_ok(),
        "steps": 5,
    }))
}

/// 场景辅助：把一个 JSON 摘要里的字符串数组读出来（供测试断言，避免测试里写重复解析）。
pub fn transcript_kinds(summary: &Value) -> Vec<String> {
    summary
        .get("transcript")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.get("kind").and_then(|k| k.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 供测试与场景复用：确定性 Agent 的 DID。
pub fn scenario_dids() -> (Did, Did) {
    (
        au4a_core::AgentKeys::from_seed(&PROPOSER_SEED).did(),
        au4a_core::AgentKeys::from_seed(&RESPONDER_SEED).did(),
    )
}
