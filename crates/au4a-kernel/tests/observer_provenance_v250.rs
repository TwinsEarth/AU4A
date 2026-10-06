//! v2.5.0 回归测试：观察报告的**来源证明**。
//!
//! 修复前 `fingerprint()` 只证明"报告自洽"，不证明"它来自某个真实内核状态"——
//! 任何人都能造一份自洽报告且指纹通过。现在报告可以带节点 DID + 签名，
//! 验签覆盖 `network_id + now + 各投影指纹`，且**逐投影重算指纹**以封住
//! "替换 payload 但保留原 fingerprint"的伪造。

use au4a_core::AgentKeys;
use au4a_kernel::{Kernel, KernelConfig, Observer};

#[test]
fn signed_report_verifies_and_tampering_is_detected() {
    let k = Kernel::new(KernelConfig::default());
    let keys = AgentKeys::from_seed(&[42u8; 32]);
    let signed = Observer::report_signed(&k, &keys).expect("签发报告");
    assert_eq!(signed.node_did.as_deref(), Some(keys.did().as_str()));
    assert!(!signed.sig.is_empty());
    assert!(
        !signed.projections[0].fingerprint.is_empty(),
        "投影指纹必须真的算出来（不能是空串）"
    );
    signed.verify_provenance().expect("合法签名必须通过");

    let mut t1 = signed.clone();
    t1.now += 1;
    assert!(t1.verify_provenance().is_err(), "改时刻必须被验签发现");

    let mut t2 = signed.clone();
    t2.projections[0].fingerprint = "00".repeat(32);
    assert!(t2.verify_provenance().is_err(), "改投影指纹必须被验签发现");

    let mut t3 = signed.clone();
    t3.node_did = Some(AgentKeys::from_seed(&[7u8; 32]).did().as_str().to_string());
    assert!(t3.verify_provenance().is_err(), "换签发者必须被验签发现");

    let mut t4 = signed.clone();
    t4.projections[0].payload = serde_json::json!({"fake": true});
    assert!(
        t4.verify_provenance().is_err(),
        "替换 payload 必须被重算指纹发现"
    );
}

#[test]
fn unsigned_report_is_not_sealed() {
    let k = Kernel::new(KernelConfig::default());
    let unsigned = Observer::report(&k);
    assert!(unsigned.node_did.is_none(), "本地渲染默认不带来源");
    assert!(unsigned.sig.is_empty());
    assert_eq!(
        unsigned.verify_provenance(),
        Err(au4a_core::CoreError::NotSealed),
        "未签名必须与签名错误区分开"
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
