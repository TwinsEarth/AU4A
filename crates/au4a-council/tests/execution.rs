//! v1.7.4 执行引擎——集成测试。
//!
//! 覆盖：通过的动议真的落成状态变更（策略 / 信誉 / 账本）、只能执行 `passed` 的动议、
//! 执行者必须是在任委员、幂等（不会重复转账）、罚没有界、账本守恒。

use au4a_core::{AgentKeys, CoreError, Credits, Did, RefusalCode};
use au4a_council::{
    Action, AgentIdentity, CommitteeKind, Council, CouncilConfig, ElectionBallot, ProposalDraft,
    ProposalState,
};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

struct Scene {
    kernel: Kernel,
    council: Council,
    agents: Vec<AgentKeys>,
    kind: CommitteeKind,
}

/// `n` 个 Agent 全部当选（席位 = n），并各自备好余额。
fn scene(n: u8, kind: CommitteeKind) -> Scene {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut council = Council::new(CouncilConfig {
        election: au4a_council::ElectionConfig {
            seats: n as usize,
            ..au4a_council::ElectionConfig::default()
        },
        ..CouncilConfig::default()
    });
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
    Scene {
        kernel,
        council,
        agents,
        kind,
    }
}

impl Scene {
    /// 由 `author_index` 提案并让全体委员投赞成票直到出结论。
    fn pass(&mut self, author_index: usize, title: &str, action: Action) -> String {
        let author = AgentIdentity::from_keys(&self.agents[author_index]);
        let draft =
            ProposalDraft::by(&self.agents[author_index], self.kind, title, action).expect("draft");
        let proposal = self
            .council
            .propose(&mut self.kernel, &author, draft)
            .expect("propose");
        let round = self
            .council
            .open_round(&mut self.kernel, &proposal.id)
            .expect("open");
        let members = self
            .council
            .committee(self.kind)
            .map(|c| c.member_dids())
            .unwrap_or_default();
        let mut state = round;
        for did in &members {
            if state.outcome.is_closed() {
                break;
            }
            let k = self
                .agents
                .iter()
                .find(|k| &k.did() == did)
                .expect("member key");
            let vote =
                au4a_council::Vote::cast(k, &proposal.id, state.round, au4a_council::Choice::Yes)
                    .expect("cast");
            state = self
                .council
                .cast_vote(&mut self.kernel, vote)
                .expect("vote");
        }
        assert_eq!(state.outcome, au4a_council::RoundOutcome::Passed, "{title}");
        proposal.id
    }

    fn identity(&self, index: usize) -> AgentIdentity {
        AgentIdentity::from_keys(&self.agents[index])
    }
}

#[test]
fn a_passed_motion_becomes_a_real_state_change() {
    let mut s = scene(4, CommitteeKind::Resource);
    let id = s.pass(
        0,
        "设定结算上限",
        Action::SetPolicy {
            key: String::from("cpu_proto_settle_cap"),
            value: 250,
        },
    );
    // 执行前：策略不存在，状态为 passed。
    assert_eq!(s.council.policy("cpu_proto_settle_cap"), None);
    assert_eq!(
        s.council.proposal(&id).expect("p").state,
        ProposalState::Passed
    );

    let executor = s.identity(1);
    let receipt = s
        .council
        .execute(&mut s.kernel, &executor, &id)
        .expect("execute");

    assert_eq!(receipt.proposal, id);
    assert_eq!(receipt.executor, s.agents[1].did());
    assert_eq!(receipt.effects.len(), 1);
    assert!(receipt
        .summary()
        .contains("policy cpu_proto_settle_cap=250"));
    assert!(receipt.conservation_ok);
    assert!(receipt.ledger_effect_consistent());
    assert!(!receipt.touches_ledger());
    // 状态落地 + 策略落地 + 收据落地。
    assert_eq!(
        s.council.proposal(&id).expect("p").state,
        ProposalState::Executed
    );
    assert_eq!(s.council.policy("cpu_proto_settle_cap"), Some(250));
    assert!(s.council.execution(&id).is_some());
    assert_eq!(s.council.policies().len(), 1);
    // 执行留痕在治理事件里。
    assert!(s
        .council
        .events()
        .iter()
        .any(|e| e.kind == "proposal.executed" && e.subject == id));
    // 内核进度事件也能看到。
    assert!(s
        .kernel
        .progress_events()
        .iter()
        .any(|e| e.kind == "council.executed"));
}

#[test]
fn only_a_seated_member_can_execute() {
    let mut s = scene(4, CommitteeKind::Task);
    let id = s.pass(
        0,
        "设定参数",
        Action::SetPolicy {
            key: String::from("x"),
            value: 1,
        },
    );
    let outsider = keys(80);
    s.kernel
        .register(&outsider, "outsider", &["governance.vote"], Credits(20))
        .expect("register");
    let identity = AgentIdentity::from_keys(&outsider);
    assert_eq!(
        s.council.execute(&mut s.kernel, &identity, &id),
        Err(CoreError::InvalidSignature)
    );
    let (_, refusal) = s.kernel.refusals().last().expect("refusal");
    assert_eq!(refusal.code, RefusalCode::Unauthorized);
    assert!(refusal.code.is_misconduct());
    // 未执行：状态与策略都没变。
    assert_eq!(
        s.council.proposal(&id).expect("p").state,
        ProposalState::Passed
    );
    assert_eq!(s.council.policy("x"), None);
}

#[test]
fn an_unregistered_executor_is_refused() {
    let mut s = scene(4, CommitteeKind::Task);
    let id = s.pass(
        0,
        "设定参数",
        Action::SetPolicy {
            key: String::from("y"),
            value: 2,
        },
    );
    let ghost = keys(81);
    let identity = AgentIdentity::from_keys(&ghost);
    assert_eq!(
        s.council.execute(&mut s.kernel, &identity, &id),
        Err(CoreError::UnknownAgent)
    );
}

#[test]
fn open_and_rejected_motions_cannot_be_executed() {
    let mut s = scene(4, CommitteeKind::Task);
    // 还没表决的动议。
    let author = s.identity(0);
    let draft = ProposalDraft::by(
        &s.agents[0],
        CommitteeKind::Task,
        "没表决",
        Action::SetPolicy {
            key: String::from("z"),
            value: 3,
        },
    )
    .expect("draft");
    let open = s
        .council
        .propose(&mut s.kernel, &author, draft)
        .expect("propose");
    assert_eq!(
        s.council.execute(&mut s.kernel, &author, &open.id),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(s.council.policy("z"), None);

    // 被表决否决的动议。
    let draft = ProposalDraft::by(
        &s.agents[0],
        CommitteeKind::Task,
        "被否决",
        Action::SetPolicy {
            key: String::from("w"),
            value: 4,
        },
    )
    .expect("draft");
    let rejected = s
        .council
        .propose(&mut s.kernel, &author, draft)
        .expect("propose");
    let round = s
        .council
        .open_round(&mut s.kernel, &rejected.id)
        .expect("open");
    let members = s
        .council
        .committee(CommitteeKind::Task)
        .map(|c| c.member_dids())
        .unwrap_or_default();
    let mut state = round;
    for did in &members {
        if state.outcome.is_closed() {
            break;
        }
        let k = s.agents.iter().find(|k| &k.did() == did).expect("member");
        let vote = au4a_council::Vote::cast(k, &rejected.id, state.round, au4a_council::Choice::No)
            .expect("cast");
        state = s.council.cast_vote(&mut s.kernel, vote).expect("vote");
    }
    assert_eq!(state.outcome, au4a_council::RoundOutcome::Rejected);
    assert_eq!(
        s.council.execute(&mut s.kernel, &author, &rejected.id),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(s.council.policy("w"), None);
}

#[test]
fn execution_is_idempotent_and_cannot_be_replayed() {
    let mut s = scene(4, CommitteeKind::Task);
    let id = s.pass(
        0,
        "设定参数",
        Action::SetPolicy {
            key: String::from("once"),
            value: 1,
        },
    );
    let executor = s.identity(1);
    let first = s
        .council
        .execute(&mut s.kernel, &executor, &id)
        .expect("first");
    assert_eq!(s.council.executions().count(), 1);
    // 第二次执行 → 拒绝，且不产生第二张收据。
    assert_eq!(
        s.council.execute(&mut s.kernel, &executor, &id),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(s.council.executions().count(), 1);
    assert_eq!(s.council.execution(&id).map(|r| r.at), Some(first.at));
    assert_eq!(
        s.kernel.refusals().last().map(|(_, r)| r.code),
        Some(RefusalCode::Conflict)
    );
}

#[test]
fn a_transfer_execution_moves_credits_and_keeps_conservation() {
    let mut s = scene(4, CommitteeKind::Task);
    let before_from = s.kernel.ledger().balance(&s.agents[2].did()).available;
    let before_to = s.kernel.ledger().balance(&s.agents[3].did()).available;
    let id = s.pass(
        0,
        "结算转账",
        Action::Transfer {
            from: s.agents[2].did(),
            to: s.agents[3].did(),
            amount: Credits(30),
        },
    );
    let executor = s.identity(1);
    let receipt = s
        .council
        .execute(&mut s.kernel, &executor, &id)
        .expect("execute");

    assert!(receipt.touches_ledger());
    assert!(receipt.ledger_effect_consistent());
    assert_eq!(receipt.total_before, receipt.total_after, "转账不改变总量");
    assert_eq!(
        s.kernel.ledger().balance(&s.agents[2].did()).available,
        Credits(before_from.get() - 30)
    );
    assert_eq!(
        s.kernel.ledger().balance(&s.agents[3].did()).available,
        Credits(before_to.get() + 30)
    );
    s.kernel
        .ledger()
        .check_conservation()
        .expect("conservation");
}

#[test]
fn an_unaffordable_transfer_leaves_no_partial_state() {
    let mut s = scene(4, CommitteeKind::Task);
    // 动议给一个余额不足的账户开出大额转账：表决能通过，执行必须失败且不留痕。
    let id = s.pass(
        0,
        "超额转账",
        Action::Transfer {
            from: s.agents[2].did(),
            to: s.agents[3].did(),
            amount: Credits(999_999),
        },
    );
    let executor = s.identity(1);
    assert_eq!(
        s.council.execute(&mut s.kernel, &executor, &id),
        Err(CoreError::InsufficientFunds)
    );
    assert_eq!(
        s.council.proposal(&id).expect("p").state,
        ProposalState::Passed,
        "仍停在 passed"
    );
    assert!(s.council.execution(&id).is_none());
    assert_eq!(s.council.policies().len(), 0);
    s.kernel
        .ledger()
        .check_conservation()
        .expect("conservation");
}

#[test]
fn a_slash_is_bounded_by_locked_stake_and_burns_it() {
    let mut s = scene(4, CommitteeKind::Arbitration);
    let target = s.agents[3].did();
    let locked_before = s.kernel.ledger().balance(&target).locked;
    assert_eq!(locked_before, Credits(20));

    // 罚没 20（全部锁定质押）→ 成功，累计罚没增加 20，总量减少 20。
    let id = s.pass(
        0,
        "罚没全部质押",
        Action::Slash {
            did: target.clone(),
            amount: Credits(20),
        },
    );
    let executor = s.identity(1);
    let receipt = s
        .council
        .execute(&mut s.kernel, &executor, &id)
        .expect("execute");
    assert!(receipt.ledger_effect_consistent());
    assert_eq!(
        receipt.slashed_after.get(),
        receipt.slashed_before.get() + 20
    );
    assert_eq!(receipt.total_after.get(), receipt.total_before.get() - 20);
    assert_eq!(s.kernel.ledger().balance(&target).locked, Credits::ZERO);
    s.kernel
        .ledger()
        .check_conservation()
        .expect("conservation");

    // 再罚没已经为空的账户 → 拒绝（不做静默截断）。
    let id2 = s.pass(
        0,
        "再罚没",
        Action::Slash {
            did: target.clone(),
            amount: Credits(5),
        },
    );
    assert_eq!(
        s.council.execute(&mut s.kernel, &executor, &id2),
        Err(CoreError::InsufficientFunds)
    );
    assert_eq!(
        s.kernel.refusals().last().map(|(_, r)| r.code),
        Some(RefusalCode::PolicyDenied)
    );
    s.kernel
        .ledger()
        .check_conservation()
        .expect("conservation");
}

#[test]
fn a_reputation_execution_writes_the_governance_book() {
    let mut s = scene(4, CommitteeKind::Evolution);
    let target = s.agents[2].did();
    assert_eq!(s.council.reputation(&target), 5_000);
    let id = s.pass(
        0,
        "降信誉",
        Action::SetReputation {
            did: target.clone(),
            reputation_bp: 1_000,
        },
    );
    let executor = s.identity(1);
    let receipt = s
        .council
        .execute(&mut s.kernel, &executor, &id)
        .expect("execute");
    assert_eq!(s.council.reputation(&target), 1_000);
    assert!(!receipt.touches_ledger());
    assert!(receipt.summary().contains("reputation"));
    // 信誉账的变化会让下一次选举花名册随之变化（治理闭环）。
    let roster = s.council.roster(&s.kernel);
    let row = roster.iter().find(|c| c.did == target).expect("candidate");
    assert_eq!(row.reputation_bp, 1_000);
}

#[test]
fn self_check_results_and_scenario_show_the_execution() {
    let checks = au4a_council::self_check();
    assert!(
        au4a_core::all_passed(&checks),
        "self_check 未全绿: {checks:?}"
    );
    for name in [
        "council.execution.applied",
        "council.executions.ledger_effects",
        "council.executions.state_matches",
    ] {
        assert!(checks.iter().any(|c| c.name == name), "缺少自检项 {name}");
    }

    let results = au4a_council::results_json().expect("results");
    assert_eq!(results["execution"]["conservation_ok"], true);
    assert_eq!(results["execution"]["ledger_effect_consistent"], true);
    assert_eq!(
        results["execution"]["policies"]["cpu_proto_settle_cap"],
        250
    );
    assert_eq!(results["proposal"]["state"], "executed");

    let mut kernel = Kernel::new(KernelConfig::default());
    let scenario = au4a_council::scenario(&mut kernel).expect("scenario");
    assert_eq!(
        scenario["execution"]["policies"]["cpu_proto_settle_cap"],
        250
    );
    assert_eq!(scenario["proposals"][0]["state"], "executed");
    assert!(scenario["execution"]["effects"][0]
        .as_str()
        .unwrap_or_default()
        .contains("cpu_proto_settle_cap=250"));
}
