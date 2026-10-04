//! 权限模型（v1.0.5）：内核必须能回答「**这个 Agent 能做什么、不能做什么**」。
//!
//! 三条约束：
//!
//! 1. **权限来自客观事实，不来自人类授权**：[`Authority`] 的根只有自证身份、质押、证据等级、
//!    自报能力与委员会决定。类型里**没有** `Operator` / `Human` / `Admin` 变体——运营方不是权限来源。
//! 2. **可回答**：[`explain`] 返回的 [`PermissionReport`] 是全部 [`Capability`] 的**恰好一次划分**
//!    （allowed ∪ denied = ALL，且两两不交），每个拒绝都带类型化 [`RefusalCode`] 与人话理由。
//! 3. **分类不丢**：只有「不在册」这种身份不成立才映射到恶意码 `unauthorized`；
//!    能力未声明 → `unsupported`、额度用尽 → `policy_denied`、未抽中委员 → `policy_denied`、
//!    能力数量超限 → `resource_exhausted`——全部是竞争码，单次只记警告。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, CoreError, CoreResult, Did, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::council::{Council, CouncilConfig};
use crate::Kernel;

/// 内核里一个 Agent 可以主张的能力。新增变体属于协议变更。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// 广播自己的能力名片。
    PublishCard,
    /// 点对点投递消息。
    DeliverDirect,
    /// 以 `verified` 证据结算。
    SettleVerified,
    /// 以 `cpu-proto` 证据结算（受额度上限约束）。
    SettleCpuProto,
    /// 向对端报价。
    Offer,
    /// 在委员会里投票。
    VoteInCouncil,
    /// 提交动议。
    ProposeMotion,
    /// 声明新能力。
    ListSkill,
    /// 解除质押。
    WithdrawStake,
}

impl Capability {
    pub const ALL: [Capability; 9] = [
        Capability::PublishCard,
        Capability::DeliverDirect,
        Capability::SettleVerified,
        Capability::SettleCpuProto,
        Capability::Offer,
        Capability::VoteInCouncil,
        Capability::ProposeMotion,
        Capability::ListSkill,
        Capability::WithdrawStake,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Capability::PublishCard => "publish_card",
            Capability::DeliverDirect => "deliver_direct",
            Capability::SettleVerified => "settle_verified",
            Capability::SettleCpuProto => "settle_cpu_proto",
            Capability::Offer => "offer",
            Capability::VoteInCouncil => "vote_in_council",
            Capability::ProposeMotion => "propose_motion",
            Capability::ListSkill => "list_skill",
            Capability::WithdrawStake => "withdraw_stake",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Capability::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// 权限的来源。**没有运营方/人类变体**：人类不是权限主体。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    /// 自证身份（DID 即公钥，验签即鉴权）。
    SelfSovereign,
    /// 已锁定质押（准入与违约成本）。
    Stake,
    /// 证据等级（结算闸门）。
    Evidence,
    /// 自报并通过准入的能力声明。
    DeclaredSkill,
    /// 委员会决定（限制或恢复）。
    CouncilDecision,
}

impl Authority {
    pub const ALL: [Authority; 5] = [
        Authority::SelfSovereign,
        Authority::Stake,
        Authority::Evidence,
        Authority::DeclaredSkill,
        Authority::CouncilDecision,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Authority::SelfSovereign => "self_sovereign",
            Authority::Stake => "stake",
            Authority::Evidence => "evidence",
            Authority::DeclaredSkill => "declared_skill",
            Authority::CouncilDecision => "council_decision",
        }
    }
}

/// 一次拒绝：能力 + 类型化理由。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Denial {
    pub capability: Capability,
    pub code: RefusalCode,
    pub reason: String,
}

impl Denial {
    pub fn new(capability: Capability, code: RefusalCode, reason: impl Into<String>) -> Self {
        Self {
            capability,
            code,
            reason: reason.into(),
        }
    }

    pub fn retryable(&self) -> bool {
        self.code.retryable()
    }

    pub fn is_misconduct(&self) -> bool {
        self.code.is_misconduct()
    }
}

/// 「这个 Agent 能做什么 / 不能做什么」的完整答案。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionReport {
    pub did: String,
    pub registered: bool,
    pub allowed: Vec<Capability>,
    pub denied: Vec<Denial>,
    /// 各能力的整数额度（如 cpu-proto 结算上限，单位微积分）。
    pub limits: BTreeMap<String, i64>,
    /// 生效的权限来源。
    pub authorities: Vec<Authority>,
}

impl PermissionReport {
    /// 划分完备性：allowed ∪ denied == ALL 且不交。任何一次 `explain` 都必须满足。
    pub fn is_total_partition(&self) -> bool {
        let mut seen: Vec<Capability> = Vec::new();
        for cap in &self.allowed {
            if seen.contains(cap) {
                return false;
            }
            seen.push(*cap);
        }
        for denial in &self.denied {
            if seen.contains(&denial.capability) {
                return false;
            }
            seen.push(denial.capability);
        }
        seen.len() == Capability::ALL.len() && Capability::ALL.iter().all(|cap| seen.contains(cap))
    }

    pub fn allows(&self, capability: Capability) -> bool {
        self.allowed.contains(&capability)
    }

    pub fn denial_for(&self, capability: Capability) -> Option<&Denial> {
        self.denied.iter().find(|d| d.capability == capability)
    }

    /// 拒绝清单里是否出现恶意码（用于把「身份不成立」与「竞争」分开）。
    pub fn has_misconduct_denial(&self) -> bool {
        self.denied.iter().any(|d| d.is_misconduct())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "did": self.did,
            "registered": self.registered,
            "can": self.allowed.iter().map(|c| c.as_str()).collect::<Vec<&str>>(),
            "cannot": self.denied.iter().map(|d| json!({
                "capability": d.capability.as_str(),
                "code": d.code.as_str(),
                "reason": d.reason,
                "retryable": d.retryable(),
            })).collect::<Vec<Value>>(),
            "limits": self.limits,
            "authorities": self.authorities.iter().map(|a| a.as_str()).collect::<Vec<&str>>(),
            "total_partition": self.is_total_partition(),
        })
    }

    pub fn fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self).map_err(|_| CoreError::Encoding)?;
        canonical_hash(&value)
    }
}

/// 解释「这个 Agent 在当前内核状态下能做什么」。
///
/// `vote_in_council` 用一个在 `kernel.now()` 那一刻抽签出的默认委员会判定；
/// 需要指定任期时用 [`explain_with_council`]。
pub fn explain(kernel: &Kernel, did: &Did) -> CoreResult<PermissionReport> {
    let council = Council::sortition(kernel, kernel.now(), CouncilConfig::default())?;
    explain_with_council(kernel, did, &council)
}

/// 用指定委员会解释权限（任期由调用方决定，结果可复算）。
pub fn explain_with_council(
    kernel: &Kernel,
    did: &Did,
    council: &Council,
) -> CoreResult<PermissionReport> {
    let config = kernel.config();
    let card = kernel.card(did);
    let registered = card.is_some();
    let balance = kernel.ledger().balance(did);

    let mut allowed = Vec::new();
    let mut denied = Vec::new();
    let mut limits: BTreeMap<String, i64> = BTreeMap::new();
    let mut authorities: Vec<Authority> = Vec::new();

    if !registered {
        // 身份不成立：内核在 `send` 里对未知发送者也是这个分类（unauthorized，单次即恶意）。
        for capability in Capability::ALL {
            denied.push(Denial::new(
                capability,
                RefusalCode::Unauthorized,
                "DID 不在册：自证身份与验签都不成立",
            ));
        }
        let report = PermissionReport {
            did: did.as_str().to_string(),
            registered,
            allowed,
            denied,
            limits,
            authorities,
        };
        return Ok(report);
    }

    let card = match card {
        Some(card) => card,
        None => {
            return Err(CoreError::UnknownAgent);
        }
    };
    authorities.push(Authority::SelfSovereign);

    let stake_ok = card.stake >= config.min_stake && balance.locked >= card.stake;
    if stake_ok {
        authorities.push(Authority::Stake);
    }

    for capability in Capability::ALL {
        match capability {
            Capability::PublishCard | Capability::DeliverDirect => {
                if stake_ok {
                    allowed.push(capability);
                } else {
                    denied.push(Denial::new(
                        capability,
                        RefusalCode::PolicyDenied,
                        "质押不足或未真正锁定：准入未成立",
                    ));
                }
            }
            Capability::SettleVerified => {
                if balance.available > au4a_core::Credits::ZERO {
                    allowed.push(capability);
                    authorities.push(Authority::Evidence);
                } else {
                    denied.push(Denial::new(
                        capability,
                        RefusalCode::ResourceExhausted,
                        "可用余额为零，无可结算资金",
                    ));
                }
            }
            Capability::SettleCpuProto => {
                let cap = config.cpu_proto_settle_cap;
                limits.insert(capability.as_str().to_string(), cap.get());
                if balance.available > au4a_core::Credits::ZERO && cap > au4a_core::Credits::ZERO {
                    allowed.push(capability);
                    authorities.push(Authority::Evidence);
                } else {
                    denied.push(Denial::new(
                        capability,
                        RefusalCode::PolicyDenied,
                        "cpu-proto 结算上限为零或余额为零：原型证据不足以承载结算",
                    ));
                }
            }
            Capability::Offer => {
                if !card.skills.is_empty() {
                    allowed.push(capability);
                    authorities.push(Authority::DeclaredSkill);
                } else {
                    denied.push(Denial::new(
                        capability,
                        RefusalCode::Unsupported,
                        "没有声明任何能力：无从报价",
                    ));
                }
            }
            Capability::VoteInCouncil => {
                if council.is_member(did) {
                    allowed.push(capability);
                    authorities.push(Authority::CouncilDecision);
                } else {
                    denied.push(Denial::new(
                        capability,
                        RefusalCode::PolicyDenied,
                        "本任期未被抽中为委员：不是恶意，是竞争",
                    ));
                }
            }
            Capability::ProposeMotion => {
                // 提案权是自证身份的直接结果：任何在册 Agent 都能提案，不需要人类同意。
                allowed.push(capability);
            }
            Capability::ListSkill => {
                if card.skills.len() < config.max_skills {
                    allowed.push(capability);
                } else {
                    denied.push(Denial::new(
                        capability,
                        RefusalCode::ResourceExhausted,
                        "能力声明数已达上限",
                    ));
                }
            }
            Capability::WithdrawStake => {
                if balance.locked > config.min_stake {
                    allowed.push(capability);
                } else {
                    denied.push(Denial::new(
                        capability,
                        RefusalCode::PolicyDenied,
                        "解除质押会跌破准入下限",
                    ));
                }
            }
        }
    }

    authorities.sort();
    authorities.dedup();

    Ok(PermissionReport {
        did: did.as_str().to_string(),
        registered,
        allowed,
        denied,
        limits,
        authorities,
    })
}

/// 权限来源清单（结构性证据：人类/运营方不是权限来源）。
pub fn authority_roots() -> Value {
    json!({
        "authorities": Authority::ALL.iter().map(|a| a.as_str()).collect::<Vec<&str>>(),
        "human_is_a_root": false,
        "capabilities": Capability::ALL.iter().map(|c| c.as_str()).collect::<Vec<&str>>(),
    })
}

/// 编译期证据：解释入口只接受共享借用（人类/调用方无法借解释去改内核）。
const _: fn(&Kernel, &Did) -> CoreResult<PermissionReport> = explain;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KernelConfig;
    use au4a_core::{AgentKeys, Credits, Envelope};

    fn keys(tag: u8) -> AgentKeys {
        AgentKeys::from_seed(&[tag; 32])
    }

    fn kernel_with(n: u8, config: KernelConfig) -> Kernel {
        let mut k = Kernel::new(config);
        for i in 0..n {
            k.register(
                &keys(60 + i),
                format!("agent-{i}"),
                &["skill.a"],
                Credits(20),
            )
            .unwrap();
        }
        k
    }

    #[test]
    fn the_report_is_a_total_partition_of_all_capabilities() {
        let k = kernel_with(3, KernelConfig::default());
        for i in 0..3 {
            let report = explain(&k, &keys(60 + i).did()).unwrap();
            assert!(report.registered);
            assert!(
                report.is_total_partition(),
                "allowed ∪ denied 必须恰好覆盖全部能力"
            );
            assert_eq!(
                report.allowed.len() + report.denied.len(),
                Capability::ALL.len()
            );
            assert_eq!(
                report.fingerprint().unwrap(),
                explain(&k, &keys(60 + i).did())
                    .unwrap()
                    .fingerprint()
                    .unwrap()
            );
        }
    }

    #[test]
    fn an_unregistered_did_is_denied_everything_with_a_misconduct_code() {
        let k = kernel_with(1, KernelConfig::default());
        let ghost = AgentKeys::from_seed(&[250u8; 32]).did();
        let report = explain(&k, &ghost).unwrap();
        assert!(!report.registered);
        assert!(report.allowed.is_empty());
        assert_eq!(report.denied.len(), Capability::ALL.len());
        assert!(report.has_misconduct_denial());
        for denial in &report.denied {
            assert_eq!(denial.code, RefusalCode::Unauthorized);
            assert!(denial.is_misconduct());
            assert!(!denial.reason.is_empty());
        }
        assert!(report.denial_for(Capability::PublishCard).is_some());
    }

    #[test]
    fn a_registered_agent_gets_a_specific_answer() {
        let k = kernel_with(1, KernelConfig::default());
        let report = explain(&k, &keys(60).did()).unwrap();
        assert!(report.allows(Capability::PublishCard));
        assert!(
            report.allows(Capability::ProposeMotion),
            "提案权来自自证身份"
        );
        assert!(report.allows(Capability::SettleVerified));
        assert!(report.allows(Capability::SettleCpuProto));
        assert!(report.allows(Capability::Offer));
        assert_eq!(
            report.limits.get("settle_cpu_proto").copied(),
            Some(100),
            "cpu-proto 的结算上限如实写在额度里"
        );
        assert!(report.authorities.contains(&Authority::SelfSovereign));
        assert!(report.authorities.contains(&Authority::Stake));
        assert!(report.authorities.contains(&Authority::DeclaredSkill));
        assert!(!report.has_misconduct_denial());
    }

    #[test]
    fn no_authority_root_is_a_human_or_an_operator() {
        for authority in Authority::ALL {
            let name = authority.as_str();
            for forbidden in ["operator", "human", "admin", "owner", "approver"] {
                assert!(
                    !name.contains(forbidden),
                    "权限来源不能是 {forbidden}：{name}"
                );
            }
        }
        let roots = authority_roots();
        assert_eq!(roots["human_is_a_root"], false);
        assert_eq!(roots["authorities"].as_array().map(|a| a.len()), Some(5));
        assert_eq!(roots["capabilities"].as_array().map(|a| a.len()), Some(9));
    }

    #[test]
    fn competition_denials_never_use_misconduct_codes() {
        let config = KernelConfig {
            max_skills: 1,
            ..KernelConfig::default()
        };
        let k = kernel_with(2, config);
        let report = explain(&k, &keys(60).did()).unwrap();
        // 能力数已达上限 → resource_exhausted（竞争）。
        let denial = report.denial_for(Capability::ListSkill).unwrap();
        assert_eq!(denial.code, RefusalCode::ResourceExhausted);
        assert!(!denial.is_misconduct());
        assert!(denial.retryable());

        // 未被抽中委员 → policy_denied（竞争）。
        let council = Council::sortition(
            &k,
            1,
            CouncilConfig {
                size: 1,
                ..CouncilConfig::default()
            },
        )
        .unwrap();
        let member = council.members()[0].clone();
        let insider = explain_with_council(&k, &member, &council).unwrap();
        assert!(insider.allows(Capability::VoteInCouncil));
        let outsider_did = (0..2)
            .map(|i| keys(60 + i).did())
            .find(|did| did != &member)
            .unwrap();
        let outsider = explain_with_council(&k, &outsider_did, &council).unwrap();
        let denial = outsider.denial_for(Capability::VoteInCouncil).unwrap();
        assert_eq!(denial.code, RefusalCode::PolicyDenied);
        assert!(!denial.is_misconduct(), "没抽中不是恶意");
    }

    #[test]
    fn withdrawing_below_the_admission_floor_is_refused() {
        let k = kernel_with(1, KernelConfig::default());
        // 质押 20 == min_stake 10 之上，但锁定 20 > 10 → 允许解押。
        let report = explain(&k, &keys(60).did()).unwrap();
        assert!(report.allows(Capability::WithdrawStake));

        // 把配置下限抬高到 20：锁定恰好等于下限，再解押就会跌破准入。
        let config = KernelConfig {
            min_stake: Credits(20),
            ..KernelConfig::default()
        };
        let k2 = kernel_with(1, config);
        let report2 = explain(&k2, &keys(60).did()).unwrap();
        let denial = report2.denial_for(Capability::WithdrawStake).unwrap();
        assert_eq!(denial.code, RefusalCode::PolicyDenied);
        assert!(!denial.is_misconduct());
    }

    #[test]
    fn denying_an_offer_needs_no_declared_skill() {
        let mut k = Kernel::new(KernelConfig::default());
        let keys = keys(70);
        k.register(&keys, "skill-less", &[], Credits(20)).unwrap();
        let report = explain(&k, &keys.did()).unwrap();
        let denial = report.denial_for(Capability::Offer).unwrap();
        assert_eq!(denial.code, RefusalCode::Unsupported);
        assert!(!denial.is_misconduct());
        assert!(!report.allows(Capability::Offer));
    }

    #[test]
    fn explaining_is_read_only_and_matches_the_kernel_classification() {
        let mut k = kernel_with(2, KernelConfig::default());
        let before = k.registry_fingerprint().unwrap();
        let view_before = k.observe_json();
        let _ = explain(&k, &keys(60).did()).unwrap();
        assert_eq!(before, k.registry_fingerprint().unwrap());
        assert_eq!(view_before, k.observe_json());

        // 内核自己对未知发送者的分类也是 unauthorized（与 explain 一致）。
        let ghost = AgentKeys::from_seed(&[251u8; 32]);
        let env = Envelope::new(ghost.did(), None, "agent.card", 1, None, json!({}))
            .unwrap()
            .seal(&ghost)
            .unwrap();
        assert!(k.send(&env).is_err());
        assert_eq!(
            k.refusals().last().map(|(_, r)| r.code),
            Some(RefusalCode::Unauthorized)
        );
        assert_eq!(
            explain(&k, &ghost.did()).unwrap().denied[0].code,
            RefusalCode::Unauthorized
        );
    }
}
