//! v1.3.9 集成测试：文档的能力清单必须可证伪。
//!
//! 这一版把「文档说了什么」变成代码里的常量表，再用两个方向的交叉检查钉死：
//! 每条声明都要有一个真实存在且通过的自检项；每个自检项也都要被某条声明认领。
//! 只要有人在文档里写一条代码里做不到的能力，或者写了能力却忘了写自检，这里就红。

use au4a_core::EvidenceGrade;
use au4a_state::{
    capability_manifest, capabilities, coverage_check, results_json, self_check, CAPABILITIES,
};

/// 声明里允许出现的模块前缀（api 字段必须落在这些模块里）。
const MODULES: [&str; 11] = [
    "snapshot",
    "store",
    "diff",
    "transfer",
    "signed",
    "recovery",
    "integrity",
    "udos",
    "perf",
    "chain",
    "capabilities",
];

#[test]
fn every_declared_check_exists_and_passes() {
    let report = coverage_check().unwrap();
    assert!(report.unknown_checks.is_empty(), "{report:#?}");
    assert!(report.failing_checks.is_empty(), "{report:#?}");
    assert!(report.bad_grades.is_empty(), "{report:#?}");
}

#[test]
fn no_self_check_is_left_unclaimed() {
    let report = coverage_check().unwrap();
    assert!(report.unclaimed_checks.is_empty(), "孤儿自检: {report:#?}");
    assert_eq!(report.capabilities, report.checks);
    assert_eq!(report.capabilities, CAPABILITIES.len());
    assert_eq!(self_check().len(), CAPABILITIES.len());
}

#[test]
fn every_declaration_points_into_a_real_module() {
    for capability in capabilities() {
        let module = capability.api.split("::").next().unwrap_or_default();
        assert!(
            MODULES.contains(&module),
            "{} 的 api 指向未知模块 {module}",
            capability.id
        );
        assert!(
            capability.api.len() > module.len(),
            "{} 的 api 没有具体到函数",
            capability.id
        );
        assert!(
            EvidenceGrade::parse(capability.grade).is_some(),
            "{} 的等级 {} 非法",
            capability.id,
            capability.grade
        );
        assert!(!capability.test.is_empty() && !capability.note.is_empty());
    }
}

#[test]
fn capability_ids_are_unique_and_versioned() {
    let mut ids: Vec<&str> = capabilities().iter().map(|c| c.id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), before, "能力 id 必须唯一");
    for capability in capabilities() {
        assert!(
            capability.since.starts_with("v1.3."),
            "{} 的 since 不合法: {}",
            capability.id,
            capability.since
        );
    }
    // 十个版本都应该至少被一条能力引用（版本号没有空转）。
    for minor in 1..=9 {
        let tag = format!("v1.3.{minor}");
        assert!(
            capabilities().iter().any(|c| c.since == tag),
            "版本 {tag} 没有任何能力声明"
        );
    }
}

#[test]
fn the_manifest_reports_grades_without_inflation() {
    let manifest = capability_manifest();
    let total = manifest["count"].as_u64().unwrap();
    let verified = manifest["verified"].as_u64().unwrap();
    let proto = manifest["cpu_proto"].as_u64().unwrap();
    let unverified = manifest["unverified"].as_u64().unwrap();
    assert_eq!(verified + proto + unverified, total);
    assert_eq!(unverified, 0, "未验证的能力不该出现在清单里");
    // 跨节点传输是原型，必须至少有一条 cpu-proto 标注。
    assert!(proto >= 1);
    let items = manifest["items"].as_array().unwrap();
    assert_eq!(items.len() as u64, total);
    for item in items {
        assert!(item["check"].as_str().unwrap().contains('.'));
        assert!(item["grade"] == "verified" || item["grade"] == "cpu-proto");
    }
}

#[test]
fn the_cpu_proto_capability_says_so_in_its_note() {
    let transfer = capabilities()
        .iter()
        .find(|c| c.id == "transfer.resumable")
        .expect("跨节点传输必须在清单里");
    assert_eq!(transfer.grade, "cpu-proto");
    assert!(
        transfer.note.contains("原型") && transfer.note.contains("无真实网络"),
        "原型必须在说明里写清楚: {}",
        transfer.note
    );
}

#[test]
fn results_json_carries_the_same_manifest() {
    let value = results_json().unwrap();
    assert_eq!(value["capabilities"]["count"], value["coverage"]["capabilities"]);
    assert_eq!(value["coverage"]["unclaimed_checks"], serde_json::json!([]));
    assert_eq!(value["coverage"]["unknown_checks"], serde_json::json!([]));
    assert_eq!(value["checks"], self_check().len());
}

#[test]
fn self_check_names_are_namespaced() {
    for check in self_check() {
        assert!(check.track == "1.3");
        assert!(
            check.name.contains('.'),
            "自检名应带命名空间: {}",
            check.name
        );
        assert!(!check.detail.is_empty(), "{} 没有证据说明", check.name);
    }
}

#[test]
fn coverage_check_is_deterministic() {
    assert_eq!(coverage_check().unwrap(), coverage_check().unwrap());
}
