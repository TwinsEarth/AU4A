//! 安全委员会紧急通道（v1.7.9）。
//!
//! 安全事件等不了法定人数表决，所以安全委员会有一条**即时通道**：可以直接下发策略。
//! 但「即时」不等于「免检」——本模块把这条通道做成一个事后必须被治理确认的状态机：
//!
//! ```text
//! issue（安全委员会成员）──▶ Active ──确认达法定人数──▶ Confirmed（策略保留）
//!                              │
//!                              ├──否决达法定人数────▶ Rejected（策略回滚）
//!                              └──超过确认期限──────▶ Expired（策略回滚）
//! ```
//!
//! 三条不可协商的规则：
//!
//! * **只有安全委员会**能下发紧急指令（其他四类委员会调用即 `unauthorized`）；
//! * 指令必须带公开理由，并记录**下发前的策略值**，以便回滚；
//! * 确认与否决都是**委员会成员签名**的表决（不是人类操作、也不是下发者自己说了算），
//!   法定人数按全体在任委员计算：`f = ⌊(n-1)/3⌋`、`quorum = n - f`。

use std::collections::BTreeSet;

use au4a_core::{canonical_hash, canonicalize, AgentKeys, CoreError, CoreResult, Did, RefusalCode};
use au4a_kernel::Kernel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::committee::CommitteeKind;
use crate::proposal::Action;
use crate::{AgentIdentity, Council};

/// 紧急指令状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmergencyStatus {
    /// 已即时生效，等待治理事后确认。
    Active,
    /// 事后确认通过：策略保留。
    Confirmed,
    /// 事后被治理否决：策略已回滚。
    Rejected,
    /// 超过确认期限未确认：策略已回滚。
    Expired,
}

impl EmergencyStatus {
    /// 稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            EmergencyStatus::Active => "active",
            EmergencyStatus::Confirmed => "confirmed",
            EmergencyStatus::Rejected => "rejected",
            EmergencyStatus::Expired => "expired",
        }
    }

    /// 是否已终结。
    pub fn is_final(self) -> bool {
        !matches!(self, EmergencyStatus::Active)
    }

    /// 策略是否仍然保留。
    pub fn policy_kept(self) -> bool {
        matches!(self, EmergencyStatus::Active | EmergencyStatus::Confirmed)
    }
}

/// 一名委员对紧急指令的确认票。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmergencyApproval {
    /// 委员。
    pub voter: Did,
    /// 目标指令 id。
    pub directive: String,
    /// `true` 确认、`false` 否决。
    pub approve: bool,
    /// hex 签名。
    pub sig: String,
}

impl EmergencyApproval {
    /// 由委员私钥铸造确认票。
    pub fn cast(keys: &AgentKeys, directive: &str, approve: bool) -> CoreResult<Self> {
        let mut approval = Self {
            voter: keys.did(),
            directive: directive.to_string(),
            approve,
            sig: String::new(),
        };
        approval.sig = keys.sign_json(&approval.payload())?;
        Ok(approval)
    }

    /// 被签名的规范载荷。
    pub fn payload(&self) -> Value {
        json!({
            "voter": self.voter,
            "directive": self.directive,
            "approve": self.approve,
        })
    }

    /// 验签。
    pub fn verify(&self) -> CoreResult<()> {
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        let bytes = canonicalize(&self.payload())?;
        self.voter.verify(bytes.as_bytes(), &self.sig)
    }
}

/// 事后确认的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmergencyConfirmation {
    /// 确认票数。
    pub approvals: usize,
    /// 否决票数。
    pub rejections: usize,
    /// 参与人数。
    pub participation: usize,
    /// 全体在任委员数 `n`。
    pub n: usize,
    /// 法定人数 `n - f`。
    pub quorum: usize,
    /// 结论。
    pub status: EmergencyStatus,
    /// 确认时刻。
    pub at: u64,
    /// 是否已回滚策略。
    pub rolled_back: bool,
}

/// 一条紧急指令。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmergencyDirective {
    /// 内容地址。
    pub id: String,
    /// 下发委员会（必须是安全委员会）。
    pub committee: CommitteeKind,
    /// 下发者。
    pub issued_by: Did,
    /// 策略键。
    pub key: String,
    /// 策略值。
    pub value: i64,
    /// 公开理由。
    pub reason: String,
    /// 下发时刻。
    pub issued_at: u64,
    /// 确认期限（逻辑刻度）。
    pub confirm_deadline: u64,
    /// 下发前的策略值（`None` 表示此前没有这条策略），用于回滚。
    pub previous_value: Option<i64>,
    /// 状态。
    pub status: EmergencyStatus,
    /// 事后确认结果。
    pub confirmation: Option<EmergencyConfirmation>,
}

impl EmergencyDirective {
    /// 是否还在确认窗口内。
    pub fn awaiting_confirmation(&self) -> bool {
        self.status == EmergencyStatus::Active
    }
}

/// 下发一条紧急指令（**即时生效**，等待治理事后确认）。
pub fn issue(
    council: &mut Council,
    kernel: &mut Kernel,
    issuer: &AgentIdentity,
    key: &str,
    value: i64,
    reason: &str,
) -> CoreResult<EmergencyDirective> {
    // 1. 只有安全委员会持有紧急通道。
    let security = council.committee(CommitteeKind::Security);
    let seated = security
        .map(|c| c.has_member(issuer.did()))
        .unwrap_or(false);
    if !seated {
        kernel.refuse(
            issuer.did(),
            RefusalCode::Unauthorized,
            "only a seated security committee member may use the emergency channel",
        );
        return Err(CoreError::InvalidSignature);
    }
    if kernel.card(issuer.did()).is_none() {
        kernel.refuse(
            issuer.did(),
            RefusalCode::Unauthorized,
            "issuer is not a registered agent",
        );
        return Err(CoreError::UnknownAgent);
    }
    // 2. 策略与理由都要合法、公开。
    Action::SetPolicy {
        key: key.to_string(),
        value,
    }
    .validate()
    .map_err(|err| {
        kernel.refuse(
            issuer.did(),
            RefusalCode::Malformed,
            format!("illegal emergency policy: {err}"),
        );
        err
    })?;
    if reason.trim().is_empty() {
        kernel.refuse(
            issuer.did(),
            RefusalCode::Malformed,
            "emergency directive without a public reason",
        );
        return Err(CoreError::InvalidKind);
    }
    let at = council.tick();
    let id = canonical_hash(&json!({
        "committee": CommitteeKind::Security,
        "issuer": issuer.did(),
        "key": key,
        "value": value,
        "reason": reason,
        "issued_at": at,
    }))?;
    let previous_value = council.policy(key);
    // 3. 即时生效。
    council.set_policy(key.to_string(), value);
    let directive = EmergencyDirective {
        id: id.clone(),
        committee: CommitteeKind::Security,
        issued_by: issuer.did().clone(),
        key: key.to_string(),
        value,
        reason: reason.to_string(),
        issued_at: at,
        confirm_deadline: at.saturating_add(council.config().emergency_confirm_window),
        previous_value,
        status: EmergencyStatus::Active,
        confirmation: None,
    };
    kernel.emit(
        "council.emergency.issued",
        format!(
            "{} {}={} by {} :: {}",
            au4a_core::short_id(&id),
            key,
            value,
            au4a_core::short_id(issuer.did().as_str()),
            reason
        ),
    );
    council.record_event(
        "emergency.issued",
        &id,
        format!(
            "{key}={value} by {} :: {reason}",
            au4a_core::short_id(issuer.did().as_str())
        ),
    );
    council.store_emergency(directive.clone());
    Ok(directive)
}

/// 事后确认（或否决、或超期作废）一条紧急指令。
///
/// 确认票来自**任一委员会的在任委员**（一人一票、签名、去重），法定人数按全体在任委员计算。
pub fn confirm(
    council: &mut Council,
    kernel: &mut Kernel,
    directive_id: &str,
    approvals: &[EmergencyApproval],
) -> CoreResult<EmergencyDirective> {
    let directive = match council.emergency(directive_id) {
        Some(d) => d.clone(),
        None => return Err(CoreError::UnknownAgent),
    };
    if directive.status.is_final() {
        return Err(CoreError::InvalidKind);
    }
    let now = council.tick();
    // 全体在任委员：这是「治理确认」的范围。
    // 注意去重：同一 Agent 可以同时在多个委员会任职，但确认票一人一票。
    let all_members: Vec<Did> = council
        .committees()
        .flat_map(|c| c.member_dids())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let n = all_members.len();
    let f = n.saturating_sub(1) / 3;
    let quorum = n.saturating_sub(f);

    // 超期：政策回滚，指令作废。
    if now > directive.confirm_deadline {
        let rolled_back = rollback(council, &directive);
        let confirmation = EmergencyConfirmation {
            approvals: 0,
            rejections: 0,
            participation: 0,
            n,
            quorum,
            status: EmergencyStatus::Expired,
            at: now,
            rolled_back,
        };
        let updated =
            council.finish_emergency(directive_id, EmergencyStatus::Expired, confirmation.clone());
        kernel.emit(
            "council.emergency.expired",
            format!(
                "{} 超过确认期限，已回滚策略 {}",
                au4a_core::short_id(directive_id),
                directive.key
            ),
        );
        council.record_event(
            "emergency.expired",
            directive_id,
            format!(
                "deadline={} rolled_back={rolled_back}",
                directive.confirm_deadline
            ),
        );
        return updated;
    }

    let mut seen: BTreeSet<Did> = BTreeSet::new();
    let mut yes = 0usize;
    let mut no = 0usize;
    for approval in approvals {
        if approval.directive != directive_id {
            kernel.refuse(
                &approval.voter,
                RefusalCode::Malformed,
                "approval bound to another directive",
            );
            return Err(CoreError::InvalidKind);
        }
        if let Err(err) = approval.verify() {
            kernel.refuse(
                &approval.voter,
                RefusalCode::Unauthorized,
                format!("approval signature: {err}"),
            );
            return Err(err);
        }
        if !all_members.contains(&approval.voter) {
            kernel.refuse(
                &approval.voter,
                RefusalCode::Unauthorized,
                "approver is not a seated member",
            );
            return Err(CoreError::InvalidSignature);
        }
        if !seen.insert(approval.voter.clone()) {
            kernel.refuse(&approval.voter, RefusalCode::Conflict, "duplicate approval");
            return Err(CoreError::DuplicateAgent);
        }
        if approval.approve {
            yes += 1;
        } else {
            no += 1;
        }
    }

    let status = if yes >= quorum {
        EmergencyStatus::Confirmed
    } else if no >= quorum {
        EmergencyStatus::Rejected
    } else {
        // 票数不足：保持 Active，等待更多票或超期。
        let confirmation = EmergencyConfirmation {
            approvals: yes,
            rejections: no,
            participation: seen.len(),
            n,
            quorum,
            status: EmergencyStatus::Active,
            at: now,
            rolled_back: false,
        };
        council.record_event(
            "emergency.pending",
            directive_id,
            format!("yes={yes} no={no} quorum={quorum}：票数不足，保持 active"),
        );
        return council.finish_emergency(directive_id, EmergencyStatus::Active, confirmation);
    };

    let rolled_back = if status == EmergencyStatus::Rejected {
        rollback(council, &directive)
    } else {
        false
    };
    let confirmation = EmergencyConfirmation {
        approvals: yes,
        rejections: no,
        participation: seen.len(),
        n,
        quorum,
        status,
        at: now,
        rolled_back,
    };
    let updated = council.finish_emergency(directive_id, status, confirmation.clone());
    kernel.emit(
        "council.emergency.confirmed",
        format!(
            "{} {} yes={} no={} quorum={} rolled_back={}",
            au4a_core::short_id(directive_id),
            status.as_str(),
            yes,
            no,
            quorum,
            rolled_back
        ),
    );
    council.record_event(
        "emergency.confirmed",
        directive_id,
        format!(
            "{} yes={yes} no={no} quorum={quorum} rolled_back={rolled_back}",
            status.as_str()
        ),
    );
    updated
}

/// 回滚到指令下发前的策略值。
fn rollback(council: &mut Council, directive: &EmergencyDirective) -> bool {
    match directive.previous_value {
        Some(previous) => {
            council.set_policy(directive.key.clone(), previous);
            true
        }
        None => council.remove_policy(&directive.key),
    }
}

/// 紧急通道自检辅助：是否只有安全委员会下发的指令。
pub fn only_security_issued(council: &Council) -> bool {
    council
        .emergency_directives()
        .all(|d| d.committee == CommitteeKind::Security)
}

/// 一条指令是否「即时生效」：Active/Confirmed 时策略值应等于指令值；Rejected/Expired 时不应等于。
pub fn policy_matches_status(council: &Council, directive: &EmergencyDirective) -> bool {
    let current = council.policy(&directive.key);
    if directive.status.policy_kept() {
        current == Some(directive.value)
    } else {
        current != Some(directive.value) || directive.previous_value == Some(directive.value)
    }
}
