//! v1.7.3 表决机制——集成测试。
//!
//! 覆盖 BFT-lite 法定人数（`quorum = n - f`，`n ≥ 3f+1`）、重复投票拒绝、
//! 模棱两可（双签）作废整轮、越权投票拒绝、关闭轮次拒绝，以及结论落地到动议状态。

use au4a_core::{AgentKeys, CoreError, Credits, Did, RefusalCode};
use au4a_council::{
    Action, AgentIdentity, Choice, CommitteeKind, Council, CouncilConfig, ElectionBallot,
    ElectionConfig, ProposalDraft, ProposalState, RoundOutcome, Vote,
};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

/// 建场：`n` 个 Agent 全部当选某委员会（席位 = n），并提交一条动议。
struct Scene {
    kernel: Kernel,
    council: Council,
    agents: Vec<AgentKeys>,
    proposal: String,
}

fn scene(n: u8, kind: CommitteeKind) -> Scene {
    let mut kernel = Kernel::new(KernelConfig::default());
    let cfg = CouncilConfig {
        election: ElectionConfig {
            seats: n as usize,
            ..ElectionConfig::default()
        },
        ..CouncilConfig::default()
    };
    let mut council = Council::new(cfg);
    let agents: Vec<AgentKeys> = (0..n).map(keys).collect();
    for k in &agents {
        kernel
            .register(k, "t", &["governance.vote"], Credits(20))
            .expect("register");
        council.note_reputation(&k.did(), 5_000);
        council.note_uptime(&k.did(), 300);
    }
    let picks: Vec<Did> = agents.iter().map(|k| k.did()).collect();
    let ballots: Vec<ElectionBallot> = agents
        .iter()
        .map(|k| ElectionBallot::cast(k, kind, &picks).expect("cast"))
        .collect();
    council.elect(&mut kernel, kind, &ballots).expect("elect");

    let author = AgentIdentity::from_keys(&agents[0]);
    let draft = ProposalDraft::by(
        &agents[0],
        kind,
        format!("动议 n={n}"),
        Action::SetPolicy {
            key: String::from("k"),
            value: 1,
        },
    )
    .expect("draft");
    let proposal = council
        .propose(&mut kernel, &author, draft)
        .expect("propose");

    Scene {
        kernel,
        council,
        agents,
        proposal: proposal.id,
    }
}

#[test]
fn quorum_is_n_minus_f_and_exactly_decides() {
    // n = 4 → f = 1、quorum = 3。
    let mut s = scene(4, CommitteeKind::Task);
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");
    assert_eq!(round.tally.n, 4);
    assert_eq!(round.tally.f, 1);
    assert_eq!(round.tally.quorum, 3);
    assert_eq!(round.outcome, RoundOutcome::Pending);

    // 2 票赞成还不够。
    for i in 0..2 {
        let v = Vote::cast(&s.agents[i], &s.proposal, round.round, Choice::Yes).expect("cast");
        let state = s.council.cast_vote(&mut s.kernel, v).expect("vote");
        assert_eq!(state.outcome, RoundOutcome::Pending);
        assert_eq!(state.tally.yes, i + 1);
    }
    assert_eq!(
        s.council.proposal(&s.proposal).expect("p").state,
        ProposalState::Open
    );

    // 第 3 票达到法定人数 → 通过。
    let v = Vote::cast(&s.agents[2], &s.proposal, round.round, Choice::Yes).expect("cast");
    let state = s.council.cast_vote(&mut s.kernel, v).expect("vote");
    assert_eq!(state.outcome, RoundOutcome::Passed);
    assert_eq!(state.tally.yes, 3);
    assert_eq!(state.tally.quorum, 3);
    assert!(state.tally.is_consistent());
    assert_eq!(
        s.council.proposal(&s.proposal).expect("p").state,
        ProposalState::Passed
    );
}

#[test]
fn seven_member_committee_needs_five_and_tolerates_two_faults() {
    // n = 7 = 3f+1 → f = 2、quorum = 5 = 2f+1；缺席 2 人仍可出结论。
    let mut s = scene(7, CommitteeKind::Evolution);
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");
    assert_eq!(
        (round.tally.n, round.tally.f, round.tally.quorum),
        (7, 2, 5)
    );
    let mut last = round.clone();
    for i in 0..5 {
        let v = Vote::cast(&s.agents[i], &s.proposal, round.round, Choice::Yes).expect("cast");
        last = s.council.cast_vote(&mut s.kernel, v).expect("vote");
    }
    assert_eq!(last.outcome, RoundOutcome::Passed);
    assert_eq!(last.tally.participation, 5);
    assert!(
        last.tally.participation < last.tally.n,
        "2 名委员缺席仍出结论"
    );
}

#[test]
fn enough_no_votes_reject_the_motion() {
    let mut s = scene(4, CommitteeKind::Task);
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");
    let mut last = round.clone();
    for i in 0..3 {
        let v = Vote::cast(&s.agents[i], &s.proposal, round.round, Choice::No).expect("cast");
        last = s.council.cast_vote(&mut s.kernel, v).expect("vote");
    }
    assert_eq!(last.outcome, RoundOutcome::Rejected);
    assert_eq!(last.tally.no, 3);
    assert_eq!(
        s.council.proposal(&s.proposal).expect("p").state,
        ProposalState::Rejected
    );
}

#[test]
fn abstentions_never_produce_a_conclusion() {
    let mut s = scene(4, CommitteeKind::Task);
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");
    let mut last = round.clone();
    for k in s.agents.iter() {
        let v = Vote::cast(k, &s.proposal, round.round, Choice::Abstain).expect("cast");
        last = s.council.cast_vote(&mut s.kernel, v).expect("vote");
    }
    assert_eq!(last.tally.abstain, 4);
    assert_eq!(last.tally.participation, 4);
    assert_eq!(last.outcome, RoundOutcome::Pending, "全票弃权不产生结论");
    assert_eq!(
        s.council.proposal(&s.proposal).expect("p").state,
        ProposalState::Open
    );
}

#[test]
fn a_duplicate_vote_in_the_same_round_is_refused_and_the_round_survives() {
    let mut s = scene(4, CommitteeKind::Task);
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");
    let v = Vote::cast(&s.agents[0], &s.proposal, round.round, Choice::Yes).expect("cast");
    s.council.cast_vote(&mut s.kernel, v).expect("first");

    // 同一选择再投一次 → 拒绝。
    let again = Vote::cast(&s.agents[0], &s.proposal, round.round, Choice::Yes).expect("cast");
    assert_eq!(
        s.council.cast_vote(&mut s.kernel, again),
        Err(CoreError::DuplicateAgent)
    );
    let state = s.council.current_round(&s.proposal).expect("round");
    assert_eq!(state.outcome, RoundOutcome::Pending, "重复票不影响本轮");
    assert_eq!(state.tally.yes, 1);
    assert_eq!(state.tally.participation, 1);
    assert!(s
        .kernel
        .refusals()
        .iter()
        .any(|(_, r)| r.code == RefusalCode::Conflict));
    assert!(
        !RefusalCode::Conflict.is_misconduct(),
        "重复票按竞争语义分类"
    );
}

#[test]
fn an_ambiguous_double_vote_voids_the_whole_round() {
    let mut s = scene(4, CommitteeKind::Task);
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");
    // 先投 2 张赞成（离法定人数只差 1）。
    for i in 0..2 {
        let v = Vote::cast(&s.agents[i], &s.proposal, round.round, Choice::Yes).expect("cast");
        s.council.cast_vote(&mut s.kernel, v).expect("vote");
    }
    // 同一委员改投反对 → 模棱两可：整轮作废。
    let flip = Vote::cast(&s.agents[1], &s.proposal, round.round, Choice::No).expect("cast");
    assert_eq!(
        s.council.cast_vote(&mut s.kernel, flip),
        Err(CoreError::DuplicateAgent)
    );
    let voided = s.council.round(&s.proposal, round.round).expect("round");
    assert_eq!(voided.outcome, RoundOutcome::VoidAmbiguous);
    assert_eq!(
        s.council.proposal(&s.proposal).expect("p").state,
        ProposalState::Open,
        "作废轮不产生结论，动议回到待表决"
    );
    // 作废轮不再接受投票。
    let late = Vote::cast(&s.agents[2], &s.proposal, round.round, Choice::Yes).expect("cast");
    assert_eq!(
        s.council.cast_vote(&mut s.kernel, late),
        Err(CoreError::InvalidVersion)
    );
    // 重开一轮，这次一帆风顺。
    let second = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("reopen");
    assert_eq!(second.round, round.round + 1);
    let mut last = second.clone();
    for i in 0..3 {
        let v = Vote::cast(&s.agents[i], &s.proposal, second.round, Choice::Yes).expect("cast");
        last = s.council.cast_vote(&mut s.kernel, v).expect("vote");
    }
    assert_eq!(last.outcome, RoundOutcome::Passed);
    assert_eq!(
        s.council.proposal(&s.proposal).expect("p").state,
        ProposalState::Passed
    );
    let events: Vec<&str> = s
        .council
        .events()
        .iter()
        .filter(|e| e.kind == "vote.void")
        .map(|e| e.subject.as_str())
        .collect();
    assert_eq!(events, vec![s.proposal.as_str()], "作废必须留痕");
}

#[test]
fn non_members_cannot_vote() {
    let mut s = scene(4, CommitteeKind::Task);
    let outsider = keys(77);
    s.kernel
        .register(&outsider, "outsider", &["governance.vote"], Credits(20))
        .expect("register");
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");
    let v = Vote::cast(&outsider, &s.proposal, round.round, Choice::Yes).expect("cast");
    assert_eq!(
        s.council.cast_vote(&mut s.kernel, v),
        Err(CoreError::InvalidSignature)
    );
    let (_, refusal) = s.kernel.refusals().last().expect("refusal");
    assert_eq!(refusal.code, RefusalCode::Unauthorized);
    assert!(refusal.code.is_misconduct(), "越权投票是单次即恶意");
}

#[test]
fn forged_votes_and_wrong_rounds_are_refused() {
    let mut s = scene(5, CommitteeKind::Resource);
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");

    // 签名被篡改。
    let mut forged = Vote::cast(&s.agents[0], &s.proposal, round.round, Choice::Yes).expect("cast");
    forged.choice = Choice::No;
    assert_eq!(
        s.council.cast_vote(&mut s.kernel, forged),
        Err(CoreError::InvalidSignature)
    );

    // 投给未来轮次 / 不存在的轮次 → stale_epoch。
    let future = Vote::cast(&s.agents[1], &s.proposal, round.round + 3, Choice::Yes).expect("cast");
    assert_eq!(
        s.council.cast_vote(&mut s.kernel, future),
        Err(CoreError::InvalidVersion)
    );
    assert!(s
        .kernel
        .refusals()
        .iter()
        .any(|(_, r)| r.code == RefusalCode::StaleEpoch));
}

#[test]
fn a_closed_round_takes_no_more_votes_and_there_is_no_second_conclusion() {
    let mut s = scene(4, CommitteeKind::Task);
    let round = s
        .council
        .open_round(&mut s.kernel, &s.proposal)
        .expect("open");
    let mut last = round.clone();
    for i in 0..3 {
        let v = Vote::cast(&s.agents[i], &s.proposal, round.round, Choice::Yes).expect("cast");
        last = s.council.cast_vote(&mut s.kernel, v).expect("vote");
    }
    assert_eq!(last.outcome, RoundOutcome::Passed);

    // 第 4 名委员迟到，轮次已关闭。
    let late = Vote::cast(&s.agents[3], &s.proposal, round.round, Choice::Yes).expect("cast");
    assert_eq!(
        s.council.cast_vote(&mut s.kernel, late),
        Err(CoreError::InvalidVersion)
    );
    // 已通过的动议不能重开轮次，也不能被改判。
    assert_eq!(
        s.council.open_round(&mut s.kernel, &s.proposal),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        s.council
            .transition_to(&mut s.kernel, &s.proposal, ProposalState::Rejected),
        Err(CoreError::InvalidKind)
    );
    let state = s.council.round(&s.proposal, round.round).expect("round");
    assert_eq!(state.tally.yes, 3);
    assert_eq!(state.votes.len(), 3);
}

#[test]
fn self_check_results_and_scenario_expose_the_vote_math() {
    let checks = au4a_council::self_check();
    assert!(
        au4a_core::all_passed(&checks),
        "self_check 未全绿: {checks:?}"
    );
    for name in [
        "council.vote.quorum",
        "council.votes.tally_consistent",
        "council.votes.members_only",
        "council.rounds.decided_matches_proposal",
    ] {
        assert!(checks.iter().any(|c| c.name == name), "缺少自检项 {name}");
    }

    let results = au4a_council::results_json().expect("results");
    assert_eq!(results["voting"]["outcome"], "passed");
    assert_eq!(results["voting"]["quorum"], 4);
    assert_eq!(results["voting"]["n"], 5);
    assert_eq!(results["proposal"]["state"], "executed");

    let mut kernel = Kernel::new(KernelConfig::default());
    let scenario = au4a_council::scenario(&mut kernel).expect("scenario");
    assert_eq!(scenario["voting"]["outcome"], "passed");
    // 达到法定人数（4）即出结论：第 5 名委员的票不再需要，也不会被接受。
    assert_eq!(scenario["voting"]["yes"], 4);
    assert_eq!(scenario["voting"]["quorum"], 4);
    assert_eq!(
        scenario["voting"]["voters"].as_array().map(|v| v.len()),
        Some(4)
    );
    assert_eq!(scenario["proposals"][0]["state"], "executed");
}
