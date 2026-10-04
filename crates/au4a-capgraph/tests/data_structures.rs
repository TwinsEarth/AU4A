//! v1.1.1 集成测试：能力图的数据结构。
//!
//! 覆盖三条线：正常路径（构造/校验/内容寻址）、拒绝路径（非法度量与非法名字）、
//! 不变式（同一能力在任何构造顺序下得到同一指纹；度量全是整数）。

use au4a_capgraph::{Capability, FormatId, SkillId, BP_SCALE};
use au4a_core::{CoreError, Credits};
use au4a_kernel::{Kernel, KernelConfig};

fn skill(name: &str) -> SkillId {
    SkillId::new(name).expect("valid skill name")
}

fn cap(name: &str, price: i64) -> Capability {
    Capability::new(skill(name), Credits(price))
}

#[test]
fn a_declared_capability_round_trips_through_canonical_json() {
    let original = cap("translate.en-zh", 7)
        .with_formats(&["text/plain", "application/json"], &["text/plain"])
        .expect("valid formats")
        .with_latency(80, 200)
        .with_throughput(120)
        .with_load_bp(2_500)
        .with_reliability_bp(9_800);
    original.validate().expect("coherent metrics");

    let value = original.to_value().expect("serialisable");
    let restored = Capability::from_value(&value).expect("valid capability");
    assert_eq!(restored, original);
    assert_eq!(
        restored.fingerprint().expect("hash"),
        original.fingerprint().expect("hash")
    );
}

#[test]
fn floats_are_refused_at_the_data_structures_boundary() {
    // 价格是 Credits（i64）：JSON 里的 1.5 必须被拒，而不是被四舍五入。
    let mut value = cap("x", 3).to_value().expect("serialisable");
    value["price_per_unit"] = serde_json::json!(1.5);
    assert_eq!(Capability::from_value(&value), Err(CoreError::Encoding));

    // 即使有人绕过类型直接构造 JSON，规范 JSON 也不接受浮点。
    assert_eq!(
        au4a_core::canonicalize(&serde_json::json!({"latency_p99_ms": 12.5})),
        Err(CoreError::FloatForbidden)
    );
}

#[test]
fn incoherent_metrics_are_refused_at_ingest_time() {
    let mut value = cap("x", 3).to_value().expect("serialisable");
    value["latency_p99_ms"] = serde_json::json!(10);
    value["latency_p50_ms"] = serde_json::json!(500);
    assert_eq!(Capability::from_value(&value), Err(CoreError::Encoding));

    let mut value = cap("x", 3).to_value().expect("serialisable");
    value["current_load_bp"] = serde_json::json!(10_001);
    assert_eq!(Capability::from_value(&value), Err(CoreError::Encoding));

    // 空格式集合的能力无法参与路径规划，因此不允许存在。
    let mut value = cap("x", 3).to_value().expect("serialisable");
    value["supported_formats"] = serde_json::json!([]);
    assert_eq!(Capability::from_value(&value), Err(CoreError::Encoding));
}

#[test]
fn fingerprints_do_not_depend_on_construction_order() {
    // 同一份能力语义，格式集合的写入顺序不同，指纹必须相同。
    let a = cap("sentiment.analyze", 2)
        .with_formats(&["text/plain", "application/json"], &["application/json"])
        .expect("valid");
    let b = cap("sentiment.analyze", 2)
        .with_formats(&["application/json", "text/plain"], &["application/json"])
        .expect("valid");
    assert_eq!(
        a.fingerprint().expect("hash"),
        b.fingerprint().expect("hash")
    );

    // 任一度量变化 → 指纹变化（内容寻址必须能发现改动）。
    let c = b.clone().with_throughput(999);
    assert_ne!(
        a.fingerprint().expect("hash"),
        c.fingerprint().expect("hash")
    );
}

#[test]
fn names_are_validated_from_both_rust_and_json() {
    assert_eq!(SkillId::new("Bad Name"), Err(CoreError::InvalidKind));
    assert_eq!(
        FormatId::new("Application/JSON"),
        Err(CoreError::InvalidKind)
    );
    assert!(serde_json::from_value::<FormatId>(serde_json::json!("application/json")).is_ok());
    assert!(serde_json::from_value::<FormatId>(serde_json::json!("Application/JSON")).is_err());
}

#[test]
fn every_metric_is_bounded_and_integer() {
    let cap = cap("x", 1).with_load_bp(BP_SCALE);
    assert_eq!(cap.available_bp(), 0);
    assert_eq!(cap.effective_reliability_bp(), 0);
    let value = cap.to_value().expect("serialisable");
    for field in [
        "latency_p50_ms",
        "latency_p99_ms",
        "throughput_per_min",
        "current_load_bp",
        "price_per_unit",
        "reliability_bp",
    ] {
        assert!(value[field].as_i64().is_some(), "{field} 不是整数");
    }
}

#[test]
fn scenario_is_deterministic() {
    // scenario 会随小版本演进，但「同样输入给同样输出」是从 v1.1.1 起不可协商的契约。
    let mut first = Kernel::new(KernelConfig::default());
    let mut second = Kernel::new(KernelConfig::default());
    let a = au4a_capgraph::scenario(&mut first).expect("scenario runs");
    let b = au4a_capgraph::scenario(&mut second).expect("scenario runs");
    assert_eq!(a, b, "同样的输入必须给同样的输出（可重放）");
    assert!(a["version"].is_string());
    assert!(
        !first.observe().progress.is_empty(),
        "scenario 必须留下进度事件"
    );
}
