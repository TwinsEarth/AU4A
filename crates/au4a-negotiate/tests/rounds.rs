//! v1.2.4 集成测试：多轮协商引擎经 PMB 真跑，含轮数上限与轮转规则。
//!
//! 覆盖：正常路径（多轮还价 → 拒绝 → 再还价 → 接受）、拒绝路径（超限类型化拒绝、轮转违规）、
//! 不变式（消息链 `in_reply_to` 连续、每条转换双签、归档可逐字节重放）。

use au4a_core::{AgentKeys, CoreError, Credits, EvidenceGrade, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{Journal, Negotiation, NegotiationMsg, Phase, Terms};

fn kernel() -> Kernel {
    Kernel::new(KernelConfig::default())
}

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn terms(price: i64) -> Terms {
    Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
}

fn pair(k: &mut Kernel, s1: u8, s2: u8) -> (AgentKeys, AgentKeys) {
    let a = agent(s1);
    let b = agent(s2);
    k.register(&a, "proposer", &["summarize.zh"], Credits(20))
        .unwrap();
    k.register(&b, "responder", &["summarize.zh"], Credits(20))
        .unwrap();
    (a, b)
}

#[test]
fn multi_round_negotiation_over_pmb_is_archivable_and_replayable() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 1, 2);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(150), 6).unwrap();
    n.counter(&mut k, &b, &a, terms(140)).unwrap();
    n.counter(&mut k, &a, &b, terms(130)).unwrap();
    // 拒绝 a 的 130 报价之后，由拒绝方 b 接手还价（还价必须来自上一次报价的对端）。
    n.reject(&mut k, &b, &a, "round to 120 and we have a deal")
        .unwrap();
    n.counter(&mut k, &b, &a, terms(125)).unwrap();
    n.counter(&mut k, &a, &b, terms(120)).unwrap();
    n.accept(&mut k, &b, &a).unwrap();

    assert_eq!(n.phase(), Phase::Accepted);
    assert_eq!(n.rounds_used(), 4);
    assert_eq!(n.price_trail(), vec![150, 140, 130, 125, 120]);
    assert_eq!(n.rejections().len(), 1);

    // 七条消息全部经过内核投递、逐条验签、且引用链连续。
    let delivered = k.drain();
    assert_eq!(delivered.len(), 7);
    let mut kinds = vec![];
    for (i, env) in delivered.iter().enumerate() {
        let msg = NegotiationMsg::from_env(env).unwrap();
        kinds.push(msg.kind());
        if i == 0 {
            assert!(env.in_reply_to.is_none());
        } else {
            assert_eq!(
                env.in_reply_to.as_deref(),
                Some(delivered[i - 1].id.as_str())
            );
        }
    }
    assert_eq!(
        kinds,
        vec![
            au4a_negotiate::kinds::NEGOTIATE_REQUEST,
            au4a_negotiate::kinds::NEGOTIATE_COUNTER,
            au4a_negotiate::kinds::NEGOTIATE_COUNTER,
            au4a_negotiate::kinds::NEGOTIATE_REJECT,
            au4a_negotiate::kinds::NEGOTIATE_COUNTER,
            au4a_negotiate::kinds::NEGOTIATE_COUNTER,
            au4a_negotiate::kinds::NEGOTIATE_ACCEPT,
        ]
    );

    // 每条转换记录都是双方签名。
    for record in n.machine().history() {
        assert_eq!(record.sigs.len(), 2, "{} 必须双签", record.event.as_str());
    }

    // 归档逐字节定点，且重放摘要与实时一致。
    let bytes = n.archive().unwrap();
    let restored = Journal::decode(&bytes).unwrap();
    assert_eq!(restored.encode().unwrap(), bytes);
    assert_eq!(
        restored.replay_digest().unwrap(),
        n.journal().replay_digest().unwrap()
    );
    assert_eq!(restored.phase(), Phase::Accepted);
    assert_eq!(restored.machine().round(), 4);
    assert!(k.ledger().check_conservation().is_ok());
}

#[test]
fn the_quota_refusal_is_typed_and_not_misconduct() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 3, 4);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 1).unwrap();
    n.counter(&mut k, &b, &a, terms(90)).unwrap();
    assert_eq!(n.rounds_used(), 1);

    let before = n.archive().unwrap();
    assert_eq!(
        n.counter(&mut k, &a, &b, terms(80)),
        Err(CoreError::Overflow)
    );
    assert_eq!(n.archive().unwrap(), before, "被拒的还价不得改变任何状态");

    let (who, refusal) = k.refusals().last().unwrap();
    assert_eq!(who, &a.did());
    assert_eq!(refusal.code, RefusalCode::PolicyDenied);
    assert!(
        !refusal.code.is_misconduct(),
        "超额是竞争/容量语义，不是恶意"
    );
    assert_eq!(
        k.escalation_for(&a.did(), RefusalCode::PolicyDenied),
        au4a_core::Escalation::None
    );
}

#[test]
fn a_turn_violation_is_refused_as_a_state_conflict() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 5, 6);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
    assert_eq!(
        n.counter(&mut k, &a, &b, terms(90)),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(k.refusals().last().unwrap().1.code, RefusalCode::Conflict);
    assert_eq!(n.rounds_used(), 0);
    assert_eq!(n.offers().len(), 1);
    assert_eq!(n.price_trail(), vec![100]);

    // 对端接手后协商继续。
    n.counter(&mut k, &b, &a, terms(95)).unwrap();
    assert_eq!(n.price_trail(), vec![100, 95]);
}

#[test]
fn rejections_are_free_but_still_dual_signed_records() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 7, 8);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 1).unwrap();
    for i in 0..6 {
        let by = if i % 2 == 0 { &b } else { &a };
        let to = if i % 2 == 0 { &a } else { &b };
        n.reject(&mut k, by, to, "no").unwrap();
    }
    assert_eq!(n.rounds_used(), 0);
    assert_eq!(n.machine().seq(), 7, "1 次开局 + 6 次拒绝");
    for record in n
        .machine()
        .history()
        .iter()
        .filter(|r| r.event.as_str() == "reject")
    {
        assert_eq!(record.sigs.len(), 2);
        assert_eq!(record.to, record.from, "拒绝是自环");
    }
    let bytes = n.archive().unwrap();
    assert_eq!(Journal::decode(&bytes).unwrap().encode().unwrap(), bytes);
}
