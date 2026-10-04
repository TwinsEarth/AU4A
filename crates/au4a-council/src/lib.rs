//! AU4A 轨道 1.7 — Committee Governance 委员会治理（v1.7.1 → v1.7.10）。
//!
//! 治理在 AU4A 里不是「人类审批」，而是 **Agent 之间的制度**：
//!
//! * 五类委员会（资源 / 任务 / 仲裁 / 进化 / 安全）由**选举**产生，席位来自信誉与在线时长；
//! * 动议只能由 **Agent** 提出——[`proposal::AgentIdentity`] 是唯一入口，而人类观察者
//!   [`veto::HumanObserver`] 在类型层面**没有**任何提案/修改能力；
//! * 决议按 BFT-lite 法定人数表决（`n ≥ 3f+1`），重复投票被拒，模棱两可（双签）作废该轮；
//! * 通过的决议由执行引擎落成真实状态变更（策略、信誉、账本）；
//! * 人类只在极端情况行使**否决权**：只能阻断、必须公开理由、不能改动决议内容。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责，并留下自检与证据。
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）。
//! 本层不读墙钟、不碰文件、不开网络：时间一律来自 [`au4a_core::LogicalClock`]。

#![forbid(unsafe_code)]

pub mod committee;
pub mod election;

pub use committee::{Committee, CommitteeKind, Member, COMMITTEE_COUNT};
pub use election::{
    Candidate, ElectionBallot, ElectionConfig, ElectionOutcome, Elected, IgnoredBallot, Ineligible,
    ScoreRow,
};

use std::collections::{BTreeMap, BTreeSet};

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
}

impl Default for CouncilConfig {
    fn default() -> Self {
        Self {
            election: ElectionConfig::default(),
            min_stake: Credits(10),
        }
    }
}

/// 委员会治理本体：席位、信誉账、选举日志。
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

        let mut seen: BTreeSet<Did> = BTreeSet::new();
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
        Ok(outcome)
    }

    /// 治理层自检（v1.7.1 覆盖选举与席位；后续版本追加表决、否决、审计）。
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

/// 一次完整选举（用于自检与结果摘要）。
fn one_election() -> CoreResult<(Council, ElectionOutcome, Vec<Did>)> {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut council = Council::new(CouncilConfig::default());
    let (agents, socks) = enroll(&mut kernel, &mut council)?;
    let picks: Vec<Did> = agents.iter().take(3).map(|k| k.did()).collect();
    let mut ballots: Vec<ElectionBallot> = Vec::new();
    for keys in agents.iter().chain(socks.iter()) {
        ballots.push(ElectionBallot::cast(keys, CommitteeKind::Resource, &picks)?);
    }
    let outcome = council.elect(&mut kernel, CommitteeKind::Resource, &ballots)?;
    Ok((council, outcome, socks.iter().map(|k| k.did()).collect()))
}

struct MiniElection {
    first: ElectionOutcome,
    second: ElectionOutcome,
    sock_dids: Vec<Did>,
    checks: Vec<SelfCheck>,
}

/// 同一输入跑两遍：用来断言选举可复现。
fn mini_election() -> CoreResult<MiniElection> {
    let (council, first, sock_dids) = one_election()?;
    let (_, second, _) = one_election()?;
    Ok(MiniElection {
        first,
        second,
        sock_dids,
        checks: council.checks(),
    })
}

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
///
/// 每一项都是**真实断言**（会真的跑一遍选举），不是占位。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = Vec::new();

    let kinds_ok = CommitteeKind::ALL.len() == COMMITTEE_COUNT
        && CommitteeKind::ALL
            .iter()
            .all(|k| CommitteeKind::parse(k.as_str()) == Some(*k) && k.has_emergency_channel() == (k.as_str() == "security"));
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

    match mini_election() {
        Ok(m) => {
            checks.extend(m.checks.iter().cloned());
            checks.push(if m.first.id == m.second.id {
                SelfCheck::pass(
                    TRACK,
                    "council.election.reproducible",
                    format!("同一花名册与选票跑两遍，选举 id 相同：{}", au4a_core::short_id(&m.first.id)),
                )
            } else {
                SelfCheck::fail(TRACK, "council.election.reproducible", "两次选举结果不一致")
            });
            let sock_elected = m
                .first
                .elected_dids()
                .iter()
                .filter(|d| m.sock_dids.contains(d))
                .count();
            checks.push(if sock_elected == 0 && m.first.ballots_ignored.len() == m.sock_dids.len() {
                SelfCheck::pass(
                    TRACK,
                    "council.election.sybil",
                    format!(
                        "{} 个信誉为零的空壳 DID 全部当选失败，其 {} 张选票被忽略（权重 0 < 门槛 {}）",
                        m.sock_dids.len(),
                        m.first.ballots_ignored.len(),
                        CouncilConfig::default().election.min_voter_weight
                    ),
                )
            } else {
                SelfCheck::fail(
                    TRACK,
                    "council.election.sybil",
                    format!("刷票未完全被拦：当选空壳 {sock_elected}，忽略票 {}", m.first.ballots_ignored.len()),
                )
            });
        }
        Err(err) => checks.push(SelfCheck::fail(TRACK, "council.election.reproducible", err.to_string())),
    }

    checks
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    let mini = mini_election()?;
    let checks = self_check();
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "committees": COMMITTEE_COUNT,
        "election": {
            "id": mini.first.id,
            "reproducible": mini.first.id == mini.second.id,
            "elected": mini.first.elected.len(),
            "ballots_counted": mini.first.ballots_counted,
            "ballots_ignored": mini.first.ballots_ignored.len(),
            "weight_counted": mini.first.weight_counted,
        },
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
    let eligible: Vec<Did> = agents.iter().map(|k| k.did()).collect();
    let sock_dids: Vec<Did> = socks.iter().map(|k| k.did()).collect();

    let mut rowsholder: Vec<Value> = Vec::new();
    for (j, kind) in CommitteeKind::ALL.iter().enumerate() {
        let picks: Vec<Did> = (0..3).map(|n| eligible[(j + n) % eligible.len()].clone()).collect();
        let mut ballots: Vec<ElectionBallot> = Vec::new();
        for keys in &agents {
            ballots.push(ElectionBallot::cast(keys, *kind, &picks)?);
        }
        if *kind == CommitteeKind::Resource {
            for keys in &socks {
                ballots.push(ElectionBallot::cast(keys, *kind, &picks)?);
            }
        }
        let outcome = council.elect(kernel, *kind, &ballots)?;
        rowsholder.push(json!({
            "committee": kind.as_str(),
            "title": kind.title(),
            "mandate": kind.mandate(),
            "epoch": outcome.epoch,
            "election_id": outcome.id,
            "members": outcome.elected.iter().map(|e| e.did.as_str().to_string()).collect::<Vec<_>>(),
            "ballots_counted": outcome.ballots_counted,
            "ballots_ignored": outcome.ballots_ignored.len(),
        }));
    }

    let sock_elected = council
        .committees()
        .flat_map(|c| c.member_dids())
        .filter(|d| sock_dids.contains(d))
        .count();
    let ignored: usize = council.elections().iter().map(|e| e.ballots_ignored.len()).sum();
    let seats: usize = council.committees().map(|c| c.size()).sum();

    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!("五类委员会选举完成：{} 个在任席位，忽略空壳选票 {}", seats, ignored),
    );

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "agents": kernel.agent_count(),
        "committees": rowsholder,
        "elections": council.elections().len(),
        "seats_filled": seats,
        "sock_ballots_ignored": ignored,
        "sock_elected": sock_elected,
    }))
}
