//! v1.2.5 集成测试：合约签订接在多轮协商之后，哈希可锚定、可离线复核。
//!
//! 覆盖：正常路径（接受 → 双方签 → 双方各发 CONTRACT_SIGN → 锚定）、
//! 拒绝路径（未接受就签、单方合约、篡改后失效、非签署方锚定）、
//! 不变式（合约哈希 = 规范 JSON SHA-256、双方签名齐备、锚点可复核）。

use au4a_core::{AgentKeys, CoreError, Credits, EvidenceGrade};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{Contract, DualSigned, Negotiation, Phase, Terms, ANCHOR_EVENT};

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
    k.register(&a, "proposer", &["summarize.zh"], Credits(20)).unwrap();
    k.register(&b, "responder", &["summarize.zh"], Credits(20)).unwrap();
    (a, b)
}

#[test]
fn a_negotiated_deal_becomes_an_anchored_dual_signed_contract() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 1, 2);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(120), 4).unwrap();
    n.counter(&mut k, &b, &a, terms(105)).unwrap();
    n.accept(&mut k, &a, &b).unwrap();
    let contract = n.sign_contract(&mut k, &a, &b).unwrap();

    assert_eq!(n.phase(), Phase::ContractSigned);
    assert_eq!(contract.hash.len(), 64);
    assert_eq!(contract.hash, contract.compute_hash().unwrap());
    assert_eq!(contract.terms.price, Credits(105));
    assert_eq!(contract.signatures.len(), 2);
    assert!(contract.signed_by(&a.did()) && contract.signed_by(&b.did()));

    // 合约文本可离线带走、逐字节往返、解码时重新验签。
    let text = contract.encode().unwrap();
    let offline = Contract::decode(&text).unwrap();
    assert_eq!(offline, contract);
    assert_eq!(offline.encode().unwrap(), text);
    assert_eq!(offline.verify(), Ok(()));

    // 锚点在只读进度流里，且能离线复核。
    contract.verify_anchor().unwrap();
    assert!(k
        .observe()
        .progress
        .iter()
        .any(|p| p.kind == ANCHOR_EVENT && p.detail.contains(&contract.hash)));

    // 双签见证接口可用于后续违约仲裁。
    assert!(DualSigned::is_dual_signed(&offline));
    assert_eq!(DualSigned::contract_id(&offline), contract.id.as_str());
    assert_eq!(DualSigned::contract_hash(&offline), contract.hash.as_str());
    assert!(k.ledger().check_conservation().is_ok());
}

#[test]
fn a_single_signed_contract_is_never_a_valid_contract() {
    let a = agent(3);
    let b = agent(4);
    let mut c = Contract::draft(&a, &b.did(), &terms(100), "session-x", 1).unwrap();
    c.sign(&a).unwrap();
    assert_eq!(c.verify(), Err(CoreError::NotSealed));
    assert!(!c.is_dual_signed());
    assert!(c.summary()["signatures"] == serde_json::json!(1));

    // 单签合约无法通过 decode（decode 会重新验签）。
    let text = c.encode().unwrap();
    assert_eq!(Contract::decode(&text), Err(CoreError::NotSealed));
}

#[test]
fn tampering_a_signed_contract_invalidates_hash_and_signatures() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 5, 6);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
    n.accept(&mut k, &b, &a).unwrap();
    let contract = n.sign_contract(&mut k, &a, &b).unwrap();

    let mut tampered = contract.clone();
    tampered.terms.price = Credits(1);
    assert_eq!(tampered.verify(), Err(CoreError::InvalidSignature));
    assert_ne!(tampered.compute_hash().unwrap(), contract.hash);
    assert!(!tampered.is_dual_signed(), "篡改后不能再作为双签见证");

    let text = contract.encode().unwrap();
    let forged = text.replace("\"price\":100", "\"price\":1");
    assert_ne!(forged, text);
    assert_eq!(Contract::decode(&forged), Err(CoreError::InvalidSignature));
}

#[test]
fn signing_requires_the_accepted_phase_and_a_party() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 7, 8);
    let outsider = agent(9);
    k.register(&outsider, "outsider", &[], Credits(20)).unwrap();

    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
    // 协商中（NEGOTIATING）不能签。
    assert_eq!(n.sign_contract(&mut k, &a, &b), Err(CoreError::InvalidKind));
    n.accept(&mut k, &b, &a).unwrap();
    // 非当事人不能作为签署方。
    assert_eq!(
        n.sign_contract(&mut k, &a, &outsider),
        Err(CoreError::UnknownAgent)
    );
    assert_eq!(n.phase(), Phase::Accepted);
    // 当事人可以正常签。
    n.sign_contract(&mut k, &a, &b).unwrap();
    assert_eq!(n.phase(), Phase::ContractSigned);
}

#[test]
fn only_a_party_may_anchor_and_anchors_bind_the_exact_hash() {
    let mut k = kernel();
    let (a, b) = pair(&mut k, 10, 11);
    let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
    n.accept(&mut k, &b, &a).unwrap();
    let mut contract = n.sign_contract(&mut k, &a, &b).unwrap();

    let outsider = agent(12);
    assert_eq!(contract.anchor(&outsider, 5), Err(CoreError::UnknownAgent));

    // 应答方也可以锚定同一份哈希。
    let anchor = contract.anchor(&b, 6).unwrap();
    assert_eq!(anchor.contract_hash, contract.hash);
    contract.verify_anchor().unwrap();

    // 锚点被改动后复核失败。
    let mut moved = contract.clone();
    if let Some(target) = moved.anchor.as_mut() {
        target.at = 99;
    }
    assert_eq!(moved.verify_anchor(), Err(CoreError::InvalidSignature));
}

#[test]
fn scenario_signs_and_anchors_a_contract() {
    let mut k = kernel();
    let summary = au4a_negotiate::scenario(&mut k).unwrap();
    // 场景在 v1.2.7 已推进到 SETTLED；合约本身仍是双签且已锚定。
    assert_eq!(summary["phase"], "settled");
    assert_eq!(summary["contract"]["dual_signed"], true);
    assert_eq!(summary["contract"]["anchored"], true);
    assert_eq!(summary["contract"]["price"], 95);
    assert_eq!(summary["contract_anchored"], true);
    assert_eq!(summary["contract"]["hash"].as_str().unwrap().len(), 64);
    assert!(au4a_core::all_passed(&au4a_negotiate::self_check()));
}
