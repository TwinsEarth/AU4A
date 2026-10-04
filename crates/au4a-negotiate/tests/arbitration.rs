//! v1.2.7 集成测试：违约 → 立案 → 双仲裁员裁决 → 罚没 + 赔付 → 结案。
//!
//! 覆盖：正常路径（两条完整链路：协商达成后违约进仲裁、成功执行后结算）、
//! 拒绝路径（当事人当仲裁员、单仲裁员裁决、证据闸门拦大额 cpu-proto 赔付、无裁决先执行）、
//! 不变式（裁决双签、罚没销毁、赔付过闸门、执行前后账本守恒）。

use au4a_core::{AgentKeys, CoreError, Credits, EvidenceGrade, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{
    ArbitrationCase, ArbitrationPolicy, BreachKind, Negotiation, Phase, Terms, Verdict,
};

fn kernel() -> Kernel {
    Kernel::new(KernelConfig::default())
}

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn terms(price: i64) -> Terms {
    Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
}

/// 注册客户/服务方/两名仲裁员。
fn roster(k: &mut Kernel) -> (AgentKeys, AgentKeys, AgentKeys, AgentKeys) {
    let client = agent(1);
    let provider = agent(2);
    let arb1 = agent(3);
    let arb2 = agent(4);
    k.register(&client, "client", &["summarize.zh"], Credits(50))
        .unwrap();
    k.register(&provider, "provider", &["summarize.zh"], Credits(50))
        .unwrap();
    k.register(&arb1, "arbiter.1", &[], Credits(20)).unwrap();
    k.register(&arb2, "arbiter.2", &[], Credits(20)).unwrap();
    (client, provider, arb1, arb2)
}

/// 跑到 EXECUTING 的真实协商。
fn executing(k: &mut Kernel, client: &AgentKeys, provider: &AgentKeys, price: i64) -> Negotiation {
    let mut n = Negotiation::open(k, client, provider, terms(price), 4).unwrap();
    n.accept(k, provider, client).unwrap();
    n.sign_contract(k, client, provider).unwrap();
    n.execute(k, client, provider).unwrap();
    n
}

#[test]
fn the_whole_arbitration_flow_keeps_the_ledger_conserved() {
    let mut k = kernel();
    let (client, provider, arb1, arb2) = roster(&mut k);
    let mut n = executing(&mut k, &client, &provider, 100);

    // 违约申诉 → 仲裁。
    let claim = n
        .report_breach(
            &mut k,
            &client,
            BreachKind::NonDelivery,
            EvidenceGrade::Verified,
            "nothing was delivered",
        )
        .unwrap();
    assert_eq!(n.phase(), Phase::Arbitration);

    // 立案：两名第三方仲裁员。
    let case = n.open_case(&[arb1.did(), arb2.did()], k.tick()).unwrap();
    assert_eq!(case.case_id().len(), 64);
    assert_eq!(case.claim, claim);
    assert!(n.case().is_some());

    let before = k.ledger().view();
    let price = n.contract().unwrap().terms.price;

    // 双仲裁员裁决。
    let ruling = n
        .case_mut()
        .unwrap()
        .rule(
            &ArbitrationPolicy::default(),
            &[&arb1, &arb2],
            price,
            "non-delivery proven by the breach claim",
            k.tick(),
        )
        .unwrap();
    assert_eq!(ruling.verdict, Verdict::Upheld);
    assert_eq!(ruling.slash, Credits(20));
    assert_eq!(ruling.compensate, Credits(50));
    assert_eq!(ruling.signatures.len(), 2);

    // 执行：罚没销毁 + 赔付过闸门。
    let at = k.tick();
    let report = n.case_mut().unwrap().enforce(&mut k, at).unwrap();
    assert_eq!(report.slashed, Credits(20));
    assert_eq!(report.compensated, Credits(50));
    assert!(report.conservation_ok);
    k.ledger().check_conservation().unwrap();

    let after = k.ledger().view();
    assert_eq!(after.minted, before.minted, "罚没是销毁，不是转移给谁");
    assert_eq!(
        after.slashed,
        before.slashed.checked_add(Credits(20)).unwrap()
    );
    let provider_after = &after.accounts[provider.did().as_str()];
    assert_eq!(provider_after.locked, Credits(30), "锁定质押 50 - 罚没 20");
    assert_eq!(
        provider_after.available,
        Credits(900),
        "可用 950（1000 创世 - 50 质押）- 赔付 50"
    );

    // 结案：ARBITRATION → SETTLED（双方联署）。
    let resolved = n.resolve(&mut k, &provider, &client).unwrap();
    assert_eq!(resolved.sigs.len(), 2);
    assert_eq!(n.phase(), Phase::Settled);

    // 归档与重放依然自洽（含条款授权 + 结案记录）。
    let bytes = n.archive().unwrap();
    let restored = au4a_negotiate::Journal::decode(&bytes).unwrap();
    assert_eq!(restored.encode().unwrap(), bytes);
    assert_eq!(restored.phase(), Phase::Settled);
}

#[test]
fn a_successful_delivery_settles_without_arbitration() {
    let mut k = kernel();
    let (client, provider, _, _) = roster(&mut k);
    let mut n = executing(&mut k, &client, &provider, 100);
    let before = k.ledger().view();

    let paid = n.settle(&mut k, &client, &provider).unwrap();
    assert_eq!(paid, Credits(100));
    assert_eq!(n.phase(), Phase::Settled);
    assert_eq!(k.ledger().balance(&client.did()).available, Credits(850));
    assert_eq!(
        k.ledger().balance(&provider.did()).available,
        Credits(1_050),
        "1000 创世 - 50 质押 + 100 收款"
    );
    k.ledger().check_conservation().unwrap();
    let after = k.ledger().view();
    assert_eq!(after.minted, before.minted);
    assert_eq!(after.slashed, before.slashed);

    // 结算后再想违约/结算都被拒（终态）。
    assert_eq!(
        n.settle(&mut k, &client, &provider),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        n.report_breach(
            &mut k,
            &client,
            BreachKind::LateDelivery,
            EvidenceGrade::Verified,
            "too late"
        ),
        Err(CoreError::InvalidKind)
    );
}

#[test]
fn settlement_must_follow_the_contract_roles() {
    let mut k = kernel();
    let (client, provider, _, _) = roster(&mut k);
    let mut n = executing(&mut k, &client, &provider, 100);
    // 反向付款（服务方付钱给客户）不是这份合约的语义。
    assert_eq!(
        n.settle(&mut k, &provider, &client),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(k.refusals().last().unwrap().1.code, RefusalCode::Conflict);
    assert_eq!(n.phase(), Phase::Executing);
    // 正常方向仍然可行。
    n.settle(&mut k, &client, &provider).unwrap();
    assert_eq!(n.phase(), Phase::Settled);
}

#[test]
fn parties_and_single_arbiters_cannot_decide_the_case() {
    let mut k = kernel();
    let (client, provider, arb1, arb2) = roster(&mut k);
    let mut n = executing(&mut k, &client, &provider, 100);
    n.report_breach(
        &mut k,
        &client,
        BreachKind::NonDelivery,
        EvidenceGrade::Verified,
        "x",
    )
    .unwrap();

    // 当事人当仲裁员：立案就被拒。
    assert_eq!(
        n.open_case(&[client.did(), arb1.did()], k.tick()),
        Err(CoreError::UnknownAgent)
    );
    assert!(n.case().is_none());
    // 只有一名仲裁员：拒。
    assert_eq!(
        n.open_case(&[arb1.did()], k.tick()),
        Err(CoreError::InvalidKind)
    );

    // 正名立案后，单仲裁员无法裁决。
    n.open_case(&[arb1.did(), arb2.did()], k.tick()).unwrap();
    let price = n.contract().unwrap().terms.price;
    assert_eq!(
        n.case_mut().unwrap().rule(
            &ArbitrationPolicy::default(),
            &[&arb1],
            price,
            "solo",
            k.tick()
        ),
        Err(CoreError::InvalidKind)
    );
    assert!(n.case().unwrap().ruling().is_none());

    // 名单外的 Agent 也不能裁决。
    let outsider = agent(9);
    assert_eq!(
        n.case_mut().unwrap().rule(
            &ArbitrationPolicy::default(),
            &[&arb1, &outsider],
            price,
            "outsider",
            k.tick()
        ),
        Err(CoreError::UnknownAgent)
    );
}

#[test]
fn the_evidence_gate_blocks_an_oversized_cpu_proto_payout() {
    let mut k = kernel();
    let (client, provider, arb1, arb2) = roster(&mut k);
    // 1000 微积分的合约 → cpu-proto 赔付 500，超过默认原型结算上限 100。
    let mut n = executing(&mut k, &client, &provider, 1_000);
    n.report_breach(
        &mut k,
        &client,
        BreachKind::UnderDelivery,
        EvidenceGrade::CpuProto,
        "half the work arrived",
    )
    .unwrap();
    n.open_case(&[arb1.did(), arb2.did()], k.tick()).unwrap();
    let price = n.contract().unwrap().terms.price;
    let ruling = n
        .case_mut()
        .unwrap()
        .rule(
            &ArbitrationPolicy::default(),
            &[&arb1, &arb2],
            price,
            "cpu-proto evidence",
            k.tick(),
        )
        .unwrap();
    assert_eq!(ruling.verdict, Verdict::Upheld);

    let before = k.ledger().view();
    let at = k.tick();
    assert_eq!(
        n.case_mut().unwrap().enforce(&mut k, at),
        Err(CoreError::InsufficientFunds),
        "证据闸门拒绝超限的原型赔付"
    );
    // 全有或全无：罚没也没执行。
    let after = k.ledger().view();
    assert_eq!(after.accounts, before.accounts);
    assert_eq!(after.slashed, before.slashed);
    assert_eq!(
        k.refusals().last().unwrap().1.code,
        RefusalCode::PolicyDenied
    );
    k.ledger().check_conservation().unwrap();
}

#[test]
fn enforcement_needs_a_dual_signed_ruling() {
    let mut k = kernel();
    let (client, provider, arb1, arb2) = roster(&mut k);
    let mut n = executing(&mut k, &client, &provider, 100);
    n.report_breach(
        &mut k,
        &client,
        BreachKind::WrongEvidence,
        EvidenceGrade::Verified,
        "wrong evidence grade",
    )
    .unwrap();
    let case = n.open_case(&[arb1.did(), arb2.did()], k.tick()).unwrap();
    assert!(case.ruling().is_none());
    let at = k.tick();
    assert_eq!(
        n.case_mut().unwrap().enforce(&mut k, at),
        Err(CoreError::NotSealed)
    );
    assert_eq!(n.phase(), Phase::Arbitration);
}

#[test]
fn scenario_settles_the_success_path() {
    let mut k = kernel();
    let summary = au4a_negotiate::scenario(&mut k).unwrap();
    assert_eq!(summary["phase"], "settled");
    assert_eq!(summary["settled_amount"], 95);
    assert_eq!(summary["conservation_ok"], true);
    assert_eq!(summary["steps"], 8);
    assert_eq!(summary["paths"], 2);
    assert_eq!(summary["breach"]["ruling"]["verdict"], "upheld");
    assert!(au4a_core::all_passed(&au4a_negotiate::self_check()));
}

#[test]
fn cases_are_content_addressed_per_claim() {
    let client = agent(11);
    let provider = agent(12);
    let arb1 = agent(13);
    let arb2 = agent(14);
    let mut contract =
        au4a_negotiate::Contract::draft(&client, &provider.did(), &terms(100), "s-1", 1).unwrap();
    contract.sign(&client).unwrap();
    contract.sign(&provider).unwrap();

    let claim_a = au4a_negotiate::BreachClaim::file(
        &client,
        &contract,
        BreachKind::NonDelivery,
        EvidenceGrade::Verified,
        "a",
        2,
    )
    .unwrap();
    let claim_b = au4a_negotiate::BreachClaim::file(
        &client,
        &contract,
        BreachKind::LateDelivery,
        EvidenceGrade::Verified,
        "b",
        2,
    )
    .unwrap();

    let case_a = ArbitrationCase::file(&claim_a, &contract, &[arb1.did(), arb2.did()], 3).unwrap();
    let case_b = ArbitrationCase::file(&claim_b, &contract, &[arb1.did(), arb2.did()], 3).unwrap();
    assert_ne!(case_a.case_id(), case_b.case_id(), "不同申诉 → 不同案件");
    assert_eq!(case_a.contract_hash, contract.hash);
    case_a.summary();
}
