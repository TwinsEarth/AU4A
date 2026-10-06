//! v2.6.0 回归测试：签名**覆盖面**按产品决策收窄。
//!
//! 覆盖：`network_id + now + 结果面板 + 收益面板`（审计结论与资金账本 = 高风险）。
//! 不覆盖：**进度面板**（事件流体量大、风险低；其篡改不改变任何账或审计结论）。
//! 这些测试把"覆盖什么/不覆盖什么"钉成可执行断言。

use au4a_core::AgentKeys;
use au4a_kernel::{Kernel, KernelConfig, Observer, ObserverReport};

#[test]
fn signed_scope_covers_results_and_yield_only() {
    assert_eq!(
        ObserverReport::signed_routes(),
        ["results", "yield"],
        "签名覆盖面必须与产品决策一致：只覆盖结果与收益"
    );
}

#[test]
fn tampering_a_signed_panel_is_detected() {
    let k = Kernel::new(KernelConfig::default());
    let keys = AgentKeys::from_seed(&[42u8; 32]);
    let signed = Observer::report_signed(&k, &keys).expect("签发");
    signed.verify_provenance().expect("合法签名必须通过");

    // 改时刻
    let mut t1 = signed.clone();
    t1.now += 1;
    assert!(t1.verify_provenance().is_err(), "改 now 必须被发现");

    // 改「收益」面板的内容指纹（在范围内）
    let mut t2 = signed.clone();
    let yield_idx = t2
        .projections
        .iter()
        .position(|p| p.route == "yield")
        .unwrap();
    t2.projections[yield_idx].fingerprint = "00".repeat(32);
    assert!(t2.verify_provenance().is_err(), "改收益指纹必须被发现");

    // 改「收益」面板的载荷但保留原指纹（靠重算指纹发现）
    let mut t3 = signed.clone();
    t3.projections[yield_idx].payload = serde_json::json!({"ledger": {"total": 999999}});
    assert!(t3.verify_provenance().is_err(), "替换收益载荷必须被发现");

    // 换签发者
    let mut t4 = signed.clone();
    t4.node_did = Some(AgentKeys::from_seed(&[7u8; 32]).did().as_str().to_string());
    assert!(t4.verify_provenance().is_err(), "换签发者必须被发现");
}

#[test]
fn progress_panel_is_intentionally_out_of_scope() {
    let k = Kernel::new(KernelConfig::default());
    let keys = AgentKeys::from_seed(&[11u8; 32]);
    let signed = Observer::report_signed(&k, &keys).expect("签发");

    // 只改进度面板（不在签名范围内）→ 验签仍然通过。
    // 这是**刻意的**：逐条事件签名会让成本随事件数增长，而进度被改不改变账或审计结论。
    let mut t = signed.clone();
    let idx = t
        .projections
        .iter()
        .position(|p| p.route == "progress")
        .unwrap();
    t.projections[idx].payload = serde_json::json!({"event_count": 0, "events": []});
    t.verify_provenance()
        .expect("进度面板不在签名范围内，篡改它不应导致验签失败");

    // 但把「收益」也改了 → 立刻失败（说明范围不是"全都放行"）
    let mut t2 = signed.clone();
    let y = t2
        .projections
        .iter()
        .position(|p| p.route == "yield")
        .unwrap();
    t2.projections[y].fingerprint = "11".repeat(32);
    assert!(t2.verify_provenance().is_err());
}

#[test]
fn unsigned_report_is_not_sealed() {
    let k = Kernel::new(KernelConfig::default());
    let unsigned = Observer::report(&k);
    assert!(unsigned.node_did.is_none());
    assert_eq!(
        unsigned.verify_provenance(),
        Err(au4a_core::CoreError::NotSealed)
    );

    let mut half = unsigned.clone();
    half.node_did = Some(AgentKeys::from_seed(&[9u8; 32]).did().as_str().to_string());
    assert_eq!(
        half.verify_provenance(),
        Err(au4a_core::CoreError::NotSealed)
    );

    let mut junk = unsigned.clone();
    junk.node_did = Some(AgentKeys::from_seed(&[9u8; 32]).did().as_str().to_string());
    junk.sig = "ab".repeat(64);
    assert_eq!(
        junk.verify_provenance(),
        Err(au4a_core::CoreError::InvalidSignature)
    );

    let mut bad_did = unsigned.clone();
    bad_did.node_did = Some("did:au4a:zz".to_string());
    bad_did.sig = "ab".repeat(64);
    assert_eq!(
        bad_did.verify_provenance(),
        Err(au4a_core::CoreError::InvalidDid)
    );
}
