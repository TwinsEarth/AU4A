//! 表决机制（v1.7.3）。
//!
//! BFT-lite：委员会 `n` 人，容错上限 `f = ⌊(n-1)/3⌋`，**法定人数 `quorum = n - f`**
//! （在 `n = 3f+1` 的规模上就是 `2f+1`）。一条动议要拿到结论，必须有 `quorum` 张
//! 相同的有效票（`yes ≥ quorum` 通过，`no ≥ quorum` 否决）。
//!
//! 具名失败模式（参考项目认为最有价值的部分就是把失败模式写清楚）：
//!
//! * **重复投票**：同一委员在同一轮投同一选择 → 拒绝，`conflict` 留痕，轮次不受影响。
//! * **模棱两可 / 双签**：同一委员在同一轮投**不同**选择 → 整轮作废（`void_ambiguous`），
//!   已投的票全部不计入结论，必须重开一轮。这是「一个委员不能在两个结论上都签名」的
//!   可执行版本——不是靠信任，而是靠记分规则。
//! * **越权投票**：非在任委员 → `unauthorized`（单次即恶意证据）。
//! * **过期轮次**：投给已关闭或不存在轮次 → `stale_epoch`（属竞争语义，不升级为恶意）。
//!
//! 安全性来自一个结构性事实：`yes` 与 `no` 来自互斥的委员集合，因此
//! `yes ≥ quorum` 与 `no ≥ quorum` 不可能同时成立（`Tally::is_consistent` 断言它）。

use std::collections::BTreeMap;

use au4a_core::{canonicalize, AgentKeys, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::committee::CommitteeKind;

/// 表决选择。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// 赞成。
    Yes,
    /// 反对。
    No,
    /// 弃权（计入参与，但既不算赞成也不算反对）。
    Abstain,
}

impl Choice {
    /// 稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Choice::Yes => "yes",
            Choice::No => "no",
            Choice::Abstain => "abstain",
        }
    }
}

/// 一张签名表决票。绑定（动议、轮次、选择），因此不能跨轮或跨动议重放。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vote {
    /// 投票委员。
    pub voter: Did,
    /// 动议 id。
    pub proposal: String,
    /// 轮次。
    pub round: u32,
    /// 选择。
    pub choice: Choice,
    /// hex 编码的 Ed25519 签名。
    pub sig: String,
}

impl Vote {
    /// 由委员私钥铸造表决票。
    pub fn cast(keys: &AgentKeys, proposal: &str, round: u32, choice: Choice) -> CoreResult<Self> {
        let mut vote = Self {
            voter: keys.did(),
            proposal: proposal.to_string(),
            round,
            choice,
            sig: String::new(),
        };
        vote.sig = keys.sign_json(&vote.payload())?;
        Ok(vote)
    }

    /// 被签名的规范载荷。
    pub fn payload(&self) -> Value {
        json!({
            "voter": self.voter,
            "proposal": self.proposal,
            "round": self.round,
            "choice": self.choice,
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

/// 一轮表决的结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundOutcome {
    /// 还没到法定人数。
    Pending,
    /// `yes ≥ quorum`：动议通过。
    Passed,
    /// `no ≥ quorum`：动议被否决。
    Rejected,
    /// 出现模棱两可（双签）：整轮作废，全部票不计入结论。
    VoidAmbiguous,
}

impl RoundOutcome {
    /// 稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            RoundOutcome::Pending => "pending",
            RoundOutcome::Passed => "passed",
            RoundOutcome::Rejected => "rejected",
            RoundOutcome::VoidAmbiguous => "void_ambiguous",
        }
    }

    /// 本轮是否已关闭（关闭后不再接受投票）。
    pub fn is_closed(self) -> bool {
        !matches!(self, RoundOutcome::Pending)
    }
}

/// 计票结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    /// 赞成票数。
    pub yes: usize,
    /// 反对票数。
    pub no: usize,
    /// 弃权票数。
    pub abstain: usize,
    /// 参与表决的委员数（去重后）。
    pub participation: usize,
    /// 委员会在任人数 `n`。
    pub n: usize,
    /// 容错上限 `f`。
    pub f: usize,
    /// 法定人数 `n - f`。
    pub quorum: usize,
}

impl Tally {
    /// 计票自洽：票数守恒、参与不超员、且不可能同时达成通过与否决。
    pub fn is_consistent(&self) -> bool {
        let counted = self.yes + self.no + self.abstain;
        let exclusive = !(self.yes >= self.quorum && self.no >= self.quorum);
        counted == self.participation && self.participation <= self.n && exclusive
    }

    /// 当前结论。
    pub fn outcome(&self) -> RoundOutcome {
        if self.yes >= self.quorum {
            RoundOutcome::Passed
        } else if self.no >= self.quorum {
            RoundOutcome::Rejected
        } else {
            RoundOutcome::Pending
        }
    }
}

/// 一轮表决的快照（可序列化、可审计）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundState {
    /// 动议 id。
    pub proposal: String,
    /// 受理委员会。
    pub committee: CommitteeKind,
    /// 轮次。
    pub round: u32,
    /// 结论。
    pub outcome: RoundOutcome,
    /// 计票。
    pub tally: Tally,
    /// 已投票委员与其选择（按 DID 升序，确定性）。
    pub votes: Vec<(String, Choice)>,
    /// 开轮时刻（逻辑刻度）。
    pub opened_at: u64,
}

/// 表决轮次本体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VotingRound {
    /// 动议 id。
    pub proposal: String,
    /// 受理委员会。
    pub committee: CommitteeKind,
    /// 轮次号（从 1 开始）。
    pub round: u32,
    /// 在任人数 `n`。
    pub n: usize,
    /// 容错上限 `f`。
    pub f: usize,
    /// 法定人数。
    pub quorum: usize,
    /// 已投的票（键为 DID 文本，保证遍历确定性）。
    pub votes: BTreeMap<String, Vote>,
    /// 结论。
    pub outcome: RoundOutcome,
    /// 开轮时刻。
    pub opened_at: u64,
}

impl VotingRound {
    /// 当前计票。
    pub fn tally(&self) -> Tally {
        let mut tally = Tally {
            yes: 0,
            no: 0,
            abstain: 0,
            participation: self.votes.len(),
            n: self.n,
            f: self.f,
            quorum: self.quorum,
        };
        for vote in self.votes.values() {
            match vote.choice {
                Choice::Yes => tally.yes += 1,
                Choice::No => tally.no += 1,
                Choice::Abstain => tally.abstain += 1,
            }
        }
        tally
    }

    /// 快照。
    pub fn state(&self) -> RoundState {
        RoundState {
            proposal: self.proposal.clone(),
            committee: self.committee,
            round: self.round,
            outcome: self.outcome,
            tally: self.tally(),
            votes: self
                .votes
                .iter()
                .map(|(did, vote)| (did.clone(), vote.choice))
                .collect(),
            opened_at: self.opened_at,
        }
    }

    /// 是否已有一位委员投了**不同**选择（模棱两可证据）。
    pub fn is_ambiguous(&self) -> bool {
        self.outcome == RoundOutcome::VoidAmbiguous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vote(seed: u8, round: u32, choice: Choice) -> Vote {
        Vote::cast(&AgentKeys::from_seed(&[seed; 32]), "p1", round, choice).expect("cast")
    }

    #[test]
    fn quorum_is_n_minus_f() {
        for (n, f, quorum) in [
            (1usize, 0usize, 1usize),
            (4, 1, 3),
            (5, 1, 4),
            (7, 2, 5),
            (10, 3, 7),
        ] {
            let round = VotingRound {
                proposal: String::from("p"),
                committee: CommitteeKind::Task,
                round: 1,
                n,
                f,
                quorum,
                votes: BTreeMap::new(),
                outcome: RoundOutcome::Pending,
                opened_at: 0,
            };
            assert_eq!(round.tally().quorum, round.n - round.f);
            assert!(n > 3 * f);
        }
    }

    #[test]
    fn yes_and_no_can_never_both_reach_quorum() {
        // 极端构造：n = 4、quorum = 3，最多 3 票，因此两种结论不可能同时成立。
        let mut round = VotingRound {
            proposal: String::from("p"),
            committee: CommitteeKind::Task,
            round: 1,
            n: 4,
            f: 1,
            quorum: 3,
            votes: BTreeMap::new(),
            outcome: RoundOutcome::Pending,
            opened_at: 0,
        };
        round.votes.insert(
            vote(1, 1, Choice::Yes).voter.as_str().to_string(),
            vote(1, 1, Choice::Yes),
        );
        assert_eq!(round.tally().outcome(), RoundOutcome::Pending);
        round.votes.insert(
            vote(2, 1, Choice::No).voter.as_str().to_string(),
            vote(2, 1, Choice::No),
        );
        assert_eq!(round.tally().outcome(), RoundOutcome::Pending);
        round.votes.insert(
            vote(3, 1, Choice::Yes).voter.as_str().to_string(),
            vote(3, 1, Choice::Yes),
        );
        // yes = 2 < quorum = 3：还没有结论。
        assert_eq!(round.tally().outcome(), RoundOutcome::Pending);
        round.votes.insert(
            vote(4, 1, Choice::Yes).voter.as_str().to_string(),
            vote(4, 1, Choice::Yes),
        );
        let tally = round.tally();
        assert_eq!(tally.outcome(), RoundOutcome::Passed, "yes=3 达到 quorum=3");
        assert!(tally.is_consistent());
        assert!(tally.yes + tally.no <= tally.n);
    }

    #[test]
    fn abstain_counts_participation_but_not_yes() {
        let mut round = VotingRound {
            proposal: String::from("p"),
            committee: CommitteeKind::Task,
            round: 1,
            n: 4,
            f: 1,
            quorum: 3,
            votes: BTreeMap::new(),
            outcome: RoundOutcome::Pending,
            opened_at: 0,
        };
        for (i, choice) in [
            (1u8, Choice::Abstain),
            (2, Choice::Abstain),
            (3, Choice::Abstain),
        ] {
            let v = vote(i, 1, choice);
            round.votes.insert(v.voter.as_str().to_string(), v);
        }
        let tally = round.tally();
        assert_eq!(tally.participation, 3);
        assert_eq!(tally.abstain, 3);
        assert_eq!(tally.outcome(), RoundOutcome::Pending, "弃权不产生结论");
        assert!(tally.is_consistent());
    }

    #[test]
    fn votes_are_bound_to_proposal_and_round() {
        let keys = AgentKeys::from_seed(&[9u8; 32]);
        let a = Vote::cast(&keys, "p1", 1, Choice::Yes).expect("cast");
        a.verify().expect("verify");
        let mut tampered = a.clone();
        tampered.choice = Choice::No;
        assert_eq!(tampered.verify(), Err(CoreError::InvalidSignature));
        let mut moved = a.clone();
        moved.round = 2;
        assert_eq!(moved.verify(), Err(CoreError::InvalidSignature));
        let mut replayed = a.clone();
        replayed.proposal = String::from("p2");
        assert_eq!(replayed.verify(), Err(CoreError::InvalidSignature));
        assert_eq!(Vote::cast(&keys, "p1", 1, Choice::Yes).expect("cast"), a);
    }
}
