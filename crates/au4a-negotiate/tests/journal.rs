//! v1.2.3 集成测试：协商记录归档 → 规范 JSON → 解码重放，逐字节一致。
//!
//! 覆盖：正常路径（真实投递后再归档）、拒绝路径（篡改/错版本/乱序）、
//! 不变式（重放推导出的状态与实时状态相同、归档里没有可篡改的相位字段）。

use au4a_core::{AgentKeys, CoreError, Credits, Did, EvidenceGrade};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{Event, Journal, NegotiationMsg, Phase, StateMachine, Terms, JOURNAL_VERSION};

fn kernel() -> Kernel {
    Kernel::new(KernelConfig::default())
}

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn terms(price: i64) -> Terms {
    Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
}

/// 真跑一次「报价 → 还价 → 接受」，把消息与转换都归档，返回归档与双方 DID。
fn archived_session() -> (Journal, AgentKeys, AgentKeys, Vec<Did>) {
    let mut k = kernel();
    let a = agent(11);
    let b = agent(12);
    k.register(&a, "proposer", &["summarize.zh"], Credits(20)).unwrap();
    k.register(&b, "responder", &["summarize.zh"], Credits(20)).unwrap();
    let parties = vec![a.did(), b.did()];
    let session = au4a_negotiate::msg::session_id(&a.did(), &b.did(), &terms(120)).unwrap();

    let request_env = NegotiationMsg::request(&session, terms(120))
        .unwrap()
        .signed(&a, &b.did(), k.tick(), None)
        .unwrap();
    k.send(&request_env).unwrap();
    let counter_env = NegotiationMsg::counter(&session, 1, terms(100))
        .unwrap()
        .signed(&b, &a.did(), k.tick(), Some(request_env.id.clone()))
        .unwrap();
    k.send(&counter_env).unwrap();

    let mut machine = StateMachine::open(&session).unwrap();
    let r1 = machine.transact(Event::Request, &a, &b, k.tick(), &parties).unwrap();
    let r2 = machine.transact(Event::Counter, &b, &a, k.tick(), &parties).unwrap();
    let r3 = machine.transact(Event::Accept, &a, &b, k.tick(), &parties).unwrap();

    let mut journal = Journal::open(&session, &parties).unwrap();
    for env in k.drain() {
        journal.append(&env).unwrap();
    }
    journal.append_transition(&r1, None).unwrap();
    journal.append_transition(&r2, None).unwrap();
    journal.append_transition(&r3, None).unwrap();
    (journal, a, b, parties)
}

#[test]
fn archive_roundtrips_byte_exactly_and_replays_the_same_state() {
    let (journal, _, _, parties) = archived_session();
    let bytes = journal.encode().unwrap();
    assert!(bytes.starts_with('{'));
    assert!(!bytes.contains('\n'), "规范 JSON 不允许空白");

    let restored = Journal::decode(&bytes).unwrap();
    assert_eq!(restored.encode().unwrap(), bytes, "encode→decode→encode 必须定点");
    assert_eq!(restored, journal);
    assert_eq!(restored.replay_digest().unwrap(), journal.replay_digest().unwrap());

    let replayed = restored.replay().unwrap();
    assert_eq!(replayed.phase(), Phase::Accepted);
    assert_eq!(replayed.seq(), 3);
    assert_eq!(replayed.round(), 1);
    assert_eq!(replayed.history().len(), 3);
    replayed.verify_history(&parties).unwrap();
    assert_eq!(restored.machine(), &replayed);

    // 归档里没有「相位」这种可被直接篡改的冗余字段。
    assert!(!bytes.contains("\"phase\""));
    assert!(bytes.contains("\"transitions\""));
    assert_eq!(JOURNAL_VERSION, 1);
}

#[test]
fn a_tampered_archive_never_decodes() {
    let (journal, _, _, _) = archived_session();
    let bytes = journal.encode().unwrap();

    let price_tampered = bytes.replace("\"price\":100", "\"price\":1");
    assert_ne!(price_tampered, bytes);
    assert_eq!(
        Journal::decode(&price_tampered),
        Err(CoreError::InvalidSignature),
        "改条款就必须废掉签名"
    );

    let round_tampered = bytes.replace("\"round\":1,\"session\"", "\"round\":2,\"session\"");
    assert_ne!(round_tampered, bytes);
    assert!(Journal::decode(&round_tampered).is_err());

    assert_eq!(
        Journal::decode(&bytes.replace("\"version\":1", "\"version\":9")),
        Err(CoreError::InvalidVersion)
    );
    assert_eq!(Journal::decode("null"), Err(CoreError::Encoding));
}

#[test]
fn archiving_requires_verified_messages_and_dual_signed_transitions() {
    let (journal, a, b, parties) = archived_session();

    // 未封口信封：验签失败，不进归档。
    let unsigned = au4a_core::Envelope::new(
        a.did(),
        Some(b.did()),
        au4a_negotiate::kinds::NEGOTIATE_REJECT,
        1,
        None,
        NegotiationMsg::reject(journal.session(), 1, "nope").unwrap().body().unwrap(),
    )
    .unwrap();
    let mut j = journal.clone();
    assert_eq!(j.append(&unsigned), Err(CoreError::NotSealed));
    assert_eq!(j.messages().len(), journal.messages().len());

    // 单签转换：不能归档。（归档到 ACCEPTED 后，合法事件是 SIGN。）
    let machine = StateMachine::rebuild(journal.session(), &parties, journal.transitions()).unwrap();
    assert_eq!(machine.phase(), Phase::Accepted);
    let staged = machine.stage(Event::Sign, &a, 99).unwrap();
    let mut j = journal.clone();
    assert_eq!(j.append_transition(&staged, None), Err(CoreError::NotSealed));
    assert_eq!(j.transitions().len(), journal.transitions().len());

    // 补上对端签名后可以归档，相位推进到 CONTRACT_SIGNED。
    let mut dual = staged;
    StateMachine::co_sign(&mut dual, &b).unwrap();
    j.append_transition(&dual, None).unwrap();
    assert_eq!(j.phase(), Phase::ContractSigned);
    assert_eq!(j.transitions().len(), 4);
    assert_ne!(j.replay_digest().unwrap(), journal.replay_digest().unwrap());
    // 归档后仍然可以逐字节往返。
    let text = j.encode().unwrap();
    assert_eq!(Journal::decode(&text).unwrap().encode().unwrap(), text);
}

#[test]
fn scenario_reports_a_byte_exact_replay() {
    let mut k = kernel();
    let summary = au4a_negotiate::scenario(&mut k).unwrap();
    assert_eq!(summary["replay_byte_exact"], true);
    assert_eq!(summary["steps"], 8);
    assert_eq!(summary["paths"], 2, "v1.2.10 起 scenario 真跑两条链路");
    assert!(summary["journal_bytes"].as_u64().unwrap() > 200);
    assert_eq!(
        summary["replay_digest"].as_str().unwrap().len(),
        64,
        "重放摘要是内容寻址 SHA-256"
    );
    assert!(au4a_core::all_passed(&au4a_negotiate::self_check()));
}
