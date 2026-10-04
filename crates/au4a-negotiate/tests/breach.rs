//! v1.2.6 集成测试：违约申诉进入 ARBITRATION。
//!
//! 覆盖：正常路径（执行中违约 → 申诉 → 仲裁；合约签署后直接违约 → 仲裁）、
//! 拒绝路径（无合约申诉、第三方申诉、被篡改的合约、单方合约）、
//! 不变式（申诉载荷覆盖合约哈希与类型、转换记录由合约条款授权、归档仍可重放）。

use au4a_core::{AgentKeys, CoreError, Credits, EvidenceGrade, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{BreachClaim, BreachKind, Journal, Negotiation, Phase, Terms};

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

/// 走到 EXECUTING 的协商。
fn executing(k: &mut Kernel, s1: u8, s2: u8) -> (Negotiation, AgentKeys, AgentKeys) {
    let (a, b) = pair(k, s1, s2);
    let mut n = Negotiation::open(k, &a, &b, terms(100), 4).unwrap();
    n.accept(k, &b, &a).unwrap();
    n.sign_contract(k, &a, &b).unwrap();
    n.execute(k, &a, &b).unwrap();
    (n, a, b)
}

#[test]
fn a_breach_in_execution_opens_arbitration_with_a_signed_claim() {
    let mut k = kernel();
    let (mut n, a, b) = executing(&mut k, 1, 2);
    assert_eq!(n.phase(), Phase::Executing);

    let claim = n
        .report_breach(
            &mut k,
            &b,
            BreachKind::LateDelivery,
            EvidenceGrade::Verified,
            "delivered two rounds late",
        )
        .unwrap();

    assert_eq!(n.phase(), Phase::Arbitration);
    claim.verify().unwrap();
    assert_eq!(claim.claimant, b.did());
    assert_eq!(claim.accused, a.did(), "被诉方必须是合约的另一方");
    assert!(claim.accused_is_counterparty(n.contract().unwrap()));
    assert_eq!(n.breach().unwrap().kind, BreachKind::LateDelivery);
    assert_eq!(n.breach().unwrap().evidence, EvidenceGrade::Verified);

    // 一条可独立验签的 CONTRACT_BREACH 消息已经投递。
    let delivered = k.drain();
    let breach_env = delivered
        .iter()
        .find(|e| e.kind.as_str() == au4a_negotiate::kinds::CONTRACT_BREACH)
        .expect("breach message must be delivered");
    let parsed = au4a_negotiate::NegotiationMsg::from_env(breach_env).unwrap();
    assert_eq!(parsed.kind(), au4a_negotiate::kinds::CONTRACT_BREACH);
    assert_eq!(
        parsed.subject_hash(),
        Some(n.contract().unwrap().hash.as_str())
    );

    // 进入仲裁的那条转换：申诉方单签 + 双签合约条款授权。
    let last = n.machine().history().last().unwrap();
    assert_eq!(last.to, Phase::Arbitration);
    assert_eq!(last.sigs.len(), 1, "条款授权只需申诉方当场签名");
    assert_eq!(last.sigs[0].did, b.did());
    n.machine()
        .verify_history(n.parties())
        .expect("重放仍然成立");

    // 归档仍可逐字节重放（含条款授权记录）。
    let bytes = n.archive().unwrap();
    let restored = Journal::decode(&bytes).unwrap();
    assert_eq!(restored.encode().unwrap(), bytes);
    assert_eq!(restored.phase(), Phase::Arbitration);
}

#[test]
fn a_breach_right_after_signing_is_allowed_without_execution() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 3, 4);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
    n.accept(&mut k, &b, &a).unwrap();
    n.sign_contract(&mut k, &a, &b).unwrap();
    let claim = n
        .report_breach(
            &mut k,
            &a,
            BreachKind::NonDelivery,
            EvidenceGrade::CpuProto,
            "never started",
        )
        .unwrap();
    assert_eq!(n.phase(), Phase::Arbitration);
    assert_eq!(claim.kind, BreachKind::NonDelivery);
    assert_eq!(claim.evidence, EvidenceGrade::CpuProto);
}

#[test]
fn a_breach_without_a_contract_is_refused() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 5, 6);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
    // 先清掉开局报价等在途消息，再验证被拒的申诉不多发一条消息。
    assert_eq!(k.drain().len(), 1, "只有开局报价");
    // NEGOTIATING 阶段：没有合约，申诉被拒，状态不变。
    assert_eq!(
        n.report_breach(
            &mut k,
            &a,
            BreachKind::NonDelivery,
            EvidenceGrade::Verified,
            "x"
        ),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(n.phase(), Phase::Negotiating);
    assert!(n.breach().is_none());
    assert!(k.drain().is_empty(), "被拒的申诉不得发消息");

    // ACCEPTED（合约还没签）同样不行。
    n.accept(&mut k, &b, &a).unwrap();
    assert_eq!(
        n.report_breach(
            &mut k,
            &a,
            BreachKind::NonDelivery,
            EvidenceGrade::Verified,
            "x"
        ),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(n.phase(), Phase::Accepted);
}

#[test]
fn an_outsider_cannot_claim_against_someone_elses_contract() {
    let mut k = kernel();
    let (mut n, _, _) = executing(&mut k, 7, 8);
    let outsider = agent(9);
    k.register(&outsider, "outsider", &[], Credits(20)).unwrap();
    assert_eq!(
        n.report_breach(
            &mut k,
            &outsider,
            BreachKind::NonDelivery,
            EvidenceGrade::Verified,
            "not my business"
        ),
        Err(CoreError::UnknownAgent)
    );
    assert_eq!(n.phase(), Phase::Executing);
}

#[test]
fn a_single_signed_contract_cannot_authorize_arbitration() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 10, 11);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
    n.accept(&mut k, &b, &a).unwrap();
    n.sign_contract(&mut k, &a, &b).unwrap();

    // 破坏合约的双签（模拟只有一方签过字的合约）。
    let broken = {
        let mut c = n.contract().unwrap().clone();
        c.signatures.pop();
        c
    };
    assert_eq!(broken.verify(), Err(CoreError::NotSealed));
    assert_eq!(
        BreachClaim::file(
            &a,
            &broken,
            BreachKind::NonDelivery,
            EvidenceGrade::Verified,
            "x",
            1
        ),
        Err(CoreError::NotSealed)
    );
    // 合约本身仍然有效（我们只是复制出来破坏的）。
    n.contract().unwrap().verify().unwrap();
}

#[test]
fn the_breach_claim_binds_kind_evidence_and_note() {
    let mut k = kernel();
    let (mut n, a, _) = executing(&mut k, 12, 13);
    let claim = n
        .report_breach(
            &mut k,
            &a,
            BreachKind::WrongEvidence,
            EvidenceGrade::Unverified,
            "claimed verified but shipped cpu-proto",
        )
        .unwrap();
    let text = serde_json::to_string(&claim).unwrap();
    let decoded: BreachClaim = serde_json::from_str(&text).unwrap();
    assert_eq!(decoded, claim);
    assert_eq!(decoded.verify(), Ok(()));

    // 改一个字段就废。
    let mut tampered = claim.clone();
    tampered.kind = BreachKind::NonDelivery;
    assert_eq!(tampered.verify(), Err(CoreError::InvalidSignature));

    // 证据等级如实记录：unverified 不会因为是「申诉」就被抬高。
    assert_eq!(claim.evidence, EvidenceGrade::Unverified);
    assert!(!claim.evidence.settleable(Credits(1), Credits(1_000)));
}

#[test]
fn refusals_along_the_way_are_typed_and_not_misconduct() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 14, 15);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 1).unwrap();
    // 超轮数拒绝：PolicyDenied（非恶意）。
    n.counter(&mut k, &b, &a, terms(90)).unwrap();
    assert_eq!(
        n.counter(&mut k, &a, &b, terms(80)),
        Err(CoreError::Overflow)
    );
    assert_eq!(
        k.refusals().last().unwrap().1.code,
        RefusalCode::PolicyDenied
    );
    assert!(!k.refusals().last().unwrap().1.code.is_misconduct());
    // 协商仍能正常收尾并进入执行。
    n.accept(&mut k, &a, &b).unwrap();
    n.sign_contract(&mut k, &a, &b).unwrap();
    n.execute(&mut k, &b, &a).unwrap();
    assert_eq!(n.phase(), Phase::Executing);
    assert!(k.ledger().check_conservation().is_ok());
}
