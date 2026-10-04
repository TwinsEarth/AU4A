//! 席位选举（v1.7.1）。
//!
//! 选举是一个**确定性函数**：给定同一份花名册与同一批签名选票（无论到达顺序），
//! 结果逐字节相同（内容寻址 `id` 相同）。这条性质是审计的前提——如果结果依赖到达顺序，
//! 事后就无法复算，也就无法证明「选出来的是谁」。
//!
//! 刷票（Sybil）的结构性防线有三条，都在类型/算法层面而不是靠巡查：
//!
//! 1. 选票权重 = 选民信誉 × 在线时长，而信誉只能由治理记录写入（不可转让、不可自报）；
//! 2. 权重低于 `min_voter_weight` 的选票被**忽略**并留痕（空壳 DID 权重为 0，投了等于没投）；
//! 3. 候选人必须同时满足信誉、在线时长与质押门槛——空壳 DID 连候选资格都没有。
//!
//! 本模块不读墙钟、不做 I/O：时间由调用方以逻辑刻度的形式传入。

use std::collections::{BTreeMap, BTreeSet};

use au4a_core::{canonical_hash, canonicalize, AgentKeys, CoreError, CoreResult, Credits, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::committee::CommitteeKind;

/// 一名候选人的治理画像（由治理记录，不由 Agent 自报）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// 身份。
    pub did: Did,
    /// 信誉（万分比）。
    pub reputation_bp: u32,
    /// 在线时长（逻辑刻度累计）。
    pub uptime: u64,
    /// 质押。
    pub stake: Credits,
}

impl Candidate {
    /// 选票权重：`信誉 × 在线时长`（整数、饱和乘、无浮点）。
    pub fn vote_weight(&self) -> u64 {
        u64::from(self.reputation_bp).saturating_mul(self.uptime)
    }

    /// 是否满足候选资格（高信誉 + 长期在线 + 足额质押）。
    pub fn eligible(&self, cfg: &ElectionConfig) -> bool {
        self.reputation_bp >= cfg.min_reputation_bp
            && self.uptime >= cfg.min_uptime
            && self.stake >= cfg.min_stake
    }
}

/// 选举参数。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElectionConfig {
    /// 每类委员会的席位数。
    pub seats: usize,
    /// 候选人信誉门槛（万分比）。
    pub min_reputation_bp: u32,
    /// 候选人在线时长门槛。
    pub min_uptime: u64,
    /// 选民权重门槛：低于它的选票不计入。
    pub min_voter_weight: u64,
    /// 候选人的质押门槛。
    pub min_stake: Credits,
    /// 任期长度（逻辑刻度）。
    pub term: u64,
}

impl Default for ElectionConfig {
    fn default() -> Self {
        Self {
            seats: 5,
            min_reputation_bp: 3_000,
            min_uptime: 50,
            min_voter_weight: 1,
            min_stake: Credits(10),
            term: 1_000,
        }
    }
}

impl ElectionConfig {
    /// 参数自检。非法参数一律 [`CoreError::InvalidKind`]（基元层唯一的「参数不合法」码）。
    pub fn validate(&self) -> CoreResult<()> {
        if self.seats == 0 || self.term == 0 {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }
}

/// 一张签名选票：选民给若干候选人投票。
///
/// 选票绑定委员会类别，因此**不能**把一场选举的票重放进另一场。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElectionBallot {
    /// 选民。
    pub voter: Did,
    /// 绑定的委员会类别。
    pub committee: CommitteeKind,
    /// 候选人 DID，升序去重（规范载荷与到达顺序无关）。
    pub choices: Vec<Did>,
    /// hex 编码的 Ed25519 签名。
    pub sig: String,
}

impl ElectionBallot {
    /// 由选民私钥铸造选票（先规范化去重，再签名）。
    pub fn cast(keys: &AgentKeys, committee: CommitteeKind, choices: &[Did]) -> CoreResult<Self> {
        let deduped: Vec<Did> = choices
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut ballot = Self {
            voter: keys.did(),
            committee,
            choices: deduped,
            sig: String::new(),
        };
        ballot.sig = keys.sign_json(&ballot.payload())?;
        Ok(ballot)
    }

    /// 被签名的规范载荷。
    pub fn payload(&self) -> Value {
        json!({
            "voter": self.voter,
            "committee": self.committee,
            "choices": self.choices.iter().map(|d| d.as_str()).collect::<Vec<_>>(),
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

/// 加权票得分行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreRow {
    /// 候选人。
    pub did: Did,
    /// 加权得分。
    pub score: u64,
    /// 信誉（万分比）。
    pub reputation_bp: u32,
    /// 在线时长。
    pub uptime: u64,
}

/// 没有候选资格的花名册成员（留痕用）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ineligible {
    /// 身份。
    pub did: Did,
    /// 具体原因（信誉 / 在线时长 / 质押）。
    pub reason: String,
}

/// 被忽略的选票（留痕用，不静默丢弃）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredBallot {
    /// 选民。
    pub voter: Did,
    /// 忽略原因。
    pub reason: String,
}

/// 一名当选者。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Elected {
    /// 排名（从 1 开始）。
    pub rank: u16,
    /// 身份。
    pub did: Did,
    /// 加权得分。
    pub score: u64,
    /// 当选时的信誉。
    pub reputation_bp: u32,
    /// 当选时的在线时长。
    pub uptime: u64,
}

/// 一场选举的完整结果（可复算、可审计）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElectionOutcome {
    /// 内容寻址 id：只覆盖确定性核心（不含时刻）。
    pub id: String,
    /// 委员会类别。
    pub committee: CommitteeKind,
    /// 届数。
    pub epoch: u64,
    /// 发生时刻（逻辑刻度）。
    pub at: u64,
    /// 席位上限。
    pub seats: usize,
    /// 当选者（rank 升序）。
    pub elected: Vec<Elected>,
    /// 全部合格候选人的得分（降序），用于事后复算。
    pub scores: Vec<ScoreRow>,
    /// 不合格候选人的留痕。
    pub ineligible: Vec<Ineligible>,
    /// 收到的选票数。
    pub ballots_seen: usize,
    /// 计入的选票数。
    pub ballots_counted: usize,
    /// 被忽略的选票。
    pub ballots_ignored: Vec<IgnoredBallot>,
    /// 计入的总权重。
    pub weight_counted: u64,
}

impl ElectionOutcome {
    /// 当选 DID 列表（rank 升序）。
    pub fn elected_dids(&self) -> Vec<Did> {
        self.elected.iter().map(|e| e.did.clone()).collect()
    }
}

/// 跑一场选举。纯函数：不读时钟、不碰内核，`at` 由调用方给出。
///
/// 拒绝路径是**类型化**的：伪造签名 → `InvalidSignature`；跨委员会重放 → `InvalidKind`；
/// 同一选民投两次 → `DuplicateAgent`（基元层无 `Conflict` 变体的等价语义）；
/// 选民不在花名册 → `UnknownAgent`。
pub fn run(
    kind: CommitteeKind,
    epoch: u64,
    cfg: &ElectionConfig,
    roster: &[Candidate],
    ballots: &[ElectionBallot],
    at: u64,
) -> CoreResult<ElectionOutcome> {
    cfg.validate()?;

    let by_did: BTreeMap<&Did, &Candidate> = roster.iter().map(|c| (&c.did, c)).collect();

    // 到达顺序无关：先按选民 DID 排序，再逐张校验。
    let mut ordered: Vec<&ElectionBallot> = ballots.iter().collect();
    ordered.sort_by(|a, b| a.voter.cmp(&b.voter));

    let mut seen_voters: BTreeSet<Did> = BTreeSet::new();
    let mut scores: BTreeMap<Did, u64> = BTreeMap::new();
    let mut ignored: Vec<IgnoredBallot> = Vec::new();
    let mut counted = 0usize;
    let mut weight_counted = 0u64;

    for ballot in ordered {
        ballot.verify()?;
        if ballot.committee != kind {
            return Err(CoreError::InvalidKind);
        }
        if !seen_voters.insert(ballot.voter.clone()) {
            // 基元层没有 `Conflict` 变体：同一 DID 重复表达意志等价于「重复登记」，
            // 分类留痕走 `RefusalCode::Conflict`（调用方负责记入内核拒绝账）。
            return Err(CoreError::DuplicateAgent);
        }
        let voter = match by_did.get(&ballot.voter) {
            Some(v) => *v,
            None => return Err(CoreError::UnknownAgent),
        };
        let weight = voter.vote_weight();
        if weight < cfg.min_voter_weight {
            ignored.push(IgnoredBallot {
                voter: ballot.voter.clone(),
                reason: format!(
                    "vote_weight {weight} < floor {} (信誉 {}bp × 在线 {})",
                    cfg.min_voter_weight, voter.reputation_bp, voter.uptime
                ),
            });
            continue;
        }
        counted += 1;
        weight_counted = weight_counted.saturating_add(weight);
        for choice in &ballot.choices {
            if let Some(c) = by_did.get(choice) {
                if c.eligible(cfg) {
                    let entry = scores.entry(choice.clone()).or_insert(0);
                    *entry = entry.saturating_add(weight);
                }
            }
        }
    }

    let mut rows: Vec<ScoreRow> = Vec::new();
    let mut ineligible: Vec<Ineligible> = Vec::new();
    for c in roster {
        if c.eligible(cfg) {
            rows.push(ScoreRow {
                did: c.did.clone(),
                score: scores.get(&c.did).copied().unwrap_or(0),
                reputation_bp: c.reputation_bp,
                uptime: c.uptime,
            });
        } else {
            ineligible.push(Ineligible {
                did: c.did.clone(),
                reason: ineligible_reason(c, cfg),
            });
        }
    }
    // 排序键全序：得分 → 信誉 → 在线时长 → DID 字典序。最后一项保证没有平局。
    rows.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.reputation_bp.cmp(&a.reputation_bp))
            .then(b.uptime.cmp(&a.uptime))
            .then(a.did.cmp(&b.did))
    });

    let elected: Vec<Elected> = rows
        .iter()
        .take(cfg.seats)
        .enumerate()
        .map(|(i, r)| Elected {
            rank: (i + 1) as u16,
            did: r.did.clone(),
            score: r.score,
            reputation_bp: r.reputation_bp,
            uptime: r.uptime,
        })
        .collect();

    // id 只覆盖「花名册的计票结果」，**不含**届数与时刻：同一份治理输入在任何节点、
    // 任何时候复算都得到同一个地址，这才能被审计复现。
    let id = canonical_hash(&json!({
        "committee": kind,
        "seats": cfg.seats,
        "elected": elected
            .iter()
            .map(|e| json!({"rank": e.rank, "did": e.did.as_str(), "score": e.score}))
            .collect::<Vec<_>>(),
        "ballots_counted": counted,
        "weight_counted": weight_counted,
    }))?;

    Ok(ElectionOutcome {
        id,
        committee: kind,
        epoch,
        at,
        seats: cfg.seats,
        elected,
        scores: rows,
        ineligible,
        ballots_seen: ballots.len(),
        ballots_counted: counted,
        ballots_ignored: ignored,
        weight_counted,
    })
}

fn ineligible_reason(c: &Candidate, cfg: &ElectionConfig) -> String {
    if c.reputation_bp < cfg.min_reputation_bp {
        format!(
            "信誉 {}bp < 门槛 {}bp",
            c.reputation_bp, cfg.min_reputation_bp
        )
    } else if c.uptime < cfg.min_uptime {
        format!("在线 {} < 门槛 {}", c.uptime, cfg.min_uptime)
    } else {
        format!("质押 {} < 门槛 {}", c.stake, cfg.min_stake)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(seed: u8, rep: u32, uptime: u64) -> Candidate {
        Candidate {
            did: AgentKeys::from_seed(&[seed; 32]).did(),
            reputation_bp: rep,
            uptime,
            stake: Credits(100),
        }
    }

    #[test]
    fn vote_weight_is_integer_and_saturating() {
        let c = Candidate {
            did: AgentKeys::from_seed(&[1u8; 32]).did(),
            reputation_bp: u32::MAX,
            uptime: u64::MAX,
            stake: Credits(1),
        };
        assert_eq!(c.vote_weight(), u64::MAX);
        assert_eq!(cand(2, 5_000, 100).vote_weight(), 500_000);
    }

    #[test]
    fn ballots_are_order_independent_and_content_addressed() {
        let a = cand(1, 5_000, 100);
        let b = cand(2, 4_000, 100);
        let voter = AgentKeys::from_seed(&[3u8; 32]);
        let voter2 = AgentKeys::from_seed(&[4u8; 32]);
        let roster = vec![
            a.clone(),
            b.clone(),
            Candidate {
                did: voter.did(),
                reputation_bp: 4_000,
                uptime: 80,
                stake: Credits(100),
            },
            Candidate {
                did: voter2.did(),
                reputation_bp: 4_500,
                uptime: 90,
                stake: Credits(100),
            },
        ];
        let ballots = vec![
            ElectionBallot::cast(&voter, CommitteeKind::Task, &[a.did.clone(), b.did.clone()])
                .unwrap(),
            ElectionBallot::cast(&voter2, CommitteeKind::Task, std::slice::from_ref(&b.did))
                .unwrap(),
        ];
        let mut shuffled = ballots.clone();
        shuffled.reverse();
        let cfg = ElectionConfig {
            seats: 2,
            ..ElectionConfig::default()
        };
        let first = run(CommitteeKind::Task, 1, &cfg, &roster, &ballots, 5).unwrap();
        let second = run(CommitteeKind::Task, 1, &cfg, &roster, &shuffled, 5).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.elected.len(), 2);
        // b: 320000(voter) + 405000(voter2) = 725000；a: 320000(voter) → b 第一、a 第二。
        assert_eq!(first.elected[0].did, b.did);
        assert_eq!(first.elected[0].score, 725_000);
        assert_eq!(first.elected[1].did, a.did);
        assert_eq!(first.elected[1].score, 320_000);
    }

    #[test]
    fn cross_committee_replay_is_refused() {
        let voter = AgentKeys::from_seed(&[5u8; 32]);
        let roster = vec![Candidate {
            did: voter.did(),
            reputation_bp: 4_000,
            uptime: 80,
            stake: Credits(100),
        }];
        let ballot = ElectionBallot::cast(&voter, CommitteeKind::Task, &[voter.did()]).unwrap();
        assert_eq!(
            run(
                CommitteeKind::Security,
                1,
                &ElectionConfig::default(),
                &roster,
                &[ballot],
                1
            ),
            Err(CoreError::InvalidKind)
        );
    }

    #[test]
    fn tampered_ballot_choice_breaks_the_signature() {
        let voter = AgentKeys::from_seed(&[6u8; 32]);
        let other = AgentKeys::from_seed(&[7u8; 32]).did();
        let mut ballot = ElectionBallot::cast(&voter, CommitteeKind::Task, &[voter.did()]).unwrap();
        ballot.choices = vec![other];
        assert_eq!(ballot.verify(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn duplicate_and_foreign_voters_are_typed_errors() {
        let cfg = ElectionConfig::default();
        let v1 = AgentKeys::from_seed(&[8u8; 32]);
        let v2 = AgentKeys::from_seed(&[9u8; 32]);
        let outsider = AgentKeys::from_seed(&[10u8; 32]);
        let roster = vec![
            Candidate {
                did: v1.did(),
                reputation_bp: 4_000,
                uptime: 80,
                stake: Credits(100),
            },
            Candidate {
                did: v2.did(),
                reputation_bp: 4_000,
                uptime: 80,
                stake: Credits(100),
            },
        ];
        let dup = vec![
            ElectionBallot::cast(&v1, CommitteeKind::Task, &[v1.did()]).unwrap(),
            ElectionBallot::cast(&v1, CommitteeKind::Task, &[v2.did()]).unwrap(),
        ];
        assert_eq!(
            run(CommitteeKind::Task, 1, &cfg, &roster, &dup, 1),
            Err(CoreError::DuplicateAgent)
        );
        let foreign =
            vec![ElectionBallot::cast(&outsider, CommitteeKind::Task, &[v1.did()]).unwrap()];
        assert_eq!(
            run(CommitteeKind::Task, 1, &cfg, &roster, &foreign, 1),
            Err(CoreError::UnknownAgent)
        );
    }

    #[test]
    fn zero_weight_sock_puppets_are_ignored_not_counted() {
        let cfg = ElectionConfig::default();
        let real = AgentKeys::from_seed(&[11u8; 32]);
        let sock = AgentKeys::from_seed(&[12u8; 32]);
        let roster = vec![
            Candidate {
                did: real.did(),
                reputation_bp: 5_000,
                uptime: 100,
                stake: Credits(100),
            },
            Candidate {
                did: sock.did(),
                reputation_bp: 0,
                uptime: 1,
                stake: Credits(100),
            },
        ];
        let ballots = vec![
            ElectionBallot::cast(&real, CommitteeKind::Task, &[real.did()]).unwrap(),
            ElectionBallot::cast(&sock, CommitteeKind::Task, &[sock.did()]).unwrap(),
        ];
        let out = run(CommitteeKind::Task, 1, &cfg, &roster, &ballots, 1).unwrap();
        assert_eq!(out.ballots_seen, 2);
        assert_eq!(out.ballots_counted, 1);
        assert_eq!(out.ballots_ignored.len(), 1);
        assert_eq!(out.ballots_ignored[0].voter, sock.did());
        assert_eq!(out.elected.len(), 1);
        assert_eq!(out.elected[0].did, real.did());
        assert_eq!(out.ineligible.len(), 1);
    }
}
