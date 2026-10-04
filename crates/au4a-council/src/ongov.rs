//! 链上治理映射（v1.7.6）。
//!
//! 真实链上执行属 **v1.8（`au4a-chain`）**；本版只做**语义数据结构映射**，把治理状态
//! 投影成链上治理合约（`Governor`-style）里那张表的样子：
//!
//! | AU4A 治理状态 | GovernorToken 状态 |
//! |---|---|
//! | 动议已提交、尚无表决轮 | `Pending` |
//! | 有进行中的表决轮（含被作废需重投） | `Active` |
//! | 表决通过、尚未执行 | `Succeeded` |
//! | 已执行 | `Executed` |
//! | 被表决否决 | `Defeated` |
//! | 被人类否决权阻断 | `Canceled`（带公开理由） |
//!
//! 两件事刻意保持诚实：
//!
//! * [`ChainBinding`] 里 `real_chain = false`、`grade = cpu-proto`——**不假装有链**；
//! * `quorum` 用整数分数表达（`numerator/denominator`），与 BFT-lite 的整数法定人数同源，
//!   不做浮点近似。

use au4a_core::{canonical_hash, CoreError, CoreResult, Did, EvidenceGrade};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::committee::CommitteeKind;
use crate::proposal::ProposalState;
use crate::voting::RoundOutcome;
use crate::Council;

/// 链上治理状态机（语义映射，不是真实链上状态）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovState {
    /// 已提交，尚未开始表决。
    Pending,
    /// 表决进行中。
    Active,
    /// 被否决权撤销。
    Canceled,
    /// 表决否决。
    Defeated,
    /// 表决通过，等待执行。
    Succeeded,
    /// 已进入执行队列（本版预留：执行引擎为同步执行，直接到 `Executed`）。
    Queued,
    /// 已执行。
    Executed,
}

impl GovState {
    /// 稳定字符串（与链上事件名保持一致的写法）。
    pub fn as_str(self) -> &'static str {
        match self {
            GovState::Pending => "pending",
            GovState::Active => "active",
            GovState::Canceled => "canceled",
            GovState::Defeated => "defeated",
            GovState::Succeeded => "succeeded",
            GovState::Queued => "queued",
            GovState::Executed => "executed",
        }
    }
}

/// 链绑定：明确「这是哪条链、什么证据等级」。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainBinding {
    /// 链标识。
    pub chain: String,
    /// 证据等级字符串（`verified` / `cpu-proto` / `unverified`）。
    pub grade: String,
    /// 是否真实链上执行。本版恒为 `false`。
    pub real_chain: bool,
    /// 说明。
    pub note: String,
}

impl Default for ChainBinding {
    fn default() -> Self {
        Self {
            chain: String::from("au4a-local"),
            grade: EvidenceGrade::CpuProto.as_str().to_string(),
            real_chain: false,
            note: String::from("语义映射：真实链上执行属 v1.8 au4a-chain"),
        }
    }
}

impl ChainBinding {
    /// 证据等级（解析失败按 `unverified`，宁可低估不可高估）。
    pub fn evidence(&self) -> EvidenceGrade {
        EvidenceGrade::parse(&self.grade).unwrap_or(EvidenceGrade::Unverified)
    }
}

/// 否决在链上映射里的样子（`Canceled` 的原因）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovVeto {
    /// 人类观察者标签。
    pub observer: String,
    /// 公开理由。
    pub reason: String,
    /// 时刻。
    pub at: u64,
}

/// 执行在链上映射里的样子（`Executed` 的凭证）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovExecution {
    /// 执行时刻。
    pub at: u64,
    /// 效应摘要。
    pub effects: Vec<String>,
    /// 链上交易引用。本版恒为 `None`（没有真实交易）。
    pub tx_ref: Option<String>,
}

/// GovernorToken 语义对象：一条动议在链上治理合约里会是什么样子。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernorToken {
    /// 内容地址（由映射内容决定，可复算）。
    pub id: String,
    /// 对应的治理动议。
    pub proposal: String,
    /// 提案人。
    pub proposer: Did,
    /// 受理委员会。
    pub committee: CommitteeKind,
    /// 快照纪元（选举届数之和，代表「这条动议是在哪一届委员会里表决的」）。
    pub snapshot_epoch: u64,
    /// 赞成票数（委员一人一票）。
    pub for_votes: u64,
    /// 反对票数。
    pub against_votes: u64,
    /// 弃权票数。
    pub abstain_votes: u64,
    /// 法定人数分子。
    pub quorum_numerator: u64,
    /// 法定人数分母（= 委员会在任人数 `n`）。
    pub quorum_denominator: u64,
    /// 状态下。
    pub state: GovState,
    /// 若被否决阻断，这里是被撤销的原因。
    pub veto: Option<GovVeto>,
    /// 若已执行，这里是执行凭证。
    pub execution: Option<GovExecution>,
    /// 链绑定。
    pub binding: ChainBinding,
}

impl GovernorToken {
    /// 法定人数分数（整数，未除）。
    pub fn quorum_fraction(&self) -> (u64, u64) {
        (self.quorum_numerator, self.quorum_denominator)
    }

    /// 证据等级（永远取链绑定的等级）。
    pub fn evidence(&self) -> EvidenceGrade {
        self.binding.evidence()
    }

    /// 是否声称真实链上执行（本版必须恒为 `false`）。
    pub fn claims_real_chain(&self) -> bool {
        self.binding.real_chain
            || self
                .execution
                .as_ref()
                .map(|e| e.tx_ref.is_some())
                .unwrap_or(false)
    }

    /// 把治理层的某条动议投影成 GovernorToken 语义对象。
    pub fn project(council: &Council, proposal_id: &str) -> CoreResult<Self> {
        let proposal = match council.proposal(proposal_id) {
            Some(p) => p.clone(),
            None => return Err(CoreError::UnknownAgent),
        };
        let committee = council.committee(proposal.committee);
        let (n, quorum) = match committee {
            Some(c) => (c.size() as u64, c.quorum() as u64),
            None => (0, 0),
        };
        // 取该动议最后一次有票的轮次（含被作废的轮：作废也是治理事实）。
        let latest = council.current_round(proposal_id);
        let (for_votes, against_votes, abstain_votes) = match &latest {
            Some(r) => (
                r.tally.yes as u64,
                r.tally.no as u64,
                r.tally.abstain as u64,
            ),
            None => (0, 0, 0),
        };
        let round_voided = matches!(
            latest.as_ref().map(|r| r.outcome),
            Some(RoundOutcome::VoidAmbiguous)
        );
        let state = match proposal.state {
            ProposalState::Open => {
                if latest.is_some() {
                    GovState::Active
                } else {
                    GovState::Pending
                }
            }
            ProposalState::Passed => GovState::Succeeded,
            ProposalState::Rejected => GovState::Defeated,
            ProposalState::Blocked => GovState::Canceled,
            ProposalState::Executed => GovState::Executed,
        };
        let veto = council.veto_record(proposal_id).map(|v| GovVeto {
            observer: v.observer().to_string(),
            reason: v.reason().to_string(),
            at: v.at(),
        });
        let execution = council.execution(proposal_id).map(|r| GovExecution {
            at: r.at,
            effects: r.effects.iter().map(|e| e.describe()).collect(),
            tx_ref: None,
        });
        let snapshot_epoch: u64 = council.committees().map(|c| c.epoch).sum();
        let binding = ChainBinding::default();
        let id = canonical_hash(&json!({
            "proposal": proposal.id,
            "proposer": proposal.author,
            "committee": proposal.committee,
            "state": state,
            "for_votes": for_votes,
            "against_votes": against_votes,
            "abstain_votes": abstain_votes,
            "quorum_numerator": quorum,
            "quorum_denominator": n,
            "snapshot_epoch": snapshot_epoch,
            "round_voided": round_voided,
            "veto_reason": veto.as_ref().map(|v| v.reason.clone()),
            "executed": execution.is_some(),
        }))?;
        Ok(Self {
            id,
            proposal: proposal.id,
            proposer: proposal.author,
            committee: proposal.committee,
            snapshot_epoch,
            for_votes,
            against_votes,
            abstain_votes,
            quorum_numerator: quorum,
            quorum_denominator: n,
            state,
            veto,
            execution,
            binding,
        })
    }

    /// 只读 JSON 投影（观察层/未来链上适配器共用）。
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "proposal": self.proposal,
            "proposer": self.proposer,
            "committee": self.committee,
            "snapshot_epoch": self.snapshot_epoch,
            "for_votes": self.for_votes,
            "against_votes": self.against_votes,
            "abstain_votes": self.abstain_votes,
            "quorum_numerator": self.quorum_numerator,
            "quorum_denominator": self.quorum_denominator,
            "state": self.state,
            "veto": self.veto,
            "execution": self.execution,
            "binding": self.binding,
        })
    }
}

/// 把治理层全部动议投影成 GovernorToken 列表（按动议提交顺序）。
pub fn project_all(council: &Council) -> CoreResult<Vec<GovernorToken>> {
    let mut out = Vec::new();
    for proposal in council.proposals() {
        out.push(GovernorToken::project(council, &proposal.id)?);
    }
    Ok(out)
}

/// 链上映射的自洽检查：不允许出现「声称真实链上」的对象。
pub fn no_false_chain_claims(tokens: &[GovernorToken]) -> bool {
    tokens.iter().all(|t| !t.claims_real_chain())
}

/// 状态分布摘要（观察层一句话）。
pub fn state_summary(tokens: &[GovernorToken]) -> Value {
    let mut counts: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for token in tokens {
        *counts.entry(token.state.as_str()).or_insert(0) += 1;
    }
    json!(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::{AgentKeys, Credits};
    use au4a_kernel::{Kernel, KernelConfig};

    use crate::{Action, AgentIdentity, CouncilConfig, ElectionBallot, ProposalDraft};

    fn keys(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn seated_council() -> (Kernel, Council, Vec<AgentKeys>) {
        let mut kernel = Kernel::new(KernelConfig::default());
        let mut council = Council::new(CouncilConfig {
            election: crate::ElectionConfig {
                seats: 4,
                ..crate::ElectionConfig::default()
            },
            ..CouncilConfig::default()
        });
        let agents: Vec<AgentKeys> = (0..4).map(keys).collect();
        for k in &agents {
            kernel
                .register(k, "t", &["governance.vote"], Credits(20))
                .expect("register");
            council.note_reputation(&k.did(), 5_000);
            council.note_uptime(&k.did(), 200);
        }
        let picks: Vec<Did> = agents.iter().map(|k| k.did()).collect();
        let ballots: Vec<ElectionBallot> = agents
            .iter()
            .map(|k| ElectionBallot::cast(k, CommitteeKind::Task, &picks).expect("cast"))
            .collect();
        council
            .elect(&mut kernel, CommitteeKind::Task, &ballots)
            .expect("elect");
        (kernel, council, agents)
    }

    #[test]
    fn lifecycle_maps_to_governor_states() {
        let (mut kernel, mut council, agents) = seated_council();
        let identity = AgentIdentity::from_keys(&agents[0]);
        let draft = ProposalDraft::by(
            &agents[0],
            CommitteeKind::Task,
            "链上映射",
            Action::SetPolicy {
                key: String::from("k"),
                value: 1,
            },
        )
        .expect("draft");
        let proposal = council
            .propose(&mut kernel, &identity, draft)
            .expect("propose");

        // 提交后、未开轮 → pending。
        let token = GovernorToken::project(&council, &proposal.id).expect("project");
        assert_eq!(token.state, GovState::Pending);
        assert_eq!(token.quorum_fraction(), (3, 4), "n=4 → quorum=3");
        assert!(!token.claims_real_chain());
        assert_eq!(token.evidence(), EvidenceGrade::CpuProto);

        // 开轮 → active。
        let round = council.open_round(&mut kernel, &proposal.id).expect("open");
        let token = GovernorToken::project(&council, &proposal.id).expect("project");
        assert_eq!(token.state, GovState::Active);

        // 通过 → succeeded。
        for agent in agents.iter().take(3) {
            let v = crate::Vote::cast(agent, &proposal.id, round.round, crate::Choice::Yes)
                .expect("cast");
            council.cast_vote(&mut kernel, v).expect("vote");
        }
        let token = GovernorToken::project(&council, &proposal.id).expect("project");
        assert_eq!(token.state, GovState::Succeeded);
        assert_eq!(token.for_votes, 3);

        // 执行 → executed（但没有真实交易引用）。
        council
            .execute(&mut kernel, &identity, &proposal.id)
            .expect("execute");
        let token = GovernorToken::project(&council, &proposal.id).expect("project");
        assert_eq!(token.state, GovState::Executed);
        assert!(token.execution.is_some());
        assert!(token
            .execution
            .as_ref()
            .map(|e| e.tx_ref.is_none())
            .unwrap_or(false));
        assert!(!token.claims_real_chain());
    }

    #[test]
    fn rejection_and_veto_map_to_defeated_and_canceled() {
        let (mut kernel, mut council, agents) = seated_council();
        let identity = AgentIdentity::from_keys(&agents[0]);

        // 被表决否决 → defeated。
        let draft = ProposalDraft::by(
            &agents[0],
            CommitteeKind::Task,
            "会被否决",
            Action::SetPolicy {
                key: String::from("d1"),
                value: 1,
            },
        )
        .expect("draft");
        let rejected = council
            .propose(&mut kernel, &identity, draft)
            .expect("propose");
        let round = council.open_round(&mut kernel, &rejected.id).expect("open");
        for agent in agents.iter().take(3) {
            let v = crate::Vote::cast(agent, &rejected.id, round.round, crate::Choice::No)
                .expect("cast");
            council.cast_vote(&mut kernel, v).expect("vote");
        }
        let token = GovernorToken::project(&council, &rejected.id).expect("project");
        assert_eq!(token.state, GovState::Defeated);
        assert_eq!(token.against_votes, 3);
        assert!(token.veto.is_none());

        // 被人类否决阻断 → canceled + 公开理由。
        let draft = ProposalDraft::by(
            &agents[0],
            CommitteeKind::Task,
            "会被否决权阻断",
            Action::SetPolicy {
                key: String::from("d2"),
                value: 1,
            },
        )
        .expect("draft");
        let blocked = council
            .propose(&mut kernel, &identity, draft)
            .expect("propose");
        let human = crate::HumanObserver::new("operator");
        let veto = human
            .veto(&council, &blocked.id, "会伤害新加入者")
            .expect("veto");
        council.apply_veto(&mut kernel, &veto).expect("apply");
        let token = GovernorToken::project(&council, &blocked.id).expect("project");
        assert_eq!(token.state, GovState::Canceled);
        assert_eq!(
            token.veto.as_ref().map(|v| v.reason.as_str()),
            Some("会伤害新加入者")
        );
        assert!(no_false_chain_claims(&[token]));
    }

    #[test]
    fn projection_is_deterministic_and_content_addressed() {
        let (mut kernel, mut council, agents) = seated_council();
        let identity = AgentIdentity::from_keys(&agents[0]);
        let draft = ProposalDraft::by(
            &agents[0],
            CommitteeKind::Task,
            "可复算",
            Action::SetPolicy {
                key: String::from("r"),
                value: 2,
            },
        )
        .expect("draft");
        let proposal = council
            .propose(&mut kernel, &identity, draft)
            .expect("propose");
        let a = GovernorToken::project(&council, &proposal.id).expect("a");
        let b = GovernorToken::project(&council, &proposal.id).expect("b");
        assert_eq!(a.id, b.id);
        assert_eq!(a, b);
        // JSON 投影可序列化往返。
        let text = serde_json::to_string(&a).expect("ser");
        let back: GovernorToken = serde_json::from_str(&text).expect("de");
        assert_eq!(back.id, a.id);
        assert_eq!(back.state, a.state);
        assert!(!back.claims_real_chain());
    }
}
