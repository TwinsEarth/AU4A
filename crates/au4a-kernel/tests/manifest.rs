//! v1.0.10 轨道清单的集成测试（只用公开 API）。
//!
//! 「文档」这一版的可交付物是**可校验的数据**：十个版本齐备、顺序正确、字段完整、
//! 证据合法，并且能被节点与审计消费。

use au4a_core::all_passed;
use au4a_kernel::{
    manifest_is_valid, results_json, self_check, track_manifest, validate_manifest, VersionSpec,
    VERSION_ORDER,
};

#[test]
fn the_manifest_lists_exactly_the_official_ten_versions() {
    let manifest = track_manifest();
    assert_eq!(manifest.track, "1.0");
    assert_eq!(manifest.crate_name, "au4a-kernel");
    assert_eq!(manifest.range, "v1.0.1 → v1.0.10");
    assert_eq!(manifest.versions.len(), 10);
    let ids: Vec<&str> = manifest
        .versions
        .iter()
        .map(|v| v.version.as_str())
        .collect();
    assert_eq!(ids, VERSION_ORDER.to_vec());
    assert!(manifest.all_done());
    assert!(manifest.version("v1.0.10").is_some());
    assert!(manifest.version("v1.0.11").is_none());
}

#[test]
fn every_version_carries_goal_deliverables_interfaces_acceptance_and_evidence() {
    let manifest = track_manifest();
    for spec in &manifest.versions {
        assert!(!spec.goal.is_empty(), "{}", spec.version);
        assert!(spec.deliverables.len() >= 3, "{}", spec.version);
        assert!(spec.interfaces.len() >= 3, "{}", spec.version);
        assert!(spec.acceptance.len() >= 3, "{}", spec.version);
        assert_eq!(spec.status, "done");
        assert_eq!(spec.evidence.grade, "verified");
        assert!(spec.evidence.test_command.contains("au4a-kernel"));
        assert!(
            !spec.evidence.notes.is_empty(),
            "{} 证据必须写清来源",
            spec.version
        );
    }
    // 证据链是递增的：后面版本包含前面版本的全部测试。
    let mut last = 0;
    for spec in &manifest.versions {
        assert!(
            spec.evidence.tests_total >= last,
            "{} 测试数回退",
            spec.version
        );
        last = spec.evidence.tests_total;
    }
}

#[test]
fn validation_is_green_and_rejects_a_tampered_manifest() {
    let checks = validate_manifest(&track_manifest());
    assert!(
        all_passed(&checks),
        "{:?}",
        checks.iter().filter(|c| !c.passed).collect::<Vec<_>>()
    );
    assert!(manifest_is_valid());
    assert!(checks.iter().any(|c| c.name == "manifest.roundtrip"));

    // 篡改：把某个版本标记成未完成 → 校验必须报 fail。
    let mut broken = track_manifest();
    if let Some(spec) = broken.versions.first_mut() {
        spec.status = "planned".to_string();
    }
    let checks = validate_manifest(&broken);
    assert!(!all_passed(&checks));
    assert!(checks
        .iter()
        .any(|c| c.name == "manifest.fields" && !c.passed));

    // 篡改：版本号写错 → 顺序检查必须报 fail。
    let mut reordered = track_manifest();
    if let Some(spec) = reordered.versions.first_mut() {
        spec.version = "v9.9.9".to_string();
    }
    let checks = validate_manifest(&reordered);
    assert!(checks
        .iter()
        .any(|c| c.name == "manifest.order" && !c.passed));
}

#[test]
fn the_track_self_check_and_results_aggregate_the_manifest() {
    let checks = self_check();
    assert!(
        all_passed(&checks),
        "{:?}",
        checks.iter().filter(|c| !c.passed).collect::<Vec<_>>()
    );
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "track.wired",
        "host.audit",
        "registry.deterministic",
        "manifest.version_count",
        "manifest.order",
        "manifest.identity",
        "manifest.fields",
        "manifest.evidence",
        "manifest.test_growth",
        "manifest.roundtrip",
    ] {
        assert!(names.contains(&expected), "缺少自检项 {expected}");
    }

    let results = results_json().unwrap();
    assert_eq!(results["track"], "1.0");
    assert_eq!(results["versions"], 10);
    assert_eq!(results["manifest_valid"], true);
    assert_eq!(results["all_passed"], true);
    assert_eq!(
        results["version_ids"].as_array().map(|v| v.len()),
        Some(VERSION_ORDER.len())
    );
    assert!(results["tests_total_latest"].as_u64().unwrap_or(0) >= 130);

    // 版本规格类型是公开可构造的（其它轨道/节点可以复用）。
    let spec = VersionSpec {
        version: "v0.0.0".to_string(),
        title: "example".to_string(),
        goal: "example".to_string(),
        deliverables: vec!["x".to_string()],
        interfaces: vec!["y".to_string()],
        acceptance: vec!["z".to_string()],
        evidence: au4a_kernel::VersionEvidence::verified(1, "example"),
        status: "done".to_string(),
    };
    assert_eq!(spec.evidence.tests_total, 1);
}
