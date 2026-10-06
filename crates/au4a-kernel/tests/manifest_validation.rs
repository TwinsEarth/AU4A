//! v2.4.0 回归测试：版本证据元数据的**反序列化校验**。
//!
//! 修复前 `VersionEvidence` / `VersionSpec` / `TrackManifest` 都是纯 `derive(Deserialize)`：
//! 外部 JSON 可以写出 `tests_passed = 9999 / tests_total = 0`、`grade = "definitely-verified"`。
//! 而"证据分级"正是本项目"不吹不藏"的载体——被伪造的证据等于把诚实机制关掉。

use au4a_kernel::{track_manifest, TrackManifest};

fn spec(version: &str, passed: u32, total: u32, grade: &str) -> String {
    format!(
        r#"{{"version":"{version}","title":"t","goal":"g","deliverables":[],"interfaces":[],"acceptance":[],"evidence":{{"test_command":"cargo test","tests_passed":{passed},"tests_total":{total},"grade":"{grade}","notes":""}},"status":"done"}}"#
    )
}

fn manifest(versions: &str) -> String {
    format!(
        r#"{{"track":"1.0","crate_name":"au4a-kernel","medium_title":"m","range":"v1.0.1 -> v1.0.10","owner":"kernel","versions":[{versions}]}}"#
    )
}

#[test]
fn a_well_formed_manifest_roundtrips() {
    let real = track_manifest();
    assert!(real.validate().is_ok(), "仓库自带清单必须合法");
    let text = serde_json::to_string(&real).unwrap();
    let back: TrackManifest = serde_json::from_str(&text).unwrap();
    assert_eq!(back, real);

    let one = manifest(&spec("v1.0.1", 3, 3, "verified"));
    assert!(serde_json::from_str::<TrackManifest>(&one).is_ok());
}

#[test]
fn forged_evidence_is_refused_by_serde() {
    // ① 自封的分级名（不在 verified / cpu-proto / unverified 之内）
    let bogus_grade = manifest(&spec("v1.0.1", 3, 3, "definitely-verified"));
    assert!(
        serde_json::from_str::<TrackManifest>(&bogus_grade).is_err(),
        "自封证据分级必须被拒"
    );

    // ② 通过数大于总数
    let impossible = manifest(&spec("v1.0.1", 9999, 3, "verified"));
    assert!(serde_json::from_str::<TrackManifest>(&impossible).is_err());

    // ③ 总数为零（等于没有证据）
    let no_tests = manifest(&spec("v1.0.1", 0, 0, "verified"));
    assert!(serde_json::from_str::<TrackManifest>(&no_tests).is_err());

    // ④ 版本号形状不对（缺 v 前缀）
    let bad_version = manifest(&spec("1.0.1", 3, 3, "verified"));
    assert!(serde_json::from_str::<TrackManifest>(&bad_version).is_err());

    // ⑤ 空清单 / 重复版本号
    assert!(serde_json::from_str::<TrackManifest>(&manifest("")).is_err());
    let dupes = manifest(&format!(
        "{},{}",
        spec("v1.0.1", 3, 3, "verified"),
        spec("v1.0.1", 3, 3, "verified")
    ));
    assert!(serde_json::from_str::<TrackManifest>(&dupes).is_err());
}
