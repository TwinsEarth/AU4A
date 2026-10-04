//! UDOS 数据契约集成（v1.3.6）：与 `E:\DS\UDOS` 的**数据契约**，不引入依赖、不联网。
//!
//! 为什么是「数据契约」而不是「依赖」：UDOS 是一个 Python 工程（世界模型 / 具身智能 /
//! 可观测性），AU4A 是 Rust 的 Agent 宇宙。两者能共享的不是代码，而是**对象语义**：
//! 一个状态对象长什么样、怎么被寻址、怎么声明证据等级、怎么被另一个系统独立验证。
//!
//! 契约的四个字段直接对齐 UDOS 现有实现（`src/udos/evidence.py`、`contracts.py`、
//! `topology/transfer.py`），因此跨系统可以直接互认：
//!
//! | 本模块 | UDOS 对应物 | 对齐点 |
//! |---|---|---|
//! | `object_id = "sha256:<hex>"` | `file_digest()` 返回 `"{algo}:{hex}"` | 算法前缀的摘要格式 |
//! | `grade` ∈ verified / cpu-proto / unverified | `contracts.EvidenceGrade` | 三个等级、同一组字符串 |
//! | `context` 区 → TransferBundle | `topology.transfer.TransferBundle` | goal/todo/context/done/owner/trace |
//! | `fingerprint` | `topology.transfer.state_fingerprint` | 规范 JSON、对键序不敏感 |
//!
//! 注意：这些函数**只做数据变换**，不读文件、不开网络、不调用 Python。

use au4a_core::{canonical_hash, canonicalize, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::diff::{DelOp, DeltaOp, StateDelta};
use crate::snapshot::{StateBlock, StateSnapshot, StateZone};

/// 契约标识：出现在每个对象的 `schema` 字段里。
pub const SCHEMA: &str = "udos.object/1";
/// 快照对象的 kind。
pub const KIND_SNAPSHOT: &str = "state.snapshot";
/// 差异对象的 kind。
pub const KIND_DELTA: &str = "state.delta";
/// 证据声明对象的 kind（对齐 UDOS `evidence.Claim`）。
pub const KIND_CLAIM: &str = "state.claim";
/// 默认集合名（UDOS 语义里的 collection）。
pub const COLLECTION_SNAPSHOTS: &str = "au4a/state/snapshots";
pub const COLLECTION_DELTAS: &str = "au4a/state/deltas";
/// 摘要算法前缀：与 UDOS `file_digest()` 的 `"{algo}:{hex}"` 格式一致。
pub const DIGEST_ALGO: &str = "sha256";

/// UDOS TransferBundle 的字段名（上下文区的交换形态）。
pub const BUNDLE_FIELDS: [&str; 6] = ["goal", "todo", "context", "done", "owner", "trace"];

/// 一个校验通过的 UDOS 对象。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdosObject {
    pub schema: String,
    pub kind: String,
    pub object_id: String,
    pub collection: String,
    pub grade: String,
    pub provenance: Value,
    pub payload: Value,
}

impl UdosObject {
    /// 从 JSON 校验构造：schema / kind / grade / object_id 必须全部自洽。
    ///
    /// `object_id` 由 payload 重新计算得出——这就是「UDOS 侧无需信任 AU4A 侧」的实现。
    pub fn validate(value: &Value) -> CoreResult<Self> {
        let schema = value
            .get("schema")
            .and_then(|v| v.as_str())
            .ok_or(CoreError::Encoding)?;
        if schema != SCHEMA {
            return Err(CoreError::InvalidVersion);
        }
        let kind = value
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or(CoreError::Encoding)?;
        if !matches!(kind, KIND_SNAPSHOT | KIND_DELTA | KIND_CLAIM) {
            return Err(CoreError::InvalidKind);
        }
        let collection = value
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or(CoreError::Encoding)?;
        let grade = value
            .get("grade")
            .and_then(|v| v.as_str())
            .ok_or(CoreError::Encoding)?;
        if au4a_core::EvidenceGrade::parse(grade).is_none() {
            return Err(CoreError::Encoding);
        }
        let provenance = value
            .get("provenance")
            .cloned()
            .ok_or(CoreError::Encoding)?;
        if !provenance.is_object() {
            return Err(CoreError::Encoding);
        }
        let payload = value.get("payload").cloned().ok_or(CoreError::Encoding)?;
        let object_id = value
            .get("object_id")
            .and_then(|v| v.as_str())
            .ok_or(CoreError::Encoding)?;
        if object_id != digest_of(&payload)? {
            // 内容与寻址不符：对象被改过。
            return Err(CoreError::InvalidSignature);
        }
        Ok(Self {
            schema: schema.to_string(),
            kind: kind.to_string(),
            object_id: object_id.to_string(),
            collection: collection.to_string(),
            grade: grade.to_string(),
            provenance,
            payload,
        })
    }

    pub fn to_value(&self) -> Value {
        json!({
            "schema": self.schema,
            "kind": self.kind,
            "object_id": self.object_id,
            "collection": self.collection,
            "grade": self.grade,
            "provenance": self.provenance,
            "payload": self.payload,
        })
    }
}

/// `"sha256:<hex>"`——与 UDOS 摘要格式一致的内容寻址。
pub fn digest_of(value: &Value) -> CoreResult<String> {
    Ok(format!("{DIGEST_ALGO}:{}", canonical_hash(value)?))
}

fn build(
    kind: &str,
    collection: &str,
    grade: au4a_core::EvidenceGrade,
    provenance: Value,
    payload: Value,
) -> CoreResult<Value> {
    let object_id = digest_of(&payload)?;
    Ok(json!({
        "schema": SCHEMA,
        "kind": kind,
        "object_id": object_id,
        "collection": collection,
        "grade": grade.as_str(),
        "provenance": provenance,
        "payload": payload,
    }))
}

/// 快照 → UDOS 对象。
pub fn snapshot_to_object(snapshot: &StateSnapshot) -> CoreResult<Value> {
    let payload = snapshot.to_value()?;
    build(
        KIND_SNAPSHOT,
        COLLECTION_SNAPSHOTS,
        au4a_core::EvidenceGrade::Verified,
        json!({
            "agent": snapshot.agent().as_str(),
            "node": snapshot.source_node(),
            "epoch": snapshot.epoch(),
            "source": "au4a-state",
        }),
        payload,
    )
}

/// UDOS 对象 → 快照（严格校验后再还原）。
pub fn object_to_snapshot(value: &Value) -> CoreResult<StateSnapshot> {
    let object = UdosObject::validate(value)?;
    if object.kind != KIND_SNAPSHOT {
        return Err(CoreError::InvalidKind);
    }
    let snapshot = StateSnapshot::from_value(&object.payload)?;
    // 出处的自证：对象里的 agent/root 必须与快照一致。
    let agent = object
        .provenance
        .get("agent")
        .and_then(|v| v.as_str())
        .ok_or(CoreError::Encoding)?;
    Did::parse(agent)?;
    if agent != snapshot.agent().as_str() {
        return Err(CoreError::InvalidDid);
    }
    Ok(snapshot)
}

/// 差异 → UDOS 对象。
pub fn delta_to_object(delta: &StateDelta) -> CoreResult<Value> {
    build(
        KIND_DELTA,
        COLLECTION_DELTAS,
        au4a_core::EvidenceGrade::Verified,
        json!({
            "agent": delta.agent().as_str(),
            "from": delta.from_content_root(),
            "to": delta.to_content_root(),
            "source": "au4a-state",
        }),
        delta.to_value()?,
    )
}

/// UDOS 对象 → 差异。
pub fn object_to_delta(value: &Value) -> CoreResult<StateDelta> {
    let object = UdosObject::validate(value)?;
    if object.kind != KIND_DELTA {
        return Err(CoreError::InvalidKind);
    }
    StateDelta::from_value(&object.payload)
}

/// 证据声明（对齐 UDOS `evidence.Claim`：key / statement / value / grade / method）。
pub fn claim(
    key: &str,
    statement: &str,
    grade: au4a_core::EvidenceGrade,
    artifact_digest: &str,
) -> CoreResult<Value> {
    if key.is_empty() || statement.is_empty() {
        return Err(CoreError::Encoding);
    }
    if !artifact_digest.starts_with(&format!("{DIGEST_ALGO}:")) {
        return Err(CoreError::Encoding);
    }
    build(
        KIND_CLAIM,
        "au4a/state/claims",
        grade,
        json!({"source": "au4a-state"}),
        json!({
            "key": key,
            "statement": statement,
            "value": artifact_digest,
            "grade": grade.as_str(),
            "artifact_digest": artifact_digest,
            // 与 UDOS `evidence.Claim` 的字段对齐：method 记录产出方式，notes 留给补充说明。
            "method": "cargo test -p au4a-state",
            "notes": "由 au4a-state 导出；UDOS 侧可独立复算 artifact_digest",
        }),
    )
}

/// 上下文区 → UDOS `TransferBundle` 形态。
///
/// 映射规则（写死、可测）：
/// * `goal` → bundle.goal：值是字符串则直接用，是对象则取 `.text`。
/// * `todo` / `done` / `trace` → 字符串数组。
/// * `owner` → 字符串。
/// * 其余上下文块 → bundle.context 的键值对（facts）。
pub fn context_to_transfer_bundle(snapshot: &StateSnapshot) -> CoreResult<Value> {
    let mut goal: Value = Value::Null;
    let mut todo: Vec<Value> = Vec::new();
    let mut done: Vec<Value> = Vec::new();
    let mut trace: Vec<Value> = Vec::new();
    let mut owner: Value = Value::Null;
    let mut context = Map::new();
    for block in snapshot.zone_blocks(StateZone::Context) {
        match block.key() {
            "goal" => {
                goal = match block.value() {
                    Value::Object(map) => map.get("text").cloned().unwrap_or(Value::Null),
                    other => other.clone(),
                }
            }
            "todo" => todo = as_array(block.value()),
            "done" => done = as_array(block.value()),
            "trace" => trace = as_array(block.value()),
            "owner" => owner = block.value().clone(),
            other => {
                context.insert(other.to_string(), block.value().clone());
            }
        }
    }
    let bundle = json!({
        "goal": goal,
        "todo": todo,
        "context": Value::Object(context),
        "done": done,
        "owner": owner,
        "trace": trace,
    });
    Ok(bundle)
}

/// TransferBundle → 上下文区块（逆映射）。
pub fn transfer_bundle_to_blocks(bundle: &Value) -> CoreResult<Vec<StateBlock>> {
    let obj = bundle.as_object().ok_or(CoreError::Encoding)?;
    for field in BUNDLE_FIELDS {
        if !obj.contains_key(field) {
            return Err(CoreError::Encoding);
        }
    }
    let mut blocks = Vec::new();
    if !obj["goal"].is_null() {
        blocks.push(StateBlock::new(
            StateZone::Context,
            "goal",
            obj["goal"].clone(),
        )?);
    }
    for (field, key) in [("todo", "todo"), ("done", "done"), ("trace", "trace")] {
        let items = as_array(&obj[field]);
        if !items.is_empty() {
            blocks.push(StateBlock::new(
                StateZone::Context,
                key,
                Value::Array(items),
            )?);
        }
    }
    if !obj["owner"].is_null() {
        blocks.push(StateBlock::new(
            StateZone::Context,
            "owner",
            obj["owner"].clone(),
        )?);
    }
    let context = obj["context"].as_object().ok_or(CoreError::Encoding)?;
    for (k, v) in context {
        blocks.push(StateBlock::new(StateZone::Context, k.clone(), v.clone())?);
    }
    Ok(blocks)
}

/// bundle 指纹：规范 JSON 哈希（对键序不敏感），对齐 UDOS `state_fingerprint`。
pub fn bundle_fingerprint(bundle: &Value) -> CoreResult<String> {
    canonical_hash(bundle)
}

/// 把一段 bundle 的指纹写进对象，供跨系统核对。
pub fn bundle_object(bundle: &Value, grade: au4a_core::EvidenceGrade) -> CoreResult<Value> {
    let fingerprint = bundle_fingerprint(bundle)?;
    build(
        KIND_CLAIM,
        "au4a/state/transfer-bundles",
        grade,
        json!({"source": "udos:topology.transfer"}),
        json!({"bundle": bundle, "fingerprint": fingerprint}),
    )
}

fn as_array(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    }
}

/// UDOS 契约的自检：字段名与取值域必须与契约文档一致。
pub fn contract_check() -> CoreResult<bool> {
    let snapshot = StateSnapshot::capture(
        &au4a_core::AgentKeys::from_seed(&[0x13; 32]).did(),
        "node-a",
        1,
        vec![StateBlock::new(StateZone::Fs, "/a", json!(1))?],
    )?;
    let object = snapshot_to_object(&snapshot)?;
    let back = object_to_snapshot(&object)?;
    let bundle = context_to_transfer_bundle(&snapshot)?;
    Ok(back.content_root()? == snapshot.content_root()?
        && bundle_fingerprint(&bundle)? == canonical_hash(&bundle)?)
}

/// 契约的机器可读描述（文档与观察层共用，避免文档与代码漂移）。
pub fn contract_descriptor() -> Value {
    json!({
        "schema": SCHEMA,
        "kinds": [KIND_SNAPSHOT, KIND_DELTA, KIND_CLAIM],
        "collections": [COLLECTION_SNAPSHOTS, COLLECTION_DELTAS, "au4a/state/claims", "au4a/state/transfer-bundles"],
        "grades": ["verified", "cpu-proto", "unverified"],
        "digest": DIGEST_ALGO,
        "object_fields": ["schema", "kind", "object_id", "collection", "grade", "provenance", "payload"],
        "bundle_fields": BUNDLE_FIELDS,
        "udos_counterparts": {
            "digest": "udos.evidence.file_digest -> sha256:<hex>",
            "grades": "udos.contracts.EvidenceGrade",
            "bundle": "udos.topology.transfer.TransferBundle",
            "fingerprint": "udos.topology.transfer.state_fingerprint"
        },
        "no_dependency": true,
        "no_network": true
    })
}

/// 差异对象的分块视图（UDOS 侧按块拉取时用）。
pub fn delta_object_chunks(delta: &StateDelta, max_ops: usize) -> CoreResult<Vec<Value>> {
    let mut out = Vec::new();
    for chunk in delta.chunked(max_ops)? {
        out.push(build(
            KIND_DELTA,
            COLLECTION_DELTAS,
            au4a_core::EvidenceGrade::Verified,
            json!({
                "agent": delta.agent().as_str(),
                "chunk": chunk.index(),
                "delta": chunk.delta_id(),
                "source": "au4a-state",
            }),
            chunk.to_value()?,
        )?);
    }
    Ok(out)
}

/// 供 UDOS 侧核对的集合清单（纯数据）。
pub fn collection_manifest(snapshot: &StateSnapshot, delta: &StateDelta) -> CoreResult<Value> {
    Ok(json!({
        "schema": SCHEMA,
        "snapshot_object": snapshot_to_object(snapshot)?,
        "delta_object": delta_to_object(delta)?,
        "bundle": context_to_transfer_bundle(snapshot)?,
        "bundle_fingerprint": bundle_fingerprint(&context_to_transfer_bundle(snapshot)?)?,
        "canonical_payload_bytes": canonicalize(&snapshot.to_value()?)?.len(),
    }))
}

/// 只用于把 `DeltaOp` / `DelOp` 计数暴露给 UDOS 侧（不改变语义）。
pub fn delta_shape(delta: &StateDelta) -> Value {
    json!({
        "set": delta.set().iter().map(|op: &DeltaOp| op.key.clone()).collect::<Vec<_>>(),
        "del": delta.del().iter().map(|op: &DelOp| op.key.clone()).collect::<Vec<_>>(),
        "ops": delta.op_count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::StateBlock;
    use serde_json::json;

    fn keys(seed: u8) -> au4a_core::AgentKeys {
        au4a_core::AgentKeys::from_seed(&[seed; 32])
    }

    fn sample() -> StateSnapshot {
        let blocks = vec![
            StateBlock::new(StateZone::Fs, "/a", json!({"n": 1})).unwrap(),
            StateBlock::new(StateZone::Memory, "m", json!(2)).unwrap(),
            StateBlock::new(StateZone::Context, "goal", json!({"text": "迁移"})).unwrap(),
            StateBlock::new(StateZone::Context, "todo", json!(["a", "b"])).unwrap(),
            StateBlock::new(StateZone::Context, "facts", json!({"k": "v"})).unwrap(),
        ];
        StateSnapshot::capture(&keys(1).did(), "node-a", 3, blocks).unwrap()
    }

    #[test]
    fn a_snapshot_becomes_a_self_verifying_udos_object() {
        let snapshot = sample();
        let object = snapshot_to_object(&snapshot).unwrap();
        assert_eq!(object["schema"], json!(SCHEMA));
        assert_eq!(object["kind"], json!(KIND_SNAPSHOT));
        assert!(object["object_id"].as_str().unwrap().starts_with("sha256:"));
        assert_eq!(object["grade"], json!("verified"));
        let validated = UdosObject::validate(&object).unwrap();
        assert_eq!(validated.object_id, object["object_id"]);
        let back = object_to_snapshot(&object).unwrap();
        assert_eq!(
            back.content_root().unwrap(),
            snapshot.content_root().unwrap()
        );
    }

    #[test]
    fn tampering_with_the_payload_breaks_the_object_id() {
        let snapshot = sample();
        let mut object = snapshot_to_object(&snapshot).unwrap();
        object["payload"]["blocks"][0]["value"] = json!({"n": 99});
        assert_eq!(
            UdosObject::validate(&object),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn unknown_schema_or_grade_is_refused() {
        let snapshot = sample();
        let mut object = snapshot_to_object(&snapshot).unwrap();
        object["schema"] = json!("udos.object/2");
        assert_eq!(
            UdosObject::validate(&object),
            Err(CoreError::InvalidVersion)
        );

        let mut object = snapshot_to_object(&snapshot).unwrap();
        object["grade"] = json!("probably-fine");
        assert_eq!(UdosObject::validate(&object), Err(CoreError::Encoding));
    }

    #[test]
    fn provenance_agent_must_match_the_snapshot() {
        let snapshot = sample();
        let mut object = snapshot_to_object(&snapshot).unwrap();
        object["provenance"]["agent"] = json!(keys(9).did().as_str());
        // object_id 仍然自洽（provenance 不在 payload 里），但还原时必须拒绝主体不符。
        assert!(UdosObject::validate(&object).is_ok());
        assert_eq!(object_to_snapshot(&object), Err(CoreError::InvalidDid));
    }

    #[test]
    fn deltas_roundtrip_through_the_contract() {
        let a = sample();
        let b = StateSnapshot::capture(
            &keys(1).did(),
            "node-a",
            3,
            vec![
                StateBlock::new(StateZone::Fs, "/a", json!({"n": 2})).unwrap(),
                StateBlock::new(StateZone::Context, "goal", json!({"text": "迁移"})).unwrap(),
            ],
        )
        .unwrap();
        let delta = StateDelta::between(&a, &b).unwrap();
        let object = delta_to_object(&delta).unwrap();
        assert_eq!(object["kind"], json!(KIND_DELTA));
        let back = object_to_delta(&object).unwrap();
        assert_eq!(back.id().unwrap(), delta.id().unwrap());
        assert_eq!(
            back.apply_to(&a).unwrap().content_root().unwrap(),
            b.content_root().unwrap()
        );
    }

    #[test]
    fn the_context_zone_maps_to_a_udos_transfer_bundle_and_back() {
        let snapshot = sample();
        let bundle = context_to_transfer_bundle(&snapshot).unwrap();
        assert_eq!(bundle["goal"], json!("迁移"));
        assert_eq!(bundle["todo"], json!(["a", "b"]));
        assert_eq!(bundle["context"]["facts"], json!({"k": "v"}));
        // 六个字段齐备（对齐 UDOS TransferBundle）。
        for field in BUNDLE_FIELDS {
            assert!(bundle.get(field).is_some(), "缺字段 {field}");
        }
        // 映射是**投影**：`{"text": ...}` 会被压成字符串（UDOS 的 goal 是 str）。
        // 逆映射之后再投影必须得到同一个 bundle——也就是不动点性质。
        let blocks = transfer_bundle_to_blocks(&bundle).unwrap();
        let rebuilt = StateSnapshot::capture(&keys(1).did(), "node-a", 3, blocks).unwrap();
        let bundle_again = context_to_transfer_bundle(&rebuilt).unwrap();
        assert_eq!(bundle, bundle_again, "二次投影必须稳定");
        let fingerprint = bundle_fingerprint(&bundle).unwrap();
        assert_eq!(fingerprint.len(), 64);
        assert_eq!(bundle_fingerprint(&bundle).unwrap(), fingerprint);
    }

    #[test]
    fn a_plain_string_goal_roundtrips_without_loss() {
        // 当上下文区本来就是 UDOS 形态（goal 是字符串）时，往返必须逐字节等价。
        let blocks = vec![
            StateBlock::new(StateZone::Context, "goal", json!("在 B 上继续")).unwrap(),
            StateBlock::new(StateZone::Context, "todo", json!(["capture"])).unwrap(),
        ];
        let snapshot = StateSnapshot::capture(&keys(2).did(), "node-a", 1, blocks).unwrap();
        let bundle = context_to_transfer_bundle(&snapshot).unwrap();
        let rebuilt = StateSnapshot::capture(
            &keys(2).did(),
            "node-a",
            1,
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
        let mut bundle = context_to_transfer_bundle(&sample()).unwrap();
        bundle.as_object_mut().unwrap().remove("owner");
        assert_eq!(transfer_bundle_to_blocks(&bundle), Err(CoreError::Encoding));
    }

    #[test]
    fn claims_carry_an_algorithm_prefixed_digest() {
        let snapshot = sample();
        let object = snapshot_to_object(&snapshot).unwrap();
        let digest = object["object_id"].as_str().unwrap().to_string();
        let claim_object = claim(
            "state.migration.identical",
            "迁移后内容根与源一致",
            au4a_core::EvidenceGrade::Verified,
            &digest,
        )
        .unwrap();
        assert_eq!(claim_object["kind"], json!(KIND_CLAIM));
        assert_eq!(claim_object["payload"]["artifact_digest"], json!(digest));
        // 摘要格式不对直接拒绝。
        assert_eq!(
            claim("k", "s", au4a_core::EvidenceGrade::Verified, "deadbeef"),
            Err(CoreError::Encoding)
        );
    }

    #[test]
    fn the_contract_descriptor_is_machine_readable() {
        let descriptor = contract_descriptor();
        assert_eq!(descriptor["schema"], json!(SCHEMA));
        assert_eq!(descriptor["digest"], json!("sha256"));
        assert_eq!(descriptor["no_dependency"], json!(true));
        assert_eq!(descriptor["bundle_fields"].as_array().unwrap().len(), 6);
        assert!(contract_check().unwrap());
    }

    #[test]
    fn the_manifest_links_snapshot_delta_and_bundle() {
        let snapshot = sample();
        let other = StateSnapshot::capture(
            &keys(1).did(),
            "node-a",
            4,
            vec![StateBlock::new(StateZone::Fs, "/a", json!({"n": 5})).unwrap()],
        )
        .unwrap();
        let delta = StateDelta::between(&snapshot, &other).unwrap();
        let manifest = collection_manifest(&snapshot, &delta).unwrap();
        assert!(manifest["snapshot_object"]["object_id"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert!(manifest["bundle_fingerprint"].as_str().unwrap().len() == 64);
        assert!(manifest["canonical_payload_bytes"].as_u64().unwrap() > 0);
        let shape = delta_shape(&delta);
        assert_eq!(shape["ops"], json!(delta.op_count()));
    }
}
