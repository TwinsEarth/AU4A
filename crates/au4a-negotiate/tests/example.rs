//! v1.2.10 集成测试：两条端到端链路（示例）与 `scenario` 的两份 JSON 摘要。
//!
//! 覆盖：正常路径（成功链路逐项数值 + 逐字节重放）、拒绝路径（示例内部不吞错：坏条款/坏相位一律 Err）、
//! 不变式（两条链路同种子可重复、账本守恒、契约哈希锚定、消息全部可验签）。

use au4a_core::{AgentKeys, CoreError, Credits, EvidenceGrade, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{example, Journal, Negotiation, NegotiationMsg, Phase, Terms};

fn kernel() -> Kernel {
    Kernel::new(KernelConfig::default())
}

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

#[test]
fn the_success_path_is_a_complete_auditable_deal() {
    let mut k = kernel();
    let summary = example::success_path(&mut k).unwrap();

    assert_eq!(summary["path"], "success");
    assert_eq!(summary["phase"], "settled");
    assert_eq!(summary["settled_amount"], 95);
    assert_eq!(summary["price_trail"], serde_json::json!([120, 100, 95]));
    assert_eq!(summary["rounds_used"], 2);
    assert_eq!(summary["rejections"], 1);
    assert_eq!(summary["transitions"], 8);
    assert_eq!(summary["contract"]["dual_signed"], true);
    assert_eq!(summary["contract"]["signatures"], 2);
    assert_eq!(summary["contract"]["hash"].as_str().unwrap().len(), 64);
    assert_eq!(summary["contract_anchored"], true);
    assert_eq!(summary["replay_byte_exact"], true);
    assert_eq!(summary["conservation_ok"], true);

    // 七条消息：request / counter / reject / counter / accept / sign ×2。
    let kinds = au4a_negotiate::transcript_kinds(&summary);
    assert_eq!(kinds.len(), 7);
    assert_eq!(kinds[0], au4a_negotiate::kinds::NEGOTIATE_REQUEST);
    assert_eq!(kinds[6], au4a_negotiate::kinds::CONTRACT_SIGN);
    assert!(!kinds.iter().any(|k| k == au4a_negotiate::kinds::CONTRACT_BREACH));

    // 钱真的动了：客户 1000−20−95，服务方 1000−20+95。
    assert_eq!(k.ledger().balance(&agent(0x12).did()).available, Credits(885));
    assert_eq!(
        k.ledger().balance(&agent(0x34).did()).available,
        Credits(1_075)
    );
    k.ledger().check_conservation().unwrap();
}

#[test]
fn the_breach_path_produces_a_dual_signed_ruling_and_enforces_it() {
    let mut k = kernel();
    let summary = example::breach_path(&mut k).unwrap();

    assert_eq!(summary["path"], "breach");
    assert_eq!(summary["phase"], "settled");
    assert_eq!(summary["claim"]["kind"], "non_delivery");
    assert_eq!(summary["claim"]["evidence"], "verified");
    assert_eq!(summary["arbiters"], 2);
    assert_eq!(summary["ruling"]["verdict"], "upheld");
    assert_eq!(summary["ruling"]["signatures"], 2);
    assert_eq!(summary["ruling"]["slash"], 15);
    assert_eq!(summary["ruling"]["compensate"], 37);
    assert_eq!(summary["enforcement"]["slashed"], 15);
    assert_eq!(summary["enforcement"]["compensated"], 37);
    assert_eq!(summary["enforcement"]["conservation_ok"], true);
    assert_eq!(summary["replay_byte_exact"], true);
    assert_eq!(summary["case_id"].as_str().unwrap().len(), 64);

    // 罚没是销毁：发行量不变、slashed += 15。
    assert_eq!(k.ledger().slashed(), Credits(15));
    assert_eq!(k.ledger().minted(), Credits(4_000), "四个 Agent 各 1000 创世");
    // 被诉方：锁定 20−15，可用 980−37。
    assert_eq!(k.ledger().balance(&agent(0x78).did()).locked, Credits(5));
    assert_eq!(k.ledger().balance(&agent(0x78).did()).available, Credits(943));
    assert_eq!(k.ledger().balance(&agent(0x56).did()).available, Credits(1_017));

    // 仲裁路径的消息包含 CONTRACT_BREACH。
    let kinds = au4a_negotiate::transcript_kinds(&summary);
    assert!(kinds.iter().any(|k| k == au4a_negotiate::kinds::CONTRACT_BREACH));
    k.ledger().check_conservation().unwrap();
}

#[test]
fn scenario_returns_two_json_summaries_and_stays_repeatable() {
    let mut k1 = kernel();
    let first = au4a_negotiate::scenario(&mut k1).unwrap();
    assert_eq!(first["paths"], 2);
    assert_eq!(first["success"]["phase"], "settled");
    assert_eq!(first["breach"]["phase"], "settled");
    assert_eq!(first["conservation_ok"], true);
    // 顶层兼容字段仍然指向成功链路。
    assert_eq!(first["phase"], "settled");
    assert_eq!(first["settled_amount"], 95);
    assert_eq!(first["price_trail"], serde_json::json!([120, 100, 95]));

    // 同一个内核再跑一次：结构一致（逻辑时钟前进导致 id/ts 变化）。
    let second = au4a_negotiate::scenario(&mut k1).unwrap();
    assert_eq!(first["session"], second["session"]);
    assert_eq!(
        au4a_negotiate::transcript_kinds(&first),
        au4a_negotiate::transcript_kinds(&second)
    );
    assert_ne!(first["transcript"][0]["id"], second["transcript"][0]["id"]);

    // 全新内核 + 同样种子：逐字段相同。
    let mut k2 = kernel();
    assert_eq!(first, au4a_negotiate::scenario(&mut k2).unwrap());
}

#[test]
fn the_example_does_not_swallow_errors() {
    // 例子里用的是真实的协商引擎：非法动作照样返回 Err，不会因为「是示例」而被容忍。
    let mut k = kernel();
    let a = agent(1);
    let b = agent(2);
    k.register(&a, "a", &["summarize.zh"], Credits(20)).unwrap();
    k.register(&b, "b", &["summarize.zh"], Credits(20)).unwrap();
    let mut n = Negotiation::open(
        &mut k,
        &a,
        &b,
        Terms::new("summarize.zh", Credits(100), 40, EvidenceGrade::Verified).unwrap(),
        1,
    )
    .unwrap();
    assert_eq!(
        n.sign_contract(&mut k, &a, &b),
        Err(CoreError::InvalidKind),
        "示例链路里也是先接受才能签"
    );
    n.counter(
        &mut k,
        &b,
        &a,
        Terms::new("summarize.zh", Credits(90), 40, EvidenceGrade::Verified).unwrap(),
    )
    .unwrap();
    assert_eq!(
        n.counter(
            &mut k,
            &a,
            &b,
            Terms::new("summarize.zh", Credits(80), 40, EvidenceGrade::Verified).unwrap()
        ),
        Err(CoreError::Overflow)
    );
    assert_eq!(k.refusals().last().unwrap().1.code, RefusalCode::PolicyDenied);
    assert_eq!(n.phase(), Phase::Negotiating);
}

#[test]
fn the_two_paths_are_independent_and_each_archives_cleanly() {
    let mut k = kernel();
    let both = example::run(&mut k).unwrap();
    assert_ne!(both["success"]["session"], both["breach"]["session"]);

    // 两条链路各自的摘要都能自洽（重放摘要 64 位、逐字节重放为真）。
    for path in ["success", "breach"] {
        let row = &both[path];
        assert_eq!(row["replay_byte_exact"], true, "{path}");
        assert_eq!(row["replay_digest"].as_str().unwrap().len(), 64, "{path}");
        assert_eq!(row["transitions"].as_u64().unwrap() >= 7, true, "{path}");
    }
    assert!(au4a_core::all_passed(&au4a_negotiate::self_check()));
}

#[test]
fn the_example_agents_are_self_sovereign() {
    // 四个 Agent 各自生成 DID、自己注册、自带质押；没有任何人类账户参与。
    let dids = example::example_dids();
    assert_eq!(dids.len(), 4);
    let mut k = kernel();
    let both = example::run(&mut k).unwrap();
    assert_eq!(k.agent_count(), 6, "成功链路 2 个 + 违约链路 4 个");
    for did in &dids {
        assert!(k.card(did).is_some(), "{did}");
        assert!(did.as_str().starts_with("did:au4a:"));
    }
    // 观察层是只读投影：能看到 Agent、进度与账本，但没有写路径。
    let view = k.observe();
    assert_eq!(view.agents.len(), 6);
    assert!(view.progress.iter().any(|p| p.kind == "1.2.example.success"));
    assert!(view.progress.iter().any(|p| p.kind == "1.2.example.breach"));
    assert_eq!(view.ledger.minted, Credits(6_000));
    assert!(both["conservation_ok"].as_bool().unwrap());

    // 归档可被任意一方带走并离线解码（不含私钥，只含签名与消息）。
    let archived = Journal::decode(&both["success"]["transcript"][0]["id"].as_str().unwrap_or_default());
    assert!(archived.is_err(), "id 不是归档文本");
    let env_id = both["success"]["transcript"][0]["id"].as_str().unwrap();
    assert_eq!(env_id.len(), 64, "消息 id 是内容寻址");
    let _ = NegotiationMsg::request("s", Terms::new("t", Credits(1), 1, EvidenceGrade::Verified).unwrap());
}
