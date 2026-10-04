//! v1.7.9 示例版——紧急通道 + 完整 scenario 的集成测试。
//!
//! 断言安全委员会紧急通道的三条规则：**只有安全委员会**能下发、**即时生效**、
//! **事后必须由治理确认**（否决或超期则回滚策略）；以及示例本身真的能跑通。

use au4a_core::{AgentKeys, CoreError, Credits, Did, RefusalCode};
use au4a_council::{
    Action, AgentIdentity, CommitteeKind, Council, CouncilConfig, ElectionBallot, ElectionConfig,
    EmergencyApproval, EmergencyStatus, HumanObserver, ProposalDraft, Vote,
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
    let everyone: Vec<usize> = (0..n as usize).collect();
    scene_with(n, n as usize, &everyone)
}

/// `seats` 决定委员会规模，`security_picks` 决定安全委员会里被投票的候选人。
/// 由于选举会填满席位，安全委员会要真正「只包含某些人」，必须同时缩小席位。
fn scene_with(n: u8, seats: usize, security_picks: &[usize]) -> Scene {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut council = Council::new(CouncilConfig {
        election: ElectionConfig { seats, ..ElectionConfig::default() },
        ..CouncilConfig::default()
    });
    let agents: Vec<AgentKeys> = (0..n).map(keys).collect();
    for k in &agents {
        kernel.register(k, "t", &["governance.vote"], Credits(20)).expect("register");
        council.note_reputation(&k.did(), 5_000);
        council.note_uptime(&k.did(), 300);
    }
    let everyone: Vec<Did> = agents.iter().map(|k| k.did()).collect();
    for kind in CommitteeKind::ALL {
        let picks: Vec<Did> = if kind == CommitteeKind::Security {
            security_picks.iter().map(|i| agents[*i].did()).collect()
        } else {
            everyone.clone()
        };
        let ballots: Vec<ElectionBallot> = agents
            .iter()
            .map(|k| ElectionBallot::cast(k, kind, &picks).expect("cast"))
            .collect();
        council.elect(&mut kernel, kind, &ballots).expect("elect");
    }
    Scene { kernel, council, agents }
}

impl Scene {
    fn identity(&self, i: usize) -> AgentIdentity {
        AgentIdentity::from_keys(&self.agents[i])
    }
}

#[test]
fn only_the_security_committee_holds_the_emergency_channel() {
    // 安全委员会只有第 2、3 号 Agent（2 个席位、只被这两人拉到票）；第 0 号不是安全委员会成员。
    let mut s = scene_with(4, 2, &[2, 3]);
    let outsider_of_security = s.identity(0);
    assert!(!s
        .council
        .committee(CommitteeKind::Security)
        .expect("security")
        .has_member(&s.agents[0].did()));
    // 非安全委员会成员尝试走紧急通道 → unauthorized（单次即恶意）。
    assert_eq!(
        s.council.issue_emergency(&mut s.kernel, &outsider_of_security, "freeze", 1, "紧急"),
        Err(CoreError::InvalidSignature)
    );
    assert_eq!(
        s.kernel.refusals().last().map(|(_, r)| r.code),
        Some(RefusalCode::Unauthorized)
    );
    assert!(s.council.policy("freeze").is_none());
    assert_eq!(s.council.emergency_directives().count(), 0);

    // 安全委员会成员可以下发。
    let security_identity = s.identity(2);
    let directive = s
        .council
        .issue_emergency(&mut s.kernel, &security_identity, "freeze", 1, "检测到异常")
        .expect("security member may issue");
    assert_eq!(directive.committee, CommitteeKind::Security);
    assert_eq!(directive.status, EmergencyStatus::Active);
    assert!(s.council.policy("freeze") == Some(1));
}

#[test]
fn an_emergency_directive_takes_effect_immediately_but_must_be_confirmed() {
    let mut s = scene(4);
    let issuer = s.identity(0);
    let directive = s
        .council
        .issue_emergency(&mut s.kernel, &issuer, "freeze", 1, "异常结算速率")
        .expect("issue");
    // 即时生效：还没有任何确认票，策略已经变了。
    assert_eq!(directive.status, EmergencyStatus::Active);
    assert_eq!(s.council.policy("freeze"), Some(1));
    assert_eq!(directive.previous_value, None);
    assert!(directive.awaiting_confirmation());

    // 票数不足：保持 active，不回滚。
    let one = vec![EmergencyApproval::cast(&s.agents[0], &directive.id, true).expect("cast")];
    let pending = s
        .council
        .confirm_emergency(&mut s.kernel, &directive.id, &one)
        .expect("confirm");
    assert_eq!(pending.status, EmergencyStatus::Active);
    assert_eq!(s.council.policy("freeze"), Some(1));
    assert_eq!(pending.confirmation.as_ref().map(|c| c.quorum), Some(3)); // n=4 → f=1 → 3

    // 达到法定人数：确认，策略保留。
    let approvals: Vec<EmergencyApproval> = s
        .agents
        .iter()
        .map(|k| EmergencyApproval::cast(k, &directive.id, true).expect("cast"))
        .collect();
    let confirmed = s
        .council
        .confirm_emergency(&mut s.kernel, &directive.id, &approvals)
        .expect("confirm");
    assert_eq!(confirmed.status, EmergencyStatus::Confirmed);
    assert!(!confirmed.confirmation.as_ref().map(|c| c.rolled_back).unwrap_or(true));
    assert_eq!(s.council.policy("freeze"), Some(1));
    // 不变式必须在**有真实紧急指令**的状态下也全绿（否则这些检查只是空跑）。
    let checks = au4a_council::invariants::check_all(&s.council);
    assert!(
        au4a_core::all_passed(&checks),
        "有紧急指令时不变式未全绿：{checks:?}"
    );
    assert!(checks
        .iter()
        .any(|c| c.name == "council.emergency.security_only" && c.detail.contains('1')));
    // 已终结的指令不能再确认。
    assert_eq!(
        s.council.confirm_emergency(&mut s.kernel, &directive.id, &approvals),
        Err(CoreError::InvalidKind)
    );
}

#[test]
fn a_rejected_emergency_directive_rolls_the_policy_back() {
    let mut s = scene(4);
    let issuer = s.identity(0);
    // 先有一条常规策略值，再让紧急指令覆盖它。
    let draft = ProposalDraft::by(
        &s.agents[0],
        CommitteeKind::Task,
        "常规设定",
        Action::SetPolicy { key: String::from("limit"), value: 10 },
    )
    .expect("draft");
    let identity = s.identity(0);
    let proposal = s
        .council
        .propose(&mut s.kernel, &identity, draft)
        .expect("propose");
    let round = s.council.open_round(&mut s.kernel, &proposal.id).expect("open");
    let mut state = round.clone();
    for i in 0..3 {
        let vote = Vote::cast(&s.agents[i], &proposal.id, round.round, au4a_council::Choice::Yes)
            .expect("cast");
        state = s.council.cast_vote(&mut s.kernel, vote).expect("vote");
    }
    assert_eq!(state.outcome, au4a_council::RoundOutcome::Passed);
    s.council.execute(&mut s.kernel, &identity, &proposal.id).expect("execute");
    assert_eq!(s.council.policy("limit"), Some(10));

    // 紧急指令把 limit 改成 999（即时生效，previous_value = 10）。
    let directive = s
        .council
        .issue_emergency(&mut s.kernel, &issuer, "limit", 999, "临时冻结上限")
        .expect("issue");
    assert_eq!(directive.previous_value, Some(10));
    assert_eq!(s.council.policy("limit"), Some(999));

    // 治理否决（3 票 = 法定人数）→ 策略回滚到 10。
    let approvals: Vec<EmergencyApproval> = s
        .agents
        .iter()
        .take(3)
        .map(|k| EmergencyApproval::cast(k, &directive.id, false).expect("cast"))
        .collect();
    let rejected = s
        .council
        .confirm_emergency(&mut s.kernel, &directive.id, &approvals)
        .expect("confirm");
    assert_eq!(rejected.status, EmergencyStatus::Rejected);
    assert_eq!(rejected.confirmation.as_ref().map(|c| c.rolled_back), Some(true));
    assert_eq!(s.council.policy("limit"), Some(10), "否决必须回滚");
    assert!(au4a_core::all_passed(&au4a_council::invariants::check_all(&s.council)));
}

#[test]
fn an_expired_directive_rolls_back_and_is_marked_expired() {
    let mut s = scene(4);
    let issuer = s.identity(0);
    let directive = s
        .council
        .issue_emergency(&mut s.kernel, &issuer, "temp", 7, "临时措施")
        .expect("issue");
    assert_eq!(s.council.policy("temp"), Some(7));
    // 推进逻辑时钟越过确认窗口（逻辑时钟是唯一时间来源）。
    let deadline = directive.confirm_deadline;
    while s.council.now() <= deadline {
        s.council.tick();
    }
    let expired = s
        .council
        .confirm_emergency(&mut s.kernel, &directive.id, &[])
        .expect("confirm");
    assert_eq!(expired.status, EmergencyStatus::Expired);
    assert_eq!(expired.confirmation.as_ref().map(|c| c.rolled_back), Some(true));
    assert_eq!(s.council.policy("temp"), None, "超期必须回滚（此前没有这条策略）");
}

#[test]
fn emergency_approvals_are_signed_and_deduplicated() {
    let mut s = scene(4);
    let issuer = s.identity(0);
    let directive = s
        .council
        .issue_emergency(&mut s.kernel, &issuer, "dup", 1, "重复票测试")
        .expect("issue");

    // 篡改签名的确认票被拒。
    let mut forged = EmergencyApproval::cast(&s.agents[1], &directive.id, true).expect("cast");
    forged.approve = false;
    assert_eq!(
        s.council.confirm_emergency(&mut s.kernel, &directive.id, &[forged]),
        Err(CoreError::InvalidSignature)
    );

    // 非委员的确认票被拒。
    let outsider = keys(90);
    s.kernel
        .register(&outsider, "outsider", &["governance.vote"], Credits(20))
        .expect("register");
    let foreign = EmergencyApproval::cast(&outsider, &directive.id, true).expect("cast");
    assert_eq!(
        s.council.confirm_emergency(&mut s.kernel, &directive.id, &[foreign]),
        Err(CoreError::InvalidSignature)
    );

    // 同一委员两次确认 → 拒绝。
    let twice = vec![
        EmergencyApproval::cast(&s.agents[0], &directive.id, true).expect("cast"),
        EmergencyApproval::cast(&s.agents[1], &directive.id, true).expect("cast"),
        EmergencyApproval::cast(&s.agents[0], &directive.id, true).expect("cast"),
    ];
    assert_eq!(
        s.council.confirm_emergency(&mut s.kernel, &directive.id, &twice),
        Err(CoreError::DuplicateAgent)
    );
    assert_eq!(s.council.emergency(&directive.id).map(|d| d.status), Some(EmergencyStatus::Active));
}

#[test]
fn the_scenario_runs_the_full_flow_including_emergency() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let scenario = au4a_council::scenario(&mut kernel).expect("scenario");

    // 选举：空壳刷票无效。
    assert_eq!(scenario["sock_elected"], 0);
    assert!(scenario["sock_ballots_ignored"].as_u64().unwrap_or(0) > 0);
    assert_eq!(scenario["elections"], 5);

    // 表决：第一条动议的第一轮被双签作废，重开后通过；第二条动议另算一轮 → 共 3 轮。
    assert_eq!(scenario["ambiguity"]["outcome"], "void_ambiguous");
    assert_eq!(scenario["ambiguity"]["second_vote_rejected"], true);
    assert_eq!(scenario["ambiguity"]["rounds_total"], 3);
    assert_eq!(scenario["voting"]["outcome"], "passed");

    // 执行：策略落成。
    assert_eq!(scenario["execution"]["policies"]["cpu_proto_settle_cap"], 250);
    assert_eq!(scenario["execution"]["conservation_ok"], true);

    // 人类否决：只能阻断。
    assert_eq!(scenario["veto"]["state"], "blocked");
    assert_eq!(scenario["veto"]["execute_after_veto_refused"], true);
    assert_eq!(scenario["veto"]["action_unchanged"], true);

    // 紧急通道：即时生效 + 事后确认。
    assert_eq!(scenario["emergency"]["applied_immediately"], true);
    assert_eq!(scenario["emergency"]["confirmation"]["status"], "confirmed");
    assert_eq!(scenario["emergency"]["confirmation"]["rolled_back"], false);
    assert_eq!(scenario["emergency"]["policy_kept"], true);
    assert!(scenario["emergency"]["confirmation"]["approvals"].as_u64().unwrap_or(0)
        >= scenario["emergency"]["confirmation"]["quorum"].as_u64().unwrap_or(u64::MAX));

    // 链上映射：两条动议分别 executed / canceled。
    let tokens = scenario["governor_tokens"].as_array().expect("tokens");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0]["state"], "executed");
    assert_eq!(tokens[1]["state"], "canceled");

    // 人类只观察：投影里有动议与否决，且观察不改变状态。
    assert_eq!(scenario["human_view"]["proposals"], 2);
    assert_eq!(scenario["human_view"]["vetoes"], 1);
    let human = HumanObserver::new("operator");
    let _ = human; // 观察者类型依旧只有 observe/veto（compile_fail 文档测试把守）
}

#[test]
fn the_example_source_is_a_real_runnable_program() {
    // 示例必须真的存在且不依赖任何外部资源（只读源码做结构断言，编译由 cargo test 保证）。
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("governance_demo.rs");
    let source = std::fs::read_to_string(&path).expect("示例源码必须存在");
    assert!(source.contains("fn main() -> Result<(), CoreError>"));
    assert!(source.contains("au4a_council::scenario"));
    assert!(source.contains("au4a_council::self_check"));
    assert!(!source.contains("std::fs::read"), "示例不得读文件");
    assert!(!source.contains("TcpStream"), "示例不得开网络");
    assert!(!source.contains("SystemTime"), "示例不得读墙钟");
}
