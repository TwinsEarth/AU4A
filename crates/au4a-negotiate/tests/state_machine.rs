//! v1.2.2 集成测试：状态机在内核逻辑时钟下跑完整生命周期。
//!
//! 覆盖：正常路径（IDLE→…→SETTLED，每条记录双签）、拒绝路径（非法转换/单签/第三方）、
//! 不变式（相位只沿合法表前进、历史重放可复算）。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{transition, DualSigned, Event, Phase, StateMachine};

fn kernel() -> Kernel {
    Kernel::new(KernelConfig::default())
}

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn parties(a: &AgentKeys, b: &AgentKeys) -> Vec<Did> {
    vec![a.did(), b.did()]
}

/// 测试用双签合约见证。
struct Witness {
    id: String,
    hash: String,
    dual: bool,
}

impl DualSigned for Witness {
    fn contract_id(&self) -> &str {
        &self.id
    }
    fn contract_hash(&self) -> &str {
        &self.hash
    }
    fn is_dual_signed(&self) -> bool {
        self.dual
    }
}

#[test]
fn full_lifecycle_every_transition_is_dual_signed() {
    let mut k = kernel();
    let a = agent(1);
    let b = agent(2);
    k.register(&a, "a", &["summarize.zh"], Credits(20)).unwrap();
    k.register(&b, "b", &["summarize.zh"], Credits(20)).unwrap();
    let parties = parties(&a, &b);

    let mut m = StateMachine::open("s-lifecycle").unwrap();
    let plan = [
        (Event::Request, Phase::Negotiating),
        (Event::Counter, Phase::Negotiating),
        (Event::Reject, Phase::Negotiating),
        (Event::Counter, Phase::Negotiating),
        (Event::Accept, Phase::Accepted),
        (Event::Sign, Phase::ContractSigned),
        (Event::Execute, Phase::Executing),
        (Event::Settle, Phase::Settled),
    ];
    let mut initiator_is_a = true;
    for (event, expected) in plan {
        let at = k.tick();
        let (initiator, counterparty) = if initiator_is_a { (&a, &b) } else { (&b, &a) };
        let record = m
            .transact(event, initiator, counterparty, at, &parties)
            .unwrap();
        assert_eq!(record.sigs.len(), 2, "{} 必须双方签名", event.as_str());
        assert!(record.signed_by(&a.did()) && record.signed_by(&b.did()));
        assert_eq!(record.verify_against(&parties).unwrap(), ());
        assert_eq!(m.phase(), expected, "{} 后相位不符", event.as_str());
        initiator_is_a = !initiator_is_a;
    }
    assert!(m.phase().is_terminal());
    assert_eq!(m.round(), 2, "两次 COUNTER 消耗两轮额度，REJECT 不占");
    assert_eq!(m.seq(), 8);
    m.verify_history(&parties).unwrap();
    assert!(k.ledger().check_conservation().is_ok());
}

#[test]
fn breach_from_executing_reaches_arbitration_only_with_a_dual_signed_contract() {
    let mut k = kernel();
    let a = agent(3);
    let b = agent(4);
    k.register(&a, "a", &[], Credits(20)).unwrap();
    k.register(&b, "b", &[], Credits(20)).unwrap();
    let parties = parties(&a, &b);
    let mut m = StateMachine::open("s-breach").unwrap();
    for event in [Event::Request, Event::Accept, Event::Sign, Event::Execute] {
        let at = k.tick();
        m.transact(event, &a, &b, at, &parties).unwrap();
    }
    let hash = "c".repeat(64);
    let witness = Witness {
        id: "c-breach".into(),
        hash: hash.clone(),
        dual: true,
    };
    let claim = m
        .stage_under_contract(Event::Breach, &b, k.tick(), "c-breach", &hash)
        .unwrap();
    assert_eq!(
        m.commit(claim.clone(), &parties, None),
        Err(CoreError::NotSealed)
    );
    assert_eq!(m.phase(), Phase::Executing, "被拒的申诉不得改变相位");
    m.commit(claim, &parties, Some(&witness)).unwrap();
    assert_eq!(m.phase(), Phase::Arbitration);

    let resolve = m
        .transact(Event::Resolve, &a, &b, k.tick(), &parties)
        .unwrap();
    assert_eq!(resolve.to, Phase::Settled);
    assert_eq!(resolve.sigs.len(), 2, "结案仍需双方签名");
    m.verify_history(&parties).unwrap();
}

#[test]
fn illegal_events_are_refused_without_mutating_state() {
    let a = agent(5);
    let b = agent(6);
    let parties = parties(&a, &b);
    let mut m = StateMachine::open("s-illegal").unwrap();
    for event in [Event::Sign, Event::Execute, Event::Settle, Event::Resolve] {
        assert_eq!(
            m.transact(event, &a, &b, 1, &parties),
            Err(CoreError::InvalidKind),
            "{} 不能在 IDLE 发生",
            event.as_str()
        );
    }
    assert_eq!(m.phase(), Phase::Idle);
    assert_eq!(m.seq(), 0);
    assert!(m.history().is_empty());

    // 跳过 ACCEPT 直接 SIGN 不成立。
    m.transact(Event::Request, &a, &b, 1, &parties).unwrap();
    assert_eq!(
        m.transact(Event::Sign, &a, &b, 2, &parties),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(m.phase(), Phase::Negotiating);
    assert_eq!(
        transition(Phase::Negotiating, Event::Sign),
        Err(CoreError::InvalidKind)
    );
}

#[test]
fn a_third_party_cannot_push_the_machine_forward() {
    let mut k = kernel();
    let a = agent(7);
    let b = agent(8);
    let outsider = agent(9);
    k.register(&a, "a", &[], Credits(20)).unwrap();
    k.register(&b, "b", &[], Credits(20)).unwrap();
    k.register(&outsider, "outsider", &[], Credits(20)).unwrap();
    let parties = parties(&a, &b);
    let mut m = StateMachine::open("s-outsider").unwrap();
    let mut record = m.stage(Event::Request, &outsider, k.tick()).unwrap();
    StateMachine::co_sign(&mut record, &b).unwrap();
    assert_eq!(record.sigs.len(), 2);
    assert_eq!(
        m.commit(record, &parties, None),
        Err(CoreError::InvalidSignature),
        "签名有效但不是当事人：拒绝"
    );
    assert_eq!(m.phase(), Phase::Idle);
}

#[test]
fn scenario_summary_reports_the_machine_state() {
    let mut k = kernel();
    let summary = au4a_negotiate::scenario(&mut k).unwrap();
    assert_eq!(summary["phase"], "settled");
    // 开局 + 两次还价 + 一次拒绝 + 接受 + 签合约 + 执行 + 结算 = 8 条双签转换。
    assert_eq!(summary["transitions"], 8);
    assert_eq!(summary["rounds_used"], 2);
    assert_eq!(summary["offers"], 3);
    assert_eq!(summary["rejections"], 1);
    assert_eq!(summary["price_trail"], serde_json::json!([120, 100, 95]));
    assert_eq!(summary["contract"]["dual_signed"], true);
    assert_eq!(summary["contract_anchored"], true);
    assert_eq!(summary["contract"]["price"], 95);
    assert!(au4a_core::all_passed(&au4a_negotiate::self_check()));
}
