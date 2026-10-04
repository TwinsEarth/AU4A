//! v1.3.3 集成测试：快照签名与验证（内容寻址 + Ed25519）。
//!
//! 这一版要证明的核心只有一句：**签名有效 ≠ 这是我要的状态**。
//! 两条拒绝路径必须分开：篡改/替换是 `Unauthorized`（单次即恶意），
//! 过期是 `StaleEpoch`（竞争，单次只记警告）。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    sample_state, self_check, SignedSnapshot, SnapshotPolicy, StateBlock, StateSnapshot, StateZone,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn snap(keys: &AgentKeys, node: &str, epoch: u64) -> StateSnapshot {
    StateSnapshot::capture(&keys.did(), node, epoch, sample_state().unwrap()).expect("capture")
}

#[test]
fn a_snapshot_signed_by_its_agent_verifies_and_travels_as_a_frame() {
    let keys = agent(1);
    let signed = SignedSnapshot::sign(snap(&keys, "node-a", 5), &keys).unwrap();
    signed.verify().unwrap();
    assert_eq!(signed.signer(), &keys.did());
    assert_eq!(signed.sig().len(), 128);
    assert_eq!(signed.version(), au4a_state::SIGNED_VERSION);

    let frame = signed.to_frame().unwrap();
    let back = SignedSnapshot::from_frame(&frame).unwrap();
    assert_eq!(back.snapshot().root(), signed.snapshot().root());
}

#[test]
fn nobody_can_sign_for_someone_else() {
    let a = agent(2);
    let b = agent(3);
    assert_eq!(
        SignedSnapshot::sign(snap(&a, "node-a", 1), &b),
        Err(CoreError::InvalidSignature)
    );
}

#[test]
fn content_tampering_is_refused_at_deserialization() {
    let keys = agent(4);
    let signed = SignedSnapshot::sign(snap(&keys, "node-a", 1), &keys).unwrap();
    let mut value = signed.to_value().unwrap();
    value["snapshot"]["blocks"][0]["value"] = json!({"hacked": true});
    assert!(SignedSnapshot::from_value(&value).is_err());

    // root 也跟着改（攻击者试图让快照自洽）：仍然过不了签名这一关。
    let mut rehashed = signed.to_value().unwrap();
    rehashed["snapshot"]["root"] = json!("00".repeat(32));
    assert!(SignedSnapshot::from_value(&rehashed).is_err());
}

#[test]
fn a_replaced_snapshot_with_a_valid_signature_of_its_own_is_refused_by_policy() {
    let keys = agent(5);
    let authentic = SignedSnapshot::sign(snap(&keys, "node-a", 5), &keys).unwrap();
    let mut other_blocks = sample_state().unwrap();
    other_blocks.push(StateBlock::new(StateZone::Memory, "injected", json!(true)).unwrap());
    let swapped = SignedSnapshot::sign(
        StateSnapshot::capture(&keys.did(), "node-a", 5, other_blocks).unwrap(),
        &keys,
    )
    .unwrap();
    swapped.verify().unwrap();

    let policy = SnapshotPolicy::for_agent(keys.did())
        .expecting_content_root(authentic.snapshot().content_root().unwrap());
    assert_eq!(swapped.verify_policy(&policy), Err(CoreError::InvalidSignature));
    assert!(authentic.verify_policy(&policy).is_ok());
}

#[test]
fn a_stale_snapshot_is_refused_by_epoch_but_not_treated_as_forgery() {
    let keys = agent(6);
    let signed = SignedSnapshot::sign(snap(&keys, "node-a", 3), &keys).unwrap();
    let stale = SnapshotPolicy::for_agent(keys.did()).with_min_epoch(4);
    assert_eq!(signed.verify_policy(&stale), Err(CoreError::InvalidVersion));
    // 签名本身仍然有效：过期是竞争，不是伪造。
    signed.verify().unwrap();
}

#[test]
fn signing_is_deterministic_for_the_same_seed_and_state() {
    let keys = agent(7);
    let a = SignedSnapshot::sign(snap(&keys, "node-a", 5), &keys).unwrap();
    let b = SignedSnapshot::sign(snap(&keys, "node-a", 5), &keys).unwrap();
    assert_eq!(a.sig(), b.sig(), "Ed25519 对同一载荷必须给出同一签名");
    assert_eq!(a.to_value().unwrap(), b.to_value().unwrap());
}

#[test]
fn unsigned_and_wrong_version_payloads_are_refused() {
    let keys = agent(8);
    let signed = SignedSnapshot::sign(snap(&keys, "node-a", 1), &keys).unwrap();
    let mut unsigned = signed.to_value().unwrap();
    unsigned["sig"] = json!("");
    assert_eq!(SignedSnapshot::from_value(&unsigned), Err(CoreError::NotSealed));

    let mut future = signed.to_value().unwrap();
    future["version"] = json!(2);
    assert_eq!(SignedSnapshot::from_value(&future), Err(CoreError::InvalidVersion));
}

#[test]
fn a_forged_signer_field_is_refused() {
    let a = agent(9);
    let b = agent(10);
    let signed = SignedSnapshot::sign(snap(&a, "node-a", 1), &a).unwrap();
    let mut value = signed.to_value().unwrap();
    value["signer"] = json!(b.did().as_str());
    assert_eq!(SignedSnapshot::from_value(&value), Err(CoreError::InvalidSignature));
}

#[test]
fn only_a_policy_verified_snapshot_yields_the_verified_type() {
    let keys = agent(11);
    let signed = SignedSnapshot::sign(snap(&keys, "node-a", 1), &keys).unwrap();
    let verified = signed
        .verify_policy(&SnapshotPolicy::for_agent(keys.did()))
        .unwrap();
    assert_eq!(verified.signer(), &keys.did());
    assert_eq!(verified.snapshot().root(), signed.snapshot().root());
    // 验证过的快照可以直接落库：内容根不变。
    let inner = verified.into_snapshot();
    assert_eq!(inner.content_root().unwrap(), signed.snapshot().content_root().unwrap());
}

#[test]
fn tamper_and_stale_are_recorded_as_different_refusal_classes() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(12);
    kernel.register(&keys, "carrier", &["state.sign"], Credits(20)).unwrap();
    let signed = SignedSnapshot::sign(snap(&keys, "node-a", 1), &keys).unwrap();

    // 篡改 → unauthorized（单次即恶意）。
    kernel.refuse(&keys.did(), au4a_core::RefusalCode::Unauthorized, "forged signature");
    // 过期 → stale_epoch（竞争语义，单次只记警告）。
    kernel.refuse(&keys.did(), au4a_core::RefusalCode::StaleEpoch, "epoch below policy");
    assert_eq!(
        kernel.escalation_for(&keys.did(), au4a_core::RefusalCode::Unauthorized),
        au4a_core::Escalation::Quarantine
    );
    assert_eq!(
        kernel.escalation_for(&keys.did(), au4a_core::RefusalCode::StaleEpoch),
        au4a_core::Escalation::None
    );
    signed.verify().unwrap();
}

#[test]
fn self_check_is_green_and_covers_signature_paths() {
    let checks = self_check();
    assert_eq!(checks.len(), 9);
    assert!(au4a_core::all_passed(&checks), "{checks:?}");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"signature.tamper_refused"));
    assert!(names.contains(&"signature.identity_bound"));
}

#[test]
fn scenario_signs_the_migrated_state_and_verifies_it_on_the_target() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(13);
    kernel.register(&keys, "carrier", &["state.sign"], Credits(20)).unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["signature"]["verified"], json!(true));
    // 签名者必须是状态的主人（scenario 用本轨道的确定性身份）。
    assert_eq!(out["signature"]["signer"], out["agent"]);
    assert_eq!(out["signature"]["content_root"], out["target_content_root"]);
    assert_eq!(out["signature"]["tamper_refused"], json!(true));
    assert_eq!(out["signature"]["stale_refused"], json!(true));
    assert_eq!(out["signature"]["impersonation_refused"], json!(true));
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn scenario_records_both_misconduct_and_race_refusals() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["track"], json!("1.3"));
    let codes: Vec<String> = kernel
        .refusals()
        .iter()
        .map(|(_, r)| r.code.as_str().to_string())
        .collect();
    assert!(codes.contains(&"unauthorized".to_string()), "{codes:?}");
    assert!(codes.contains(&"stale_epoch".to_string()), "{codes:?}");
}

#[test]
fn verified_snapshot_survives_a_store_roundtrip_on_the_target() {
    let keys = agent(14);
    let signed = SignedSnapshot::sign(snap(&keys, "node-a", 7), &keys).unwrap();
    let policy = SnapshotPolicy::for_agent(keys.did())
        .expecting_content_root(signed.snapshot().content_root().unwrap())
        .with_min_epoch(7);
    let verified = signed.verify_policy(&policy).unwrap();
    let target = verified.into_snapshot().rebase("node-b", 8).unwrap();

    let mut store = au4a_state::MemoryStore::new();
    au4a_state::write_snapshot(&mut store, "live:", &target).unwrap();
    let back = au4a_state::read_snapshot(&store, "live:", &keys.did(), "node-b", 8).unwrap();
    assert_eq!(back.root(), target.root());
    assert_eq!(back.content_root().unwrap(), signed.snapshot().content_root().unwrap());
}

#[test]
fn a_snapshot_whose_agent_never_signed_it_cannot_pass_policy() {
    let owner = agent(15);
    let impostor = agent(16);
    let impostor_state = StateSnapshot::capture(
        &impostor.did(),
        "node-b",
        1,
        vec![StateBlock::new(StateZone::Fs, "/fake", json!(true)).unwrap()],
    )
    .unwrap();
    let signed = SignedSnapshot::sign(impostor_state, &impostor).unwrap();
    signed.verify().unwrap();
    // 用主人的策略去验冒名者的快照：签名有效，但主体不对 → 拒绝。
    let policy = SnapshotPolicy::for_agent(owner.did())
        .expecting_content_root(signed.snapshot().content_root().unwrap());
    assert_eq!(signed.verify_policy(&policy), Err(CoreError::InvalidDid));
}

#[test]
fn migrated_snapshot_root_and_signed_root_agree() {
    // 迁移链的最终断言：源签下的 content_root == 目标重建后的 content_root。
    let keys = agent(17);
    let source = snap(&keys, "node-a", 11);
    let signed = SignedSnapshot::sign(source.clone(), &keys).unwrap();
    let target = source.rebase("node-b", 12).unwrap();
    let policy = SnapshotPolicy::for_agent(keys.did())
        .expecting_content_root(target.content_root().unwrap())
        .with_min_epoch(11);
    let verified = signed.verify_policy(&policy).unwrap();
    assert_eq!(
        verified.content_root(),
        target.content_root().unwrap(),
        "内容寻址让「迁移等价」可被一条等式断言"
    );
    let _unused: Did = keys.did();
}
