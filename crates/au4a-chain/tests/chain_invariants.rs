//! v1.8.7 集成测试（一）：跨模块不变量与拒绝清单。
//!
//! 每个适配器自己的单元测试已经覆盖了本地分支；这里检查**跨模块性质**：
//!
//! * 四个适配器都必须公布**具名拒绝清单**，且未知操作一定被按名字拒绝（绝不静默通过）；
//! * 任何「成功」收据的证据等级都恒为 `cpu-proto`，`real_network` 恒为 false；
//! * 双轨守恒在随机的桥出/桥回序列下始终成立，且 fail-closed 闸门始终可用；
//! * 双花 / 重放 / 最终性回滚都被拦住。

use au4a_chain::bridge::BridgeBook;
use au4a_chain::erc8004::{Erc8004Adapter, ERC8004_REFUSALS, ERC8004_SUPPORTED};
use au4a_chain::reputation::{
    ChainReputationEvent, ReputationBridge, ReputationEventKind, MAX_STEP_BP, NEUTRAL_BP,
};
use au4a_chain::rgb::{RgbAdapter, TransferBundle, RGB_REFUSALS, RGB_SUPPORTED};
use au4a_chain::routing::{route, settle, Rail, RouteAction, RoutingTable, SettlementRequest};
use au4a_chain::taproot::{TaprootAdapter, TAPROOT_REFUSALS, TAPROOT_SUPPORTED};
use au4a_chain::testnet::{ChainId, ChainRefusal, ChainTx, RefusalSpec, Testnet, ONCHAIN_GRADE};
use au4a_chain::x402::{Invoice, X402Adapter, X402_REFUSALS, X402_SUPPORTED};
use au4a_core::{AgentKeys, Credits, Did, Ledger, RefusalCode};
use serde_json::json;

/// 确定性伪随机（LCG）：不引入依赖，重放完全一致。
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}

fn did(seed: u8) -> Did {
    AgentKeys::from_seed(&[seed; 32]).did()
}

fn all_refusal_lists() -> Vec<(&'static str, &'static [RefusalSpec], usize)> {
    vec![
        ("rgb", &RGB_REFUSALS, RGB_SUPPORTED.len()),
        ("taproot", &TAPROOT_REFUSALS, TAPROOT_SUPPORTED.len()),
        ("erc8004", &ERC8004_REFUSALS, ERC8004_SUPPORTED.len()),
        ("x402", &X402_REFUSALS, X402_SUPPORTED.len()),
    ]
}

#[test]
fn every_adapter_publishes_a_named_refusal_list_with_unique_ops() {
    for (name, list, supported) in all_refusal_lists() {
        assert!(!list.is_empty(), "{name} 必须公布拒绝清单");
        assert!(supported > 0, "{name} 必须至少支持一个操作");
        let mut seen = std::collections::BTreeSet::new();
        for spec in list {
            assert!(!spec.op.is_empty(), "{name} 的拒绝项必须有操作名");
            assert!(!spec.reason.is_empty(), "{name}:{} 必须有原因", spec.op);
            assert!(
                seen.insert(spec.op),
                "{name} 的拒绝清单里出现重复 op：{}",
                spec.op
            );
        }
        // 未知操作必须被按名字拒绝，并在 detail 里列出已知清单。
        let unknown = ChainRefusal::unsupported("adapter.unknown_op", list);
        assert_eq!(unknown.op, "adapter.unknown_op");
        assert_eq!(unknown.code, RefusalCode::Unsupported);
        assert!(unknown.detail.contains(list[0].op));
    }
}

#[test]
fn every_adapter_refuses_unknown_operations_by_name() {
    // RGB
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let mut rgb = RgbAdapter::new();
    let tx = ChainTx::new(ChainId::BtcRegtest, "rgb.mystery", &did(1), 1, json!({})).unwrap();
    let err = rgb.execute(&mut net, &tx).unwrap_err();
    assert_eq!(err.op, "rgb.mystery");
    assert_eq!(err.code, RefusalCode::Unsupported);

    // Taproot
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let mut tap = TaprootAdapter::new();
    let tx = ChainTx::new(
        ChainId::BtcRegtest,
        "taproot.mystery",
        &did(1),
        1,
        json!({}),
    )
    .unwrap();
    assert_eq!(
        tap.execute(&mut net, &tx).unwrap_err().code,
        RefusalCode::Unsupported
    );

    // ERC-8004
    let mut net = Testnet::new(ChainId::EthLocal, 1);
    let mut erc = Erc8004Adapter::new();
    let tx = ChainTx::new(ChainId::EthLocal, "erc8004.mystery", &did(1), 1, json!({})).unwrap();
    assert_eq!(
        erc.execute(&mut net, &tx).unwrap_err().code,
        RefusalCode::Unsupported
    );

    // x402
    let mut net = Testnet::new(ChainId::EthLocal, 1);
    let mut x = X402Adapter::new();
    let tx = ChainTx::new(ChainId::EthLocal, "x402.mystery", &did(1), 1, json!({})).unwrap();
    assert_eq!(
        x.execute(&mut net, &tx).unwrap_err().code,
        RefusalCode::Unsupported
    );

    // 信誉桥（本地映射，不走交易，但同样按名字拒绝）。
    let mut rep = ReputationBridge::new();
    assert_eq!(
        rep.execute("reputation.mystery").unwrap_err().code,
        RefusalCode::Unsupported
    );
    assert_eq!(rep.refusals().len(), 1);
}

#[test]
fn every_successful_receipt_is_cpu_proto() {
    // RGB 发行
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let mut rgb = RgbAdapter::new();
    let issue = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.issue",
        &did(1),
        1,
        json!({ "ticker": "T", "supply": 100 }),
    )
    .unwrap();
    let receipt = rgb.execute(&mut net, &issue).unwrap().receipt.unwrap();
    assert_eq!(receipt.grade, ONCHAIN_GRADE);
    assert_eq!(receipt.grade.as_str(), "cpu-proto");
    assert!(!receipt.finalized);

    // x402 开票
    let mut eth = Testnet::new(ChainId::EthLocal, 1);
    let mut x = X402Adapter::new();
    let invoice_tx = ChainTx::new(
        ChainId::EthLocal,
        "x402.invoice",
        &did(2),
        1,
        json!({ "amount": 10, "asset": "usdc", "expires_at": 100 }),
    )
    .unwrap();
    let receipt = x.execute(&mut eth, &invoice_tx).unwrap().receipt.unwrap();
    assert_eq!(receipt.grade.as_str(), "cpu-proto");

    // 所有适配器的投影都标注 real_network = false / grade = cpu-proto。
    assert_eq!(net.to_json()["real_network"], json!(false));
    assert_eq!(eth.to_json()["grade"], json!("cpu-proto"));
    assert_eq!(
        rgb.contract().unwrap().to_json()["grade"],
        json!("cpu-proto")
    );
    assert_eq!(x.to_json()["real_network"], json!(false));
}

#[test]
fn dual_track_conservation_survives_a_random_bridge_sequence() {
    let who = did(3);
    let mut ledger = Ledger::new();
    ledger.mint(&who, Credits(10_000)).unwrap();
    let mut book = BridgeBook::new();
    let mut rng = Lcg::new(20_261_007);
    let mut transitions = 0u64;
    for _ in 0..200 {
        let amount = Credits((rng.below(300) + 1) as i64);
        if rng.below(2) == 0 {
            if book
                .bridge_out(&mut ledger, &who, amount, "rgb:USDT", "rgb")
                .is_ok()
            {
                transitions += 1;
            }
        } else if book
            .bridge_in(&mut ledger, &who, &did(4), amount, "rgb:USDT", "rgb")
            .is_ok()
        {
            transitions += 1;
        }
        // 每一步之后：账本守恒 + 双轨一致 + 总量不变。
        ledger.check_conservation().unwrap();
        let report = book.require_consistent(&ledger).unwrap();
        assert_eq!(report.escrowed, report.chain_supply);
        assert_eq!(
            ledger.total().unwrap(),
            Credits(10_000),
            "双轨桥接不得改变本地总量"
        );
    }
    assert!(transitions > 50, "序列应该真的发生过桥接动作");
}

#[test]
fn double_spending_and_replaying_are_impossible() {
    // 1) 测试网级重放：同一笔交易二次提交。
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.transfer",
        &did(5),
        1,
        json!({ "x": 1 }),
    )
    .unwrap();
    net.accept(&tx).unwrap();
    assert_eq!(net.accept(&tx).unwrap_err().code, RefusalCode::Conflict);

    // 2) RGB 封印双花：同一封印用两次。
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let mut rgb = RgbAdapter::new();
    let issue = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.issue",
        &did(6),
        1,
        json!({ "ticker": "USDT", "supply": 500 }),
    )
    .unwrap();
    rgb.execute(&mut net, &issue).unwrap();
    net.mine();
    let bundle = TransferBundle::new(
        &rgb.contract().unwrap().asset_id,
        vec![au4a_chain::rgb::Seal::new(issue.id.clone(), 0)],
        vec![(au4a_chain::rgb::Seal::new("out", 0), Credits(500))],
        "blind",
    )
    .unwrap();
    let transfer = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.transfer",
        &did(6),
        2,
        serde_json::to_value(&bundle).unwrap(),
    )
    .unwrap();
    rgb.execute(&mut net, &transfer).unwrap();
    net.mine();
    let bundle2 = TransferBundle::new(
        &rgb.contract().unwrap().asset_id,
        vec![au4a_chain::rgb::Seal::new(issue.id.clone(), 0)],
        vec![(au4a_chain::rgb::Seal::new("out2", 0), Credits(500))],
        "blind",
    )
    .unwrap();
    let double_spend = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.transfer",
        &did(6),
        3,
        serde_json::to_value(&bundle2).unwrap(),
    )
    .unwrap();
    let err = rgb.execute(&mut net, &double_spend).unwrap_err();
    assert_eq!(err.code, RefusalCode::Conflict);
    assert!(err.detail.contains("双花"));
    assert_eq!(rgb.contract().unwrap().circulating().unwrap(), Credits(500));
}

#[test]
fn a_reorg_below_finality_invalidates_the_receipt_and_above_it_is_refused() {
    let mut net = Testnet::new(ChainId::BtcRegtest, 2);
    let tx = ChainTx::new(ChainId::BtcRegtest, "rgb.transfer", &did(7), 1, json!({})).unwrap();
    let receipt = net.accept(&tx).unwrap();
    net.mine();
    assert!(!net.is_final(receipt.height));
    // 未最终化 → 可以回滚，交易重新可打包。
    let dropped = net.reorg(1).unwrap();
    assert_eq!(dropped, vec![tx.id.clone()]);
    assert!(net.accept(&tx).is_ok());
    net.mine();
    net.mine_to(net.height() + net.finality_depth());
    assert!(net.is_final(1));
    // 已最终化 → 回滚到最终化高度以下被拒绝。
    assert_eq!(net.reorg(3).unwrap_err().code, RefusalCode::Conflict);
    // 回滚最终化高度以上的块是允许的，但**最终性已冻结**：高度 1 仍然是最终的。
    let dropped = net.reorg(2).unwrap();
    assert!(dropped.is_empty());
    assert!(net.is_final(1), "最终性一旦达成不得被重组撤销");
    assert_eq!(net.reorg(1).unwrap_err().code, RefusalCode::Conflict);
}

#[test]
fn the_fail_closed_gate_covers_every_rail() {
    let table = RoutingTable::default_table();
    let payer = did(8);
    let payee = did(9);
    // 对每个金额档，双轨不一致都必须缓办。
    for amount in [60i64, 200, 1_000, 99_999, 500_000] {
        let request = SettlementRequest::new(&payer, &payee, amount).unwrap();
        let ok = route(&request, &table, true).unwrap();
        let blocked = route(&request, &table, false).unwrap();
        assert_eq!(blocked.action, RouteAction::Defer, "amount={amount}");
        assert_eq!(
            blocked.reason.as_str(),
            "reconciliation_failed",
            "amount={amount}"
        );
        if amount >= 50 {
            assert_eq!(ok.action, RouteAction::Onchain, "amount={amount}");
            assert!(ok.rail.is_onchain());
        }
    }
    // 真动账时也一样：故障注入 → Err。
    let mut ledger = Ledger::new();
    ledger.mint(&payer, Credits(100_000)).unwrap();
    let mut book = BridgeBook::new();
    book.bridge_out(&mut ledger, &payer, Credits(10), "rail:rgb", "rgb")
        .unwrap();
    book.force_chain_supply(Credits(999_999));
    let request = SettlementRequest::new(&payer, &payee, 1_000).unwrap();
    assert!(settle(&mut ledger, &mut book, &request, &table).is_err());
    assert_eq!(
        ledger.balance(&payer).locked,
        Credits(10),
        "账本不得被链上数字改写"
    );
}

#[test]
fn reputation_updates_stay_bounded_and_non_transferable() {
    let who = did(10);
    let mut net = Testnet::new(ChainId::EthLocal, 1);
    net.mine_to(50);
    let mut bridge = ReputationBridge::new();
    let mut previous = NEUTRAL_BP;
    for i in 0..25u64 {
        let event = ChainReputationEvent::new(&who, ReputationEventKind::Validation, 10_000, 2 + i)
            .unwrap();
        let after = bridge.apply_event(&net, &event).unwrap();
        let step = after.honesty_bp - previous;
        assert!(
            step <= MAX_STEP_BP,
            "单步位移 {step} 超过上限 {MAX_STEP_BP}"
        );
        assert!(step >= 0, "满分事件不应让信誉下降");
        assert!(after.honesty_bp <= 10_000);
        previous = after.honesty_bp;
    }
    assert!(previous >= 9_999, "应收敛到 1bp 以内，实际 {previous}");
    // 不可转让。
    for op in ["reputation.transfer", "reputation.buy", "reputation.reset"] {
        assert_eq!(
            bridge.execute(op).unwrap_err().code,
            RefusalCode::PolicyDenied
        );
    }
    assert_eq!(bridge.to_json()["transferable"], json!(false));
}

#[test]
fn invoices_and_rails_are_content_addressed() {
    let a = Invoice::new(&did(11), Credits(100), "usdc", 1, 50, "m").unwrap();
    let b = Invoice::new(&did(11), Credits(100), "usdc", 1, 50, "m").unwrap();
    let c = Invoice::new(&did(11), Credits(101), "usdc", 1, 50, "m").unwrap();
    assert_eq!(a.id, b.id, "同条款必须同 id");
    assert_ne!(a.id, c.id, "金额不同必须不同 id");
    assert_eq!(a.id.len(), 64);
    assert_eq!(Rail::ALL.len(), 4);
    assert!(!Rail::Internal.is_onchain());
    for rail in Rail::ALL {
        if rail.is_onchain() {
            assert!(RoutingTable::default_table().terms_for(rail).is_some());
        }
    }
}
