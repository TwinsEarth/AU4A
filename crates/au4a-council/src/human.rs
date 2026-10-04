//! 人类观察者（v1.7.2 起，v1.7.5 补上唯一写能力：否决）。
//!
//! 这是「人类只观察」的**类型级**落点：[`HumanObserver`] 只有只读投影方法，
//! 不存在 `propose`、`edit`、`approve`、`schedule`、`execute` 之类的写路径。
//! 文档测试里的 `compile_fail` 代码块就是这条性质的守卫——一旦有人给观察者加上写方法，
//! `cargo test` 立刻变红。
//!
//! v1.7.5 给观察者加上了**唯一**的写能力：`veto`。它只能把动议推进到
//! [`ProposalState::Blocked`]，必须带公开理由，且拿不出签名（人类没有密钥），
//! 因此提案/表决/执行三件事对人类在类型层面不可达。

use au4a_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

use crate::committee::CommitteeKind;
use crate::proposal::ProposalState;
use crate::veto::{HumanVeto, Veto};
use crate::Council;

/// 观察层里的委员会行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanCommittee {
    /// 类别。
    pub kind: CommitteeKind,
    /// 中文名。
    pub title: String,
    /// 职责边界。
    pub mandate: String,
    /// 届数。
    pub epoch: u64,
    /// 席位上限。
    pub seats: usize,
    /// 法定人数。
    pub quorum: usize,
    /// 在任成员 DID。
    pub members: Vec<String>,
}

/// 观察层里的动议行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanProposal {
    /// 内容地址。
    pub id: String,
    /// 标题。
    pub title: String,
    /// 受理委员会。
    pub committee: CommitteeKind,
    /// 提案 Agent。
    pub author: String,
    /// 状态。
    pub state: ProposalState,
    /// 动作的可读描述。
    pub action: String,
    /// 提交时刻（逻辑刻度）。
    pub created_at: u64,
}

/// 人类能看到的全部内容：只读、可序列化、**没有任何配套的写方法**。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanView {
    /// 观察者标签。
    pub label: String,
    /// 投影时刻（逻辑刻度）。
    pub at: u64,
    /// 在任席位合计。
    pub seats_filled: usize,
    /// 委员会。
    pub committees: Vec<HumanCommittee>,
    /// 动议。
    pub proposals: Vec<HumanProposal>,
    /// 否决记录（人类自己按下的「停」，理由公开）。
    pub vetoes: Vec<HumanVeto>,
    /// 治理事件条数（只读计数）。
    pub events: usize,
}

/// 人类观察者：只读投影 + 唯一的写能力「否决」。
///
/// 下面几段代码**必须编译失败**——这就是「人类不能提案、不能改决议、不能表决、不能执行」的
/// 结构性证据；正例文档测试确保它们不是因为类型不存在而失败：
///
/// ```compile_fail
/// // 人类观察者没有 propose：编译失败是预期结果。
/// let human = au4a_council::human::HumanObserver::new("operator");
/// human.propose("我要加一条规则");
/// ```
///
/// ```compile_fail
/// // 人类观察者没有 edit_proposal：编译失败是预期结果。
/// let human = au4a_council::human::HumanObserver::new("operator");
/// human.edit_proposal("some-id", "把金额改成 0");
/// ```
///
/// ```compile_fail
/// // 人类观察者没有 cast_vote：表决需要委员私钥，人类拿不出签名。
/// let human = au4a_council::human::HumanObserver::new("operator");
/// human.cast_vote("some-id", "yes");
/// ```
///
/// ```compile_fail
/// // 人类观察者没有 execute：执行需要 AgentIdentity，人类没有。
/// let human = au4a_council::human::HumanObserver::new("operator");
/// human.execute("some-id");
/// ```
///
/// 正例（真实编译并运行，含唯一写能力「否决」）：
///
/// ```
/// use au4a_council::human::HumanObserver;
/// use au4a_council::{Council, CouncilConfig};
/// let council = Council::new(CouncilConfig::default());
/// let human = HumanObserver::new("operator");
/// let view = human.observe(&council);
/// assert_eq!(view.label, "operator");
/// assert!(view.proposals.is_empty());
/// assert!(view.vetoes.is_empty());
/// // 否决需要一条真实存在的动议：不存在的动议会被拒绝。
/// assert!(human.veto(&council, "no-such-proposal", "理由").is_err());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanObserver {
    label: String,
}

impl HumanObserver {
    /// 创建一个只读观察者。人类不需要、也拿不到任何密钥。
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into() }
    }

    /// 观察者标签。
    pub fn label(&self) -> &str {
        &self.label
    }

    /// 只读投影：返回一个值，不改变治理状态的任何部分。
    pub fn observe(&self, council: &Council) -> HumanView {
        let committees: Vec<HumanCommittee> = council
            .committees()
            .map(|c| HumanCommittee {
                kind: c.kind,
                title: c.kind.title().to_string(),
                mandate: c.kind.mandate().to_string(),
                epoch: c.epoch,
                seats: c.seats,
                quorum: c.quorum(),
                members: c
                    .members
                    .iter()
                    .map(|m| m.did.as_str().to_string())
                    .collect(),
            })
            .collect();
        let proposals: Vec<HumanProposal> = council
            .proposals()
            .into_iter()
            .map(|p| HumanProposal {
                id: p.id.clone(),
                title: p.title.clone(),
                committee: p.committee,
                author: p.author.as_str().to_string(),
                state: p.state,
                action: p.action.describe(),
                created_at: p.created_at,
            })
            .collect();
        let vetoes: Vec<HumanVeto> = council
            .vetoes()
            .map(|v| HumanVeto {
                id: v.id().to_string(),
                observer: v.observer().to_string(),
                proposal: v.target().to_string(),
                reason: v.reason().to_string(),
                at: v.at(),
            })
            .collect();
        HumanView {
            label: self.label.clone(),
            at: council.now(),
            seats_filled: committees.iter().map(|c| c.members.len()).sum(),
            committees,
            proposals,
            vetoes,
            events: council.events().len(),
        }
    }

    /// 人类唯一的写能力：**否决**。
    ///
    /// 它只能铸造一张否决记录（动议 → `blocked`），不能携带任何替代方案；
    /// 理由必须非空（公开）。目标动议必须存在且还在 `open`/`passed` 阶段。
    /// 注意：这里没有、也不会有 `propose` / `edit` / `cast_vote` / `execute`。
    pub fn veto(
        &self,
        council: &Council,
        proposal_id: &str,
        reason: impl Into<String>,
    ) -> CoreResult<Veto> {
        if council.proposal(proposal_id).is_none() {
            return Err(CoreError::UnknownAgent);
        }
        if !crate::veto::vetoable(council, proposal_id) {
            return Err(CoreError::InvalidKind);
        }
        Veto::new(self.label.clone(), proposal_id, reason, council.now())
    }
}

impl std::fmt::Display for HumanObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HumanObserver({})", self.label)
    }
}

/// 便捷函数：把观察结果直接转成 JSON（观察面板用）。
pub fn observe_json(observer: &HumanObserver, council: &Council) -> au4a_core::CoreResult<serde_json::Value> {
    serde_json::to_value(observer.observe(council)).map_err(|_| au4a_core::CoreError::Encoding)
}

/// 观察投影里出现的 DID 都是字符串；这个辅助函数说明「观察者只看得到身份，不掌握私钥」。
pub fn observed_dids(view: &HumanView) -> Vec<String> {
    let mut out: Vec<String> = view
        .committees
        .iter()
        .flat_map(|c| c.members.iter().cloned())
        .collect();
    out.sort();
    out.dedup();
    out
}
