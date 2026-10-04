//! v1.7.5 否决权——集成测试。
//!
//! 这是全项目「人类只观察」的核心证据所在：人类唯一的写能力是**否决**，
//! 而否决在类型与状态机两层都被限制为「只能阻断」。编译期证据由
//! `veto::Veto` / `human::HumanObserver` 的 `compile_fail` 文档测试把守。

use au4a_core::{AgentKeys, CoreError, Credits, Did, RefusalCode};
use au4a_council::veto::{label_has_no_did, observer_is_not_an_agent, vetoable};
use au4a_council::{
    Action, AgentIdentity, CommitteeKind, Council, CouncilConfig, ElectionBallot, HumanObserver,
    ProposalDraft, ProposalState,
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
        kernel.register(k, "t", &["governance.vote"], Credits(20)).expect("register");
        council.note_reputation(&k.did(), 5_000);
        council.note_uptime(&k.did(), 300);
    }
    let picks: Vec<Did> = agents.iter().map(|k| k.did()).collect();
    let ballots: Vec<ElectionBallot> = agents
        .iter()
        .map(|k| ElectionBallot::cast(k, kind, &picks).expect("cast"))
        .collect();
    council.elect(&mut kernel, kind, &ballots).expect("elect");
    Scene { kernel, council, agents, kind }
}

impl Scene {
    fn pass(&mut self, title: &str, action: Action) -> String {
        let author = AgentIdentity::from_keys(&self.agents[0]);
        let draft = ProposalDraft::by(&self.agents[0], self.kind, title, action).expect("draft");
        let proposal = self.council.propose(&mut self.kernel, &author, draft).expect("propose");
        let round = self.council.open_round(&mut self.kernel, &proposal.id).expect("open");
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
            let k = self.agents.iter().find(|k| &k.did() == did).expect("member");
            let vote =
                au4a_council::Vote::cast(k, &proposal.id, state.round, au4a_council::Choice::Yes)
                    .expect("cast");
            state = self.council.cast_vote(&mut self.kernel, vote).expect("vote");
        }
        assert_eq!(state.outcome, au4a_council::RoundOutcome::Passed);
        proposal.id
    }

    fn identity(&self, i: usize) -> AgentIdentity {
        AgentIdentity::from_keys(&self.agents[i])
    }
}

#[test]
fn a_human_veto_blocks_a_passed_motion_and_nothing_else_changes() {
    let mut s = scene(4, CommitteeKind::Task);
    let id = s.pass("提高准入门槛", Action::SetPolicy { key: String::from("bar"), value: 400 });
    assert_eq!(s.council.proposal(&id).expect("p").state, ProposalState::Passed);

    let human = HumanObserver::new("operator-1");
    let before_proposals = s.council.proposals().len();
    let action_before = s.council.proposal(&id).expect("p").action.clone();
    assert!(vetoable(&s.council, &id));

    let veto = human.veto(&s.council, &id, "会挡住长期贡献者").expect("veto");
    assert_eq!(veto.target(), id);
    assert_eq!(veto.observer(), "operator-1");
    assert_eq!(veto.reason(), "会挡住长期贡献者");
    assert!(observer_is_not_an_agent(&veto));
    assert!(label_has_no_did(veto.observer()));
    // 否决载荷里没有任何「可执行内容」字段。
    let payload = veto.payload();
    for forbidden in ["action", "patch", "alternative", "payload", "value", "sig"] {
        assert!(payload.get(forbidden).is_none(), "Veto 不应带 {forbidden}");
    }

    let receipt = s.council.apply_veto(&mut s.kernel, &veto).expect("apply_veto");
    assert_eq!(receipt.state, ProposalState::Blocked);
    assert_eq!(receipt.previous_state, ProposalState::Passed);
    assert_eq!(receipt.reason, "会挡住长期贡献者");

    let after = s.council.proposal(&id).expect("p");
    assert_eq!(after.state, ProposalState::Blocked);
    assert_eq!(after.action, action_before, "决议内容不得被改写");
    assert_eq!(s.council.proposals().len(), before_proposals, "人类没有新增动议");
    assert_eq!(s.council.veto_record(&id).map(|v| v.reason()), Some("会挡住长期贡献者"));
    assert_eq!(s.council.policy("bar"), None, "被阻断的动议不得产生任何状态变更");
}

#[test]
fn a_blocked_motion_can_never_be_executed_or_revived() {
    let mut s = scene(4, CommitteeKind::Resource);
    let id = s.pass("设定参数", Action::SetPolicy { key: String::from("p"), value: 1 });
    let human = HumanObserver::new("operator");
    let veto = human.veto(&s.council, &id, "不安全").expect("veto");
    s.council.apply_veto(&mut s.kernel, &veto).expect("apply");

    // 1) 执行被拒（只能阻断：阻断之后就没有任何后续动作）。
    let executor = s.identity(1);
    assert_eq!(
        s.council.execute(&mut s.kernel, &executor, &id),
        Err(CoreError::InvalidKind)
    );
    assert!(s.council.execution(&id).is_none());
    assert_eq!(s.council.policy("p"), None);
    // 2) 不能复活成 passed / executed。
    assert_eq!(
        s.council.transition_to(&mut s.kernel, &id, ProposalState::Passed),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        s.council.transition_to(&mut s.kernel, &id, ProposalState::Executed),
        Err(CoreError::InvalidKind)
    );
    // 3) 不能重新开一轮表决。
    assert_eq!(
        s.council.open_round(&mut s.kernel, &id),
        Err(CoreError::InvalidKind)
    );
    // 4) 也不能对同一条动议再否决一次。
    assert!(!vetoable(&s.council, &id));
    assert!(human.veto(&s.council, &id, "再来一次").is_err());
    assert_eq!(
        s.council.apply_veto(&mut s.kernel, &veto),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(s.council.vetoes().count(), 1);
    assert!(s.kernel.refusals().iter().any(|(_, r)| r.code == RefusalCode::Conflict));
}

#[test]
fn a_veto_must_carry_a_public_reason() {
    let mut s = scene(4, CommitteeKind::Task);
    let id = s.pass("设定参数", Action::SetPolicy { key: String::from("r"), value: 1 });
    let human = HumanObserver::new("operator");
    // 空理由在铸造阶段就被拒。
    assert_eq!(
        human.veto(&s.council, &id, "   "),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(human.veto(&s.council, &id, ""), Err(CoreError::InvalidKind));
    // 否决理由进入治理事件与只读投影。
    let veto = human.veto(&s.council, &id, "公开理由：影响公平").expect("veto");
    s.council.apply_veto(&mut s.kernel, &veto).expect("apply");
    let event = s
        .council
        .events()
        .iter()
        .find(|e| e.kind == "veto.blocked")
        .expect("event");
    assert!(event.detail.contains("公开理由：影响公平"));
    assert!(event.detail.contains("operator"));
    let view = human.observe(&s.council);
    assert_eq!(view.vetoes.len(), 1);
    assert_eq!(view.vetoes[0].reason, "公开理由：影响公平");
    assert_eq!(view.vetoes[0].proposal, id);
}

#[test]
fn a_veto_on_an_open_motion_also_only_blocks() {
    let mut s = scene(4, CommitteeKind::Task);
    let author = s.identity(0);
    let draft = ProposalDraft::by(
        &s.agents[0],
        CommitteeKind::Task,
        "还没表决就被否决",
        Action::SetPolicy { key: String::from("open"), value: 9 },
    )
    .expect("draft");
    let proposal = s.council.propose(&mut s.kernel, &author, draft).expect("propose");
    let human = HumanObserver::new("operator");
    let veto = human.veto(&s.council, &proposal.id, "紧急叫停").expect("veto");
    let receipt = s.council.apply_veto(&mut s.kernel, &veto).expect("apply");
    assert_eq!(receipt.previous_state, ProposalState::Open);
    assert_eq!(receipt.state, ProposalState::Blocked);
    assert!(!vetoable(&s.council, &proposal.id));
}

#[test]
fn an_executed_motion_cannot_be_vetoed_retroactively() {
    let mut s = scene(4, CommitteeKind::Task);
    let id = s.pass("已经执行的动议", Action::SetPolicy { key: String::from("done"), value: 3 });
    let executor = s.identity(1);
    s.council.execute(&mut s.kernel, &executor, &id).expect("execute");
    let human = HumanObserver::new("operator");
    // 已经执行的动议不再可否决（否决只能阻断，不能回滚）。
    assert!(human.veto(&s.council, &id, "事后反对").is_err());
    assert_eq!(s.council.proposal(&id).expect("p").state, ProposalState::Executed);
    assert_eq!(s.council.policy("done"), Some(3));
    assert_eq!(s.council.vetoes().count(), 0);
}

#[test]
fn vetoing_an_unknown_proposal_is_refused() {
    let s = scene(4, CommitteeKind::Task);
    let human = HumanObserver::new("operator");
    assert_eq!(
        human.veto(&s.council, "no-such-id", "理由"),
        Err(CoreError::UnknownAgent)
    );
    assert_eq!(s.council.vetoes().count(), 0);
}

#[test]
fn the_agent_side_has_no_veto_path() {
    let mut s = scene(4, CommitteeKind::Task);
    let id = s.pass("正常动议", Action::SetPolicy { key: String::from("ok"), value: 1 });
    // 只有 HumanObserver 能铸造 Veto；Agent 侧类型（AgentIdentity）没有 veto 方法，
    // 这一点由 veto.rs / human.rs 的 compile_fail 文档测试把守。
    let identity = s.identity(0);
    assert_eq!(identity.did(), &s.agents[0].did());
    // 委员本人执行是允许的（这是 Agent 的路径，不是人类的）。
    let receipt = s.council.execute(&mut s.kernel, &identity, &id).expect("execute");
    assert_eq!(receipt.effects.len(), 1);
    // Agent 试图用「否决」以外的方式回滚执行结果：没有这样的 API（状态机终态）。
    assert_eq!(
        s.council.transition_to(&mut s.kernel, &id, ProposalState::Blocked),
        Err(CoreError::InvalidKind)
    );
    assert!(s.council.vetoes().count() == 0);
}

#[test]
fn self_check_results_and_scenario_carry_the_veto_evidence() {
    let checks = au4a_council::self_check();
    assert!(au4a_core::all_passed(&checks), "self_check 未全绿: {checks:?}");
    for name in [
        "council.veto.blocks_only",
        "council.veto.read_only",
        "council.vetoes.blocks_only",
        "council.vetoes.reasons_public",
    ] {
        assert!(checks.iter().any(|c| c.name == name), "缺少自检项 {name}");
    }

    let results = au4a_council::results_json().expect("results");
    assert_eq!(results["veto"]["state"], "blocked");
    assert_eq!(results["veto"]["execute_after_veto_refused"], true);
    assert_eq!(results["veto"]["proposals_unchanged"], true);
    assert_eq!(results["veto"]["action_unchanged"], true);
    assert!(results["veto"]["reason"].as_str().unwrap_or_default().len() > 4);

    let mut kernel = Kernel::new(KernelConfig::default());
    let scenario = au4a_council::scenario(&mut kernel).expect("scenario");
    assert_eq!(scenario["veto"]["state"], "blocked");
    assert_eq!(scenario["veto"]["execute_after_veto_refused"], true);
    assert_eq!(scenario["veto"]["proposals_unchanged"], true);
    assert_eq!(scenario["veto"]["action_unchanged"], true);
    assert_eq!(scenario["second_motion"]["previous_state"], "passed");
    assert_eq!(scenario["second_motion"]["state_after_veto"], "blocked");
    assert_eq!(scenario["human_view"]["vetoes"], 1);
    assert_eq!(scenario["human_view"]["proposals"], 2);
}
