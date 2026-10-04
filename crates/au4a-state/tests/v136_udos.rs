//! v1.3.6 集成测试：UDOS 数据契约。
//!
//! 这一版**不引入任何依赖、不开网络、不调用 Python**。它证明的是：
//! AU4A 的状态对象可以被 UDOS 侧**独立验证**——摘要格式、证据等级、上下文 bundle
//! 与 UDOS 现有的 `evidence.py` / `contracts.py` / `topology/transfer.py` 对齐。

use au4a_core::{AgentKeys, CoreError, Credits, EvidenceGrade};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    bundle_fingerprint, contract_descriptor, context_to_transfer_bundle, delta_to_object,
    object_to_delta, object_to_snapshot, self_check, snapshot_to_object, transfer_bundle_to_blocks,
    udos, StateBlock, StateDelta, StateSnapshot, StateZone, UdosObject, UDOS_SCHEMA,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn sample(seed: u8) -> StateSnapshot {
    let blocks = vec![
        StateBlock::new(StateZone::Fs, "/work/notes.md", json!({"bytes": 42})).unwrap(),
        StateBlock::new(StateZone::Memory, "last_task", json!("translate")).unwrap(),
        StateBlock::new(StateZone::Context, "goal", json!("从 A 迁到 B")).unwrap(),
        StateBlock::new(StateZone::Context, "todo", json!(["capture", "transfer"])).unwrap(),
        StateBlock::new(StateZone::Context, "facts", json!({"budget": 100})).unwrap(),
    ];
    StateSnapshot::capture(&agent(seed).did(), "node-a", 5, blocks).unwrap()
}

#[test]
fn a_snapshot_exports_as_a_self_verifying_udos_object() {
    let snapshot = sample(1);
    let object = snapshot_to_object(&snapshot).unwrap();
    assert_eq!(object["schema"], json!(UDOS_SCHEMA));
    assert_eq!(object["kind"], json!("state.snapshot"));
    assert_eq!(object["collection"], json!("au4a/state/snapshots"));
    assert_eq!(object["grade"], json!("verified"));
    assert_eq!(object["provenance"]["agent"], json!(agent(1).did().as_str()));
    assert_eq!(object["provenance"]["node"], json!("node-a"));
    assert_eq!(object["provenance"]["epoch"], json!(5));
    // 摘要格式对齐 UDOS file_digest：`sha256:<hex>`。
    let object_id = object["object_id"].as_str().unwrap();
    assert!(object_id.starts_with("sha256:"));
    assert_eq!(object_id.len(), "sha256:".len() + 64);
    // 独立验证者只靠 payload 就能复算 object_id。
    assert_eq!(
        udos::digest_of(&object["payload"]).unwrap(),
        object_id.to_string()
    );
    let validated = UdosObject::validate(&object).unwrap();
    assert_eq!(validated.object_id, object_id);
}

#[test]
fn the_object_roundtrips_back_to_the_identical_snapshot() {
    let snapshot = sample(2);
    let object = snapshot_to_object(&snapshot).unwrap();
    let back = object_to_snapshot(&object).unwrap();
    assert_eq!(back.root(), snapshot.root());
    assert_eq!(back.content_root().unwrap(), snapshot.content_root().unwrap());
    assert_eq!(back.blocks(), snapshot.blocks());
}

#[test]
fn tampering_is_caught_by_the_object_id_alone() {
    let snapshot = sample(3);
    let object = snapshot_to_object(&snapshot).unwrap();

    let mut payload_changed = object.clone();
    payload_changed["payload"]["blocks"][0]["value"] = json!({"bytes": 43});
    assert_eq!(
        UdosObject::validate(&payload_changed),
        Err(CoreError::InvalidSignature)
    );

    let mut id_changed = object.clone();
    id_changed["object_id"] = json!(format!("sha256:{}", "00".repeat(32)));
    assert_eq!(UdosObject::validate(&id_changed), Err(CoreError::InvalidSignature));
}

#[test]
fn schema_kind_and_grade_are_closed_sets() {
    let object = snapshot_to_object(&sample(4)).unwrap();

    let mut wrong_schema = object.clone();
    wrong_schema["schema"] = json!("udos.object/2");
    assert_eq!(UdosObject::validate(&wrong_schema), Err(CoreError::InvalidVersion));

    let mut wrong_kind = object.clone();
    wrong_kind["kind"] = json!("state.whatever");
    assert_eq!(UdosObject::validate(&wrong_kind), Err(CoreError::InvalidKind));

    let mut wrong_grade = object.clone();
    wrong_grade["grade"] = json!("高可信");
    assert_eq!(UdosObject::validate(&wrong_grade), Err(CoreError::Encoding));

    let mut no_provenance = object;
    no_provenance["provenance"] = json!("node-a");
    assert_eq!(UdosObject::validate(&no_provenance), Err(CoreError::Encoding));
}

#[test]
fn a_forged_provenance_agent_is_refused_on_import() {
    let object = snapshot_to_object(&sample(5)).unwrap();
    let mut forged = object;
    forged["provenance"]["agent"] = json!(agent(99).did().as_str());
    // object_id 仍然自洽，但主体不符：导入方必须拒绝。
    assert!(UdosObject::validate(&forged).is_ok());
    assert_eq!(object_to_snapshot(&forged), Err(CoreError::InvalidDid));
}

#[test]
fn deltas_export_and_import_with_identical_ids() {
    let from = sample(6);
    let to = StateSnapshot::capture(
        &agent(6).did(),
        "node-a",
        5,
        vec![
            StateBlock::new(StateZone::Fs, "/work/notes.md", json!({"bytes": 84})).unwrap(),
            StateBlock::new(StateZone::Context, "goal", json!("已在 B 上")).unwrap(),
        ],
    )
    .unwrap();
    let delta = StateDelta::between(&from, &to).unwrap();
    let object = delta_to_object(&delta).unwrap();
    assert_eq!(object["kind"], json!("state.delta"));
    assert_eq!(object["collection"], json!("au4a/state/deltas"));
    assert_eq!(object["provenance"]["from"], json!(delta.from_content_root()));
    assert_eq!(object["provenance"]["to"], json!(delta.to_content_root()));

    let back = object_to_delta(&object).unwrap();
    assert_eq!(back.id().unwrap(), delta.id().unwrap());
    assert_eq!(
        back.apply_to(&from).unwrap().content_root().unwrap(),
        to.content_root().unwrap()
    );
    // 分块导出：每块也是自校验对象。
    for chunk_object in udos::delta_object_chunks(&delta, 1).unwrap() {
        assert!(UdosObject::validate(&chunk_object).is_ok());
    }
}

#[test]
fn the_context_zone_is_exchangeable_as_a_udos_transfer_bundle() {
    let snapshot = sample(7);
    let bundle = context_to_transfer_bundle(&snapshot).unwrap();
    assert_eq!(bundle["goal"], json!("从 A 迁到 B"));
    assert_eq!(bundle["todo"], json!(["capture", "transfer"]));
    assert_eq!(bundle["context"]["facts"], json!({"budget": 100}));
    assert_eq!(bundle["done"], json!([]));
    assert_eq!(bundle["owner"], json!(null));
    // 六个字段与 UDOS TransferBundle 同名。
    for field in ["goal", "todo", "context", "done", "owner", "trace"] {
        assert!(bundle.get(field).is_some(), "缺字段 {field}");
    }
    // 指纹是规范 JSON 哈希：对键序不敏感。
    let a = bundle_fingerprint(&bundle).unwrap();
    let reordered: serde_json::Value =
        serde_json::from_str(&format!("{{\"trace\":{},\"owner\":{},\"done\":{},\"context\":{},\"todo\":{},\"goal\":{}}}",
            bundle["trace"], bundle["owner"], bundle["done"], bundle["context"], bundle["todo"], bundle["goal"]))
        .unwrap();
    assert_eq!(bundle_fingerprint(&reordered).unwrap(), a);
    // 逆映射回到上下文区（本样例中 goal 是字符串，往返无损）。
    let rebuilt = StateSnapshot::capture(
        &agent(7).did(),
        "node-a",
        5,
        transfer_bundle_to_blocks(&bundle).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rebuilt.zone_root(StateZone::Context).unwrap(),
        snapshot.zone_root(StateZone::Context).unwrap()
    );
}

#[test]
fn a_bundle_missing_a_field_is_refused() {
    let mut bundle = context_to_transfer_bundle(&sample(8)).unwrap();
    bundle.as_object_mut().unwrap().remove("trace");
    assert_eq!(transfer_bundle_to_blocks(&bundle), Err(CoreError::Encoding));
}

#[test]
fn claims_reference_an_algorithm_prefixed_artifact_digest() {
    let snapshot = sample(9);
    let object = snapshot_to_object(&snapshot).unwrap();
    let digest = object["object_id"].as_str().unwrap().to_string();
    let claim = udos::claim(
        "state.snapshot.blocks",
        "三区共 5 块，内容寻址可复算",
        EvidenceGrade::Verified,
        &digest,
    )
    .unwrap();
    assert_eq!(claim["kind"], json!("state.claim"));
    assert_eq!(claim["payload"]["artifact_digest"], json!(digest));
    assert_eq!(claim["payload"]["method"], json!("cargo test -p au4a-state"));
    // 只接受 `sha256:` 前缀的摘要。
    assert_eq!(
        udos::claim("k", "s", EvidenceGrade::Verified, "md5:abcd"),
        Err(CoreError::Encoding)
    );
    assert_eq!(
        udos::claim("", "s", EvidenceGrade::Verified, &digest),
        Err(CoreError::Encoding)
    );
}

#[test]
fn the_contract_descriptor_pins_the_alignment() {
    let descriptor = contract_descriptor();
    assert_eq!(descriptor["schema"], json!(UDOS_SCHEMA));
    assert_eq!(descriptor["digest"], json!("sha256"));
    assert_eq!(descriptor["no_dependency"], json!(true));
    assert_eq!(descriptor["no_network"], json!(true));
    assert_eq!(descriptor["grades"].as_array().unwrap().len(), 3);
    assert_eq!(descriptor["bundle_fields"].as_array().unwrap().len(), 6);
    assert_eq!(
        descriptor["udos_counterparts"]["grades"],
        json!("udos.contracts.EvidenceGrade")
    );
    assert!(udos::contract_check().unwrap());
}

#[test]
fn evidence_grades_match_the_udos_names_exactly() {
    // AU4A 与 UDOS 的证据等级是同一组字符串，因此跨系统不需要翻译层。
    for grade in [
        EvidenceGrade::Verified,
        EvidenceGrade::CpuProto,
        EvidenceGrade::Unverified,
    ] {
        assert_eq!(EvidenceGrade::parse(grade.as_str()), Some(grade));
    }
    assert_eq!(EvidenceGrade::parse("cpu-proto"), Some(EvidenceGrade::CpuProto));
    assert_eq!(EvidenceGrade::parse("cpu_proto"), None);
}

#[test]
fn self_check_covers_the_udos_contract() {
    let checks = self_check();
    assert!(checks.len() >= 15, "{checks:?}");
    assert!(au4a_core::all_passed(&checks), "{checks:?}");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"udos.object_roundtrip"));
    assert!(names.contains(&"udos.contract_rejects_tampering"));
}

#[test]
fn scenario_exports_the_migrated_state_as_udos_objects() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(10);
    kernel.register(&keys, "carrier", &["state.export"], Credits(20)).unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["udos"]["schema"], json!(UDOS_SCHEMA));
    assert_eq!(out["udos"]["roundtrip_identical"], json!(true));
    assert_eq!(out["udos"]["tamper_refused"], json!(true));
    assert_eq!(out["udos"]["claim_grade"], json!("verified"));
    assert_eq!(out["udos"]["contract"]["no_dependency"], json!(true));
    assert!(out["udos"]["object_id"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert_eq!(
        out["udos"]["bundle_fingerprint"].as_str().unwrap().len(),
        64
    );
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn scenario_is_still_replayable_after_the_udos_export() {
    let mut k1 = Kernel::new(KernelConfig::default());
    let mut k2 = Kernel::new(KernelConfig::default());
    let a = au4a_state::scenario(&mut k1).unwrap();
    let b = au4a_state::scenario(&mut k2).unwrap();
    assert_eq!(a, b);
}
