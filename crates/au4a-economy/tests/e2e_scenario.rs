//! v1.4.7 端到端测试：在**共享内核**上跑完整经济流程，并检查 JSON 契约。
//!
//! 覆盖：注册 → 自主定价 → 签名广播 → 发现 → 支付 → 兑换路由 → 质押 → 仲裁 → 结算 → 收益。
//! 另外验证两件事：伪造报价会被内核记为恶意证据；`Unverified` 证据永远结算不了。

use au4a_core::{kinds, AgentKeys, Credits, Envelope, EvidenceGrade, RefusalCode};
use au4a_economy::settlement::{
    route as settlement_route, SettlementPolicy, SettlementRequest, SettlementRoute,
};
use au4a_kernel::{Kernel, KernelConfig};
use serde_json::{json, Value};

fn fresh_kernel() -> Kernel {
    Kernel::new(KernelConfig::default())
}

fn run_scenario() -> (Kernel, Value) {
    let mut kernel = fresh_kernel();
    let value = au4a_economy::scenario(&mut kernel).expect("场景必须成功");
    (kernel, value)
}

#[test]
fn the_scenario_contract_has_every_section_and_the_pinned_numbers() {
    let (_kernel, value) = run_scenario();
    for section in [
        "track",
        "title",
        "range",
        "agents",
        "pricing",
        "exchange",
        "stake",
        "dispute",
        "settlement",
        "revenue_panel",
        "conservation",
    ] {
        assert!(
            value.get(section).is_some(),
            "场景 JSON 缺少 section: {section}"
        );
    }
    assert_eq!(value["track"], json!("1.4"));
    // 定价：卖方 408（高信誉 + 空闲）击败对手 636（低信誉 + 满载）。
    assert_eq!(value["pricing"]["discovered"], json!(2));
    assert_eq!(value["pricing"]["chosen_unit_price"], json!(408));
    // 兑换：小额内部、时效不足缓办、可执行走 ETH（费用 3、到账 397），链上未执行。
    assert_eq!(value["exchange"]["decisions"][2]["venue"], json!("eth"));
    assert_eq!(value["exchange"]["chain_executed"], json!(false));
    assert_eq!(value["exchange"]["book"]["pending"], json!(1));
    // 质押：认领 100 → 解质押 40 → 冷静期到点释放 40。
    assert_eq!(value["stake"]["released"], json!(40));
    assert_eq!(value["stake"]["consistent"], json!(true));
    // 仲裁：罚没被锁定余额截断为 100，申诉后重裁驳回，累计仍 100。
    assert_eq!(
        value["dispute"]["first_ruling"]["cap"],
        json!("locked_balance")
    );
    assert_eq!(value["dispute"]["first_ruling"]["slashed"], json!(100));
    assert_eq!(value["dispute"]["second_ruling"]["slashed"], json!(0));
    assert_eq!(
        value["dispute"]["second_ruling"]["slashed_total"],
        json!(100)
    );
    // 结算：拒付 50、托管 100 后退回、分成 140 + 60。
    assert_eq!(value["settlement"]["withheld"]["route"], json!("withheld"));
    assert_eq!(value["settlement"]["escrow"]["refunded"], json!(100));
    assert_eq!(value["settlement"]["receipts"][0]["amount"], json!(140));
    assert_eq!(value["settlement"]["receipts"][1]["amount"], json!(60));
    assert_eq!(
        value["settlement"]["human_operator"]["may_decide"],
        json!(false)
    );
    // 只读收益面板：与节点 /api/revenue 字段对齐（minted/slashed/total/accounts.*.available|locked）。
    assert_eq!(value["revenue_panel"]["read_only"], json!(true));
    assert_eq!(
        value["revenue_panel"]["ledger"]["conservation_ok"],
        json!(true)
    );
    assert_eq!(value["revenue_panel"]["ledger"]["account_count"], json!(5));
    assert_eq!(value["revenue_panel"]["total_earned"], json!(200));
    let owner = AgentKeys::from_seed(&[45; 32]).did();
    let owner_row = &value["revenue_panel"]["ledger"]["accounts"][owner.as_str()];
    assert_eq!(owner_row["earned"], json!(60));
    assert_eq!(owner_row["locked"], json!(0));
    assert_eq!(owner_row["kind"], json!("human_operator"));
    let total_available = value["revenue_panel"]["ledger"]["available"]
        .as_i64()
        .unwrap_or(0);
    let total_locked = value["revenue_panel"]["ledger"]["locked"]
        .as_i64()
        .unwrap_or(0);
    let total = value["revenue_panel"]["ledger"]["total"]
        .as_i64()
        .unwrap_or(-1);
    assert_eq!(total_available + total_locked, total, "面板总量必须自洽");
    // 守恒。
    assert_eq!(value["conservation"]["ok"], json!(true));
    assert_eq!(value["conservation"]["slashed"], json!(100));
}

#[test]
fn the_scenario_is_reproducible_on_a_fresh_kernel_and_conserves() {
    let (kernel_a, a) = run_scenario();
    let (kernel_b, b) = run_scenario();
    assert_eq!(
        au4a_core::canonicalize(&a).unwrap(),
        au4a_core::canonicalize(&b).unwrap(),
        "空内核上的场景必须逐字节可复现"
    );
    for kernel in [&kernel_a, &kernel_b] {
        kernel.ledger().check_conservation().unwrap();
        let view = kernel.observe();
        // 4 个注册 Agent + 1 个「人类操作者」收益账户（它不是 Agent，只出现在账本账户里）。
        assert_eq!(view.agents.len(), 4, "注册 Agent 数量");
        assert_eq!(
            view.ledger.accounts.len(),
            5,
            "账本账户数（含人类操作者收益账户）"
        );
        let owner = AgentKeys::from_seed(&[45; 32]).did();
        let owner_account = view
            .ledger
            .accounts
            .get(owner.as_str())
            .expect("人类操作者应有收益账户");
        assert_eq!(
            owner_account.available,
            Credits(60),
            "人类操作者只收收益 60"
        );
        assert_eq!(owner_account.locked, Credits::ZERO, "人类操作者不质押");
        assert!(view.ledger.total <= view.ledger.minted);
    }
}

#[test]
fn a_tampered_price_announcement_is_recorded_as_misconduct() {
    let mut kernel = fresh_kernel();
    let attacker = AgentKeys::from_seed(&[41; 32]);
    kernel
        .register(
            &attacker,
            "economy.seller",
            &["translate.en-zh"],
            Credits(100),
        )
        .unwrap();
    let mut envelope = Envelope::new(
        attacker.did(),
        None,
        "economy.price",
        1,
        None,
        json!({ "provider": attacker.did(), "skill": "translate.en-zh", "unit_price": 1 }),
    )
    .unwrap()
    .seal(&attacker)
    .unwrap();
    // 篡改报价（伪造低价）→ 验签失败 → 一次性恶意证据。
    envelope.body =
        json!({ "provider": attacker.did(), "skill": "translate.en-zh", "unit_price": 0 });
    assert!(kernel.send(&envelope).is_err());
    let (_, refusal) = &kernel.refusals()[0];
    assert_eq!(refusal.code, RefusalCode::Unauthorized);
    assert!(refusal.code.is_misconduct(), "伪造签名必须是单次即恶意");
    assert_eq!(
        kernel.escalation_for(&attacker.did(), refusal.code),
        au4a_core::Escalation::Quarantine
    );

    // 即使内核里已经有恶意记录，场景仍然跑得通且守恒。
    let value = au4a_economy::scenario(&mut kernel).unwrap();
    assert_eq!(value["conservation"]["ok"], json!(true));
    kernel.ledger().check_conservation().unwrap();
    assert!(kernel.refusals().len() >= 1);
}

#[test]
fn unverified_evidence_never_settles_through_the_kernel_gate() {
    let mut kernel = fresh_kernel();
    let payer = AgentKeys::from_seed(&[70; 32]);
    let payee = AgentKeys::from_seed(&[71; 32]);
    kernel
        .register(&payer, "payer", &["buy"], Credits(10))
        .unwrap();
    kernel
        .register(&payee, "skill", &["skill"], Credits(10))
        .unwrap();
    let before = kernel.ledger().balance(&payer.did());
    assert!(
        kernel
            .settle(
                &payer.did(),
                &payee.did(),
                Credits(10),
                EvidenceGrade::Unverified
            )
            .is_err(),
        "Unverified 证据不得结算"
    );
    assert_eq!(
        kernel.ledger().balance(&payer.did()),
        before,
        "被拒绝的结算不得动账"
    );
    let request = SettlementRequest {
        payer: payer.did(),
        payee: payee.did(),
        total: Credits(10),
        evidence: EvidenceGrade::Unverified,
        dispute_open: false,
        at: 1,
    };
    let decision = settlement_route(&request, &SettlementPolicy::DEFAULT).unwrap();
    assert_eq!(decision.route, SettlementRoute::Withheld);
    // cpu-proto 超上限同样拒付。
    let over_cap = SettlementRequest {
        total: Credits(101),
        evidence: EvidenceGrade::CpuProto,
        ..request.clone()
    };
    assert_eq!(
        settlement_route(&over_cap, &SettlementPolicy::DEFAULT)
            .unwrap()
            .route,
        SettlementRoute::Withheld
    );
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn the_kernel_shared_with_other_tracks_can_run_the_scenario_twice() {
    // 幂等：同一个内核跑两次场景（模拟节点重复调用）——不 panic、不越界、守恒。
    let mut kernel = fresh_kernel();
    let first = au4a_economy::scenario(&mut kernel).unwrap();
    let second = au4a_economy::scenario(&mut kernel).unwrap();
    for value in [&first, &second] {
        assert_eq!(value["conservation"]["ok"], json!(true));
    }
    // 首次运行创世额度够用，不需要补足；第二次运行时买方已花掉钱，补足量必须如实上报。
    assert_eq!(first["topups"]["buyer"], json!(0));
    assert!(second["topups"]["buyer"].as_i64().unwrap_or(0) > 0);
    assert!(second["topups"]["seller"].as_i64().unwrap_or(0) >= 0);
    kernel.ledger().check_conservation().unwrap();
    // 第二次运行不会重复注册（ensure_registered 幂等）：注册 Agent 仍是 4 个，
    // 账本账户仍是 5 个（多出来的那个是人类操作者的收益账户）。
    assert_eq!(kernel.observe().agents.len(), 4);
    assert_eq!(kernel.observe().ledger.accounts.len(), 5);
}

#[test]
fn results_json_and_self_check_are_wired_for_the_node() {
    let value = au4a_economy::results_json().unwrap();
    assert_eq!(value["track"], json!("1.4"));
    assert_eq!(value["all_passed"], json!(true));
    assert_eq!(au4a_economy::TRACK, "1.4");
    assert_eq!(au4a_economy::RANGE, "v1.4.1 → v1.4.10");
    assert!(au4a_economy::TITLE.contains("Economic Autonomy"));
    let checks = au4a_economy::self_check();
    assert!(checks.len() >= 8, "自检项应覆盖各模块：{}", checks.len());
    for check in &checks {
        assert_eq!(check.track, "1.4");
        assert!(check.passed, "自检失败：{} / {}", check.name, check.detail);
    }
    // 内核的消息类型常量仍然可用（场景广播用的是自定义 kind，不修改冻结基元）。
    assert_eq!(kinds::AGENT_CARD, "agent.card");
}
