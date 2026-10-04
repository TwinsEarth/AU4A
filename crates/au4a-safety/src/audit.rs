//! v1.5.10 审计：链完整性 + **状态 == 重放(事件链)** + 账本交叉核对。
//!
//! 审计要回答的不是「有没有 bug」，而是三个可以被别人独立复核的问题：
//!
//! 1. **链还完整吗？** 序号、前驱、哈希三项逐条复算（断链要能说出断在哪里）。
//! 2. **当前状态真的等于事件链的重放结果吗？** 这条检查的意义在于：它把
//!    「每次状态变更都写了链」从一句纪律变成一条**可被验算的等式**。任何绕过
//!    日志的改动都会让两边对不上。
//! 3. **账本和历史记录对得上吗？** 账本 `slashed`（历史罚没，含已归还）必须等于
//!    处罚记录里实际执行额的合计；守恒式必须成立。
//!
//! 审计只读：它接收 `&Kernel`，不改任何状态。发现的问题以**结构化代码**返回，
//! 而不是一段日志文本。

use std::collections::BTreeMap;

use au4a_core::{CoreError, CoreResult, Credits};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::appeal::Appeal;
use crate::arbitration::{ArbitrationVerdict, VerdictOutcome};
use crate::case::{Case, CaseStatus, ViolationReport};
use crate::chain::{verify_chain, ChainVerdict, SafetyEvent, SafetyEventKind};
use crate::notify::Subscription;
use crate::penalty::PenaltyRecord;

/// 审计发现的问题代码（封闭集合）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditCode {
    /// 链断裂（序号/前驱/哈希不一致）。
    ChainBroken,
    /// 当前状态与事件链的重放结果不一致。
    StateReplayMismatch,
    /// 事件/记录引用了一个不存在的案件。
    CaseMissing,
    /// 处罚记录的主体与案件主体不一致。
    SubjectMismatch,
    /// 账本历史罚没与处罚记录合计不一致。
    LedgerMismatch,
    /// 守恒式不成立。
    ConservationBroken,
    /// 通知指向了一个不存在的订阅。
    NotificationOrphan,
}

impl AuditCode {
    pub const ALL: [AuditCode; 7] = [
        AuditCode::ChainBroken,
        AuditCode::StateReplayMismatch,
        AuditCode::CaseMissing,
        AuditCode::SubjectMismatch,
        AuditCode::LedgerMismatch,
        AuditCode::ConservationBroken,
        AuditCode::NotificationOrphan,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AuditCode::ChainBroken => "chain_broken",
            AuditCode::StateReplayMismatch => "state_replay_mismatch",
            AuditCode::CaseMissing => "case_missing",
            AuditCode::SubjectMismatch => "subject_mismatch",
            AuditCode::LedgerMismatch => "ledger_mismatch",
            AuditCode::ConservationBroken => "conservation_broken",
            AuditCode::NotificationOrphan => "notification_orphan",
        }
    }
}

/// 一条发现：代码 + 结构化细节（禁止自由文本作为唯一证据）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFinding {
    pub code: AuditCode,
    pub detail: Value,
}

impl AuditFinding {
    pub fn new(code: AuditCode, detail: Value) -> Self {
        Self { code, detail }
    }
}

/// 审计报告。它是一个**值**：可以序列化、可以 diff、可以被其它轨道消费。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReport {
    pub ok: bool,
    pub events: usize,
    pub cases: usize,
    pub appeals: usize,
    pub penalties: usize,
    pub subscriptions: usize,
    pub notifications: usize,
    pub chain: ChainVerdict,
    /// 历史罚没合计（与账本 `slashed` 对应）。
    pub gross_slashed: Credits,
    /// 净罚没（扣除已归还）。
    pub net_slashed: Credits,
    /// 已归还合计。
    pub refunded: Credits,
    /// 账本里的历史罚没。
    pub ledger_slashed: Credits,
    pub findings: Vec<AuditFinding>,
}

impl AuditReport {
    pub fn codes(&self) -> Vec<AuditCode> {
        self.findings.iter().map(|f| f.code).collect()
    }

    pub fn has(&self, code: AuditCode) -> bool {
        self.findings.iter().any(|f| f.code == code)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

/// 重放出来的状态（只包含被写入事件链的部分；通知是派生投影，不在此列）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayState {
    pub cases: BTreeMap<String, Case>,
    pub appeals: BTreeMap<String, Appeal>,
    pub penalties: Vec<PenaltyRecord>,
    pub subscriptions: BTreeMap<String, Subscription>,
    /// 重放过程中发现的结构性问题（例如事件引用了不存在的案件）。
    pub findings: Vec<AuditFinding>,
}

/// 从事件链重放状态。**这是「状态可审计」的核心**：它不使用任何实时状态，
/// 只读事件；因此它可以被任何第三方独立执行。
pub(crate) fn replay(events: &[SafetyEvent]) -> CoreResult<ReplayState> {
    let mut state = ReplayState::default();
    for event in events {
        let payload = &event.payload;
        let case_id = payload
            .get("case")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        match event.kind {
            SafetyEventKind::Reported => {
                let report = ViolationReport::from_json(
                    payload.get("report").ok_or(CoreError::Encoding)?,
                )?;
                let mut case = Case::from_report(&report);
                case.status_seq = event.seq;
                state.cases.insert(case.id.clone(), case);
            }
            SafetyEventKind::Subscribed => {
                let subscription = Subscription::from_json(
                    payload.get("subscription").ok_or(CoreError::Encoding)?,
                )?;
                if !state.cases.contains_key(&subscription.case) {
                    state.findings.push(AuditFinding::new(
                        AuditCode::CaseMissing,
                        json!({"seq": event.seq, "case": subscription.case}),
                    ));
                }
                state.subscriptions.insert(subscription.id.clone(), subscription);
            }
            SafetyEventKind::Unsubscribed => {
                let id = payload
                    .get("subscription")
                    .and_then(|value| value.as_str())
                    .ok_or(CoreError::Encoding)?;
                state.subscriptions.remove(id);
            }
            SafetyEventKind::Appealed => {
                let appeal =
                    Appeal::from_json(payload.get("appeal").ok_or(CoreError::Encoding)?)?;
                let mut missing = false;
                match state.cases.get_mut(&case_id) {
                    Some(case) => {
                        case.status = CaseStatus::Appealed;
                        case.status_seq = event.seq;
                        case.appeals.push(appeal.id.clone());
                    }
                    None => missing = true,
                }
                if missing {
                    state.findings.push(AuditFinding::new(
                        AuditCode::CaseMissing,
                        json!({"seq": event.seq, "case": case_id}),
                    ));
                }
                state.appeals.insert(appeal.id.clone(), appeal);
            }
            SafetyEventKind::Penalized => {
                let record = PenaltyRecord::from_json(
                    payload.get("penalty").ok_or(CoreError::Encoding)?,
                )?;
                let mut missing = false;
                let mut mismatched = false;
                match state.cases.get_mut(&case_id) {
                    Some(case) => {
                        mismatched = case.subject != record.subject;
                        case.status = CaseStatus::Penalized;
                        case.status_seq = event.seq;
                        case.penalties.push(record.id.clone());
                    }
                    None => missing = true,
                }
                if missing {
                    state.findings.push(AuditFinding::new(
                        AuditCode::CaseMissing,
                        json!({"seq": event.seq, "case": case_id, "penalty": record.id}),
                    ));
                }
                if mismatched {
                    state.findings.push(AuditFinding::new(
                        AuditCode::SubjectMismatch,
                        json!({"seq": event.seq, "penalty": record.id}),
                    ));
                }
                state.penalties.push(record);
            }
            SafetyEventKind::Arbitrated => {
                let verdict = ArbitrationVerdict::from_json(
                    payload.get("verdict").ok_or(CoreError::Encoding)?,
                )?;
                let applied = Credits(
                    payload
                        .get("applied")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(0),
                );
                let mut missing = false;
                let mut derived: Option<PenaltyRecord> = None;
                match state.cases.get_mut(&case_id) {
                    Some(case) => {
                        case.status = CaseStatus::Arbitrated;
                        case.status_seq = event.seq;
                        case.outcome = Some(verdict.outcome);
                        case.arbitrated_at = Some(event.at);
                        if verdict.outcome == VerdictOutcome::Upheld {
                            // 裁决执行的处罚记录由事件本身派生（与运行时同一构造规则）。
                            let record = PenaltyRecord {
                                id: verdict.id.clone(),
                                case: case_id.clone(),
                                subject: case.subject.clone(),
                                sanction: verdict.sanction,
                                requested: verdict.amount,
                                applied,
                                arbiter: verdict.arbiter.clone(),
                                at: event.at,
                                reversed: false,
                            };
                            case.penalties.push(record.id.clone());
                            derived = Some(record);
                        }
                    }
                    None => missing = true,
                }
                if missing {
                    state.findings.push(AuditFinding::new(
                        AuditCode::CaseMissing,
                        json!({"seq": event.seq, "case": case_id, "verdict": verdict.id}),
                    ));
                }
                if let Some(record) = derived {
                    state.penalties.push(record);
                }
                if verdict.outcome == VerdictOutcome::Rejected {
                    for record in state.penalties.iter_mut() {
                        if record.case == case_id
                            && !record.reversed
                            && record.applied > Credits::ZERO
                        {
                            record.reversed = true;
                        }
                    }
                }
            }
        }
    }
    Ok(state)
}

/// 只审计事件链（不需要实例状态）：给外部验证者用的最小入口。
pub fn verify_journal(events: &[SafetyEvent]) -> CoreResult<(ChainVerdict, ReplayState)> {
    let verdict = verify_chain(events);
    let state = replay(events)?;
    Ok((verdict, state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::ViolationKind;
    use crate::evidence::{EvidenceKind, EvidenceRef};
    use crate::setup;
    use au4a_core::AgentKeys;

    fn report_event(seq: u64) -> SafetyEvent {
        let reporter = AgentKeys::from_seed(&[0xC1; 32]);
        let subject = AgentKeys::from_seed(&[0xC2; 32]);
        let evidence = EvidenceRef::commit(EvidenceKind::Transcript, &json!({"x": 1})).unwrap();
        let report = ViolationReport::new(
            reporter.did(),
            subject.did(),
            ViolationKind::Spam,
            evidence,
            3,
        )
        .sign(&reporter)
        .unwrap();
        SafetyEvent::seal(
            seq,
            3,
            SafetyEventKind::Reported,
            json!({"case": report.id, "report": report.to_json().unwrap()}),
            crate::chain::GENESIS_PREV,
        )
        .unwrap()
    }

    #[test]
    fn replaying_a_report_rebuilds_the_case() {
        let event = report_event(0);
        let case_id = event.payload["case"].as_str().unwrap().to_string();
        let state = replay(&[event.clone()]).unwrap();
        assert_eq!(state.cases.len(), 1);
        let case = &state.cases[&case_id];
        assert_eq!(case.status, CaseStatus::Reported);
        assert_eq!(case.status_seq, 0);
        assert!(state.findings.is_empty());
        let (verdict, replayed) = verify_journal(&[event]).unwrap();
        assert!(verdict.ok);
        assert_eq!(replayed.cases.len(), 1);
    }

    #[test]
    fn a_penalty_for_an_unknown_case_is_reported_not_hidden() {
        let record = PenaltyRecord {
            id: "p-1".to_string(),
            case: "missing-case".to_string(),
            subject: setup::keys(setup::ROLE_SUBJECT).did(),
            sanction: crate::penalty::SanctionKind::StakeSlash,
            requested: Credits(5),
            applied: Credits(5),
            arbiter: setup::keys(setup::ROLE_ARBITER).did(),
            at: 9,
            reversed: false,
        };
        let event = SafetyEvent::seal(
            0,
            9,
            SafetyEventKind::Penalized,
            json!({"case": "missing-case", "penalty": record.to_json().unwrap()}),
            crate::chain::GENESIS_PREV,
        )
        .unwrap();
        let state = replay(&[event]).unwrap();
        assert_eq!(state.penalties.len(), 1);
        assert_eq!(state.findings.len(), 1);
        assert_eq!(state.findings[0].code, AuditCode::CaseMissing);
    }

    #[test]
    fn audit_codes_are_unique_and_named() {
        assert_eq!(AuditCode::ALL.len(), 7);
        let mut names: Vec<&str> = AuditCode::ALL.iter().map(|c| c.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), AuditCode::ALL.len());
    }

    #[test]
    fn a_finding_serializes_with_a_code_and_structured_detail() {
        let finding = AuditFinding::new(AuditCode::LedgerMismatch, json!({"expected": 5, "actual": 3}));
        let value = serde_json::to_value(&finding).unwrap();
        assert_eq!(value["code"], json!("ledger_mismatch"));
        assert_eq!(value["detail"]["expected"], json!(5));
    }
}
