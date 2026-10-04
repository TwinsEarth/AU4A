//! v1.5.1 权限查询：Agent 查询自己的**权限边界**。
//!
//! 设计要点：边界是**结构化**的，不是一段自由文本。
//! 每个权限点要么在 `allowed` 里，要么在 `denied` 里并带一个**机器可判定的拒绝码**
//! （`DenialReason`）。这样 Agent 可以自己写策略（"如果 ReportViolation 因质押不足被拒，
//! 先补质押"），而人类观察层读到的也是同一份结构，不存在「策略文档与实现不一致」的空间。
//!
//! 质押门槛是**唯一**的资源类门槛，且它是公开可复算的：
//! `required = stake_requirement(permission, kernel.config().min_stake)`。

use au4a_core::{CoreError, CoreResult, Credits, Did};
use au4a_kernel::Kernel;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::SafetyConfig;

/// 权限点。新增权限点是接口扩展，不改动已有语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// 自主注册（自证身份 + 自带质押）。
    Register,
    /// 发送 PMB 信封。
    SendMessage,
    /// 查询安全状态（权限边界 / 处罚记录）。
    QuerySafety,
    /// 举报他人违规（必须带可验证证据哈希）。
    ReportViolation,
    /// 对处罚提交申诉证据。
    AppealPenalty,
    /// 订阅案件状态变更。
    SubscribeCase,
    /// 参与结算。
    Settle,
    /// 出具仲裁裁决（只有配置里的仲裁者可以）。
    IssueVerdict,
}

impl Permission {
    pub const ALL: [Permission; 8] = [
        Permission::Register,
        Permission::SendMessage,
        Permission::QuerySafety,
        Permission::ReportViolation,
        Permission::AppealPenalty,
        Permission::SubscribeCase,
        Permission::Settle,
        Permission::IssueVerdict,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Register => "register",
            Permission::SendMessage => "send_message",
            Permission::QuerySafety => "query_safety",
            Permission::ReportViolation => "report_violation",
            Permission::AppealPenalty => "appeal_penalty",
            Permission::SubscribeCase => "subscribe_case",
            Permission::Settle => "settle",
            Permission::IssueVerdict => "issue_verdict",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Permission::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

/// 结构化的拒绝码。人类可读的解释由观察层从这些码生成，而不是由服务端写死。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DenialReason {
    /// 已注册的 Agent 不能重复注册。
    AlreadyRegistered,
    /// 未注册：除自助注册外的一切动作都不可达。
    NotRegistered,
    /// 锁定质押低于该权限所需下限。
    StakeBelowMinimum { required: Credits, actual: Credits },
    /// 只有配置中的仲裁者可以出具裁决。
    ArbiterOnly,
}

impl DenialReason {
    pub fn code(&self) -> &'static str {
        match self {
            DenialReason::AlreadyRegistered => "already_registered",
            DenialReason::NotRegistered => "not_registered",
            DenialReason::StakeBelowMinimum { .. } => "stake_below_minimum",
            DenialReason::ArbiterOnly => "arbiter_only",
        }
    }
}

/// 一个被拒绝的权限点。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeniedPermission {
    pub permission: Permission,
    pub reason: DenialReason,
}

/// 质押门槛的可复算视图。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StakeGate {
    pub permission: Permission,
    pub required: Credits,
    pub current: Credits,
    pub met: bool,
}

/// 权限边界——[`crate::scenario`] 与 PMB `safety.query` 回执共用的结构。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionBoundary {
    pub did: Did,
    /// 是否已在网络上注册。
    pub registered: bool,
    /// 当前可发起的权限点（升序）。
    pub allowed: Vec<Permission>,
    /// 当前被拒绝的权限点及其结构化原因。
    pub denied: Vec<DeniedPermission>,
    /// 所有正门槛的当前值/要求值。
    pub stake_gates: Vec<StakeGate>,
}

impl PermissionBoundary {
    /// 由**公开状态**推导边界：注册表（`kernel.card`）+ 锁定质押 + 创世额度 + 信任锚。
    /// 没有隐藏输入，因此任何 Agent 都能复算自己的边界，也能复算别人的。
    ///
    /// 注册的门槛资金来自创世额度：内核在余额为零时先铸后锁，所以「余额为零的新 DID
    /// 可以自助注册」不是特权，而是网络规则。边界必须如实反映这一点，否则 Agent 会
    /// 从权限查询里得到一个错误的自我认知。
    pub fn of(kernel: &Kernel, config: &SafetyConfig, did: &Did) -> Self {
        let registered = kernel.card(did).is_some();
        let account = kernel.ledger().balance(did);
        let locked = account.locked;
        let min_stake = kernel.config().min_stake;
        // 余额为零 → 注册时内核会发放创世额度；否则只能用现有可用余额。
        let register_funds = if account.available == Credits::ZERO {
            kernel.config().genesis_mint
        } else {
            account.available
        };

        let mut allowed = Vec::new();
        let mut denied = Vec::new();
        let mut stake_gates = Vec::new();

        for permission in Permission::ALL {
            let required = stake_requirement(permission, min_stake);
            let (current, met) = if permission == Permission::Register {
                (register_funds, register_funds >= required)
            } else {
                (locked, locked >= required)
            };
            if required > Credits::ZERO {
                stake_gates.push(StakeGate {
                    permission,
                    required,
                    current,
                    met,
                });
            }
            let reason = if permission == Permission::Register {
                if registered {
                    Some(DenialReason::AlreadyRegistered)
                } else if !met {
                    Some(DenialReason::StakeBelowMinimum {
                        required,
                        actual: current,
                    })
                } else {
                    None
                }
            } else if !registered {
                Some(DenialReason::NotRegistered)
            } else if permission == Permission::IssueVerdict {
                if config.is_arbiter(did) {
                    None
                } else {
                    Some(DenialReason::ArbiterOnly)
                }
            } else if !met {
                Some(DenialReason::StakeBelowMinimum {
                    required,
                    actual: current,
                })
            } else {
                None
            };
            match reason {
                Some(reason) => denied.push(DeniedPermission { permission, reason }),
                None => allowed.push(permission),
            }
        }

        Self {
            did: did.clone(),
            registered,
            allowed,
            denied,
            stake_gates,
        }
    }

    pub fn allows(&self, permission: Permission) -> bool {
        self.allowed.contains(&permission)
    }

    pub fn denial(&self, permission: Permission) -> Option<&DenialReason> {
        self.denied
            .iter()
            .find(|d| d.permission == permission)
            .map(|d| &d.reason)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 每个权限点的质押门槛。查询与订阅是**免费**的：读自己的权利不该被余额挡住。
pub fn stake_requirement(permission: Permission, min_stake: Credits) -> Credits {
    match permission {
        Permission::Register
        | Permission::SendMessage
        | Permission::ReportViolation
        | Permission::AppealPenalty
        | Permission::Settle => min_stake,
        Permission::QuerySafety | Permission::SubscribeCase | Permission::IssueVerdict => {
            Credits::ZERO
        }
    }
}

/// `safety.query` 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionQuery {
    /// 被查询的 Agent。v1.5.1 只允许查自己（见 [`PermissionQuery::is_self_query`]）。
    pub about: Did,
}

impl PermissionQuery {
    pub fn new(about: Did) -> Self {
        Self { about }
    }

    pub fn is_self_query(&self, from: &Did) -> bool {
        &self.about == from
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 一次权限查询的完整回答（结构化 JSON）。
pub fn query_permissions(
    kernel: &Kernel,
    config: &SafetyConfig,
    about: &Did,
) -> CoreResult<Value> {
    PermissionBoundary::of(kernel, config, about).to_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{setup, SafetyConfig};
    use au4a_core::{AgentKeys, CoreError};
    use au4a_kernel::{Kernel, KernelConfig};

    fn world() -> (Kernel, SafetyConfig, AgentKeys, AgentKeys) {
        let mut kernel = Kernel::new(KernelConfig::default());
        let reporter = setup::keys(setup::ROLE_REPORTER);
        let arbiter = setup::keys(setup::ROLE_ARBITER);
        let service = setup::keys(setup::ROLE_SERVICE);
        setup::ensure_agent(&mut kernel, &reporter, "reporter", &["audit"], Credits(20)).unwrap();
        setup::ensure_agent(&mut kernel, &arbiter, "arbiter", &["arbitrate"], Credits(20)).unwrap();
        let config = SafetyConfig::single_arbiter(service.did(), arbiter.did());
        (kernel, config, reporter, arbiter)
    }

    #[test]
    fn a_registered_agent_can_see_its_allowed_and_denied_sets() {
        let (kernel, config, reporter, _) = world();
        let boundary = PermissionBoundary::of(&kernel, &config, &reporter.did());
        assert!(boundary.registered);
        for p in [
            Permission::SendMessage,
            Permission::QuerySafety,
            Permission::ReportViolation,
            Permission::AppealPenalty,
            Permission::SubscribeCase,
            Permission::Settle,
        ] {
            assert!(boundary.allows(p), "{} 应可达", p.as_str());
            assert!(boundary.denial(p).is_none());
        }
        // 已注册者不能重复注册；非仲裁者不能出具裁决。
        assert_eq!(
            boundary.denial(Permission::Register),
            Some(&DenialReason::AlreadyRegistered)
        );
        assert_eq!(
            boundary.denial(Permission::IssueVerdict),
            Some(&DenialReason::ArbiterOnly)
        );
    }

    #[test]
    fn an_arbiter_can_issue_verdicts_and_a_stranger_cannot() {
        let (kernel, config, reporter, arbiter) = world();
        let arbiter_boundary = PermissionBoundary::of(&kernel, &config, &arbiter.did());
        assert!(arbiter_boundary.allows(Permission::IssueVerdict));
        let stranger = AgentKeys::from_seed(&[0x77; 32]);
        let stranger_boundary = PermissionBoundary::of(&kernel, &config, &stranger.did());
        assert_eq!(
            stranger_boundary.denial(Permission::IssueVerdict),
            Some(&DenialReason::NotRegistered)
        );
        assert!(!PermissionBoundary::of(&kernel, &config, &reporter.did())
            .allows(Permission::IssueVerdict));
    }

    #[test]
    fn an_unregistered_did_may_only_self_register() {
        let (kernel, config, _, _) = world();
        let fresh = AgentKeys::from_seed(&[0x42; 32]);
        let boundary = PermissionBoundary::of(&kernel, &config, &fresh.did());
        assert!(!boundary.registered);
        assert_eq!(boundary.allowed, vec![Permission::Register]);
        for p in Permission::ALL.into_iter().filter(|p| *p != Permission::Register) {
            assert_eq!(boundary.denial(p), Some(&DenialReason::NotRegistered));
        }
        // 免费权限（查询/订阅）不需要质押，但需要先注册——顺序在语义上很重要。
        assert!(!boundary.allows(Permission::QuerySafety));
        // 余额为零的新 DID 由创世额度资助注册：门槛可见且已满足。
        let gate = boundary
            .stake_gates
            .iter()
            .find(|g| g.permission == Permission::Register)
            .unwrap();
        assert_eq!(gate.required, kernel.config().min_stake);
        assert_eq!(gate.current, kernel.config().genesis_mint);
        assert!(gate.met);
    }

    #[test]
    fn a_zero_balance_newcomer_without_genesis_cannot_register() {
        let (mut kernel, config, _, _) = world();
        // 造一个「有余额但不够质押」的 DID：内核不会给已有余额的账户补创世额度，
        // 因此它的注册门槛资金就是那点余额，注册应当被结构化拒绝。
        let poor = AgentKeys::from_seed(&[0x43; 32]);
        kernel.ledger_mut().mint(&poor.did(), Credits(3)).unwrap();
        let boundary = PermissionBoundary::of(&kernel, &config, &poor.did());
        assert_eq!(
            boundary.denial(Permission::Register),
            Some(&DenialReason::StakeBelowMinimum {
                required: kernel.config().min_stake,
                actual: Credits(3),
            })
        );
        assert!(boundary.allowed.is_empty());
    }

    #[test]
    fn stake_floor_is_recomputed_and_reported_as_a_gate() {
        let (mut kernel, config, reporter, _) = world();
        let min_stake = kernel.config().min_stake;
        let boundary = PermissionBoundary::of(&kernel, &config, &reporter.did());
        let gate = boundary
            .stake_gates
            .iter()
            .find(|g| g.permission == Permission::ReportViolation)
            .unwrap();
        assert_eq!(gate.required, min_stake);
        assert_eq!(gate.current, Credits(20));
        assert!(gate.met);
        // 查询与订阅没有质押门槛。
        assert!(boundary
            .stake_gates
            .iter()
            .all(|g| g.permission != Permission::QuerySafety));

        // 罚没把锁定质押清空：举报权立刻被结构化地收回。
        let did = reporter.did();
        let locked = kernel.ledger().balance(&did).locked;
        kernel.ledger_mut().slash(&did, locked).unwrap();
        kernel.ledger().check_conservation().unwrap();
        let after = PermissionBoundary::of(&kernel, &config, &did);
        assert!(!after.allows(Permission::ReportViolation));
        assert_eq!(
            after.denial(Permission::ReportViolation),
            Some(&DenialReason::StakeBelowMinimum {
                required: min_stake,
                actual: Credits::ZERO
            })
        );
        // 免费权限不受罚没影响。
        assert!(after.allows(Permission::QuerySafety));
    }

    #[test]
    fn boundary_json_is_structured_and_roundtrips() {
        let (kernel, config, reporter, _) = world();
        let boundary = PermissionBoundary::of(&kernel, &config, &reporter.did());
        let json = boundary.to_json().unwrap();

        // 拒绝原因不是自由文本：一定是带 code 的对象，且 code 属于封闭集合。
        let denied = json["denied"].as_array().unwrap();
        assert!(!denied.is_empty());
        for entry in denied {
            let code = entry["reason"]["code"].as_str().unwrap();
            assert!(
                [
                    "already_registered",
                    "not_registered",
                    "stake_below_minimum",
                    "arbiter_only"
                ]
                .contains(&code),
                "未知拒绝码 {code}"
            );
            assert!(Permission::parse(entry["permission"].as_str().unwrap()).is_some());
        }
        // allowed ∪ denied == 全部权限点，且两两不交。
        let allowed = json["allowed"].as_array().unwrap().len();
        assert_eq!(allowed + denied.len(), Permission::ALL.len());
        assert_eq!(PermissionBoundary::from_json(&json).unwrap(), boundary);
    }

    #[test]
    fn permission_names_roundtrip_and_are_unique() {
        assert_eq!(Permission::ALL.len(), 8);
        let mut names: Vec<&str> = Permission::ALL.iter().map(|p| p.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Permission::ALL.len());
        for p in Permission::ALL {
            assert_eq!(Permission::parse(p.as_str()), Some(p));
        }
        assert_eq!(Permission::parse("become_admin"), None);
    }

    #[test]
    fn queries_are_deterministic_and_self_only() {
        let (kernel, config, reporter, _) = world();
        let a = query_permissions(&kernel, &config, &reporter.did()).unwrap();
        let b = query_permissions(&kernel, &config, &reporter.did()).unwrap();
        assert_eq!(a, b, "同一状态必须得到同一串字节");

        let query = PermissionQuery::new(reporter.did());
        assert!(query.is_self_query(&reporter.did()));
        assert!(!query.is_self_query(&AgentKeys::from_seed(&[9u8; 32]).did()));
        let round = PermissionQuery::from_json(&query.to_json().unwrap()).unwrap();
        assert_eq!(round, query);
    }

    #[test]
    fn boundary_of_unknown_did_is_not_an_error_but_a_structured_denial() {
        let (kernel, config, _, _) = world();
        let ghost = AgentKeys::from_seed(&[0xAB; 32]).did();
        let boundary = PermissionBoundary::of(&kernel, &config, &ghost);
        assert!(!boundary.registered);
        assert_eq!(
            boundary.denial(Permission::Settle),
            Some(&DenialReason::NotRegistered)
        );
        assert_eq!(PermissionBoundary::from_json(&Value::Null), Err(CoreError::Encoding));
    }
}
