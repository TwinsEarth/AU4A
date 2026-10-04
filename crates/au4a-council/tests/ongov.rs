//! v1.7.6 链上治理映射——集成测试。
//!
//! 断言的是「语义映射」：治理状态 → `GovernorToken` 状态、票数、整数法定人数分数、
//! 否决/执行凭证，以及**两件不能含糊的事**：`real_chain = false`、证据等级 `cpu-proto`。

use au4a_core::{AgentKeys, Credits, Did, EvidenceGrade};
use au4a_council::ongov::{no_false_chain_claims, project_all, state_summary};
use au4a_council::{
    Action, AgentIdentity, ChainBinding, Choice, CommitteeKind, Council, CouncilConfig,
    ElectionBallot, ElectionConfig, GovernorToken, GovState, HumanObserver, ProposalDraft, Vote,
};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

struct Scene {
    kernel: Kernel,
    council: Council,
    agents: Vec<AgentKeys>,
}

fn scene(n: u8) -> Scene {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut council = Council::new(CouncilConfig {
        election: ElectionConfig { seats: n as usize, ..ElectionConfig::default() },
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
        .map(|k| ElectionBallot::cast(k, CommitteeKind::Evolution, &picks).expect("cast"))
        .collect();
    council.elect(&mut kernel, CommitteeKind::Evolution, &ballots).expect("elect");
    Scene { kernel, council, agents }
}

impl Scene {
    fn propose(&mut self, title: &str, key: &str) -> String {
        let identity = AgentIdentity::from_keys(&self.agents[0]);
        let draft = ProposalDraft::by(
            &self.agents[0],
            CommitteeKind::Evolution,
            title,
            Action::SetPolicy { key: key.to_string(), value: 42 },
        )
        .expect("draft");
        self.council
            .propose(&mut self.kernel, &identity, draft)
            .expect("propose")
            .id
    }

    fn vote_all(&mut self, id: &str, choice: Choice) -> u32 {
        let round = self.council.open_round(&mut self.kernel, id).expect("open");
        let members = self
            .council
            .committee(CommitteeKind::Evolution)
            .map(|c| c.member_dids())
            .unwrap_or_default();
        let mut state = round;
        for did in &members {
            if state.outcome.is_closed() {
                break;
            }
            let k = self.agents.iter().find(|k| &k.did() == did).expect("member");
            let vote = Vote::cast(k, id, state.round, choice).expect("cast");
            state = self.council.cast_vote(&mut self.kernel, vote).expect("vote");
        }
        state.round
    }
}

#[test]
fn the_mapping_covers_the_whole_lifecycle() {
    let mut s = scene(4);
    let id = s.propose("链上映射生命周期", "lifecycle");

    // 1) 已提交、未开轮 → pending
    let token = GovernorToken::project(&s.council, &id).expect("project");
    assert_eq!(token.state, GovState::Pending);
    assert_eq!(token.for_votes + token.against_votes + token.abstain_votes, 0);
    assert_eq!(token.quorum_fraction(), (3, 4));

    // 2) 开轮 → active
    let round = s.council.open_round(&mut s.kernel, &id).expect("open");
    let token = GovernorToken::project(&s.council, &id).expect("project");
    assert_eq!(token.state, GovState::Active);

    // 3) 部分票 → 仍 active，但票数实时反映
    let v = Vote::cast(&s.agents[0], &id, round.round, Choice::Yes).expect("cast");
    s.council.cast_vote(&mut s.kernel, v).expect("vote");
    let token = GovernorToken::project(&s.council, &id).expect("project");
    assert_eq!(token.state, GovState::Active);
    assert_eq!(token.for_votes, 1);

    // 4) 达到法定人数 → succeeded
    for i in 1..3 {
        let v = Vote::cast(&s.agents[i], &id, round.round, Choice::Yes).expect("cast");
        s.council.cast_vote(&mut s.kernel, v).expect("vote");
    }
    let token = GovernorToken::project(&s.council, &id).expect("project");
    assert_eq!(token.state, GovState::Succeeded);
    assert_eq!((token.for_votes, token.against_votes, token.abstain_votes), (3, 0, 0));

    // 5) 执行 → executed，但没有 tx_ref（不是真实链上）
    let identity = AgentIdentity::from_keys(&s.agents[0]);
    s.council.execute(&mut s.kernel, &identity, &id).expect("execute");
    let token = GovernorToken::project(&s.council, &id).expect("project");
    assert_eq!(token.state, GovState::Executed);
    let execution = token.execution.as_ref().expect("execution");
    assert_eq!(execution.tx_ref, None);
    assert!(execution.effects[0].contains("lifecycle=42"));
    assert!(!token.claims_real_chain());
}

#[test]
fn defeated_and_canceled_are_distinguishable() {
    let mut s = scene(4);
    // 否决路径 → canceled（带理由）
    let blocked = s.propose("会被人类阻断", "canceled");
    s.vote_all(&blocked, Choice::Yes);
    let human = HumanObserver::new("operator-2");
    let veto = human.veto(&s.council, &blocked, "影响长期公平").expect("veto");
    s.council.apply_veto(&mut s.kernel, &veto).expect("apply");
    let token = GovernorToken::project(&s.council, &blocked).expect("project");
    assert_eq!(token.state, GovState::Canceled);
    assert_eq!(token.veto.as_ref().map(|v| v.reason.as_str()), Some("影响长期公平"));
    assert_eq!(token.veto.as_ref().map(|v| v.observer.as_str()), Some("operator-2"));
    assert!(token.execution.is_none());

    // 表决否决路径 → defeated（没有否决记录）
    let rejected = s.propose("会被表决否决", "defeated");
    s.vote_all(&rejected, Choice::No);
    let token = GovernorToken::project(&s.council, &rejected).expect("project");
    assert_eq!(token.state, GovState::Defeated);
    assert!(token.veto.is_none());
    assert_eq!(token.against_votes, 3);
}

#[test]
fn voided_rounds_keep_the_motion_active() {
    let mut s = scene(4);
    let id = s.propose("双签作废后仍需重投", "voided");
    let round = s.council.open_round(&mut s.kernel, &id).expect("open");
    // 先投赞成，再改投反对 → 整轮作废。
    let first = Vote::cast(&s.agents[0], &id, round.round, Choice::Yes).expect("cast");
    s.council.cast_vote(&mut s.kernel, first).expect("vote");
    let flip = Vote::cast(&s.agents[0], &id, round.round, Choice::No).expect("cast");
    assert!(s.council.cast_vote(&mut s.kernel, flip).is_err());
    assert_eq!(
        s.council.round(&id, round.round).map(|r| r.outcome),
        Some(au4a_council::RoundOutcome::VoidAmbiguous)
    );
    // 作废轮不产生链上结论：动议仍是 active。
    let token = GovernorToken::project(&s.council, &id).expect("project");
    assert_eq!(token.state, GovState::Active);
}

#[test]
fn the_binding_never_claims_a_real_chain() {
    let mut s = scene(4);
    let id = s.propose("诚实标注", "honest");
    s.vote_all(&id, Choice::Yes);
    let token = GovernorToken::project(&s.council, &id).expect("project");
    assert_eq!(token.binding.chain, "au4a-local");
    assert_eq!(token.binding.grade, "cpu-proto");
    assert_eq!(token.evidence(), EvidenceGrade::CpuProto);
    assert!(!token.binding.real_chain);
    assert!(!token.claims_real_chain());
    assert!(no_false_chain_claims(&[token]));

    // 即便有人手工构造一个绑定，`claims_real_chain` 也会如实反映。
    let fake = ChainBinding {
        chain: String::from("ethereum"),
        grade: String::from("verified"),
        real_chain: true,
        note: String::from("假装上链"),
    };
    let mut forged = GovernorToken::project(&s.council, &id).expect("project");
    forged.binding = fake;
    assert!(forged.claims_real_chain());
    assert!(!no_false_chain_claims(&[forged]));
}

#[test]
fn projection_is_deterministic_and_serializable() {
    let mut s = scene(4);
    let a = s.propose("动议甲", "alpha");
    let b = s.propose("动议乙", "beta");
    s.vote_all(&a, Choice::Yes);

    let first = project_all(&s.council).expect("all");
    let second = project_all(&s.council).expect("all");
    assert_eq!(first, second);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].proposal, a);
    assert_eq!(first[1].proposal, b);
    assert!(first[0].id != first[1].id);

    // JSON 往返：投影可被序列化并在链下适配器里读回。
    let text = serde_json::to_string(&first[0]).expect("ser");
    let back: GovernorToken = serde_json::from_str(&text).expect("de");
    assert_eq!(back, first[0]);
    let summary = state_summary(&first);
    assert_eq!(summary["succeeded"], 1);
    assert_eq!(summary["pending"], 1);
    assert!(first[0].to_json()["binding"]["real_chain"] == serde_json::json!(false));
}

#[test]
fn self_check_results_and_scenario_expose_the_mapping() {
    let checks = au4a_council::self_check();
    assert!(au4a_core::all_passed(&checks), "self_check 未全绿: {checks:?}");
    for name in [
        "council.ongov.mapping",
        "council.ongov.no_false_chain",
    ] {
        assert!(checks.iter().any(|c| c.name == name), "缺少自检项 {name}");
    }

    let results = au4a_council::results_json().expect("results");
    assert_eq!(results["ongov"]["real_chain"], false);
    assert_eq!(results["ongov"]["grade"], "cpu-proto");
    assert_eq!(results["ongov"]["states"]["executed"], 1);
    assert_eq!(results["ongov"]["vetoed_states"]["canceled"], 1);

    let mut kernel = Kernel::new(KernelConfig::default());
    let scenario = au4a_council::scenario(&mut kernel).expect("scenario");
    let tokens = scenario["governor_tokens"].as_array().expect("tokens");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0]["state"], "executed");
    assert_eq!(tokens[1]["state"], "canceled");
    for token in tokens {
        assert_eq!(token["binding"]["real_chain"], false);
        assert_eq!(token["binding"]["grade"], "cpu-proto");
        assert!(token["execution"]["tx_ref"].is_null() || token["execution"].is_null());
    }
    assert_eq!(tokens[0]["quorum_numerator"], 4);
    assert_eq!(tokens[0]["quorum_denominator"], 5);
}
