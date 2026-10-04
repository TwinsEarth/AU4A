//! v1.7.1 选举机制——集成测试。
//!
//! 覆盖三条线：正常路径（高信誉+长期在线的 Agent 当选）、拒绝路径（伪造/重放/重复投票）、
//! 不变式（可复现、刷票无效、BFT-lite 数学）。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_council::{CommitteeKind, Council, CouncilConfig, ElectionBallot, ElectionConfig};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn kernel_with(agents: &[&AgentKeys]) -> Kernel {
    let mut kernel = Kernel::new(KernelConfig::default());
    for k in agents {
        kernel
            .register(k, "t", &["governance.vote"], Credits(20))
            .expect("register");
    }
    kernel
}

/// 治理账：6 个合格候选人（信誉 4000..6000bp，在线 100..350）+ 1 个空壳。
fn seeded_council(agents: &[AgentKeys], sock: &AgentKeys) -> Council {
    let mut council = Council::new(CouncilConfig::default());
    for (i, k) in agents.iter().enumerate() {
        council.note_reputation(&k.did(), 4_000 + (i as u32) * 400);
        council.note_uptime(&k.did(), 100 + (i as u64) * 50);
    }
    council.note_uptime(&sock.did(), 1);
    council
}

fn ballots_for(kind: CommitteeKind, voters: &[&AgentKeys], picks: &[Did]) -> Vec<ElectionBallot> {
    voters
        .iter()
        .map(|k| ElectionBallot::cast(k, kind, picks).expect("cast"))
        .collect()
}

#[test]
fn five_committee_kinds_are_stable_and_security_holds_the_channel() {
    assert_eq!(CommitteeKind::ALL.len(), 5);
    let names: Vec<&str> = CommitteeKind::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(
        names,
        vec!["resource", "task", "arbitration", "evolution", "security"]
    );
    for k in CommitteeKind::ALL {
        assert_eq!(CommitteeKind::parse(k.as_str()), Some(k));
    }
    assert!(CommitteeKind::Security.has_emergency_channel());
    assert!(!CommitteeKind::Resource.has_emergency_channel());
}

#[test]
fn high_reputation_long_online_agents_win_the_seats() {
    let agents: Vec<AgentKeys> = (0..6u8).map(keys).collect();
    let sock = keys(200);
    let refs: Vec<&AgentKeys> = agents.iter().collect();
    let mut kernel = kernel_with(&[&refs[..], &[&sock]].concat());
    let mut council = seeded_council(&agents, &sock);

    // 所有合格 Agent 都投「信誉最高的 3 人」（agents[5]、agents[4]、agents[3]）。
    let picks: Vec<Did> = agents.iter().rev().take(3).map(|k| k.did()).collect();
    let mut voters: Vec<&AgentKeys> = agents.iter().collect();
    voters.push(&sock);
    let ballots = ballots_for(CommitteeKind::Task, &voters, &picks);
    let outcome = council
        .elect(&mut kernel, CommitteeKind::Task, &ballots)
        .expect("elect");

    assert_eq!(outcome.epoch, 1);
    // 三人得分相同（全体选民都投了同一组），因此名次由信誉决定：5 > 4 > 3。
    for (i, e) in outcome.elected.iter().take(3).enumerate() {
        assert_eq!(e.did, agents[5 - i].did());
        assert_eq!(e.rank, (i + 1) as u16);
    }
    assert_eq!(outcome.elected[0].score, outcome.elected[2].score);
    assert!(outcome.elected[0].score > outcome.elected[3].score);
    // 空壳不在委员会里。
    let committee = council.committee(CommitteeKind::Task).expect("committee");
    assert!(!committee.has_member(&sock.did()));
    assert!(committee.is_bft_consistent());
    assert_eq!(committee.quorum(), 4); // n = 5，f = 1
    assert_eq!(committee.elected_at, 1);
    assert_eq!(
        committee.members[0].term_ends_at,
        1 + ElectionConfig::default().term
    );
}

#[test]
fn election_is_reproducible_across_runs_and_ballot_order() {
    let agents: Vec<AgentKeys> = (0..6u8).map(keys).collect();
    let sock = keys(201);
    let refs: Vec<&AgentKeys> = agents.iter().collect();
    let mut kernel = kernel_with(&[&refs[..], &[&sock]].concat());
    let mut council = seeded_council(&agents, &sock);
    let picks: Vec<Did> = agents.iter().take(2).map(|k| k.did()).collect();
    let mut voters: Vec<&AgentKeys> = agents.iter().collect();
    voters.push(&sock);
    let ballots = ballots_for(CommitteeKind::Resource, &voters, &picks);

    let first = council
        .elect(&mut kernel, CommitteeKind::Resource, &ballots)
        .expect("first");
    let mut shuffled = ballots.clone();
    shuffled.reverse();
    let second = council
        .elect(&mut kernel, CommitteeKind::Resource, &shuffled)
        .expect("second");

    // 改选了：epoch 递增；选举 id 只覆盖花名册与计票结果，因此两轮相同（可复算）。
    assert_eq!(first.id, second.id);
    assert_eq!(second.epoch, 2);
    assert_eq!(first.elected_dids(), second.elected_dids());
    assert_eq!(council.elections().len(), 2);
}

#[test]
fn sybil_flood_cannot_win_or_shift_the_outcome() {
    let agents: Vec<AgentKeys> = (0..6u8).map(keys).collect();
    let socks: Vec<AgentKeys> = (100..160u8).map(keys).collect();
    let mut all: Vec<&AgentKeys> = agents.iter().collect();
    all.extend(socks.iter());
    let mut kernel = kernel_with(&all);
    let mut council = seeded_council(&agents, &socks[0]);
    let picks: Vec<Did> = agents.iter().take(3).map(|k| k.did()).collect();

    // 基线：只有合格 Agent 投票。
    let honest = ballots_for(
        CommitteeKind::Evolution,
        &agents.iter().collect::<Vec<_>>(),
        &picks,
    );
    let baseline = council
        .elect(&mut kernel, CommitteeKind::Evolution, &honest)
        .expect("baseline");

    // 攻击：60 个空壳 DID 各自投自己（权重为 0，必须被忽略）。
    let mut attack_ballots = ballots_for(
        CommitteeKind::Evolution,
        &agents.iter().collect::<Vec<_>>(),
        &picks,
    );
    for s in &socks {
        attack_ballots
            .push(ElectionBallot::cast(s, CommitteeKind::Evolution, &[s.did()]).expect("cast"));
    }
    let outcome = council
        .elect(&mut kernel, CommitteeKind::Evolution, &attack_ballots)
        .expect("attack round");
    assert_eq!(outcome.elected_dids(), baseline.elected_dids());

    // 同一选民两张票 → 类型化拒绝，且不改变已安装的委员会。
    let dup = vec![
        ElectionBallot::cast(&agents[0], CommitteeKind::Evolution, &picks).expect("cast"),
        ElectionBallot::cast(&agents[0], CommitteeKind::Evolution, &[socks[0].did()])
            .expect("cast"),
    ];
    assert_eq!(
        council.elect(&mut kernel, CommitteeKind::Evolution, &dup),
        Err(CoreError::DuplicateAgent)
    );

    // 空壳既不当选，也不改变得分：忽略票被留痕。
    let committee = council
        .committee(CommitteeKind::Evolution)
        .expect("committee");
    for s in &socks {
        assert!(!committee.has_member(&s.did()));
    }
    assert_eq!(outcome.ballots_counted, 6);
    assert_eq!(outcome.ballots_ignored.len(), socks.len());
    assert!(outcome
        .ballots_ignored
        .iter()
        .all(|b| b.reason.contains("vote_weight 0")));
}

#[test]
fn forged_replayed_and_foreign_ballots_are_refused_with_typed_reasons() {
    let agents: Vec<AgentKeys> = (0..4u8).map(keys).collect();
    let outsider = keys(210);
    let mut kernel = Kernel::new(KernelConfig::default());
    for k in agents.iter().chain(std::iter::once(&outsider)) {
        kernel
            .register(k, "t", &["governance.vote"], Credits(20))
            .expect("register");
    }
    let mut council = seeded_council(&agents, &outsider);
    let picks: Vec<Did> = vec![agents[0].did()];

    // 1) 伪造：改了 choices 之后签名不再成立。
    let mut forged = ElectionBallot::cast(&agents[0], CommitteeKind::Task, &picks).expect("cast");
    forged.choices = vec![outsider.did()];
    let refused = council.elect(&mut kernel, CommitteeKind::Task, &[forged]);
    assert_eq!(refused, Err(CoreError::InvalidSignature));

    // 2) 跨委员会重放：选票绑定委员会类别。
    let replay = ElectionBallot::cast(&agents[0], CommitteeKind::Task, &picks).expect("cast");
    assert_eq!(
        council.elect(&mut kernel, CommitteeKind::Security, &[replay]),
        Err(CoreError::InvalidKind)
    );

    // 3) 未注册选民：DID 合法但不在花名册里。
    let unregistered = keys(211);
    let foreign = ElectionBallot::cast(&unregistered, CommitteeKind::Task, &picks).expect("cast");
    assert_eq!(
        council.elect(&mut kernel, CommitteeKind::Task, &[foreign]),
        Err(CoreError::UnknownAgent)
    );

    // 三次拒绝都留在内核拒绝账上（人类只读投影可见），且没有委员会被安装。
    assert_eq!(kernel.refusals().len(), 3);
    assert!(council.committee(CommitteeKind::Task).is_none());
}

#[test]
fn committee_of_zero_candidates_is_empty_but_consistent() {
    let agents: Vec<AgentKeys> = (0..3u8).map(keys).collect();
    let refs: Vec<&AgentKeys> = agents.iter().collect();
    let mut kernel = kernel_with(&refs);
    // 不给任何信誉 → 无人有候选资格。
    let mut council = Council::new(CouncilConfig::default());
    let ballots =
        vec![
            ElectionBallot::cast(&agents[0], CommitteeKind::Arbitration, &[agents[0].did()])
                .expect("cast"),
        ];
    let outcome = council
        .elect(&mut kernel, CommitteeKind::Arbitration, &ballots)
        .expect("elect");
    assert!(outcome.elected.is_empty());
    assert_eq!(outcome.ineligible.len(), 3);
    let committee = council
        .committee(CommitteeKind::Arbitration)
        .expect("committee");
    assert!(committee.is_empty());
    assert_eq!(committee.quorum(), 0);
    // 空委员会**没有表决资格**：否则 quorum = 0 会让「0 票 ≥ 0」的决议自动通过。
    assert!(!committee.is_bft_consistent());
    let checks = council.checks();
    assert!(!au4a_core::all_passed(&checks));
    assert!(checks
        .iter()
        .any(|c| c.name == "council.committees.nonempty" && !c.passed));
}

#[test]
fn self_check_and_results_json_are_real_assertions() {
    let checks = au4a_council::self_check();
    assert!(checks.len() >= 4);
    assert!(
        au4a_core::all_passed(&checks),
        "self_check 未全绿: {checks:?}"
    );
    for c in &checks {
        assert_eq!(c.track, "1.7");
        assert!(!c.detail.is_empty());
    }
    let results = au4a_council::results_json().expect("results");
    assert_eq!(results["track"], "1.7");
    assert_eq!(results["election"]["reproducible"], true);
    assert_eq!(results["election"]["ballots_ignored"], 8);
    assert_eq!(results["checks"], results["checks_passed"]);
}

#[test]
fn scenario_runs_the_whole_track_and_is_repeatable() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let first = au4a_council::scenario(&mut kernel).expect("scenario");
    assert_eq!(first["track"], "1.7");
    assert_eq!(first["elections"], 5);
    assert_eq!(first["sock_elected"], 0);
    assert!(first["sock_ballots_ignored"].as_u64().unwrap_or(0) > 0);

    // 同一内核再跑一遍：仍然不 panic、仍然选出五类委员会（幂等登记）。
    let second = au4a_council::scenario(&mut kernel).expect("scenario again");
    assert_eq!(second["elections"], 5);
    assert_eq!(first["committees"], second["committees"]);
}
