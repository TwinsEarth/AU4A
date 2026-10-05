//! v1.8.9 示例：跨链结算九步走（可运行、确定性、无网络 / 无文件 / 无墙钟）。
//!
//! 运行：
//!
//! ```powershell
//! $env:CARGO_TARGET_DIR = "E:\DS\_forangent\target\au4a-chain"
//! cargo run -p au4a-chain --example chain_tour
//! ```
//!
//! 每一步都直接调用本 crate 的 public API 并打印**真实数字**。
//! 请记住：本 crate 里没有真实链上执行——所有「链上」都是确定性测试网（`cpu-proto`）。

use au4a_chain::bridge::BridgeBook;
use au4a_chain::erc8004::Erc8004Adapter;
use au4a_chain::reputation::{ChainReputationEvent, ReputationBridge, ReputationEventKind};
use au4a_chain::rgb::{RgbAdapter, Seal, TransferBundle};
use au4a_chain::routing::{route, Rail, RouteAction, RoutingTable, SettlementRequest};
use au4a_chain::taproot::{verify_merkle, TaprootAdapter};
use au4a_chain::testnet::{ChainId, ChainTx, Testnet, ONCHAIN_GRADE};
use au4a_chain::x402::X402Adapter;
use au4a_core::{AgentKeys, Credits, Did, Ledger};
use au4a_kernel::{Kernel, KernelConfig};
use serde_json::json;

fn did(seed: u8) -> Did {
    AgentKeys::from_seed(&[seed; 32]).did()
}

fn main() -> au4a_core::CoreResult<()> {
    println!("AU4A 轨道 1.8 — Cross-Chain Settlement 跨链结算（v1.8.1 → v1.8.10）");
    println!(
        "边界声明：本 crate 没有任何真实链上执行；链上证据等级恒为 {}（确定性测试网）。\n",
        ONCHAIN_GRADE.as_str()
    );

    let alice = did(1);
    let bob = did(2);

    // ── 第 1 步（v1.8.1）确定性测试网：接受 / 出块 / 最终性 / 篡改被拒 ──────────────
    let mut net = Testnet::new(ChainId::BtcRegtest, 2);
    let seal_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.issue",
        &alice,
        1,
        json!({ "ticker": "USDT-RGB", "supply": 400 }),
    )?;
    let receipt = net
        .accept(&seal_tx)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    net.mine();
    net.mine_to(net.height() + net.finality_depth());
    let mut tampered = seal_tx.clone();
    tampered.payload = json!({ "ticker": "USDT-RGB", "supply": 1_000_000 });
    let tamper_refusal = net.accept(&tampered).unwrap_err();
    println!(
        "[1/9 v1.8.1 测试网] 交易 {} 入块高度 {}（最终化到 {}，is_final={}），等级 {}；篡改 → {}（{}）",
        au4a_core::short_id(&receipt.tx_id),
        receipt.height,
        net.finalized_height(),
        net.is_final(receipt.height),
        receipt.grade.as_str(),
        tamper_refusal.code.as_str(),
        tamper_refusal.op
    );

    // ── 第 2 步（v1.8.1）双轨账本：本地积分 ↔ 链上表示 ─────────────────────────────
    let mut ledger = Ledger::new();
    ledger.mint(&alice, Credits(1_000))?;
    ledger.mint(&bob, Credits(500))?;
    let mut book = BridgeBook::new();
    book.bridge_out(&mut ledger, &alice, Credits(400), "rgb:USDT-RGB", "rgb")?;
    let report = book.require_consistent(&ledger)?;
    println!(
        "[2/9 v1.8.1 双轨] 桥出 400：本地可用 {} / 锁定 {}，链上表示 {}，一致 = {}，fail_closed = {}",
        ledger.balance(&alice).available,
        ledger.balance(&alice).locked,
        book.chain_supply(),
        report.consistent,
        report.fail_closed
    );

    // ── 第 3 步（v1.8.1）RGB：创世 → 密封转移 → 最终化 ────────────────────────────
    let mut rgbnet = Testnet::new(ChainId::BtcRegtest, 1);
    let mut rgb = RgbAdapter::new();
    let issue_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.issue",
        &alice,
        1,
        json!({ "ticker": "USDT-RGB", "supply": 400 }),
    )?;
    rgb.execute(&mut rgbnet, &issue_tx)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    rgbnet.mine();
    let bundle = TransferBundle::new(
        &rgb.contract()?.asset_id,
        vec![Seal::new(issue_tx.id.clone(), 0)],
        vec![
            (Seal::new("alice-anchor", 0), Credits(250)),
            (Seal::new("bob-anchor", 1), Credits(150)),
        ],
        "blind-au4a",
    )?;
    let transfer_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.transfer",
        &alice,
        2,
        serde_json::to_value(&bundle).map_err(|_| au4a_core::CoreError::Encoding)?,
    )?;
    rgb.execute(&mut rgbnet, &transfer_tx)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    rgbnet.mine();
    let early_finalize = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.finalize",
        &alice,
        3,
        json!({ "bundle_id": bundle.commitment, "height": 2 }),
    )?;
    let early_refusal = rgb.execute(&mut rgbnet, &early_finalize).unwrap_err();
    rgbnet.mine_to(rgbnet.height() + rgbnet.finality_depth());
    let finalize_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.finalize",
        &alice,
        4,
        json!({ "bundle_id": bundle.commitment, "height": 2 }),
    )?;
    rgb.execute(&mut rgbnet, &finalize_tx)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    println!(
        "[3/9 v1.8.1 RGB] 发行 400 → 转移 250/150，流通量 {}（承诺 {}）；未最终化就最终化 → {}，最终化数量 {}",
        rgb.contract()?.circulating()?,
        au4a_core::short_id(&bundle.commitment),
        early_refusal.code.as_str(),
        rgb.contract()?.finalized.len()
    );

    // ── 第 4 步（v1.8.2）Taproot：锚定 + Merkle 证明 ──────────────────────────────
    let mut tapnet = Testnet::new(ChainId::BtcRegtest, 1);
    let mut tap = TaprootAdapter::new();
    let anchor_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "taproot.anchor",
        &bob,
        1,
        json!({
            "asset_id": "tap:USDT-RGB",
            "internal_key": au4a_core::content_hash(b"au4a-internal-key"),
            "amounts": [250, 150],
        }),
    )?;
    let anchored = tap
        .execute(&mut tapnet, &anchor_tx)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    let anchor_height = anchored
        .receipt
        .as_ref()
        .map(|r| r.height)
        .ok_or(au4a_core::CoreError::InvalidKind)?;
    tapnet.mine_to(anchor_height + tapnet.finality_depth());
    let anchor = tap
        .anchor_of("tap:USDT-RGB")
        .ok_or(au4a_core::CoreError::UnknownAgent)?
        .clone();
    let proof = anchor.proof_for(1)?;
    let proof_ok = verify_merkle(&anchor.leaves[1], &proof, &anchor.merkle_root)?;
    let fake = au4a_chain::taproot::leaf_hash(
        "tap:USDT-RGB",
        Credits(9_999),
        &Seal::new(anchor_tx.id.clone(), 0),
    )?;
    let fake_ok = verify_merkle(&fake, &proof, &anchor.merkle_root)?;
    println!(
        "[4/9 v1.8.2 Taproot] 锚定总额 {}，输出键 {}，Merkle 根 {}；真叶子证明 = {}，伪造叶子 = {}",
        anchor.amount_total,
        au4a_core::short_id(&anchor.output_key),
        au4a_core::short_id(&anchor.merkle_root),
        proof_ok,
        fake_ok
    );

    // ── 第 5 步（v1.8.3）ERC-8004：身份 + 反馈 + 摘要 ────────────────────────────
    let mut ethnet = Testnet::new(ChainId::EthLocal, 1);
    let mut erc = Erc8004Adapter::new();
    for (who, nonce) in [(&alice, 1u64), (&bob, 1)] {
        let tx = ChainTx::new(ChainId::EthLocal, "erc8004.register", who, nonce, json!({}))?;
        erc.execute(&mut ethnet, &tx)
            .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    }
    let alice_id = erc
        .identity_of_did(&alice)
        .ok_or(au4a_core::CoreError::UnknownAgent)?
        .agent_id
        .clone();
    let feedback = ChainTx::new(
        ChainId::EthLocal,
        "erc8004.give_feedback",
        &bob,
        2,
        json!({ "agent_id": alice_id, "score_bp": 8_500, "tag": "settlement" }),
    )?;
    erc.execute(&mut ethnet, &feedback)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    let selfish = ChainTx::new(
        ChainId::EthLocal,
        "erc8004.give_feedback",
        &alice,
        2,
        json!({ "agent_id": alice_id, "score_bp": 10_000 }),
    )?;
    let self_refusal = erc.execute(&mut ethnet, &selfish).unwrap_err();
    let reputation = erc.summary(&alice_id)?;
    println!(
        "[5/9 v1.8.3 ERC-8004] 身份 {} 收到 {} 条反馈，平均 {}bp、独立客户 {}、权重 {}bp；自评被 {} 拒绝（op={}）",
        au4a_core::short_id(&alice_id),
        reputation.count,
        reputation.average_bp,
        reputation.distinct_clients,
        au4a_chain::erc8004::reputation_weight_bp(&reputation),
        self_refusal.code.as_str(),
        self_refusal.op
    );

    // ── 第 6 步（v1.8.4）x402：发票 → 支付 → 最终性 → 领取 ────────────────────────
    let mut x402 = X402Adapter::new();
    let invoice_tx = ChainTx::new(
        ChainId::EthLocal,
        "x402.invoice",
        &alice,
        3,
        json!({ "amount": 100, "asset": "usdc-eth", "expires_at": 1_000, "memo": "gpu" }),
    )?;
    x402.execute(&mut ethnet, &invoice_tx)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    let invoice = x402
        .last_invoice()
        .cloned()
        .ok_or(au4a_core::CoreError::UnknownAgent)?;
    let pay_tx = ChainTx::new(
        ChainId::EthLocal,
        "x402.pay",
        &bob,
        3,
        json!({ "invoice_id": invoice.id, "amount": 100, "now": 10 }),
    )?;
    let paid = x402
        .execute(&mut ethnet, &pay_tx)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    book.bridge_out(&mut ledger, &bob, Credits(100), "usdc-eth", "x402")?;
    let pay_height = paid
        .receipt
        .as_ref()
        .map(|r| r.height)
        .ok_or(au4a_core::CoreError::InvalidKind)?;
    ethnet.mine_to(pay_height + ethnet.finality_depth());
    let claim_tx = ChainTx::new(
        ChainId::EthLocal,
        "x402.claim",
        &alice,
        4,
        json!({ "invoice_id": invoice.id }),
    )?;
    x402.claim(&mut ethnet, &mut ledger, &mut book, &claim_tx)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    println!(
        "[6/9 v1.8.4 x402] 发票 {} 支付 100 → 托管 → 领取；结清 = {}，链上表示回到 {}，服务方余额 {}",
        au4a_core::short_id(&invoice.id),
        x402.is_settled(&invoice.id),
        book.chain_supply(),
        ledger.balance(&alice).available
    );

    // ── 第 7 步（v1.8.5）结算路由：最便宜轨 + fail-closed 闸门 ────────────────────
    let table = RoutingTable::default_table();
    let request = SettlementRequest::new(&bob, &alice, 400)?;
    let normal = route(&request, &table, true)?;
    let blocked = route(&request, &table, false)?;
    println!(
        "[7/9 v1.8.5 路由] 400 → {} 走 {}（费用 {} 到账 {}，eta {} tick）；双轨不一致 → {}（{}）",
        normal.action.as_str(),
        normal.rail.as_str(),
        normal.fee,
        normal.net,
        normal.eta_ticks,
        blocked.action.as_str(),
        blocked.reason.as_str()
    );
    if normal.action != RouteAction::Onchain || normal.rail != Rail::X402 {
        return Err(au4a_core::CoreError::InvalidKind);
    }

    // ── 第 8 步（v1.8.6）信誉桥接：链上事件 → 本地 4 维信誉 ──────────────────────
    let mut trust = ReputationBridge::new();
    let settle_event = ChainReputationEvent::new(
        &alice,
        ReputationEventKind::SettlementFinal,
        9_000,
        anchor_height,
    )?;
    trust
        .apply_event(&tapnet, &settle_event)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    let feedback_event =
        ChainReputationEvent::new(&alice, ReputationEventKind::Feedback, reputation.average_bp, 1)?;
    let profile = trust
        .apply_event(&ethnet, &feedback_event)
        .map_err(|_| au4a_core::CoreError::InvalidKind)?;
    let transfer_refusal = trust.execute("reputation.transfer").unwrap_err();
    println!(
        "[8/9 v1.8.6 信誉] 2 个事件 → 可靠性 {} / 质量 {} / 诚实 {} / 可用性 {}（综合 {}bp）；转让 → {}",
        profile.reliability_bp,
        profile.quality_bp,
        profile.honesty_bp,
        profile.availability_bp,
        profile.overall_bp(),
        transfer_refusal.code.as_str()
    );

    // ── 第 9 步（v1.8.8 文档 + 自检）：节点 verify 聚合的自检项 ───────────────────
    let checks = au4a_chain::self_check();
    let passed = checks.iter().filter(|c| c.passed).count();
    println!("[9/9 v1.8.8 自检] {} 项自检 {passed} 项通过", checks.len());
    for check in &checks {
        println!("        - {:<36} {}", check.name, check.detail);
    }

    let mut kernel = Kernel::new(KernelConfig::default());
    let summary = au4a_chain::scenario(&mut kernel)?;
    println!(
        "\n场景摘要：等级 {}、real_network {}、双轨一致 {}、RGB 流通量 {}、Taproot {}、信誉综合 {}",
        summary["grade"],
        summary["real_network"],
        summary["bridge"]["consistent"],
        summary["rgb"]["circulating"],
        summary["taproot"]["amount_total"],
        summary["reputation"]["alice"]["overall_bp"]
    );
    println!("场景 JSON：");
    println!("{}", serde_json::to_string_pretty(&summary).unwrap_or_default());

    Ok(())
}
