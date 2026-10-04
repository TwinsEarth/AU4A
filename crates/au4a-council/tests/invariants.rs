//! v1.7.7 测试版——不变式与确定性重放的集成测试。
//!
//! 这一版把「治理过程是否始终自洽」变成可执行的证伪器：确定性重放 + 每步不变式检查。
//! 同一种子必须给出同一份报告——失败可复现，不是「偶发」。

use au4a_core::Credits;
use au4a_council::invariants::{self, INVARIANT_NAMES};
use au4a_council::{
    check_all, replay, replay_report, state_digest, Action, AgentIdentity, CommitteeKind, Council,
    CouncilConfig, ElectionBallot, HumanObserver, ProposalDraft,
};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> au4a_core::AgentKeys {
    au4a_core::AgentKeys::from_seed(&[seed; 32])
}

/// 一个小型但完整的治理现场（4 人委员会 + 一条执行 + 一条被阻断）。
fn built_scene() -> (Kernel, Council) {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut council = Council::new(CouncilConfig {
        election: au4a_council::ElectionConfig {
            seats: 4,
            ..au4a_council::ElectionConfig::default()
        },
        ..CouncilConfig::default()
    });
    let agents: Vec<_> = (0..4u8).map(keys).collect();
    for k in &agents {
        kernel.register(k, "t", &["governance.vote"], Credits(20)).expect("register");
        council.note_reputation(&k.did(), 5_000);
        council.note_uptime(&k.did(), 200);
    }
    let picks: Vec<au4a_core::Did> = agents.iter().map(|k| k.did()).collect();
    let ballots: Vec<ElectionBallot> = agents
        .iter()
        .map(|k| ElectionBallot::cast(k, CommitteeKind::Task, &picks).expect("cast"))
        .collect();
    council.elect(&mut kernel, CommitteeKind::Task, &ballots).expect("elect");
    (kernel, council)
}

fn propose(kernel: &mut Kernel, council: &mut Council, agents: &[au4a_core::AgentKeys], title: &str, key: &str) -> String {
    let identity = AgentIdentity::from_keys(&agents[0]);
    let draft = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        title,
        Action::SetPolicy { key: key.to_string(), value: 7 },
    )
    .expect("draft");
    council
        .propose(kernel, &identity, draft)
        .expect("propose")
        .id
}

#[test]
fn replay_is_deterministic_across_runs() {
    for seed in [1u64, 2, 3, 0x17_07] {
        let a = replay(seed, 60).expect("replay a");
        let b = replay(seed, 60).expect("replay b");
        assert_eq!(a, b, "seed {seed} 两次重放必须逐字段相同");
        assert_eq!(a.digest, b.digest, "seed {seed} 状态摘要必须相同");
        assert!(a.ok(), "seed {seed} 出现违例：{:?}", a.violations);
    }
}

#[test]
fn replay_exercises_every_failure_mode_and_holds_invariants() {
    let mut totals = (0usize, 0usize, 0usize, 0usize, 0usize);
    for seed in [11u64, 22, 33, 44] {
        let report = replay(seed, 90).expect("replay");
        assert!(report.ok(), "seed {seed} 违例：{:?}", report.violations);
        // 每步都跑满全部不变式。
        assert_eq!(report.invariants_per_step, INVARIANT_NAMES.len());
        assert!(report.proposals > 0, "seed {seed} 没有产生动议");
        assert!(report.refusals > 0, "seed {seed} 没有触发任何拒绝路径");
        totals.0 += report.executed;
        totals.1 += report.blocked;
        totals.2 += report.voided_rounds;
        totals.3 += report.refusals;
        totals.4 += report.rejected;
    }
    // 四种结局都必须被覆盖到（重放真的在跑治理，而不是空转）。
    assert!(totals.0 > 0, "没有任何动议被执行");
    assert!(totals.1 > 0, "没有任何动议被人类阻断");
    assert!(totals.2 > 0, "没有任何表决轮被双签作废");
    assert!(totals.4 > 0 || totals.3 > 0, "没有出现否决或拒绝");
}

#[test]
fn different_seeds_produce_different_histories() {
    let a = replay(1, 60).expect("a");
    let b = replay(2, 60).expect("b");
    assert_ne!(a.digest, b.digest, "不同种子应给出不同治理历史");
    assert_eq!(a.seed, 1);
    assert_eq!(b.seed, 2);
}

#[test]
fn state_digest_tracks_every_state_change() {
    let (mut kernel, mut council) = built_scene();
    let agents: Vec<_> = (0..4u8).map(keys).collect();

    let d0 = state_digest(&council).expect("digest");
    // 只读操作不改变摘要。
    let view = HumanObserver::new("operator").observe(&council);
    assert_eq!(view.committees.len(), 1);
    assert_eq!(state_digest(&council).expect("digest"), d0);

    // 提案改变摘要。
    let id = propose(&mut kernel, &mut council, &agents, "摘要测试", "digest");
    let d1 = state_digest(&council).expect("digest");
    assert_ne!(d0, d1);

    // 开轮改变摘要。
    let round = council.open_round(&mut kernel, &id).expect("open");
    let d2 = state_digest(&council).expect("digest");
    assert_ne!(d1, d2);

    // 投票改变摘要。
    let vote = au4a_council::Vote::cast(&agents[0], &id, round.round, au4a_council::Choice::Yes)
        .expect("cast");
    council.cast_vote(&mut kernel, vote).expect("vote");
    let d3 = state_digest(&council).expect("digest");
    assert_ne!(d2, d3);

    // 重复计算稳定。
    assert_eq!(state_digest(&council).expect("digest"), d3);
}

#[test]
fn the_invariant_list_is_exactly_what_is_enforced() {
    let (_kernel, council) = built_scene();
    let checks = check_all(&council);
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    // 清单 ↔ 实现严格相等（两个方向都断言）：删掉或新增不变式都必须同步改清单。
    assert_eq!(names.len(), INVARIANT_NAMES.len(), "不变式数量与清单不一致：{names:?}");
    for expected in INVARIANT_NAMES {
        assert!(names.contains(&expected), "清单里的 {expected} 没有实现");
    }
    for name in &names {
        assert!(INVARIANT_NAMES.contains(name), "实现里的 {name} 不在清单中");
    }
    assert!(au4a_core::all_passed(&checks), "不变式未全绿：{checks:?}");
}

#[test]
fn replay_report_json_is_machine_readable() {
    let value = replay_report(&[5, 6], 40).expect("report");
    assert_eq!(value["seeds"], serde_json::json!([5, 6]));
    assert_eq!(value["total_violations"], 0);
    let reports = value["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 2);
    for r in reports {
        assert_eq!(r["violations"], 0);
        assert!(r["digest"].as_str().map(|d| d.len() == 64).unwrap_or(false));
        assert!(r["proposals"].as_u64().unwrap_or(0) > 0);
    }
}

#[test]
fn zero_steps_is_a_valid_no_op() {
    let report = replay(9, 0).expect("replay");
    assert_eq!(report.steps, 0);
    assert_eq!(report.applied, 0);
    assert!(report.ok());
    assert_eq!(report.digest.len(), 64);
    // 空重放的摘要等于「只建场」的摘要：可复现。
    let again = replay(9, 0).expect("replay");
    assert_eq!(report.digest, again.digest);
}

#[test]
fn self_check_reports_the_replay_evidence() {
    let checks = au4a_council::self_check();
    assert!(au4a_core::all_passed(&checks), "self_check 未全绿：{checks:?}");
    let replay_check = checks
        .iter()
        .find(|c| c.name == "council.replay.invariants")
        .expect("缺少重放自检项");
    assert!(replay_check.detail.contains("不变式"), "{}", replay_check.detail);
    assert!(checks.iter().any(|c| c.name == "council.replay.coverage"));

    let results = au4a_council::results_json().expect("results");
    assert_eq!(results["invariants"]["names"], INVARIANT_NAMES.len());
    assert_eq!(results["invariants"]["replay"]["total_violations"], 0);
    assert_ne!(
        results["invariants"]["state_digest"],
        results["invariants"]["vetoed_digest"],
        "不同治理现场必须给出不同摘要"
    );
    assert!(invariants::replay_report(&[1], 12).is_ok());
}
