//! AU4A 轨道 1.7 — Committee Governance 委员会治理（v1.7.1 → v1.7.10）。
//!
//! 治理在 AU4A 里不是「人类审批」，而是 **Agent 之间的制度**：
//!
//! * 五类委员会（资源 / 任务 / 仲裁 / 进化 / 安全）由**选举**产生，席位来自信誉与在线时长；
//! * 动议只能由 **Agent** 提出——[`proposal::AgentIdentity`] 是唯一入口，
//!   动议内容必须由 Agent 私钥签名，人类观察者 [`human::HumanObserver`] 在类型层面
//!   **没有**任何提案/修改能力（`compile_fail` 文档测试把守）；
//! * 决议按 BFT-lite 法定人数表决（`n ≥ 3f+1`），重复投票被拒，模棱两可（双签）作废该轮；
//! * 通过的决议由执行引擎落成真实状态变更（策略、信誉、账本）；
//! * 人类只在极端情况行使**否决权**：只能阻断、必须公开理由、不能改动决议内容。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责，并留下自检与证据。
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）。
//! 本层不读墙钟、不碰文件、不开网络：时间一律来自 [`au4a_core::LogicalClock`]。
//!
//! 错误映射约定：基元层 `CoreError` 没有 `Unauthorized`/`Conflict` 变体，因此
//! 「授权不成立」映射为 [`CoreError::InvalidSignature`]、「重复表达意志」映射为
//! [`CoreError::DuplicateAgent`]；而类型化分类（`unauthorized`/`conflict`）照常写进
//! 内核的拒绝账，人类只读投影里能看到完整分类。

#![forbid(unsafe_code)]

pub mod committee;
pub mod election;
pub mod execution;
pub mod human;
pub mod proposal;
pub mod voting;

pub use committee::{Committee, CommitteeKind, Member, COMMITTEE_COUNT};
pub use election::{
    Candidate, ElectionBallot, ElectionConfig, ElectionOutcome, Elected, IgnoredBallot, Ineligible,
    ScoreRow,
};
pub use execution::{ExecutionEffect, ExecutionReceipt};
pub use human::{HumanCommittee, HumanObserver, HumanProposal, HumanView};
pub use proposal::{Action, AgentIdentity, Proposal, ProposalDraft, ProposalState};
pub use voting::{Choice, RoundOutcome, RoundState, Tally, Vote, VotingRound};

use std::collections::BTreeMap;

use au4a_core::{
    AgentKeys, CoreError, CoreResult, Credits, Did, LogicalClock, RefusalCode, SelfCheck,
};
use au4a_kernel::{Kernel, KernelConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 轨道号。
pub const TRACK: &str = "1.7";
/// 轨道标题。
pub const TITLE: &str = "Committee Governance 委员会治理";
/// 版本区间。
pub const RANGE: &str = "v1.7.1 → v1.7.10";
/// crate 名（编译期存在性标记）。
pub const CRATE: &str = "au4a_council";

/// 治理配置。人类可以设定安全底线，但不能设定「谁当选」。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouncilConfig {
    /// 选举参数（席位、门槛、任期）。
    pub election: ElectionConfig,
    /// 候选人质押下限（与内核准入一致）。
    pub min_stake: Credits,
    /// 动议标题长度上限。
    pub max_title_len: usize,
}

impl Default for CouncilConfig {
    fn default() -> Self {
        Self {
            election: ElectionConfig::default(),
            min_stake: Credits(10),
            max_title_len: 128,
        }
    }
}

/// 一条治理事件（追加式、带逻辑时刻）。
///
/// v1.7.10 会把这些事件串成哈希链：本条记录的哈希覆盖前一条，篡改任何一条都会断链。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouncilEvent {
    /// 逻辑时刻。
    pub at: u64,
    /// 事件类型（稳定字符串）。
    pub kind: String,
    /// 主题（动议 id / 选举 id / DID）。
    pub subject: String,
    /// 细节（人类可读）。
    pub detail: String,
}

/// 委员会治理本体：席位、信誉账、动议、事件日志。
///
/// 所有内部容器都是 `BTreeMap`/`Vec`，因此遍历顺序与插入顺序无关——这是「可复算」的一半。
#[derive(Clone, Debug)]
pub struct Council {
    cfg: CouncilConfig,
    clock: LogicalClock,
    reputation: BTreeMap<Did, u32>,
    uptime: BTreeMap<Did, u64>,
    committees: BTreeMap<CommitteeKind, Committee>,
    epochs: BTreeMap<CommitteeKind, u64>,
    elections: Vec<ElectionOutcome>,
    proposals: BTreeMap<String, Proposal>,
    proposal_order: Vec<String>,
    rounds: BTreeMap<(String, u32), VotingRound>,
    policies: BTreeMap<String, i64>,
    executions: BTreeMap<String, ExecutionReceipt>,
    events: Vec<CouncilEvent>,
}

impl Council {
    /// 新建治理层。
    pub fn new(cfg: CouncilConfig) -> Self {
        Self {
            cfg,
            clock: LogicalClock::new(),
            reputation: BTreeMap::new(),
            uptime: BTreeMap::new(),
            committees: BTreeMap::new(),
            epochs: BTreeMap::new(),
            elections: Vec::new(),
            proposals: BTreeMap::new(),
            proposal_order: Vec::new(),
            rounds: BTreeMap::new(),
            policies: BTreeMap::new(),
            executions: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// 只读配置。
    pub fn config(&self) -> &CouncilConfig {
        &self.cfg
    }

    /// 当前逻辑时刻。
    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    /// 推进一格逻辑时钟。
    pub fn tick(&mut self) -> u64 {
        self.clock.tick()
    }

    /// 记录信誉（万分比）。信誉**只能**由治理写，Agent 不能自报。
    pub fn note_reputation(&mut self, did: &Did, reputation_bp: u32) {
        self.reputation.insert(did.clone(), reputation_bp);
    }

    /// 记录在线时长（累加逻辑刻度）。
    pub fn note_uptime(&mut self, did: &Did, ticks: u64) {
        let entry = self.uptime.entry(did.clone()).or_insert(0);
        *entry = entry.saturating_add(ticks);
    }

    /// 读取信誉。
    pub fn reputation(&self, did: &Did) -> u32 {
        self.reputation.get(did).copied().unwrap_or(0)
    }

    /// 读取在线时长。
    pub fn uptime(&self, did: &Did) -> u64 {
        self.uptime.get(did).copied().unwrap_or(0)
    }

    /// 按类别取委员会。
    pub fn committee(&self, kind: CommitteeKind) -> Option<&Committee> {
        self.committees.get(&kind)
    }

    /// 遍历全部已选出的委员会（类别升序）。
    pub fn committees(&self) -> impl Iterator<Item = &Committee> {
        self.committees.values()
    }

    /// 选举日志（按发生顺序）。
    pub fn elections(&self) -> &[ElectionOutcome] {
        &self.elections
    }

    /// 治理事件日志（追加式）。
    pub fn events(&self) -> &[CouncilEvent] {
        &self.events
    }

    /// 追加一条治理事件，返回其在日志中的序号。
    pub fn record_event(&mut self, kind: &str, subject: &str, detail: impl Into<String>) -> u64 {
        let at = self.clock.now();
        self.events.push(CouncilEvent {
            at,
            kind: kind.to_string(),
            subject: subject.to_string(),
            detail: detail.into(),
        });
        self.events.len().saturating_sub(1) as u64
    }

    /// 由内核花名册 + 治理账构造候选人花名册（按 DID 升序，确定性）。
    pub fn roster(&self, kernel: &Kernel) -> Vec<Candidate> {
        let mut roster: Vec<Candidate> = kernel
            .agents()
            .map(|card| Candidate {
                did: card.did.clone(),
                reputation_bp: self.reputation(&card.did),
                uptime: self.uptime(&card.did),
                stake: card.stake,
            })
            .collect();
        roster.sort_by(|a, b| a.did.cmp(&b.did));
        roster
    }

    /// 跑一场选举并把结果安装为在任委员会。
    ///
    /// 恶意选票不会静默消失：跨委员会重放记 `malformed`，伪造/未注册记 `unauthorized`，
    /// 重复投票记 `conflict`，全部写进内核的拒绝账（人类只读投影里能看到）。
    pub fn elect(
        &mut self,
        kernel: &mut Kernel,
        kind: CommitteeKind,
        ballots: &[ElectionBallot],
    ) -> CoreResult<ElectionOutcome> {
        let roster = self.roster(kernel);

        let mut seen: std::collections::BTreeSet<Did> = std::collections::BTreeSet::new();
        for ballot in ballots {
            if ballot.committee != kind {
                kernel.refuse(&ballot.voter, RefusalCode::Malformed, "ballot bound to another committee");
                return Err(CoreError::InvalidKind);
            }
            if let Err(err) = ballot.verify() {
                kernel.refuse(&ballot.voter, RefusalCode::Unauthorized, format!("ballot signature: {err}"));
                return Err(err);
            }
            if !kernel.agents().any(|card| card.did == ballot.voter) {
                kernel.refuse(&ballot.voter, RefusalCode::Unauthorized, "ballot from unregistered voter");
                return Err(CoreError::UnknownAgent);
            }
            if !seen.insert(ballot.voter.clone()) {
                kernel.refuse(&ballot.voter, RefusalCode::Conflict, "duplicate ballot from same voter");
                return Err(CoreError::DuplicateAgent);
            }
        }

        let epoch = self.epochs.get(&kind).copied().unwrap_or(0) + 1;
        let at = self.clock.tick();
        let outcome = election::run(kind, epoch, &self.cfg.election, &roster, ballots, at)?;

        let members: Vec<Member> = outcome
            .elected
            .iter()
            .map(|e| Member {
                did: e.did.clone(),
                rank: e.rank,
                score: e.score,
                reputation_bp: e.reputation_bp,
                uptime: e.uptime,
                term_ends_at: at.saturating_add(self.cfg.election.term),
            })
            .collect();
        let committee = Committee {
            kind,
            epoch,
            seats: outcome.seats,
            elected_at: at,
            election_id: outcome.id.clone(),
            members,
        };
        kernel.emit(
            "council.election",
            format!(
                "{} epoch={} 在任={} 计入票={} 忽略票={} id={}",
                kind.as_str(),
                epoch,
                committee.size(),
                outcome.ballots_counted,
                outcome.ballots_ignored.len(),
                au4a_core::short_id(&outcome.id)
            ),
        );
        self.committees.insert(kind, committee);
        self.epochs.insert(kind, epoch);
        self.elections.push(outcome.clone());
        self.record_event(
            "election.seated",
            &outcome.id,
            format!(
                "{} epoch={} 在任={} 计入票={} 忽略票={}",
                kind.as_str(),
                epoch,
                outcome.elected.len(),
                outcome.ballots_counted,
                outcome.ballots_ignored.len()
            ),
        );
        Ok(outcome)
    }

    /// 提交动议。**唯一的提案入口**，需要三样东西同时成立：
    /// 能力凭证（[`AgentIdentity`]，只有私钥持有者能构造）、作者签名（绑定内容）、
    /// 以及作者须为该委员会的在任委员。
    pub fn propose(
        &mut self,
        kernel: &mut Kernel,
        author: &AgentIdentity,
        draft: ProposalDraft,
    ) -> CoreResult<Proposal> {
        if let Err(err) = draft.verify() {
            kernel.refuse(
                author.did(),
                RefusalCode::Unauthorized,
                format!("proposal signature: {err}"),
            );
            return Err(err);
        }
        if &draft.author != author.did() {
            kernel.refuse(
                author.did(),
                RefusalCode::Unauthorized,
                "identity token does not match the draft signer",
            );
            return Err(CoreError::InvalidSignature);
        }
        if kernel.card(&draft.author).is_none() {
            kernel.refuse(&draft.author, RefusalCode::Unauthorized, "proposer is not a registered agent");
            return Err(CoreError::UnknownAgent);
        }
        let seated = self
            .committees
            .get(&draft.committee)
            .map(|c| c.has_member(&draft.author));
        match seated {
            None => {
                kernel.refuse(&draft.author, RefusalCode::StaleEpoch, "no committee of that kind is seated");
                return Err(CoreError::UnknownAgent);
            }
            Some(false) => {
                kernel.refuse(&draft.author, RefusalCode::Unauthorized, "proposer is not a seated member");
                return Err(CoreError::InvalidSignature);
            }
            Some(true) => {}
        }
        if let Err(err) = draft.action.validate() {
            kernel.refuse(&draft.author, RefusalCode::Malformed, format!("illegal action: {err}"));
            return Err(err);
        }
        if draft.title.trim().is_empty() || draft.title.len() > self.cfg.max_title_len {
            kernel.refuse(&draft.author, RefusalCode::Malformed, "title empty or too long");
            return Err(CoreError::InvalidKind);
        }
        let id = draft.id()?;
        if self.proposals.contains_key(&id) {
            kernel.refuse(&draft.author, RefusalCode::Conflict, "identical proposal already exists");
            return Err(CoreError::DuplicateAgent);
        }
        self.clock.tick();
        let proposal = Proposal {
            id: id.clone(),
            author: draft.author.clone(),
            committee: draft.committee,
            title: draft.title.clone(),
            action: draft.action.clone(),
            created_at: self.clock.now(),
            state: ProposalState::Open,
            round: 0,
        };
        kernel.emit(
            "council.proposal",
            format!(
                "{} {} {}",
                au4a_core::short_id(&id),
                draft.committee.as_str(),
                draft.title
            ),
        );
        self.proposals.insert(id.clone(), proposal.clone());
        self.proposal_order.push(id.clone());
        self.record_event(
            "proposal.open",
            &id,
            format!(
                "{} by {} :: {}",
                draft.committee.as_str(),
                au4a_core::short_id(draft.author.as_str()),
                draft.title
            ),
        );
        Ok(proposal)
    }

    /// 读一条动议。
    pub fn proposal(&self, id: &str) -> Option<&Proposal> {
        self.proposals.get(id)
    }

    /// 全部动议，按提交顺序。
    pub fn proposals(&self) -> Vec<&Proposal> {
        self.proposal_order.iter().filter_map(|id| self.proposals.get(id)).collect()
    }

    /// 某个委员会的动议，按提交顺序。
    pub fn proposals_of(&self, kind: CommitteeKind) -> Vec<&Proposal> {
        self.proposals()
            .into_iter()
            .filter(|p| p.committee == kind)
            .collect()
    }

    /// 动议状态迁移（类型化状态机）。非法迁移一律 [`CoreError::InvalidKind`]。
    pub fn transition_to(
        &mut self,
        kernel: &mut Kernel,
        id: &str,
        next: ProposalState,
    ) -> CoreResult<Proposal> {
        self.clock.tick();
        self.apply_state(kernel, id, next)?;
        self.proposals.get(id).cloned().ok_or(CoreError::UnknownAgent)
    }

    /// 状态落地的唯一内部路径：先过状态机，再记账。
    fn apply_state(&mut self, kernel: &mut Kernel, id: &str, next: ProposalState) -> CoreResult<()> {
        let (state, author) = match self.proposals.get(id) {
            Some(p) => (p.state, p.author.clone()),
            None => return Err(CoreError::UnknownAgent),
        };
        if !state.can_transition_to(next) {
            kernel.refuse(
                &author,
                RefusalCode::Conflict,
                format!("illegal transition {} -> {}", state.as_str(), next.as_str()),
            );
            return Err(CoreError::InvalidKind);
        }
        match self.proposals.get_mut(id) {
            Some(p) => p.state = next,
            None => return Err(CoreError::UnknownAgent),
        }
        kernel.emit(
            "council.proposal.state",
            format!("{} {} -> {}", au4a_core::short_id(id), state.as_str(), next.as_str()),
        );
        self.record_event("proposal.state", id, format!("{} -> {}", state.as_str(), next.as_str()));
        Ok(())
    }

    /// 为一条处于 `open` 的动议开启新一轮表决，返回本轮快照。
    ///
    /// 被拒绝的情形：动议不存在、动议不在 `open`、受理委员会没有表决资格（含 0 席委员会）。
    pub fn open_round(&mut self, kernel: &mut Kernel, proposal_id: &str) -> CoreResult<RoundState> {
        let (committee_kind, state, current_round, author) = match self.proposals.get(proposal_id) {
            Some(p) => (p.committee, p.state, p.round, p.author.clone()),
            None => return Err(CoreError::UnknownAgent),
        };
        if state != ProposalState::Open {
            kernel.refuse(
                &author,
                RefusalCode::Conflict,
                format!("cannot open a round for a {} proposal", state.as_str()),
            );
            return Err(CoreError::InvalidKind);
        }
        let (seats, quorum, fault_bound) = match self.committees.get(&committee_kind) {
            Some(c) if c.is_bft_consistent() => (c.size(), c.quorum(), c.fault_bound()),
            Some(_) => {
                kernel.refuse(&author, RefusalCode::PolicyDenied, "committee has no voting capacity");
                return Err(CoreError::InvalidKind);
            }
            None => {
                kernel.refuse(&author, RefusalCode::StaleEpoch, "no committee of that kind is seated");
                return Err(CoreError::UnknownAgent);
            }
        };
        let round = current_round.saturating_add(1);
        let at = self.clock.tick();
        let voting_round = VotingRound {
            proposal: proposal_id.to_string(),
            committee: committee_kind,
            round,
            n: seats,
            f: fault_bound,
            quorum,
            votes: BTreeMap::new(),
            outcome: RoundOutcome::Pending,
            opened_at: at,
        };
        if let Some(p) = self.proposals.get_mut(proposal_id) {
            p.round = round;
        }
        self.rounds
            .insert((proposal_id.to_string(), round), voting_round.clone());
        kernel.emit(
            "council.vote.open",
            format!(
                "{} round={} n={} f={} quorum={}",
                au4a_core::short_id(proposal_id),
                round,
                seats,
                fault_bound,
                quorum
            ),
        );
        self.record_event(
            "vote.open",
            proposal_id,
            format!("round={round} n={seats} f={fault_bound} quorum={quorum}"),
        );
        Ok(voting_round.state())
    }

    /// 投一张表决票。
    ///
    /// 具名失败模式：
    /// * 签名/委员资格不成立 → [`CoreError::InvalidSignature`] + `unauthorized`（单次即恶意证据）；
    /// * 轮次不匹配或轮次已关闭 → [`CoreError::InvalidVersion`] + `stale_epoch`；
    /// * 同轮同选择重复投票 → [`CoreError::DuplicateAgent`] + `conflict`（本轮不受影响）；
    /// * 同轮不同选择（模棱两可/双签）→ [`CoreError::DuplicateAgent`] + `conflict`，
    ///   **整轮作废**（`void_ambiguous`），已投票全部不计入结论，必须重开一轮。
    pub fn cast_vote(&mut self, kernel: &mut Kernel, vote: Vote) -> CoreResult<RoundState> {
        let (committee_kind, proposal_round, proposal_state) = match self.proposals.get(&vote.proposal) {
            Some(p) => (p.committee, p.round, p.state),
            None => return Err(CoreError::UnknownAgent),
        };
        if let Err(err) = vote.verify() {
            kernel.refuse(&vote.voter, RefusalCode::Unauthorized, format!("vote signature: {err}"));
            return Err(err);
        }
        let seated = self
            .committees
            .get(&committee_kind)
            .map(|c| c.has_member(&vote.voter))
            .unwrap_or(false);
        if !seated {
            kernel.refuse(&vote.voter, RefusalCode::Unauthorized, "voter is not a seated member");
            return Err(CoreError::InvalidSignature);
        }
        // 轮次不匹配或轮次已关闭 → 竞争语义（stale_epoch），不升级为恶意。
        if proposal_round == 0 || vote.round != proposal_round {
            kernel.refuse(
                &vote.voter,
                RefusalCode::StaleEpoch,
                format!("vote for round {} but the current round is {proposal_round}", vote.round),
            );
            return Err(CoreError::InvalidVersion);
        }
        let key = (vote.proposal.clone(), vote.round);
        let (exists, closed) = match self.rounds.get(&key) {
            Some(r) => (true, r.outcome.is_closed()),
            None => (false, true),
        };
        if !exists {
            kernel.refuse(&vote.voter, RefusalCode::StaleEpoch, "no such voting round");
            return Err(CoreError::InvalidVersion);
        }
        if closed {
            kernel.refuse(&vote.voter, RefusalCode::StaleEpoch, "voting round is closed");
            return Err(CoreError::InvalidVersion);
        }
        // 轮次还开着但动议已不在待表决状态（被否决阻断 / 已执行）→ 协议冲突。
        if proposal_state != ProposalState::Open {
            kernel.refuse(
                &vote.voter,
                RefusalCode::Conflict,
                format!("proposal is {}", proposal_state.as_str()),
            );
            return Err(CoreError::InvalidKind);
        }
        let previous = self
            .rounds
            .get(&key)
            .and_then(|r| r.votes.get(vote.voter.as_str()))
            .map(|v| v.choice);
        if let Some(previous) = previous {
            self.clock.tick();
            if previous == vote.choice {
                kernel.refuse(&vote.voter, RefusalCode::Conflict, "duplicate vote in the same round");
                return Err(CoreError::DuplicateAgent);
            }
            if let Some(round) = self.rounds.get_mut(&key) {
                round.outcome = RoundOutcome::VoidAmbiguous;
                round.votes.insert(vote.voter.as_str().to_string(), vote.clone());
            }
            kernel.refuse(&vote.voter, RefusalCode::Conflict, "ambiguous double vote: round voided");
            kernel.emit(
                "council.vote.void",
                format!(
                    "{} round={} 双签作废（{} vs {}）",
                    au4a_core::short_id(&vote.proposal),
                    vote.round,
                    previous.as_str(),
                    vote.choice.as_str()
                ),
            );
            self.record_event(
                "vote.void",
                &vote.proposal,
                format!(
                    "round={} voter={} {} vs {} 本轮作废",
                    vote.round,
                    au4a_core::short_id(vote.voter.as_str()),
                    previous.as_str(),
                    vote.choice.as_str()
                ),
            );
            return Err(CoreError::DuplicateAgent);
        }

        self.clock.tick();
        match self.rounds.get_mut(&key) {
            Some(round) => {
                round.votes.insert(vote.voter.as_str().to_string(), vote.clone());
                let outcome = round.tally().outcome();
                round.outcome = outcome;
            }
            None => return Err(CoreError::InvalidVersion),
        }
        let state = match self.rounds.get(&key) {
            Some(round) => round.state(),
            None => return Err(CoreError::InvalidVersion),
        };
        self.record_event(
            "vote.cast",
            &vote.proposal,
            format!(
                "round={} {} yes={} no={} abstain={} participation={}/{} quorum={}",
                state.round,
                vote.choice.as_str(),
                state.tally.yes,
                state.tally.no,
                state.tally.abstain,
                state.tally.participation,
                state.tally.n,
                state.tally.quorum
            ),
        );
        match state.outcome {
            RoundOutcome::Passed => {
                self.apply_state(kernel, &vote.proposal, ProposalState::Passed)?;
                kernel.emit(
                    "council.vote.passed",
                    format!(
                        "{} round={} yes={} quorum={}",
                        au4a_core::short_id(&vote.proposal),
                        state.round,
                        state.tally.yes,
                        state.tally.quorum
                    ),
                );
            }
            RoundOutcome::Rejected => {
                self.apply_state(kernel, &vote.proposal, ProposalState::Rejected)?;
                kernel.emit(
                    "council.vote.rejected",
                    format!(
                        "{} round={} no={} quorum={}",
                        au4a_core::short_id(&vote.proposal),
                        state.round,
                        state.tally.no,
                        state.tally.quorum
                    ),
                );
            }
            RoundOutcome::Pending | RoundOutcome::VoidAmbiguous => {}
        }
        Ok(state)
    }

    /// 读某一轮表决的快照。
    pub fn round(&self, proposal_id: &str, round: u32) -> Option<RoundState> {
        self.rounds.get(&(proposal_id.to_string(), round)).map(|r| r.state())
    }

    /// 读动议当前轮次的快照。
    pub fn current_round(&self, proposal_id: &str) -> Option<RoundState> {
        let round = self.proposals.get(proposal_id)?.round;
        self.round(proposal_id, round)
    }

    /// 全部表决轮次（按动议 id、轮次升序）。
    pub fn rounds(&self) -> impl Iterator<Item = &VotingRound> {
        self.rounds.values()
    }

    /// 写入治理策略。**只有执行引擎会调用它**（Agent 不能直接改策略）。
    pub(crate) fn set_policy(&mut self, key: String, value: i64) {
        self.policies.insert(key, value);
    }

    /// 读一条治理策略。
    pub fn policy(&self, key: &str) -> Option<i64> {
        self.policies.get(key).copied()
    }

    /// 全部治理策略（键升序）。
    pub fn policies(&self) -> &BTreeMap<String, i64> {
        &self.policies
    }

    /// 保存一张执行收据（执行引擎内部使用）。
    pub(crate) fn store_execution(&mut self, receipt: ExecutionReceipt) {
        self.executions.insert(receipt.proposal.clone(), receipt);
    }

    /// 读某条动议的执行收据。
    pub fn execution(&self, proposal_id: &str) -> Option<&ExecutionReceipt> {
        self.executions.get(proposal_id)
    }

    /// 全部执行收据（按动议 id 升序）。
    pub fn executions(&self) -> impl Iterator<Item = &ExecutionReceipt> {
        self.executions.values()
    }

    /// 执行一条**已通过**的动议（v1.7.4 执行引擎的唯一入口，见 [`execution::execute`]）。
    pub fn execute(
        &mut self,
        kernel: &mut Kernel,
        executor: &AgentIdentity,
        proposal_id: &str,
    ) -> CoreResult<ExecutionReceipt> {
        execution::execute(self, kernel, executor, proposal_id)
    }

    /// 治理层自检（v1.7.2 覆盖选举、席位与动议；后续版本追加表决、否决、审计）。
    pub fn checks(&self) -> Vec<SelfCheck> {
        let mut checks = Vec::new();
        let installed = self.committees.len();
        let seats: usize = self.committees().map(|c| c.size()).sum();
        checks.push(if installed > 0 {
            SelfCheck::pass(
                TRACK,
                "council.committees.installed",
                format!("{installed} 类委员会在任，共 {seats} 个席位"),
            )
        } else {
            SelfCheck::fail(TRACK, "council.committees.installed", "没有任何委员会在任")
        });
        let bft_ok = self
            .committees()
            .all(|c| c.is_bft_consistent() && c.quorum() >= c.fault_bound() * 2 + 1);
        checks.push(if bft_ok {
            SelfCheck::pass(
                TRACK,
                "council.quorum.bft",
                format!("{installed} 届委员会满足 n ≥ 3f+1 且 quorum = n - f 且 quorum ≥ 2f+1"),
            )
        } else {
            SelfCheck::fail(TRACK, "council.quorum.bft", "存在不满足 BFT-lite 数学的委员会")
        });
        let empty: Vec<&str> = self
            .committees()
            .filter(|c| c.is_empty())
            .map(|c| c.kind.as_str())
            .collect();
        checks.push(if empty.is_empty() {
            SelfCheck::pass(
                TRACK,
                "council.committees.nonempty",
                format!("{installed} 类委员会均有在任成员，无 0 席委员会（0 席会使 quorum=0 自动通过）"),
            )
        } else {
            SelfCheck::fail(
                TRACK,
                "council.committees.nonempty",
                format!("存在 0 席委员会：{}", empty.join(",")),
            )
        });
        let ids_ok = self.proposals.values().all(|p| {
            Proposal::id_for(&p.author, p.committee, &p.title, &p.action)
                .map(|recomputed| recomputed == p.id)
                .unwrap_or(false)
        });
        checks.push(if ids_ok {
            SelfCheck::pass(
                TRACK,
                "council.proposals.content_addressed",
                format!("{} 条动议的 id 均可由内容复算（作者/委员会/标题/动作）", self.proposals.len()),
            )
        } else {
            SelfCheck::fail(TRACK, "council.proposals.content_addressed", "存在 id 与内容不一致的动议")
        });
        let monotonic = self.events.windows(2).all(|w| w[0].at <= w[1].at);
        checks.push(if monotonic {
            SelfCheck::pass(
                TRACK,
                "council.events.monotonic",
                format!("{} 条治理事件的逻辑时刻非递减", self.events.len()),
            )
        } else {
            SelfCheck::fail(TRACK, "council.events.monotonic", "治理事件日志的时刻倒退")
        });

        // 表决不变式：计票自洽、投票人只能是在任委员、结论必须与动议状态一致。
        let rounds = self.rounds.len();
        let tallies_ok = self.rounds.values().all(|r| r.tally().is_consistent());
        checks.push(if tallies_ok {
            SelfCheck::pass(
                TRACK,
                "council.votes.tally_consistent",
                format!("{rounds} 轮表决的计票自洽（票数守恒、无人重复计入、yes/no 不可能同时达法定人数）"),
            )
        } else {
            SelfCheck::fail(TRACK, "council.votes.tally_consistent", "存在自相矛盾的计票")
        });
        let members_only = self.rounds.values().all(|r| {
            let committee = self.committees.get(&r.committee);
            r.votes.keys().all(|did| match (Did::parse(did), committee) {
                (Ok(did), Some(c)) => c.has_member(&did),
                _ => false,
            })
        });
        checks.push(if members_only {
            SelfCheck::pass(
                TRACK,
                "council.votes.members_only",
                format!("{rounds} 轮表决的所有投票人都是对应委员会的在任委员"),
            )
        } else {
            SelfCheck::fail(TRACK, "council.votes.members_only", "存在非委员投票")
        });
        let decided_ok = self.rounds.values().all(|r| match r.outcome {
            RoundOutcome::Passed => self
                .proposals
                .get(&r.proposal)
                .map(|p| matches!(p.state, ProposalState::Passed | ProposalState::Executed | ProposalState::Blocked))
                .unwrap_or(false),
            RoundOutcome::Rejected => self
                .proposals
                .get(&r.proposal)
                .map(|p| p.state == ProposalState::Rejected)
                .unwrap_or(false),
            RoundOutcome::Pending | RoundOutcome::VoidAmbiguous => true,
        });
        checks.push(if decided_ok {
            SelfCheck::pass(
                TRACK,
                "council.rounds.decided_matches_proposal",
                format!("{rounds} 轮表决的结论与动议状态一致（作废轮不产生结论）"),
            )
        } else {
            SelfCheck::fail(
                TRACK,
                "council.rounds.decided_matches_proposal",
                "存在与动议状态不一致的表决结论",
            )
        });

        // 执行不变式：收据的账本效应必须自洽，且「已执行」与收据一一对应。
        let executions = self.executions.len();
        let effects_ok = self
            .executions
            .values()
            .all(|r| r.ledger_effect_consistent() && r.conservation_ok);
        checks.push(if effects_ok {
            SelfCheck::pass(
                TRACK,
                "council.executions.ledger_effects",
                format!("{executions} 张执行收据的账本效应自洽（转账不动总量、罚没量入 slashed、守恒成立）"),
            )
        } else {
            SelfCheck::fail(TRACK, "council.executions.ledger_effects", "存在账本效应不自洽的执行收据")
        });
        let paired = self
            .proposals
            .values()
            .filter(|p| p.state == ProposalState::Executed)
            .all(|p| self.executions.contains_key(&p.id))
            && self
                .executions
                .values()
                .all(|r| self.proposals.get(&r.proposal).map(|p| p.state == ProposalState::Executed).unwrap_or(false));
        checks.push(if paired {
            SelfCheck::pass(
                TRACK,
                "council.executions.state_matches",
                format!("{executions} 张收据与 executed 状态一一对应（无未执行的状态、无无收据的执行）"),
            )
        } else {
            SelfCheck::fail(TRACK, "council.executions.state_matches", "executed 状态与执行收据不一一对应")
        });
        checks
    }
}

/// 确定性种子：带轨道前缀，避免在共享内核里与其它轨道撞 DID。
fn council_seed(i: u8) -> [u8; 32] {
    let mut seed = [0u8; 32];
    seed[0] = 0x17;
    seed[1] = 0x07;
    seed[2] = i;
    seed[3] = 0xC0;
    for (n, b) in seed.iter_mut().enumerate().skip(4) {
        *b = (n as u8) ^ i.wrapping_mul(31) ^ 0x5A;
    }
    seed
}

/// 幂等登记：共享内核里可能已经有别的轨道注册过同一 DID，重复注册不算失败。
fn ensure_agent(
    kernel: &mut Kernel,
    keys: &AgentKeys,
    display: &str,
    skills: &[&str],
    stake: Credits,
) -> CoreResult<()> {
    match kernel.register(keys, display, skills, stake) {
        Ok(_) => Ok(()),
        Err(CoreError::DuplicateAgent) => Ok(()),
        Err(err) => Err(err),
    }
}

/// 登记治理 Agent：6 个有真实贡献的 Agent + 8 个信誉为零的空壳 DID。
fn enroll(kernel: &mut Kernel, council: &mut Council) -> CoreResult<(Vec<AgentKeys>, Vec<AgentKeys>)> {
    let mut agents = Vec::new();
    let mut socks = Vec::new();
    for i in 0..6u8 {
        let keys = AgentKeys::from_seed(&council_seed(i));
        ensure_agent(kernel, &keys, &format!("governor-{i}"), &["governance.vote"], Credits(20))?;
        council.note_reputation(&keys.did(), 4_000 + u32::from(i) * 400);
        council.note_uptime(&keys.did(), 100 + u64::from(i) * 50);
        agents.push(keys);
    }
    for i in 0..8u8 {
        let keys = AgentKeys::from_seed(&council_seed(100 + i));
        ensure_agent(kernel, &keys, &format!("sock-{i}"), &["governance.vote"], Credits(20))?;
        council.note_uptime(&keys.did(), 1);
        socks.push(keys);
    }
    Ok((agents, socks))
}

/// 选出五类委员会（资源委员会那一场带一次空壳刷票）。
fn seat_all_committees(
    kernel: &mut Kernel,
    council: &mut Council,
    agents: &[AgentKeys],
    socks: &[AgentKeys],
) -> CoreResult<()> {
    let eligible: Vec<Did> = agents.iter().map(|k| k.did()).collect();
    for (j, kind) in CommitteeKind::ALL.iter().enumerate() {
        let picks: Vec<Did> = (0..3).map(|n| eligible[(j + n) % eligible.len()].clone()).collect();
        let mut ballots: Vec<ElectionBallot> = Vec::new();
        for keys in agents {
            ballots.push(ElectionBallot::cast(keys, *kind, &picks)?);
        }
        if *kind == CommitteeKind::Resource {
            for keys in socks {
                ballots.push(ElectionBallot::cast(keys, *kind, &picks)?);
            }
        }
        council.elect(kernel, *kind, &ballots)?;
    }
    Ok(())
}

/// 一次完整建场：登记 → 选出五类委员会 → 由资源委员会委员提交一条动议。
///
/// 自检、结果摘要与 scenario 共用它，保证「文档里跑的」和「自检里跑的」是同一条路径。
struct Build {
    kernel: Kernel,
    council: Council,
    agents: Vec<AgentKeys>,
    sock_dids: Vec<Did>,
    proposal: Proposal,
    round: RoundState,
    receipt: ExecutionReceipt,
}

/// 按 DID 找回该 Agent 的密钥（建场与 scenario 共用）。
fn keys_for<'a>(agents: &'a [AgentKeys], did: &Did) -> Option<&'a AgentKeys> {
    agents.iter().find(|k| &k.did() == did)
}

/// 让受理委员会全体在任委员投赞成票，直到本轮出结论。
///
/// 返回最后一次投票后的轮次快照（若委员会人数不足或有人缺席，可能仍是 `pending`）。
fn vote_yes_all(
    kernel: &mut Kernel,
    council: &mut Council,
    agents: &[AgentKeys],
    proposal_id: &str,
) -> CoreResult<RoundState> {
    let round = council.open_round(kernel, proposal_id)?;
    let members = council
        .proposal(proposal_id)
        .and_then(|p| council.committee(p.committee))
        .map(|c| c.member_dids())
        .unwrap_or_default();
    let mut state = round;
    for did in &members {
        if state.outcome.is_closed() {
            // 本轮已经出结论：后面的票不必再发（发了也会按 stale_epoch 拒绝）。
            break;
        }
        let keys = match keys_for(agents, did) {
            Some(k) => k,
            None => continue,
        };
        let vote = Vote::cast(keys, proposal_id, state.round, Choice::Yes)?;
        state = council.cast_vote(kernel, vote)?;
    }
    Ok(state)
}

fn build_full() -> CoreResult<Build> {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut council = Council::new(CouncilConfig::default());
    let (agents, socks) = enroll(&mut kernel, &mut council)?;
    seat_all_committees(&mut kernel, &mut council, &agents, &socks)?;

    let member = council
        .committee(CommitteeKind::Resource)
        .and_then(|c| c.members.first())
        .map(|m| m.did.clone());
    let proposer = match agents.iter().find(|k| Some(&k.did()) == member.as_ref()) {
        Some(k) => k,
        None => return Err(CoreError::UnknownAgent),
    };
    let identity = AgentIdentity::from_keys(proposer);
    let draft = ProposalDraft::by(
        proposer,
        CommitteeKind::Resource,
        "把 cpu-proto 结算上限设为 250 微积分",
        Action::SetPolicy { key: String::from("cpu_proto_settle_cap"), value: 250 },
    )?;
    let proposal = council.propose(&mut kernel, &identity, draft)?;
    let round = vote_yes_all(&mut kernel, &mut council, &agents, &proposal.id)?;
    // 执行：由同一位委员触发，把通过的决议落成策略变更。
    let receipt = council.execute(&mut kernel, &identity, &proposal.id)?;

    Ok(Build {
        kernel,
        council,
        agents,
        sock_dids: socks.iter().map(|k| k.did()).collect(),
        proposal,
        round,
        receipt,
    })
}

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
///
/// 每一项都是**真实断言**（会真的跑选举与提案），不是占位。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = Vec::new();

    let kinds_ok = CommitteeKind::ALL.len() == COMMITTEE_COUNT
        && CommitteeKind::ALL.iter().all(|k| {
            CommitteeKind::parse(k.as_str()) == Some(*k) && k.has_emergency_channel() == (k.as_str() == "security")
        });
    checks.push(if kinds_ok {
        SelfCheck::pass(
            TRACK,
            "council.committees.five",
            "五类委员会（资源/任务/仲裁/进化/安全）名字往返一致，紧急通道仅安全委员会持有",
        )
    } else {
        SelfCheck::fail(TRACK, "council.committees.five", "五类委员会定义不一致")
    });

    let math_ok = [4usize, 7, 10]
        .iter()
        .all(|n| n >= &(3 * ((n - 1) / 3) + 1) && n - ((n - 1) / 3) == 2 * ((n - 1) / 3) + 1);
    checks.push(if math_ok {
        SelfCheck::pass(TRACK, "council.quorum.math", "n∈{4,7,10} 时 quorum = n - f = 2f+1 成立")
    } else {
        SelfCheck::fail(TRACK, "council.quorum.math", "BFT-lite 法定人数数学不成立")
    });

    let first = build_full();
    let second = build_full();
    match (first, second) {
        (Ok(a), Ok(b)) => {
            let id_a = a.council.committee(CommitteeKind::Resource).map(|c| c.election_id.clone()).unwrap_or_default();
            let id_b = b.council.committee(CommitteeKind::Resource).map(|c| c.election_id.clone()).unwrap_or_default();
            checks.push(if !id_a.is_empty() && id_a == id_b {
                SelfCheck::pass(
                    TRACK,
                    "council.election.reproducible",
                    format!("同一花名册与选票跑两遍，资源委员会选举 id 相同：{}", au4a_core::short_id(&id_a)),
                )
            } else {
                SelfCheck::fail(TRACK, "council.election.reproducible", "两次选举结果不一致")
            });
            let sock_elected = a
                .council
                .committees()
                .flat_map(|c| c.member_dids())
                .filter(|d| a.sock_dids.contains(d))
                .count();
            let ignored: usize = a.council.elections().iter().map(|e| e.ballots_ignored.len()).sum();
            checks.push(if sock_elected == 0 && ignored == a.sock_dids.len() {
                SelfCheck::pass(
                    TRACK,
                    "council.election.sybil",
                    format!(
                        "{} 个信誉为零的空壳 DID 全部当选失败，其 {} 张选票被忽略（权重 0 < 门槛 {}）",
                        a.sock_dids.len(),
                        ignored,
                        CouncilConfig::default().election.min_voter_weight
                    ),
                )
            } else {
                SelfCheck::fail(
                    TRACK,
                    "council.election.sybil",
                    format!("刷票未完全被拦：当选空壳 {sock_elected}，忽略票 {ignored}"),
                )
            });
            let live_state = a.council.proposal(&a.proposal.id).map(|p| p.state);
            checks.push(if live_state == Some(ProposalState::Executed) {
                SelfCheck::pass(
                    TRACK,
                    "council.proposal.agent_only",
                    format!(
                        "动议 {} 由委员 {} 签名提交、表决通过并已执行（人类观察者无 propose 方法，见 compile_fail 文档测试）",
                        au4a_core::short_id(&a.proposal.id),
                        au4a_core::short_id(a.proposal.author.as_str())
                    ),
                )
            } else {
                SelfCheck::fail(TRACK, "council.proposal.agent_only", "委员动议未能提交或未走到执行")
            });
            checks.push(if a.round.outcome == RoundOutcome::Passed
                && a.round.tally.yes >= a.round.tally.quorum
                && a.round.tally.quorum == a.round.tally.n - a.round.tally.f
            {
                SelfCheck::pass(
                    TRACK,
                    "council.vote.quorum",
                    format!(
                        "资源委员会第 {} 轮：n={} f={} quorum={} yes={} → passed",
                        a.round.round,
                        a.round.tally.n,
                        a.round.tally.f,
                        a.round.tally.quorum,
                        a.round.tally.yes
                    ),
                )
            } else {
                SelfCheck::fail(
                    TRACK,
                    "council.vote.quorum",
                    format!("表决未按 BFT-lite 法定人数出结论：{:?}", a.round.outcome),
                )
            });
            checks.push(if a.receipt.conservation_ok
                && a.receipt.ledger_effect_consistent()
                && a.council.policy("cpu_proto_settle_cap") == Some(250)
            {
                SelfCheck::pass(
                    TRACK,
                    "council.execution.applied",
                    format!(
                        "执行引擎把通过的决议落成状态变更：policy cpu_proto_settle_cap={} 账本 {}→{}（守恒成立）",
                        a.council.policy("cpu_proto_settle_cap").unwrap_or(-1),
                        a.receipt.total_before,
                        a.receipt.total_after
                    ),
                )
            } else {
                SelfCheck::fail(
                    TRACK,
                    "council.execution.applied",
                    format!("执行未落成预期状态：policy={:?}", a.council.policy("cpu_proto_settle_cap")),
                )
            });
            checks.extend(a.council.checks());
        }
        (Err(err), _) | (_, Err(err)) => {
            checks.push(SelfCheck::fail(TRACK, "council.election.reproducible", err.to_string()));
        }
    }

    checks
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    let run = build_full()?;
    let repeat = build_full()?;
    let checks = self_check();
    let resource = run.council.committee(CommitteeKind::Resource);
    let repeat_resource = repeat.council.committee(CommitteeKind::Resource);
    let reproducible = match (resource, repeat_resource) {
        (Some(a), Some(b)) => a.election_id == b.election_id,
        _ => false,
    };
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "committees": run.council.committees().count(),
        "seats_filled": run.council.committees().map(|c| c.size()).sum::<usize>(),
        "election": {
            "id": resource.map(|c| c.election_id.clone()).unwrap_or_default(),
            "ballots_ignored": run.council.elections().iter().map(|e| e.ballots_ignored.len()).sum::<usize>(),
            "reproducible": reproducible,
        },
        "proposal": {
            "id": run.proposal.id,
            "author": run.proposal.author.as_str(),
            "state": run.council.proposal(&run.proposal.id).map(|p| p.state.as_str()).unwrap_or("unknown"),
        },
        "voting": {
            "round": run.round.round,
            "outcome": run.round.outcome.as_str(),
            "n": run.round.tally.n,
            "f": run.round.tally.f,
            "quorum": run.round.tally.quorum,
            "yes": run.round.tally.yes,
            "no": run.round.tally.no,
            "abstain": run.round.tally.abstain,
        },
        "execution": {
            "proposal": run.receipt.proposal,
            "executor": run.receipt.executor.as_str(),
            "effects": run.receipt.effects.iter().map(|e| e.describe()).collect::<Vec<_>>(),
            "total_before": run.receipt.total_before.get(),
            "total_after": run.receipt.total_after.get(),
            "conservation_ok": run.receipt.conservation_ok,
            "ledger_effect_consistent": run.receipt.ledger_effect_consistent(),
            "policies": run.council.policies(),
        },
        "events": run.council.events().len(),
        "agents_enrolled": run.agents.len(),
        "kernel_delivered": run.kernel.observe().messages_delivered,
        "checks": checks.len(),
        "checks_passed": checks.iter().filter(|c| c.passed).count(),
    }))
}

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    let mut council = Council::new(CouncilConfig::default());
    let (agents, socks) = enroll(kernel, &mut council)?;
    let sock_dids: Vec<Did> = socks.iter().map(|k| k.did()).collect();
    seat_all_committees(kernel, &mut council, &agents, &socks)?;

    // 动议只能由 Agent 提出：由资源委员会的在任委员签名提交。
    let member = council
        .committee(CommitteeKind::Resource)
        .and_then(|c| c.members.first())
        .map(|m| m.did.clone());
    let proposer = agents
        .iter()
        .find(|k| Some(&k.did()) == member.as_ref())
        .ok_or(CoreError::UnknownAgent)?;
    let identity = AgentIdentity::from_keys(proposer);
    let draft = ProposalDraft::by(
        proposer,
        CommitteeKind::Resource,
        "把 cpu-proto 结算上限设为 250 微积分",
        Action::SetPolicy { key: String::from("cpu_proto_settle_cap"), value: 250 },
    )?;
    let proposal = council.propose(kernel, &identity, draft)?;
    // 表决：受理委员会按 BFT-lite 法定人数出结论。
    let round = vote_yes_all(kernel, &mut council, &agents, &proposal.id)?;
    // 执行：通过的决议落成真实状态变更。
    let receipt = council.execute(kernel, &identity, &proposal.id)?;

    // 人类只观察：拿到的只是一个值，没有任何写入口。
    let human = HumanObserver::new("operator");
    let view = human.observe(&council);

    let committees: Vec<Value> = council
        .committees()
        .map(|c| {
            json!({
                "committee": c.kind.as_str(),
                "title": c.kind.title(),
                "mandate": c.kind.mandate(),
                "epoch": c.epoch,
                "election_id": c.election_id.clone(),
                "members": c.member_dids().iter().map(|d| d.as_str().to_string()).collect::<Vec<_>>(),
                "quorum": c.quorum(),
            })
        })
        .collect();

    let sock_elected = council
        .committees()
        .flat_map(|c| c.member_dids())
        .filter(|d| sock_dids.contains(d))
        .count();
    let ignored: usize = council.elections().iter().map(|e| e.ballots_ignored.len()).sum();
    let seats: usize = council.committees().map(|c| c.size()).sum();

    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!(
            "五类委员会 {} 席；动议 {} 由 Agent 提交并经第 {} 轮表决（{}）；观察者只读看到 {} 条动议",
            seats,
            au4a_core::short_id(&proposal.id),
            round.round,
            round.outcome.as_str(),
            view.proposals.len()
        ),
    );

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "agents": kernel.agent_count(),
        "committees": committees,
        "elections": council.elections().len(),
        "seats_filled": seats,
        "sock_ballots_ignored": ignored,
        "sock_elected": sock_elected,
        "proposals": [{
            "id": proposal.id,
            "title": proposal.title,
            "committee": proposal.committee.as_str(),
            "author": proposal.author.as_str(),
            "state": council.proposal(&proposal.id).map(|p| p.state.as_str()).unwrap_or("unknown"),
            "action": proposal.action.describe(),
        }],
        "voting": {
            "round": round.round,
            "outcome": round.outcome.as_str(),
            "n": round.tally.n,
            "f": round.tally.f,
            "quorum": round.tally.quorum,
            "yes": round.tally.yes,
            "no": round.tally.no,
            "abstain": round.tally.abstain,
            "participation": round.tally.participation,
            "voters": round.votes.iter().map(|(did, choice)| json!({"did": did, "choice": choice.as_str()})).collect::<Vec<_>>(),
        },
        "execution": {
            "proposal": receipt.proposal,
            "executor": receipt.executor.as_str(),
            "effects": receipt.effects.iter().map(|e| e.describe()).collect::<Vec<_>>(),
            "total_before": receipt.total_before.get(),
            "total_after": receipt.total_after.get(),
            "conservation_ok": receipt.conservation_ok,
            "ledger_effect_consistent": receipt.ledger_effect_consistent(),
            "policies": council.policies(),
        },
        "human_view": {
            "label": view.label,
            "seats_filled": view.seats_filled,
            "proposals": view.proposals.len(),
            "events": view.events,
        },
        "events": council.events().len(),
    }))
}
