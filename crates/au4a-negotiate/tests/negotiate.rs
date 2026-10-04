//! v1.2.8 综合测试：把整条轨道按「正常 / 拒绝 / 不变式」三条线压实。
//!
//! 这一版不新增公共 API，只加断言。其中 `library_code_never_panics` 直接读本 crate 的源码，
//! 把 brief 的「库代码禁 unwrap/expect/panic」变成一条**会失败的测试**，而不是一句口头承诺。

use std::path::{Path, PathBuf};

use au4a_core::{AgentKeys, CoreError, Credits, EvidenceGrade, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{
    ArbitrationPolicy, Basis, BreachKind, Contract, Event, Journal, Negotiation, NegotiationMsg,
    Phase, StateMachine, Terms, Verdict, DEFAULT_MAX_ROUNDS,
};

fn kernel() -> Kernel {
    Kernel::new(KernelConfig::default())
}

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn terms(price: i64) -> Terms {
    Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
}

fn roster(k: &mut Kernel) -> (AgentKeys, AgentKeys, AgentKeys, AgentKeys) {
    let client = agent(1);
    let provider = agent(2);
    let arb1 = agent(3);
    let arb2 = agent(4);
    k.register(&client, "client", &["summarize.zh"], Credits(50)).unwrap();
    k.register(&provider, "provider", &["summarize.zh"], Credits(50)).unwrap();
    k.register(&arb1, "arbiter.1", &[], Credits(20)).unwrap();
    k.register(&arb2, "arbiter.2", &[], Credits(20)).unwrap();
    (client, provider, arb1, arb2)
}

// ---------------------------------------------------------------- 正常路径

#[test]
fn normal_path_bargain_sign_execute_settle() {
    let mut k = kernel();
    let (client, provider, _, _) = roster(&mut k);
    let mut n = Negotiation::open(&mut k, &client, &provider, terms(150), DEFAULT_MAX_ROUNDS).unwrap();
    n.counter(&mut k, &provider, &client, terms(140)).unwrap();
    n.counter(&mut k, &client, &provider, terms(130)).unwrap();
    n.reject(&mut k, &provider, &client, "135 is my limit").unwrap();
    n.counter(&mut k, &provider, &client, terms(135)).unwrap();
    n.accept(&mut k, &client, &provider).unwrap();
    let contract = n.sign_contract(&mut k, &client, &provider).unwrap();
    n.execute(&mut k, &client, &provider).unwrap();
    let paid = n.settle(&mut k, &client, &provider).unwrap();

    assert_eq!(paid, Credits(135));
    assert_eq!(n.phase(), Phase::Settled);
    assert_eq!(n.rounds_used(), 3, "三次还价；拒绝不占额度");
    assert_eq!(n.price_trail(), vec![150, 140, 130, 135]);
    assert!(contract.is_dual_signed() && contract.verify_anchor().is_ok());
    assert_eq!(contract.terms.price, Credits(135));
    assert!(k.ledger().check_conservation().is_ok());
    assert_eq!(
        k.ledger().balance(&client.did()).available,
        Credits(815),
        "1000 创世 - 50 质押 - 135 付款"
    );
    assert_eq!(k.ledger().balance(&provider.did()).available, Credits(1_085));

    // 消息链：request/counter×3/reject/accept/sign×2 = 8 条，全部可验签。
    let delivered = k.drain();
    assert_eq!(delivered.len(), 8);
    for env in &delivered {
        env.verify().unwrap();
        NegotiationMsg::from_env(env).unwrap();
    }
    assert!(!delivered.iter().any(|e| e.kind.as_str() == au4a_negotiate::kinds::CONTRACT_BREACH));
}

#[test]
fn normal_path_archive_is_byte_exact_and_replayable() {
    let mut k = kernel();
    let (client, provider, _, _) = roster(&mut k);
    let mut n = Negotiation::open(&mut k, &client, &provider, terms(100), 4).unwrap();
    n.accept(&mut k, &provider, &client).unwrap();
    n.sign_contract(&mut k, &client, &provider).unwrap();
    n.execute(&mut k, &client, &provider).unwrap();
    n.settle(&mut k, &client, &provider).unwrap();

    let bytes = n.archive().unwrap();
    let restored = Journal::decode(&bytes).unwrap();
    assert_eq!(restored.encode().unwrap(), bytes, "归档定点");
    assert_eq!(restored.replay_digest().unwrap(), n.journal().replay_digest().unwrap());
    assert_eq!(restored.phase(), Phase::Settled);
    assert_eq!(restored.machine().seq(), n.machine().seq());
    restored.machine().verify_history(restored.parties()).unwrap();
}

// ---------------------------------------------------------------- 拒绝路径

#[test]
fn refusal_matrix_every_hostile_move_is_typed_and_state_preserving() {
    let mut k = kernel();
    let (client, provider, arb1, arb2) = roster(&mut k);
    let outsider = agent(9);
    k.register(&outsider, "outsider", &[], Credits(20)).unwrap();

    let mut n = Negotiation::open(&mut k, &client, &provider, terms(100), 1).unwrap();
    let phase_before = n.phase();
    let seq_before = n.machine().seq();

    // 1) 非法转换：还没接受就签合约。
    assert_eq!(n.sign_contract(&mut k, &client, &provider), Err(CoreError::InvalidKind));
    // 2) 自问自答：报价方不能立刻自己还价。
    assert_eq!(n.counter(&mut k, &client, &provider, terms(90)), Err(CoreError::InvalidKind));
    // 3) 自己接受自己的报价。
    assert_eq!(n.accept(&mut k, &client, &provider), Err(CoreError::InvalidKind));
    // 4) 第三方插手。
    assert_eq!(n.counter(&mut k, &outsider, &client, terms(1)), Err(CoreError::UnknownAgent));
    assert_eq!(n.accept(&mut k, &outsider, &client), Err(CoreError::UnknownAgent));
    // 5) 无合约就申诉。
    assert_eq!(
        n.report_breach(&mut k, &client, BreachKind::NonDelivery, EvidenceGrade::Verified, "x"),
        Err(CoreError::InvalidKind)
    );
    // 6) 超轮数（上限 1）。
    n.counter(&mut k, &provider, &client, terms(95)).unwrap();
    assert_eq!(n.counter(&mut k, &client, &provider, terms(90)), Err(CoreError::Overflow));
    // 7) 相位与序号没有被任何一次拒绝推进。
    assert_eq!(n.phase(), phase_before);
    assert_eq!(n.machine().seq(), seq_before + 1, "只有那次合法还价推进了状态");

    // 8) 空合约 / 单签合约 / 篡改合约全部不能用。
    let mut unsigned = Contract::draft(&client, &provider.did(), &terms(100), n.session(), 1).unwrap();
    unsigned.sign(&client).unwrap();
    assert_eq!(unsigned.verify(), Err(CoreError::NotSealed));
    let mut tampered = unsigned.clone();
    tampered.terms.price = Credits(1);
    assert_eq!(tampered.verify(), Err(CoreError::InvalidSignature));

    // 9) 事后协商仍能正常收尾（拒绝没有把会话弄坏）。
    n.accept(&mut k, &client, &provider).unwrap();
    let contract = n.sign_contract(&mut k, &client, &provider).unwrap();
    n.execute(&mut k, &client, &provider).unwrap();
    n.report_breach(&mut k, &provider, BreachKind::LateDelivery, EvidenceGrade::Verified, "late")
        .unwrap();
    // 10) 当事人当仲裁员 / 单仲裁员 / 无裁决先执行。
    assert_eq!(n.open_case(&[provider.did(), arb1.did()], 1), Err(CoreError::UnknownAgent));
    assert_eq!(n.open_case(&[arb1.did()], 1), Err(CoreError::InvalidKind));
    n.open_case(&[arb1.did(), arb2.did()], 1).unwrap();
    let at = 2;
    assert_eq!(n.case_mut().unwrap().enforce(&mut k, at), Err(CoreError::NotSealed));
    assert_eq!(n.case_mut().unwrap().rule(&ArbitrationPolicy::default(), &[&arb1], Credits(100), "solo", 2), Err(CoreError::InvalidKind));

    // 拒绝记录全部是「竞争/容量」语义，没有一条被误判成恶意。
    assert!(k.refusals().iter().all(|(_, r)| !r.code.is_misconduct()));
    assert!(k.refusals().iter().any(|(_, r)| r.code == RefusalCode::Conflict));
    assert!(k.refusals().iter().any(|(_, r)| r.code == RefusalCode::PolicyDenied));
    contract.verify().unwrap();
}

#[test]
fn hostile_inputs_never_panic_the_library() {
    // 非法消息体：空会话、非 hex 哈希、超长文本、越界轮次。
    assert!(NegotiationMsg::accept("", 0, &"a".repeat(64)).is_err());
    assert!(NegotiationMsg::accept("s", 0, "NOT-HEX").is_err());
    assert!(NegotiationMsg::reject("s", 0, &"x".repeat(1_000)).is_err());
    assert!(NegotiationMsg::counter("s", u32::MAX, terms(1)).is_err());
    assert!(NegotiationMsg::breach("c", &"z".repeat(64), BreachKind::NonDelivery, "n").is_err());

    // 非法归档文本。
    for hostile in ["", "{", "[]", "null", "{\"version\":1}", "\u{0}"] {
        assert!(Journal::decode(hostile).is_err(), "{hostile:?}");
    }
    // 非法合约文本。
    for hostile in ["", "{}", "[]", "{\"hash\":\"x\"}"] {
        assert!(Contract::decode(hostile).is_err(), "{hostile:?}");
    }
    // 非法状态机输入。
    assert!(StateMachine::open("").is_err());
    assert!(StateMachine::rebuild("s", &[agent(1).did()], &[]).is_ok(), "单方名单只能在记录校验时被拒");
    assert!(au4a_negotiate::transition(Phase::Settled, Event::Request).is_err());
}

// ---------------------------------------------------------------- 不变式

#[test]
fn invariants_hold_across_the_whole_track() {
    // a) 穷举 63 种 (相位, 事件)：恰好 10 合法。
    let mut legal = 0;
    for phase in Phase::ALL {
        for event in Event::ALL {
            if au4a_negotiate::transition(phase, event).is_ok() {
                legal += 1;
            }
        }
    }
    assert_eq!(legal, 10);
    assert_eq!(au4a_negotiate::LEGAL_TRANSITIONS.len(), 10);

    // b) 正常链路：每条转换要么双方签名，要么合约条款授权。
    let mut k = kernel();
    let (client, provider, arb1, arb2) = roster(&mut k);
    let mut n = Negotiation::open(&mut k, &client, &provider, terms(100), 4).unwrap();
    n.accept(&mut k, &provider, &client).unwrap();
    n.sign_contract(&mut k, &client, &provider).unwrap();
    n.execute(&mut k, &client, &provider).unwrap();
    n.report_breach(&mut k, &client, BreachKind::NonDelivery, EvidenceGrade::Verified, "none")
        .unwrap();
    n.open_case(&[arb1.did(), arb2.did()], 1).unwrap();
    let price = n.contract().unwrap().terms.price;
    n.case_mut()
        .unwrap()
        .rule(&ArbitrationPolicy::default(), &[&arb1, &arb2], price, "proven", 2)
        .unwrap();
    let at = 3;
    n.case_mut().unwrap().enforce(&mut k, at).unwrap();
    n.resolve(&mut k, &client, &provider).unwrap();

    for record in n.machine().history() {
        match &record.basis {
            Basis::DualConsent => assert_eq!(record.sigs.len(), 2, "{} 必须双签", record.event.as_str()),
            Basis::ContractClause { contract_hash, .. } => {
                assert_eq!(record.sigs.len(), 1);
                assert_eq!(record.to, Phase::Arbitration);
                assert_eq!(contract_hash, &n.contract().unwrap().hash);
            }
        }
    }
    assert_eq!(n.phase(), Phase::Settled);

    // c) 账本守恒 + 罚没销毁。
    k.ledger().check_conservation().unwrap();
    assert_eq!(k.ledger().slashed(), Credits(20));

    // d) 归档逐字节定点 + 重放摘要一致。
    let bytes = n.archive().unwrap();
    let restored = Journal::decode(&bytes).unwrap();
    assert_eq!(restored.encode().unwrap(), bytes);
    assert_eq!(restored.replay_digest().unwrap(), n.journal().replay_digest().unwrap());

    // e) 自检全绿。
    assert!(au4a_core::all_passed(&au4a_negotiate::self_check()));
}

#[test]
fn the_same_seed_replays_the_same_bytes_twice() {
    fn run() -> String {
        let mut k = kernel();
        let (client, provider, arb1, arb2) = roster(&mut k);
        let mut n = Negotiation::open(&mut k, &client, &provider, terms(100), 4).unwrap();
        n.accept(&mut k, &provider, &client).unwrap();
        n.sign_contract(&mut k, &client, &provider).unwrap();
        n.execute(&mut k, &client, &provider).unwrap();
        n.report_breach(&mut k, &client, BreachKind::UnderDelivery, EvidenceGrade::Verified, "half")
            .unwrap();
        n.open_case(&[arb1.did(), arb2.did()], 1).unwrap();
        let price = n.contract().unwrap().terms.price;
        n.case_mut()
            .unwrap()
            .rule(&ArbitrationPolicy::default(), &[&arb1, &arb2], price, "proven", 2)
            .unwrap();
        let at = 3;
        n.case_mut().unwrap().enforce(&mut k, at).unwrap();
        let case = n.case().unwrap().summary();
        assert_eq!(case["ruling"]["verdict"], "upheld");
        assert_eq!(case["ruling"]["signatures"], 2);
        assert_eq!(n.case().unwrap().ruling().unwrap().verdict, Verdict::Upheld);
        n.archive().unwrap()
    }
    assert_eq!(run(), run(), "同种子两次完整运行必须逐字节一致");
}

// ---------------------------------------------------------------- 源码守卫

/// brief §7：库代码禁止 `unwrap()` / `expect()` / `panic!`。
///
/// 这条规则如果只写在文档里就一定会腐化，所以这里把它变成测试：读 `src/*.rs`，
/// 砍掉 `#[cfg(test)]` 之后的部分（测试里允许），再断言库里没有 panic 出口。
#[test]
fn library_code_never_panics() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0usize;
    let mut offenders: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(&src).expect("src/ 必须存在");
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .collect();
    files.sort();
    assert!(files.len() >= 7, "应当检查全部轨道模块");
    for file in files {
        let text = std::fs::read_to_string(&file).expect("源文件必须可读");
        // 模块内的 `#[cfg(test)]` 之前是库代码，之后是测试代码。
        let library_part = match text.find("#[cfg(test)]") {
            Some(idx) => &text[..idx],
            None => text.as_str(),
        };
        checked += 1;
        for (no, line) in library_part.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("///") || code.starts_with("//!") {
                continue; // 注释里提到这些词不算违规
            }
            for needle in ["unwrap(", "expect(", "panic!(", "todo!(", "unimplemented!("] {
                if line.contains(needle) {
                    offenders.push(format!(
                        "{}:{} 含 `{needle}`",
                        file.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                        no + 1
                    ));
                }
            }
        }
    }
    assert!(checked >= 7, "检查了 {checked} 个模块");
    assert!(offenders.is_empty(), "库代码出现 panic 出口：{offenders:?}");
}

#[test]
fn the_crate_exports_the_frozen_track_surface() {
    assert_eq!(au4a_negotiate::TRACK, "1.2");
    assert_eq!(au4a_negotiate::RANGE, "v1.2.1 → v1.2.10");
    assert_eq!(au4a_negotiate::TITLE, "Negotiation 协商协议");
    let results = au4a_negotiate::results_json().unwrap();
    assert_eq!(results["kinds"].as_array().unwrap().len(), 6);
    assert_eq!(results["legal_transitions"], 10);
    assert_eq!(results["journal_version"], 1);
    assert_eq!(
        results["checks"],
        results["checks_passed"],
        "所有自检项都必须通过"
    );
    // 来源路径正常（防止被当作别的 crate 编译进来）。
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(manifest.ends_with("au4a-negotiate"));
}
