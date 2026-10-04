//! 否决权（v1.7.5）——本项目「人类只观察」的核心证据。
//!
//! 人类在 AU4A 里只有一个写能力：**否决**。而且这个能力被刻意做成结构性的，不是靠约定：
//!
//! 1. [`Veto`] 的字段**只有一个公开理由**和指向哪条动议，没有任何可以携带「替代方案」的字段；
//!    它没有任何提案/修改/执行方法（`compile_fail` 文档测试把守）。
//! 2. 否决在状态机里只有一条出路：动议进入 [`ProposalState::Blocked`]——终态。
//!    被阻断的动议不能被执行、不能再表决、不能被改判。
//! 3. 人类没有密钥（[`crate::human::HumanObserver`] 不含任何 `AgentKeys`），因此提案、表决、
//!    执行这三件需要签名的事对人类**在类型层面不可达**。
//! 4. 否决必须带非空公开理由，理由进入治理事件日志与只读投影——「只读否决」不等于「沉默否决」。
//!
//! 换句话说：人类能按下的唯一按钮是「停」，而不是「改」。

use au4a_core::{canonical_hash, CoreError, CoreResult, RefusalCode};
use au4a_kernel::Kernel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::proposal::ProposalState;
use crate::Council;

/// 人类否决记录。
///
/// 结构上只有三样东西：**谁（人类标签）**、**否决哪条动议**、**为什么（必须公开）**。
/// 没有载荷字段、没有替代动议字段、没有签名密钥。下面两段代码必须编译失败：
///
/// ```compile_fail
/// // Veto 没有 propose：人类不能借否决权提出动议。
/// let human = au4a_council::HumanObserver::new("operator");
/// let council = au4a_council::Council::new(au4a_council::CouncilConfig::default());
/// let veto = human.veto(&council, "proposal-id", "理由").unwrap();
/// veto.propose("顺便加一条我想要的新规则");
/// ```
///
/// ```compile_fail
/// // Veto 没有 edit / execute：人类不能改写或代执行决议。
/// let human = au4a_council::HumanObserver::new("operator");
/// let council = au4a_council::Council::new(au4a_council::CouncilConfig::default());
/// let veto = human.veto(&council, "proposal-id", "理由").unwrap();
/// veto.edit_proposal("把金额改成 0");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Veto {
    /// 内容地址：`hash(观察者, 动议, 理由)`（不含时刻，便于复算）。
    id: String,
    /// 人类观察者标签（人类没有 DID，只有标签）。
    observer: String,
    /// 被否决的动议 id。
    proposal: String,
    /// 公开理由（必须非空）。
    reason: String,
    /// 否决时刻（逻辑刻度）。
    at: u64,
}

impl Veto {
    /// 由观察者铸造一张否决（唯一构造入口是 [`crate::human::HumanObserver::veto`]）。
    pub(crate) fn new(
        observer: impl Into<String>,
        proposal: impl Into<String>,
        reason: impl Into<String>,
        at: u64,
    ) -> CoreResult<Self> {
        let observer = observer.into();
        let proposal = proposal.into();
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(CoreError::InvalidKind);
        }
        let id = canonical_hash(&json!({
            "observer": observer,
            "proposal": proposal,
            "reason": reason,
        }))?;
        Ok(Self {
            id,
            observer,
            proposal,
            reason,
            at,
        })
    }

    /// 否决记录的内容地址。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 谁否决的（人类标签）。
    pub fn observer(&self) -> &str {
        &self.observer
    }

    /// 否决了哪条动议。
    pub fn target(&self) -> &str {
        &self.proposal
    }

    /// 公开理由。
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// 否决时刻。
    pub fn at(&self) -> u64 {
        self.at
    }

    /// 只读投影。**注意这里没有 `action`、没有 `patch`、没有 `alternative`**：
    /// 否决权无法携带任何会改变决议内容的东西。
    pub fn payload(&self) -> Value {
        json!({
            "id": self.id,
            "observer": self.observer,
            "proposal": self.proposal,
            "reason": self.reason,
            "at": self.at,
        })
    }
}

/// 只读投影里的否决行（观察层展示用）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanVeto {
    /// 内容地址。
    pub id: String,
    /// 人类观察者标签。
    pub observer: String,
    /// 被否决的动议。
    pub proposal: String,
    /// 公开理由。
    pub reason: String,
    /// 时刻。
    pub at: u64,
}

/// 否决落地的收据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VetoReceipt {
    /// 否决内容地址。
    pub veto: String,
    /// 被否决的动议。
    pub proposal: String,
    /// 否决前的动议状态。
    pub previous_state: ProposalState,
    /// 否决后的动议状态（**永远是 `blocked`**）。
    pub state: ProposalState,
    /// 公开理由。
    pub reason: String,
    /// 落地时刻。
    pub at: u64,
}

/// 把一张人类否决落到治理状态上。
///
/// 唯一允许的结局是 `blocked`；任何「顺手改一下决议」的企图在这里都不存在代码路径。
pub fn apply(council: &mut Council, kernel: &mut Kernel, veto: &Veto) -> CoreResult<VetoReceipt> {
    let (state, author) = match council.proposal(veto.target()) {
        Some(p) => (p.state, p.author.clone()),
        None => return Err(CoreError::UnknownAgent),
    };
    if veto.reason().trim().is_empty() {
        kernel.refuse(
            &author,
            RefusalCode::Malformed,
            "veto without a public reason",
        );
        return Err(CoreError::InvalidKind);
    }
    // 只有「还在路上」的动议可以被阻断：open（待表决）与 passed（已通过待执行）。
    if !matches!(state, ProposalState::Open | ProposalState::Passed) {
        kernel.refuse(
            &author,
            RefusalCode::Conflict,
            format!("cannot veto a {} proposal", state.as_str()),
        );
        return Err(CoreError::InvalidKind);
    }
    if council.veto_record(veto.target()).is_some() {
        kernel.refuse(
            &author,
            RefusalCode::Conflict,
            "proposal already blocked by a veto",
        );
        return Err(CoreError::InvalidKind);
    }
    let at = council.tick();
    // 状态机只允许通向 Blocked——人类的手只能按「停」。
    council.apply_state(kernel, veto.target(), ProposalState::Blocked)?;
    council.store_veto(veto.clone());
    let receipt = VetoReceipt {
        veto: veto.id().to_string(),
        proposal: veto.target().to_string(),
        previous_state: state,
        state: ProposalState::Blocked,
        reason: veto.reason().to_string(),
        at,
    };
    council.record_event(
        "veto.blocked",
        veto.target(),
        format!("by {} :: {}", veto.observer(), veto.reason()),
    );
    kernel.emit(
        "council.veto.blocked",
        format!(
            "{} by {} :: {}",
            au4a_core::short_id(veto.target()),
            veto.observer(),
            veto.reason()
        ),
    );
    Ok(receipt)
}

/// 人类能否对这条动议行使否决：只有在 `open` / `passed` 且未被否决过时为真。
pub fn vetoable(council: &Council, proposal_id: &str) -> bool {
    match council.proposal(proposal_id) {
        Some(p) => {
            matches!(p.state, ProposalState::Open | ProposalState::Passed)
                && council.veto_record(proposal_id).is_none()
        }
        None => false,
    }
}

/// 一条动议是否已经被人类否决阻断。
pub fn is_blocked(council: &Council, proposal_id: &str) -> bool {
    matches!(
        council.proposal(proposal_id).map(|p| p.state),
        Some(ProposalState::Blocked)
    )
}

/// 人类观察者标签是否出现在否决记录里（审计用）。
pub fn vetoed_by(council: &Council, observer: &str) -> Vec<String> {
    council
        .vetoes()
        .filter(|v| v.observer() == observer)
        .map(|v| v.target().to_string())
        .collect()
}

/// 观察者标签类型别名：人类没有 DID，只有标签。
pub type ObserverLabel = str;

/// 把人类标签与 Agent 身份区分开：否决记录里永远不出现 DID 形式的观察者。
pub fn label_has_no_did(label: &ObserverLabel) -> bool {
    !label.starts_with("did:")
}

/// 否决记录里的人类不是 [`Did`]（人类没有密钥，也没有自证身份）。
pub fn observer_is_not_an_agent(veto: &Veto) -> bool {
    label_has_no_did(veto.observer())
}
