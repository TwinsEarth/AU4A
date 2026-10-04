//! 提案流程（v1.7.2）。
//!
//! 「人类只观察」如果只写在文档里就毫无约束力。本模块把它落到**类型级**：
//!
//! * [`AgentIdentity`] 是「持有 Agent 私钥」的能力凭证：字段私有、**不实现 `Deserialize`**，
//!   唯一构造入口是 [`AgentIdentity::from_keys`]。没有私钥的主体无法在类型层面伪造它。
//! * 动议内容（标题 + 动作）由 [`ProposalDraft::by`] 用 Agent 私钥签名；即便有人反序列化出
//!   一个 DID，也拿不出对应签名——**提案权最终由密码学而不是由约定保证**。
//! * 人类观察者 [`crate::human::HumanObserver`] 只有只读投影方法，没有 `propose`/`edit`，
//!   这一点由 `compile_fail` 文档测试把守（一旦有人加上写方法，文档测试立刻变红）。
//!
//! 动议一旦提交就是**内容寻址**的：同一份（作者、委员会、标题、动作）只能存在一条，
//! 重复提交按重复登记拒绝。这挡住「同一动议反复刷屏」。

use au4a_core::{canonical_hash, canonicalize, AgentKeys, CoreError, CoreResult, Credits, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::committee::CommitteeKind;

/// 动议要执行的动作。**只是数据**：真正落成状态变更在 v1.7.4 的执行引擎里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Action {
    /// 设定整数策略（例如 `cpu_proto_settle_cap`）。金额与度量一律整数微积分/基点。
    SetPolicy {
        /// 策略键。
        key: String,
        /// 策略值（整数）。
        value: i64,
    },
    /// 调整某 Agent 的信誉（万分比）。
    SetReputation {
        /// 目标 Agent。
        did: Did,
        /// 新信誉（0..=10000）。
        reputation_bp: u32,
    },
    /// 账本内转账（Agent 之间，守恒不变式由账本自己断言）。
    Transfer {
        /// 付款方。
        from: Did,
        /// 收款方。
        to: Did,
        /// 金额（微积分，必须为正）。
        amount: Credits,
    },
    /// 罚没锁定质押（销毁，不转给任何人）。
    Slash {
        /// 目标 Agent。
        did: Did,
        /// 罚没额（微积分，必须为正）。
        amount: Credits,
    },
}

impl Action {
    /// 动作自检。非法动作在提案阶段就被拒，不留给执行引擎。
    pub fn validate(&self) -> CoreResult<()> {
        match self {
            Action::SetPolicy { key, value } => {
                let bytes = key.as_bytes();
                let ok = !bytes.is_empty()
                    && bytes.len() <= 64
                    && bytes.iter().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'.' || *b == b'_'
                    });
                if !ok || *value < 0 {
                    return Err(CoreError::InvalidKind);
                }
                Ok(())
            }
            Action::SetReputation { reputation_bp, .. } => {
                if *reputation_bp > 10_000 {
                    return Err(CoreError::InvalidKind);
                }
                Ok(())
            }
            Action::Transfer { amount, .. } | Action::Slash { amount, .. } => {
                if amount.get() <= 0 {
                    return Err(CoreError::ZeroAmount);
                }
                Ok(())
            }
        }
    }

    /// 人类可读描述（只读投影与审计日志共用）。
    pub fn describe(&self) -> String {
        match self {
            Action::SetPolicy { key, value } => format!("set_policy {key}={value}"),
            Action::SetReputation { did, reputation_bp } => {
                format!(
                    "set_reputation {}={}bp",
                    au4a_core::short_id(did.as_str()),
                    reputation_bp
                )
            }
            Action::Transfer { from, to, amount } => format!(
                "transfer {}->{} amount={amount}",
                au4a_core::short_id(from.as_str()),
                au4a_core::short_id(to.as_str())
            ),
            Action::Slash { did, amount } => {
                format!(
                    "slash {} amount={amount}",
                    au4a_core::short_id(did.as_str())
                )
            }
        }
    }
}

/// 动议状态机。
///
/// ```text
/// Open ──pass──▶ Passed ──execute──▶ Executed
///   │              │
///   ├──reject──▶ Rejected
///   └────────────┴──veto──▶ Blocked   （终态：只能被阻断，不能被改写）
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    /// 待表决。
    Open,
    /// 已达法定人数通过，等待执行。
    Passed,
    /// 被表决否决。
    Rejected,
    /// 被人类否决权阻断（终态）。
    Blocked,
    /// 已执行并落成状态变更（终态）。
    Executed,
}

impl ProposalState {
    /// 稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalState::Open => "open",
            ProposalState::Passed => "passed",
            ProposalState::Rejected => "rejected",
            ProposalState::Blocked => "blocked",
            ProposalState::Executed => "executed",
        }
    }

    /// 是否为终态（不可再迁移）。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ProposalState::Rejected | ProposalState::Blocked | ProposalState::Executed
        )
    }

    /// 合法迁移表。**否决只能通向 `Blocked`**，这是「只读否决」的状态机一半。
    pub fn can_transition_to(self, next: ProposalState) -> bool {
        matches!(
            (self, next),
            (ProposalState::Open, ProposalState::Passed)
                | (ProposalState::Open, ProposalState::Rejected)
                | (ProposalState::Open, ProposalState::Blocked)
                | (ProposalState::Passed, ProposalState::Executed)
                | (ProposalState::Passed, ProposalState::Blocked)
        )
    }
}

/// 一条动议。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// 内容寻址 id：`hash(作者, 委员会, 标题, 动作)`。
    pub id: String,
    /// 提案 Agent。
    pub author: Did,
    /// 受理委员会。
    pub committee: CommitteeKind,
    /// 标题。
    pub title: String,
    /// 动作。
    pub action: Action,
    /// 提交时刻（逻辑刻度）。
    pub created_at: u64,
    /// 当前状态。
    pub state: ProposalState,
    /// 已进行的表决轮次（0 表示还没表过）。
    pub round: u32,
}

impl Proposal {
    /// 被签名的规范载荷（不含状态与轮次：那些是治理过程，不是动议内容）。
    pub fn content(
        author: &Did,
        committee: CommitteeKind,
        title: &str,
        action: &Action,
    ) -> CoreResult<Value> {
        Ok(json!({
            "author": author,
            "committee": committee,
            "title": title,
            "action": action,
        }))
    }

    /// 由动议内容计算内容地址。
    pub fn id_for(
        author: &Did,
        committee: CommitteeKind,
        title: &str,
        action: &Action,
    ) -> CoreResult<String> {
        canonical_hash(&Self::content(author, committee, title, action)?)
    }
}

/// 「持有 Agent 私钥」的能力凭证。
///
/// 字段私有、**不实现 `Deserialize`**、无 `Default`：唯一的构造入口是 [`AgentIdentity::from_keys`]，
/// 因此「谁能提案」在类型系统里就是「谁持有私钥」。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentIdentity {
    did: Did,
}

impl AgentIdentity {
    /// 由 Agent 自己的密钥派生身份凭证。
    pub fn from_keys(keys: &AgentKeys) -> Self {
        Self { did: keys.did() }
    }

    /// 身份 DID。
    pub fn did(&self) -> &Did {
        &self.did
    }
}

/// 待提交的动议：内容 + 作者签名。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalDraft {
    /// 作者（由签名密钥决定，不可伪造）。
    pub author: Did,
    /// 受理委员会。
    pub committee: CommitteeKind,
    /// 标题。
    pub title: String,
    /// 动作。
    pub action: Action,
    /// hex 编码的 Ed25519 签名。
    pub sig: String,
}

impl ProposalDraft {
    /// 由 Agent 私钥铸造动议（先规范化载荷，再签名）。
    pub fn by(
        keys: &AgentKeys,
        committee: CommitteeKind,
        title: impl Into<String>,
        action: Action,
    ) -> CoreResult<Self> {
        let mut draft = Self {
            author: keys.did(),
            committee,
            title: title.into(),
            action,
            sig: String::new(),
        };
        draft.sig = keys.sign_json(&draft.payload()?)?;
        Ok(draft)
    }

    /// 被签名的规范载荷。
    pub fn payload(&self) -> CoreResult<Value> {
        Proposal::content(&self.author, self.committee, &self.title, &self.action)
    }

    /// 验签：内容与作者必须绑定。
    pub fn verify(&self) -> CoreResult<()> {
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        let bytes = canonicalize(&self.payload()?)?;
        self.author.verify(bytes.as_bytes(), &self.sig)
    }

    /// 内容地址。
    pub fn id(&self) -> CoreResult<String> {
        Proposal::id_for(&self.author, self.committee, &self.title, &self.action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    #[test]
    fn state_machine_is_closed_and_blocked_is_terminal() {
        use ProposalState::*;
        assert!(Open.can_transition_to(Passed));
        assert!(Open.can_transition_to(Rejected));
        assert!(Open.can_transition_to(Blocked));
        assert!(Passed.can_transition_to(Executed));
        assert!(Passed.can_transition_to(Blocked));
        assert!(!Open.can_transition_to(Executed), "未通过不得直接执行");
        assert!(!Passed.can_transition_to(Rejected), "已通过不得被改判");
        assert!(!Blocked.can_transition_to(Passed), "被否决的决议不得复活");
        for s in [Rejected, Blocked, Executed] {
            assert!(s.is_terminal());
            for next in [Open, Passed, Rejected, Blocked, Executed] {
                assert!(
                    !s.can_transition_to(next),
                    "终态 {s:?} 不应能迁移到 {next:?}"
                );
            }
        }
    }

    #[test]
    fn drafts_are_signed_and_content_addressed() {
        let keys = agent(1);
        let a = ProposalDraft::by(
            &keys,
            CommitteeKind::Resource,
            "调整结算上限",
            Action::SetPolicy {
                key: "cap".into(),
                value: 250,
            },
        )
        .expect("draft");
        a.verify().expect("verify");
        let b = ProposalDraft::by(
            &keys,
            CommitteeKind::Resource,
            "调整结算上限",
            Action::SetPolicy {
                key: "cap".into(),
                value: 250,
            },
        )
        .expect("draft");
        assert_eq!(a.id().expect("id"), b.id().expect("id"));
    }

    #[test]
    fn tampering_with_a_draft_breaks_the_signature() {
        let keys = agent(2);
        let mut draft = ProposalDraft::by(
            &keys,
            CommitteeKind::Task,
            "改费率",
            Action::SetPolicy {
                key: "fee".into(),
                value: 10,
            },
        )
        .expect("draft");
        draft.action = Action::SetPolicy {
            key: "fee".into(),
            value: 0,
        };
        assert_eq!(draft.verify(), Err(CoreError::InvalidSignature));
        draft.title = String::from("改别的");
        assert_eq!(draft.verify(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn action_validation_refuses_illegal_payloads() {
        assert!(Action::SetPolicy {
            key: String::new(),
            value: 1
        }
        .validate()
        .is_err());
        assert!(Action::SetPolicy {
            key: "Bad Key".into(),
            value: 1
        }
        .validate()
        .is_err());
        assert!(Action::SetPolicy {
            key: "ok".into(),
            value: -1
        }
        .validate()
        .is_err());
        assert!(Action::SetPolicy {
            key: "ok".into(),
            value: 0
        }
        .validate()
        .is_ok());
        assert!(Action::SetReputation {
            did: agent(3).did(),
            reputation_bp: 10_001
        }
        .validate()
        .is_err());
        assert!(Action::Transfer {
            from: agent(4).did(),
            to: agent(5).did(),
            amount: Credits(0)
        }
        .validate()
        .is_err());
        assert!(Action::Slash {
            did: agent(6).did(),
            amount: Credits(5)
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn describe_is_stable_and_short_id_based() {
        let d = Action::Transfer {
            from: agent(7).did(),
            to: agent(8).did(),
            amount: Credits(12),
        };
        let text = d.describe();
        assert!(text.starts_with("transfer did:au4a:"));
        assert!(text.ends_with("amount=12"));
        assert!(!text.contains(&agent(7).did().as_str()[9..]));
    }
}
