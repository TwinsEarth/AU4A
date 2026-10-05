//! v2.4.0 回归测试：内核配置的**反序列化校验**。
//!
//! 修复前 `KernelConfig` 是纯 `derive(Deserialize)`，配置可被任意 JSON 注入：
//! `cpu_proto_settle_cap`（证据闸门上限）、`min_stake`（准入下限）、`max_skills`（公告轰炸防护）
//! 都直接决定"哪些行为能过闸门"，把结算上限写成极大值等于**关掉证据闸门的一部分**。

use au4a_kernel::{KernelConfig, MAX_CPU_PROTO_SETTLE_CAP, MAX_SKILLS_LIMIT};

fn json(std: i64, cap: i64, skills: usize) -> String {
    format!(
        r#"{{"network_id":"au4a-local","min_stake":{std},"genesis_mint":1000,"cpu_proto_settle_cap":{cap},"max_skills":{skills}}}"#
    )
}

#[test]
fn injected_settlement_cap_is_refused_by_serde() {
    // 把上限写到 i64::MAX：修复前会被接受，于是 cpu-proto 结算闸门几乎失效。
    assert!(
        serde_json::from_str::<KernelConfig>(&json(10, i64::MAX, 64)).is_err(),
        "结算上限超过硬上界必须被拒"
    );
    // 恰好超过硬上界一个单位也要拒
    assert!(serde_json::from_str::<KernelConfig>(&json(
        10,
        MAX_CPU_PROTO_SETTLE_CAP.get() + 1,
        64
    ))
    .is_err());
    // 边界值本身合法
    assert!(
        serde_json::from_str::<KernelConfig>(&json(10, MAX_CPU_PROTO_SETTLE_CAP.get(), 64)).is_ok()
    );
}

#[test]
fn other_injected_policy_thresholds_are_refused_too() {
    // 负质押下限
    assert!(serde_json::from_str::<KernelConfig>(&json(-1, 100, 64)).is_err());
    // 负结算上限
    assert!(serde_json::from_str::<KernelConfig>(&json(10, -1, 64)).is_err());
    // 能力数上限为 0（等于禁止声明能力）与超过硬上界（公告轰炸防护失效）
    assert!(serde_json::from_str::<KernelConfig>(&json(10, 100, 0)).is_err());
    assert!(serde_json::from_str::<KernelConfig>(&json(10, 100, MAX_SKILLS_LIMIT + 1)).is_err());
    // 空网络标识（会进入签名载荷）
    let empty_net = r#"{"network_id":"  ","min_stake":10,"genesis_mint":1000,"cpu_proto_settle_cap":100,"max_skills":64}"#;
    assert!(serde_json::from_str::<KernelConfig>(empty_net).is_err());
}

#[test]
fn valid_config_roundtrips_and_default_is_valid() {
    let default = KernelConfig::default();
    assert!(default.validate().is_ok(), "默认配置必须合法");
    let text = serde_json::to_string(&default).unwrap();
    let back: KernelConfig = serde_json::from_str(&text).unwrap();
    assert_eq!(back, default);
    // 线上 JSON 形状不变
    assert!(text.contains("\"cpu_proto_settle_cap\":100"));
    assert!(text.contains("\"max_skills\":64"));
}
