//! v1.5.9 机器可读契约：把「文档里写的能力」变成可以被程序检查的 JSON。
//!
//! 参考项目最贵的教训是**文档声称的能力在代码里不可达**。这里的对策很直接：
//! 把枚举、线格式名、事件种类、状态机、错误映射全部导出成一份 JSON，
//! 并且用测试断言它与代码里的 `ALL` 常量**逐个对齐**——文档漂移会被测试抓住。
//!
//! 这份 schema 也是跨轨道集成的接口说明书：`au4a-node` / `au4a-council` 只要读它，
//! 就知道安全 API 的消息类型、状态集合与拒绝码，不需要读本 crate 的源码。

use au4a_core::{CoreError, CoreResult};
use serde_json::{json, Value};

use crate::case::{CaseStatus, ViolationKind};
use crate::chain::{SafetyEventKind, GENESIS_PREV};
use crate::evidence::EvidenceKind;
use crate::penalty::SanctionKind;
use crate::permission::{DenialReason, Permission};
use crate::pmb::kinds;
use crate::setup;

/// 导出机器可读契约。
pub fn schema_json() -> CoreResult<Value> {
    Ok(json!({
        "track": crate::TRACK,
        "title": crate::TITLE,
        "range": crate::RANGE,
        "current": crate::CURRENT,
        "roles": {
            "service": setup::keys(setup::ROLE_SERVICE).did().as_str(),
            "reporter": setup::keys(setup::ROLE_REPORTER).did().as_str(),
            "subject": setup::keys(setup::ROLE_SUBJECT).did().as_str(),
            "arbiter": setup::keys(setup::ROLE_ARBITER).did().as_str(),
        },
        "pmb": {
            "kinds": {
                "query": kinds::SAFETY_QUERY,
                "report": kinds::SAFETY_REPORT,
                "appeal": kinds::SAFETY_APPEAL,
                "receipt": kinds::SAFETY_RECEIPT,
            },
            "report_body": {"report": "ViolationReport", "evidence": "任意 JSON（用于复算摘要）"},
            "appeal_body": {"appeal": "Appeal", "evidence": ["任意 JSON"]},
            "author_rule": "Envelope.from 必须等于 report.reporter / appeal.appellant",
            "max_frame_bytes": au4a_core::MAX_FRAME,
        },
        "chain": {
            "hash": "sha256(canonical_json({seq,at,kind,payload,prev})) 的小写 hex",
            "genesis_prev": GENESIS_PREV,
            "event_kinds": kinds_of(SafetyEventKind::ALL.iter().map(|k| k.as_str())),
            "breaks": ["seq_mismatch", "prev_mismatch", "hash_mismatch"],
        },
        "case": {
            "statuses": kinds_of(CaseStatus::ALL.iter().map(|s| s.as_str())),
            "violations": kinds_of(ViolationKind::ALL.iter().map(|v| v.as_str())),
            "outcomes": ["upheld", "rejected"],
        },
        "evidence": {
            "kinds": kinds_of(EvidenceKind::ALL.iter().map(|k| k.as_str())),
            "digest": "sha256(canonical_json(payload)) 的小写 hex（64 位）",
        },
        "penalty": {
            "sanctions": kinds_of(SanctionKind::ALL.iter().map(|s| s.as_str())),
            "cap_rule": "applied = min(requested, locked)（锁定质押封顶）",
            "refund_rule": "rejected 裁决按等额重新发行并重新锁定归还",
        },
        "permission": {
            "points": kinds_of(Permission::ALL.iter().map(|p| p.as_str())),
            "denial_codes": [
                DenialReason::AlreadyRegistered.code(),
                DenialReason::NotRegistered.code(),
                DenialReason::StakeBelowMinimum { required: au4a_core::Credits::ZERO, actual: au4a_core::Credits::ZERO }.code(),
                DenialReason::ArbiterOnly.code(),
            ],
            "stake_floor": "kernel.config().min_stake（查询与订阅免费）",
        },
        "errors": error_table(),
        "invariants": [
            "未确认的举报不改变任何余额或信誉（report 路径无账本写入）",
            "举报与申诉必须携带可复算的证据摘要（缺失/伪造一律拒绝）",
            "只有配置中受信仲裁者签名的契约能动账本",
            "每次状态变更写入哈希链事件；断链可定位到序号与原因",
            "通知只投递给订阅者，且不重放订阅前的历史",
        ],
    }))
}

fn kinds_of<'a>(items: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    items.collect()
}

/// `CoreError` 是冻结的错误集合，本轨道不新增变体；因此把「语义 → 变体 + 拒绝码」
/// 的映射显式导出，避免使用者猜。
fn error_table() -> Value {
    json!([
        {"situation": "reporter unregistered", "core_error": "unknown_agent", "refusal": "unauthorized"},
        {"situation": "self report", "core_error": "invalid_kind", "refusal": "policy_denied"},
        {"situation": "subject unregistered / unknown case / unknown subscription",
         "core_error": "unknown_agent", "refusal": "stale_epoch"},
        {"situation": "evidence digest missing or malformed",
         "core_error": "invalid_signature", "refusal": "policy_denied"},
        {"situation": "evidence payload missing",
         "core_error": "encoding", "refusal": "policy_denied"},
        {"situation": "evidence digest mismatch (forged)",
         "core_error": "invalid_signature", "refusal": "policy_denied"},
        {"situation": "third party appeal / third party unsubscribe",
         "core_error": "invalid_signature", "refusal": "unauthorized"},
        {"situation": "untrusted arbiter signature (order or verdict)",
         "core_error": "invalid_signature", "refusal": "unauthorized"},
        {"situation": "sanction and amount disagree",
         "core_error": "invalid_kind", "refusal": "policy_denied"},
        {"situation": "zero amount slash",
         "core_error": "zero_amount", "refusal": "policy_denied"},
        {"situation": "tampered envelope", "core_error": "invalid_signature", "refusal": "unauthorized"},
        {"situation": "unrelated message kind", "core_error": "invalid_kind", "refusal": "malformed"},
        {"situation": "ledger invariant broken",
         "core_error": "overflow", "refusal": "resource_exhausted"},
    ])
}

/// 契约摘要（供 `results_json` 与观察层使用）。
pub fn schema_summary() -> CoreResult<Value> {
    let schema = schema_json()?;
    let errors = schema["errors"].as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({
        "pmb_kinds": 4,
        "event_kinds": SafetyEventKind::ALL.len(),
        "case_statuses": CaseStatus::ALL.len(),
        "permissions": Permission::ALL.len(),
        "error_rules": errors,
        "chain": schema["chain"]["hash"],
    }))
}

/// 让 `CoreError` 的类型名进入契约（防止误以为可以新增变体）。
pub fn core_error_kind_name(err: &CoreError) -> &'static str {
    match err {
        CoreError::InvalidDid => "invalid_did",
        CoreError::InvalidSignature => "invalid_signature",
        CoreError::FloatForbidden => "float_forbidden",
        CoreError::Encoding => "encoding",
        CoreError::NegativeAmount => "negative_amount",
        CoreError::ZeroAmount => "zero_amount",
        CoreError::Overflow => "overflow",
        CoreError::InsufficientFunds => "insufficient_funds",
        CoreError::InsufficientStake => "insufficient_stake",
        CoreError::UnknownAgent => "unknown_agent",
        CoreError::DuplicateAgent => "duplicate_agent",
        CoreError::InvalidKind => "invalid_kind",
        CoreError::FrameTooLarge => "frame_too_large",
        CoreError::FrameTruncated => "frame_truncated",
        CoreError::NotSealed => "not_sealed",
        CoreError::InvalidVersion => "invalid_version",
        CoreError::ConservationViolated => "conservation_violated",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_is_json_and_covers_every_enum() {
        let schema = schema_json().unwrap();
        assert_eq!(schema["track"], json!("1.5"));
        assert_eq!(
            schema["chain"]["event_kinds"].as_array().unwrap().len(),
            SafetyEventKind::ALL.len()
        );
        assert_eq!(
            schema["case"]["statuses"].as_array().unwrap().len(),
            CaseStatus::ALL.len()
        );
        assert_eq!(
            schema["case"]["violations"].as_array().unwrap().len(),
            ViolationKind::ALL.len()
        );
        assert_eq!(
            schema["evidence"]["kinds"].as_array().unwrap().len(),
            EvidenceKind::ALL.len()
        );
        assert_eq!(
            schema["penalty"]["sanctions"].as_array().unwrap().len(),
            SanctionKind::ALL.len()
        );
        assert_eq!(
            schema["permission"]["points"].as_array().unwrap().len(),
            Permission::ALL.len()
        );
        assert_eq!(
            schema["permission"]["denial_codes"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn the_schema_names_match_the_code_and_the_frozen_primitive() {
        let schema = schema_json().unwrap();
        assert_eq!(
            schema["pmb"]["kinds"]["report"],
            json!(kinds::SAFETY_REPORT)
        );
        assert_eq!(
            schema["pmb"]["kinds"]["report"],
            json!(au4a_core::kinds::SAFETY_REPORT)
        );
        assert_eq!(schema["chain"]["genesis_prev"], json!(GENESIS_PREV));
        assert_eq!(
            schema["pmb"]["max_frame_bytes"],
            json!(au4a_core::MAX_FRAME)
        );

        // 每个事件种类都必须在 schema 里出现且只出现一次。
        for kind in SafetyEventKind::ALL {
            let listed = schema["chain"]["event_kinds"].as_array().unwrap();
            let hits = listed
                .iter()
                .filter(|v| v.as_str() == Some(kind.as_str()))
                .count();
            assert_eq!(hits, 1, "{} 在 schema 中应恰好出现一次", kind.as_str());
        }
        for point in Permission::ALL {
            let listed = schema["permission"]["points"].as_array().unwrap();
            let hits = listed
                .iter()
                .filter(|v| v.as_str() == Some(point.as_str()))
                .count();
            assert_eq!(hits, 1, "{} 在 schema 中应恰好出现一次", point.as_str());
        }
    }

    #[test]
    fn the_schema_carries_no_floats_and_the_error_table_is_complete() {
        // 规范 JSON 禁止浮点：schema 本身也必须能被规范化为字节。
        let schema = schema_json().unwrap();
        au4a_core::canonicalize(&schema).unwrap();
        let errors = schema["errors"].as_array().unwrap();
        assert!(errors.len() >= 10);
        for rule in errors {
            assert!(rule["situation"].is_string());
            assert!(rule["core_error"].is_string());
            assert!(rule["refusal"].is_string());
        }
        assert_eq!(schema["invariants"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn the_summary_counts_match_the_full_schema() {
        let summary = schema_summary().unwrap();
        assert_eq!(summary["pmb_kinds"], json!(4));
        assert_eq!(summary["event_kinds"], json!(SafetyEventKind::ALL.len()));
        assert_eq!(summary["permissions"], json!(Permission::ALL.len()));
        assert!(summary["error_rules"].as_u64().unwrap() >= 10);
    }

    #[test]
    fn core_error_names_are_stable_strings() {
        assert_eq!(
            core_error_kind_name(&CoreError::InvalidSignature),
            "invalid_signature"
        );
        assert_eq!(
            core_error_kind_name(&CoreError::UnknownAgent),
            "unknown_agent"
        );
        assert_eq!(core_error_kind_name(&CoreError::Encoding), "encoding");
    }
}
