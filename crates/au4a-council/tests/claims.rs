//! v1.7.8 文档版——**文档 ↔ 代码一致性**的机器校验。
//!
//! 规则：`crate::claims::CLAIMS` 是能力清单的单一事实来源；每条能力声明的 `test`
//! 字段必须能在本 crate 的 `tests/*.rs` 或 `src/**/*.rs` 里找到对应的
//! `fn <名字>(`。找不到 → 这条测试失败，等于「文档吹了牛」。
//!
//! 同时校验 `docs/tracks/1.7.json` 的机器可读元数据：track 号、crate 名、
//! 每个小版本的 `evidence.grade` 合法（`verified`/`cpu-proto`/`unverified`），
//! 以及 `versions` 里的版本号与顺序。

use std::fs;
use std::path::{Path, PathBuf};

use au4a_core::EvidenceGrade;
use au4a_council::claims::{claims_are_well_formed, claims_json, summary, CLAIMS};

fn crate_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/crates/au4a-council
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    crate_root().join("..").join("..")
}

/// 把目录下所有 `.rs` 源码拼起来（不含 target）。
fn collect_sources(dir: &Path, out: &mut String) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().map(|n| n == "target").unwrap_or(false) {
                continue;
            }
            collect_sources(&path, out);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            if let Ok(text) = fs::read_to_string(&path) {
                out.push_str(&text);
                out.push('\n');
            }
        }
    }
}

fn all_crate_sources() -> String {
    let mut text = String::new();
    collect_sources(&crate_root().join("src"), &mut text);
    collect_sources(&crate_root().join("tests"), &mut text);
    collect_sources(&crate_root().join("examples"), &mut text);
    text
}

#[test]
fn the_claim_table_is_self_consistent() {
    assert!(claims_are_well_formed(), "能力清单自身不自洽");
    assert_eq!(CLAIMS.len(), 15);
    let text = summary();
    assert!(text.contains("0 unverified"), "{text}");
}

#[test]
fn every_claim_points_at_a_real_test() {
    let sources = all_crate_sources();
    assert!(sources.len() > 10_000, "源码扫描结果过小，疑似路径错误");
    let mut missing: Vec<&str> = Vec::new();
    for claim in CLAIMS {
        let needle = format!("fn {}(", claim.test);
        if !sources.contains(&needle) {
            missing.push(claim.test);
        }
    }
    assert!(
        missing.is_empty(),
        "以下能力声明指向了不存在的测试（文档与代码脱节）：{missing:?}"
    );
}

#[test]
fn verified_claims_have_tests_and_proto_claims_have_boundaries() {
    let sources = all_crate_sources();
    for claim in CLAIMS {
        match claim.grade {
            EvidenceGrade::Verified => {
                assert!(
                    sources.contains(&format!("fn {}(", claim.test)),
                    "{} 声称 verified，但其测试 {} 不存在",
                    claim.id,
                    claim.test
                );
            }
            EvidenceGrade::CpuProto => {
                assert!(!claim.note.is_empty(), "{} 是原型，必须写明边界", claim.id);
                assert!(
                    claim.note.contains("v1.8") || claim.note.contains("没有真实链"),
                    "{} 的原型说明必须点出「哪里不是真的」",
                    claim.id
                );
            }
            EvidenceGrade::Unverified => panic!("{} 不允许出现在清单里", claim.id),
        }
    }
}

#[test]
fn claims_json_matches_the_table() {
    let value = claims_json();
    assert_eq!(value["total"], CLAIMS.len());
    assert_eq!(value["unverified"], 0);
    let rows = value["claims"].as_array().expect("claims 数组");
    assert_eq!(rows.len(), CLAIMS.len());
    for (row, claim) in rows.iter().zip(CLAIMS.iter()) {
        assert_eq!(row["id"], claim.id);
        assert_eq!(row["test"], claim.test);
        assert_eq!(row["grade"], claim.grade.as_str());
        assert!(EvidenceGrade::parse(row["grade"].as_str().unwrap_or("")).is_some());
    }
}

#[test]
fn the_track_metadata_is_machine_readable_and_graded() {
    let path = repo_root().join("docs").join("tracks").join("1.7.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("读不到 {}: {err}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text).expect("1.7.json 必须是合法 JSON");
    assert_eq!(value["track"], "1.7");
    assert_eq!(value["crate"], "au4a-council");
    assert_eq!(value["range"], "v1.7.1 → v1.7.10");

    let versions = value["versions"].as_array().expect("versions 数组");
    assert!(versions.len() >= 8, "已完成的版本必须都写进元数据");
    let mut expected = Vec::new();
    for (i, row) in versions.iter().enumerate() {
        let version = row["version"].as_str().unwrap_or_default();
        expected.push(format!("v1.7.{}", i + 1));
        assert_eq!(version, expected[i], "版本号必须连续且顺序一致");
        let grade = row["evidence"]["grade"].as_str().unwrap_or_default();
        assert!(
            EvidenceGrade::parse(grade).is_some(),
            "{version} 的证据等级 {grade} 非法"
        );
        assert_eq!(row["status"], "done", "{version} 状态必须是 done");
        let passed = row["evidence"]["tests_passed"].as_u64().unwrap_or(0);
        let total = row["evidence"]["tests_total"].as_u64().unwrap_or(0);
        assert!(total > 0 && passed == total, "{version} 的测试数不自洽");
        assert!(!row["acceptance"].as_array().map(|a| a.is_empty()).unwrap_or(true));
    }
}

#[test]
fn the_markdown_doc_covers_every_version_section() {
    let path = repo_root().join("docs").join("tracks").join("1.7.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("读不到 {}: {err}", path.display()));
    for i in 1..=8 {
        let heading = format!("## v1.7.{i} ");
        assert!(text.contains(&heading), "缺少小节 {heading}");
    }
    // 每条 compile_fail 证据都必须在 md 里被引用（结构性证据不能被漏写）。
    assert!(text.contains("compile_fail"));
    assert!(text.contains("cpu-proto"));
}
