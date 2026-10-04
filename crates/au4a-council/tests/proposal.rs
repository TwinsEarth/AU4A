//! v1.7.2 提案流程——集成测试。
//!
//! 覆盖：只有 Agent 能提案（且只有该委员会委员）、动议内容寻址且去重、
//! 篡改内容会破坏签名、人类观察者只有只读投影、状态机拒绝非法迁移。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_council::human::{observe_json, observed_dids, HumanObserver};
use au4a_council::{
    Action, AgentIdentity, CommitteeKind, Council, CouncilConfig, ElectionBallot, ProposalDraft,
    ProposalState,
};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

/// 建一个内核 + 治理层，并把任务委员会选出来（5 名委员来自 5 个 Agent）。
fn seated(kernel: &mut Kernel) -> (Council, Vec<AgentKeys>) {
    let mut council = Council::new(CouncilConfig::default());
    let agents: Vec<AgentKeys> = (0..5u8).map(keys).collect();
    for k in &agents {
        kernel.register(k, "t", &["governance.vote"], Credits(20)).expect("register");
        council.note_reputation(&k.did(), 5_000);
        council.note_uptime(&k.did(), 200);
    }
    let picks: Vec<Did> = agents.iter().map(|k| k.did()).collect();
    let ballots: Vec<ElectionBallot> = agents
        .iter()
        .map(|k| ElectionBallot::cast(k, CommitteeKind::Task, &picks).expect("cast"))
        .collect();
    council.elect(kernel, CommitteeKind::Task, &ballots).expect("elect");
    (council, agents)
}

#[test]
fn a_seated_agent_can_propose_and_the_motion_is_content_addressed() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, agents) = seated(&mut kernel);
    let author = AgentIdentity::from_keys(&agents[0]);
    let draft = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        "提高任务准入门槛",
        Action::SetPolicy { key: String::from("task_entry_bar"), value: 400 },
    )
    .expect("draft");

    let proposal = council.propose(&mut kernel, &author, draft).expect("propose");
    assert_eq!(proposal.state, ProposalState::Open);
    assert_eq!(proposal.round, 0);
    assert_eq!(proposal.author, agents[0].did());
    assert_eq!(proposal.committee, CommitteeKind::Task);
    // id 可由内容复算。
    assert_eq!(
        proposal.id,
        au4a_council::Proposal::id_for(
            &proposal.author,
            proposal.committee,
            &proposal.title,
            &proposal.action
        )
        .expect("id")
    );
    assert_eq!(council.proposals().len(), 1);
    assert_eq!(council.proposals_of(CommitteeKind::Task).len(), 1);
    assert!(council.proposals_of(CommitteeKind::Resource).is_empty());
    assert_eq!(council.events().len(), 2); // 选举 + 动议
}

#[test]
fn a_non_member_agent_cannot_propose() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, agents) = seated(&mut kernel);
    let outsider = keys(90);
    kernel
        .register(&outsider, "outsider", &["governance.vote"], Credits(20))
        .expect("register");
    let identity = AgentIdentity::from_keys(&outsider);
    let draft = ProposalDraft::by(
        &outsider,
        CommitteeKind::Task,
        "我要改规则",
        Action::SetPolicy { key: String::from("x"), value: 1 },
    )
    .expect("draft");
    assert_eq!(
        council.propose(&mut kernel, &identity, draft),
        Err(CoreError::InvalidSignature)
    );
    let (_, refusal) = &kernel.refusals()[0];
    assert_eq!(refusal.code, au4a_core::RefusalCode::Unauthorized);
    assert!(refusal.code.is_misconduct());
    assert!(council.proposals().is_empty());
    assert_eq!(agents.len(), 5);
}

#[test]
fn an_unregistered_identity_cannot_propose() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, _) = seated(&mut kernel);
    let ghost = keys(91);
    let identity = AgentIdentity::from_keys(&ghost);
    let draft = ProposalDraft::by(
        &ghost,
        CommitteeKind::Task,
        "幽灵动议",
        Action::SetPolicy { key: String::from("x"), value: 1 },
    )
    .expect("draft");
    assert_eq!(
        council.propose(&mut kernel, &identity, draft),
        Err(CoreError::UnknownAgent)
    );
}

#[test]
fn a_proposal_for_an_unseated_committee_is_refused() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, agents) = seated(&mut kernel);
    let identity = AgentIdentity::from_keys(&agents[0]);
    // 安全委员会还没选出来。
    let draft = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Security,
        "紧急接管",
        Action::SetPolicy { key: String::from("x"), value: 1 },
    )
    .expect("draft");
    assert_eq!(
        council.propose(&mut kernel, &identity, draft),
        Err(CoreError::UnknownAgent)
    );
    assert_eq!(kernel.refusals()[0].1.code, au4a_core::RefusalCode::StaleEpoch);
}

#[test]
fn identical_motions_are_deduplicated_by_content_address() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, agents) = seated(&mut kernel);
    let identity = AgentIdentity::from_keys(&agents[1]);
    let make = || {
        ProposalDraft::by(
            &agents[1],
            CommitteeKind::Task,
            "同一动议",
            Action::SetPolicy { key: String::from("dup"), value: 7 },
        )
        .expect("draft")
    };
    council.propose(&mut kernel, &identity, make()).expect("first");
    assert_eq!(
        council.propose(&mut kernel, &identity, make()),
        Err(CoreError::DuplicateAgent)
    );
    assert_eq!(council.proposals().len(), 1);
    assert_eq!(kernel.refusals()[0].1.code, au4a_core::RefusalCode::Conflict);
}

#[test]
fn tampered_content_never_reaches_the_council() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, agents) = seated(&mut kernel);
    let identity = AgentIdentity::from_keys(&agents[2]);
    let mut draft = ProposalDraft::by(
        &agents[2],
        CommitteeKind::Task,
        "转账动议",
        Action::Transfer {
            from: agents[3].did(),
            to: agents[4].did(),
            amount: Credits(5),
        },
    )
    .expect("draft");
    // 攻击者把金额改成 500（内容变了，签名不再成立）。
    draft.action = Action::Transfer {
        from: agents[3].did(),
        to: agents[4].did(),
        amount: Credits(500),
    };
    assert_eq!(
        council.propose(&mut kernel, &identity, draft),
        Err(CoreError::InvalidSignature)
    );
    assert_eq!(kernel.refusals()[0].1.code, au4a_core::RefusalCode::Unauthorized);
    assert!(council.proposals().is_empty());
}

#[test]
fn illegal_actions_and_titles_are_refused_before_any_state_change() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, agents) = seated(&mut kernel);
    let identity = AgentIdentity::from_keys(&agents[0]);

    let zero_transfer = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        "零额转账",
        Action::Transfer { from: agents[0].did(), to: agents[1].did(), amount: Credits(0) },
    )
    .expect("draft");
    assert_eq!(
        council.propose(&mut kernel, &identity, zero_transfer),
        Err(CoreError::ZeroAmount)
    );

    let bad_reputation = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        "超范围信誉",
        Action::SetReputation { did: agents[1].did(), reputation_bp: 20_000 },
    )
    .expect("draft");
    assert_eq!(
        council.propose(&mut kernel, &identity, bad_reputation),
        Err(CoreError::InvalidKind)
    );

    let empty_title = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        "   ",
        Action::SetPolicy { key: String::from("x"), value: 1 },
    )
    .expect("draft");
    assert_eq!(
        council.propose(&mut kernel, &identity, empty_title),
        Err(CoreError::InvalidKind)
    );

    assert!(council.proposals().is_empty());
    assert_eq!(kernel.refusals().len(), 3);
    assert!(kernel
        .refusals()
        .iter()
        .all(|(_, r)| r.code == au4a_core::RefusalCode::Malformed));
}

#[test]
fn the_state_machine_refuses_illegal_transitions() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, agents) = seated(&mut kernel);
    let identity = AgentIdentity::from_keys(&agents[0]);
    let draft = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        "状态机动议",
        Action::SetPolicy { key: String::from("sm"), value: 1 },
    )
    .expect("draft");
    let proposal = council.propose(&mut kernel, &identity, draft).expect("propose");

    // open → executed 非法（未表决不得执行）。
    assert_eq!(
        council.transition_to(&mut kernel, &proposal.id, ProposalState::Executed),
        Err(CoreError::InvalidKind)
    );
    // open → passed 合法。
    let passed = council
        .transition_to(&mut kernel, &proposal.id, ProposalState::Passed)
        .expect("pass");
    assert_eq!(passed.state, ProposalState::Passed);
    // passed → rejected 非法（已通过不得改判）。
    assert_eq!(
        council.transition_to(&mut kernel, &proposal.id, ProposalState::Rejected),
        Err(CoreError::InvalidKind)
    );
    // passed → executed 合法。
    let executed = council
        .transition_to(&mut kernel, &proposal.id, ProposalState::Executed)
        .expect("execute");
    assert_eq!(executed.state, ProposalState::Executed);
    assert!(executed.state.is_terminal());
    // 终态不可再动。
    assert_eq!(
        council.transition_to(&mut kernel, &proposal.id, ProposalState::Blocked),
        Err(CoreError::InvalidKind)
    );
    // 三次非法迁移（open→executed、passed→rejected、executed→blocked）都留痕为 conflict。
    let conflicts = kernel
        .refusals()
        .iter()
        .filter(|(_, r)| r.code == au4a_core::RefusalCode::Conflict)
        .count();
    assert_eq!(conflicts, 3);
    assert!(!au4a_core::RefusalCode::Conflict.is_misconduct());
}

#[test]
fn the_human_observer_is_read_only() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let (mut council, agents) = seated(&mut kernel);
    let identity = AgentIdentity::from_keys(&agents[0]);
    let draft = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        "只读投影",
        Action::SetPolicy { key: String::from("view"), value: 3 },
    )
    .expect("draft");
    let proposal = council.propose(&mut kernel, &identity, draft).expect("propose");

    let human = HumanObserver::new("operator");
    let view = human.observe(&council);
    assert_eq!(view.label, "operator");
    assert_eq!(view.seats_filled, 5);
    assert_eq!(view.committees.len(), 1);
    assert_eq!(view.committees[0].quorum, 4); // n=5 → f=1
    assert_eq!(view.proposals.len(), 1);
    assert_eq!(view.proposals[0].id, proposal.id);
    assert_eq!(view.proposals[0].state, ProposalState::Open);
    assert_eq!(view.proposals[0].author, agents[0].did().as_str());
    assert!(view.proposals[0].action.contains("set_policy view=3"));

    // 观察是可重复的纯读：再观察一次结果完全相同。
    let again = human.observe(&council);
    assert_eq!(view, again);
    // 观察投影里只有 DID 文本，没有密钥。
    assert!(observed_dids(&view).iter().all(|d| d.starts_with("did:au4a:")));
    let json = observe_json(&human, &council).expect("json");
    assert_eq!(json["proposals"][0]["id"], serde_json::json!(proposal.id));

    // 观察不产生任何治理事件（没有写路径）。
    assert_eq!(council.events().len(), 2); // 选举 + 动议
}

#[test]
fn self_check_results_and_scenario_cover_the_proposal_flow() {
    let checks = au4a_council::self_check();
    assert!(au4a_core::all_passed(&checks), "self_check 未全绿: {checks:?}");
    assert!(checks.iter().any(|c| c.name == "council.proposal.agent_only"));
    assert!(checks.iter().any(|c| c.name == "council.proposals.content_addressed"));

    let results = au4a_council::results_json().expect("results");
    assert_eq!(results["proposal"]["state"], "executed");
    assert_eq!(results["election"]["reproducible"], true);

    let mut kernel = Kernel::new(KernelConfig::default());
    let scenario = au4a_council::scenario(&mut kernel).expect("scenario");
    assert_eq!(scenario["proposals"][0]["state"], "executed");
    // 到 v1.7.5 为止，scenario 已经跑了两条动议：一条执行、一条被人否决阻断。
    assert_eq!(scenario["human_view"]["proposals"], 2);
    assert_eq!(scenario["human_view"]["vetoes"], 1);
    assert_eq!(scenario["sock_elected"], 0);
}
