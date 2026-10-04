//! AU4A 轨道 1.8 — Cross-Chain Settlement 跨链结算（v1.8.1 → v1.8.10）。
//!
//! 跨链结算：RGB / Taproot Assets / ERC-8004 / x402 适配器、结算路由、信誉桥接、安全审计。
//!
//! # 最重要的一句话
//!
//! **这里没有任何真实链上执行。** 所有「链上」都是[确定性测试网](testnet)：
//! 纯 CPU 状态机 + 内容寻址哈希 + 逻辑高度，不联网、不读墙钟、不碰文件。
//! 因此本轨道的一切链上证据等级恒为 [`testnet::ONCHAIN_GRADE`]（`cpu-proto`），
//! 收据上的 `grade` 字段写死这个值，`real_network` 恒为 `false`——**不可能伪造链上成功**。
//!
//! # 三条纪律
//!
//! 1. **具名拒绝**：不支持的操作用 [`testnet::ChainRefusal`] 带上**操作名**与
//!    [`au4a_core::RefusalCode`] 拒绝，并记进测试网的 `refusals`；绝无静默通过。
//! 2. **双轨守恒**：[`bridge`] 保证「本地托管量 == 链上表示总量」，本地账本侧由
//!    `check_conservation()` 保证；两者在每次写路径后都被断言。
//! 3. **fail-closed**：账本是唯一真相；两条轨不一致时 [`bridge::require_consistent`] 直接拒绝，
//!    绝不「按链上数字纠正账本」，也绝不猜测。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责，并留下自检与证据。
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）。

pub mod bridge;
pub mod erc8004;
pub mod reputation;
pub mod rgb;
pub mod routing;
pub mod taproot;
pub mod testnet;
pub mod x402;

use au4a_core::{AgentKeys, CoreError, CoreResult, Credits, Did, Ledger, RefusalCode, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

pub use bridge::{
    reconcile, BridgeBook, BridgeDirection, BridgeEvent, Reconciliation, BRIDGE_INCONSISTENCY_CODE,
};
pub use erc8004::{
    reputation_weight_bp, summary_credits, Erc8004Adapter, Erc8004Outcome, Feedback, Identity,
    ReputationSummary, Validation, ERC8004_REFUSALS, ERC8004_SUPPORTED,
};
pub use reputation::{
    credibility_credits, refusal_code_for, ChainReputationEvent, LocalReputation,
    ReputationBridge, ReputationDim, ReputationEventKind, MAX_STEP_BP, NEUTRAL_BP,
    REPUTATION_REFUSALS, REPUTATION_SUPPORTED,
};
pub use rgb::{RgbAdapter, RgbContract, RgbOutcome, Seal, TransferBundle, RGB_REFUSALS, RGB_SUPPORTED};
pub use routing::{
    route as route_settlement, settle as settle_via_rails, DecisionReason as RouteReason, Rail,
    RailTerms, RouteAction, RouteDecision, RoutingTable, SettlementOutcome, SettlementRequest,
    Urgency as RouteUrgency, UNSUPPORTED_RAIL_CODE,
};
pub use taproot::{
    leaf_hash, merkle_proof, merkle_root, verify_merkle, ProofStep, TaprootAdapter, TaprootAnchor,
    TaprootOutcome, TAPROOT_REFUSALS, TAPROOT_SUPPORTED,
};
pub use testnet::{
    ChainId, ChainRefusal, ChainTx, Receipt, RefusalSpec, Testnet, ONCHAIN_GRADE,
};
pub use x402::{
    Invoice, Payment, X402Adapter, X402Outcome, X402_REFUSALS, X402_SUPPORTED,
};

/// 轨道号。
pub const TRACK: &str = "1.8";
/// 轨道标题。
pub const TITLE: &str = "Cross-Chain Settlement 跨链结算";
/// 版本区间。
pub const RANGE: &str = "v1.8.1 → v1.8.10";
/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_chain";

/// 场景/自检用的确定性身份种子。
fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn did_of(seed: u8) -> Did {
    agent(seed).did()
}

/// 把一次判定转成带真实数字的自检项。
fn check(name: &str, f: impl FnOnce() -> Result<String, String>) -> SelfCheck {
    match f() {
        Ok(detail) => SelfCheck::pass(TRACK, name, detail),
        Err(detail) => SelfCheck::fail(TRACK, name, detail),
    }
}

/// 轨道自检：节点 `verify` 聚合它。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = vec![SelfCheck::pass(
        TRACK,
        "track.wired",
        format!("{TITLE} {RANGE} 已接入 au4a-node（测试网原型，非真实链）"),
    )];

    checks.push(check("chain.evidence_grade", || {
        let net = Testnet::default_for(ChainId::BtcRegtest);
        let value = net.to_json();
        if value["grade"] != json!("cpu-proto") || value["real_network"] != json!(false) {
            return Err(format!("测试网竟然声称真实链：{value}"));
        }
        if ONCHAIN_GRADE.as_str() != "cpu-proto" {
            return Err("链上证据等级不是 cpu-proto".to_string());
        }
        Ok(format!(
            "链上证据等级恒为 {}，real_network = false；即时终局性深度 BTC 侧 {} 块",
            ONCHAIN_GRADE.as_str(),
            net.finality_depth()
        ))
    }));

    checks.push(check("chain.named_refusals", || {
        let mut net = Testnet::new(ChainId::BtcRegtest, 1);
        let mut adapter = RgbAdapter::new();
        let issue = ChainTx::new(
            ChainId::BtcRegtest,
            "rgb.issue",
            &did_of(1),
            1,
            json!({ "ticker": "T", "supply": 10 }),
        )
        .map_err(|e| e.to_string())?;
        adapter.execute(&mut net, &issue).map_err(|r| r.detail.clone())?;
        let bad = ChainTx::new(ChainId::BtcRegtest, "rgb.atomic_swap", &did_of(1), 2, json!({}))
            .map_err(|e| e.to_string())?;
        let refusal = adapter.execute(&mut net, &bad).unwrap_err();
        if refusal.op != "rgb.atomic_swap" || refusal.code != RefusalCode::Unsupported {
            return Err(format!("拒绝不正确：{refusal:?}"));
        }
        if net.refusals().is_empty() {
            return Err("拒绝没有被记录（可能静默通过）".to_string());
        }
        Ok(format!(
            "不支持的操作按名字拒绝：op={} code={}，已记录 {} 条拒绝；已知清单 {} 条",
            refusal.op,
            refusal.code.as_str(),
            net.refusals().len(),
            RGB_REFUSALS.len()
        ))
    }));

    checks.push(check("taproot.merkle", || {
        let mut leaves: Vec<String> = Vec::new();
        for i in 1..=5i64 {
            leaves.push(
                leaf_hash("tap:USDT", Credits(i * 10), &Seal::new(format!("s{i}"), 0))
                    .map_err(|e| e.to_string())?,
            );
        }
        let root = merkle_root(&leaves).map_err(|e| e.to_string())?;
        for index in 0..leaves.len() {
            let proof = merkle_proof(&leaves, index).map_err(|e| e.to_string())?;
            if !verify_merkle(&leaves[index], &proof, &root).map_err(|e| e.to_string())? {
                return Err(format!("第 {index} 个叶子的认证路径验证失败"));
            }
        }
        let fake = leaf_hash("tap:USDT", Credits(9_999), &Seal::new("s1", 0))
            .map_err(|e| e.to_string())?;
        let proof = merkle_proof(&leaves, 0).map_err(|e| e.to_string())?;
        if verify_merkle(&fake, &proof, &root).map_err(|e| e.to_string())? {
            return Err("伪造叶子竟然通过了 Merkle 验证".to_string());
        }
        Ok(format!(
            "5 个叶子的 Merkle 根 {}：全部 5 条认证路径成立，伪造叶子不被接受",
            au4a_core::short_id(&root)
        ))
    }));

    checks.push(check("erc8004.reputation_reads_back", || {
        let mut net = Testnet::new(ChainId::EthLocal, 1);
        let mut reg = Erc8004Adapter::new();
        let mut ids = Vec::new();
        for (seed, nonce) in [(21u8, 1u64), (22, 1), (23, 1)] {
            let tx = ChainTx::new(ChainId::EthLocal, "erc8004.register", &did_of(seed), nonce, json!({}))
                .map_err(|e| e.to_string())?;
            reg.execute(&mut net, &tx).map_err(|r| r.detail.clone())?;
            ids.push(reg.identity_of_did(&did_of(seed)).map(|i| i.agent_id.clone()).unwrap_or_default());
        }
        for (seed, score) in [(22u8, 9_000i64), (23, 7_000)] {
            let tx = ChainTx::new(
                ChainId::EthLocal,
                "erc8004.give_feedback",
                &did_of(seed),
                2,
                json!({ "agent_id": ids[0], "score_bp": score, "tag": "au4a" }),
            )
            .map_err(|e| e.to_string())?;
            reg.execute(&mut net, &tx).map_err(|r| r.detail.clone())?;
        }
        let summary = reg.summary(&ids[0]).map_err(|e| e.to_string())?;
        if summary.count != 2 || summary.average_bp != 8_000 || summary.distinct_clients != 2 {
            return Err(format!("信誉摘要不符：{summary:?}"));
        }
        // 自评被按名字拒绝。
        let selfish = ChainTx::new(
            ChainId::EthLocal,
            "erc8004.give_feedback",
            &did_of(21),
            2,
            json!({ "agent_id": ids[0], "score_bp": 10_000 }),
        )
        .map_err(|e| e.to_string())?;
        let refusal = reg.execute(&mut net, &selfish).unwrap_err();
        if refusal.code != RefusalCode::PolicyDenied || refusal.op != "erc8004.self_feedback" {
            return Err(format!("自评拒绝不正确：{refusal:?}"));
        }
        Ok(format!(
            "2 条反馈（9000/7000）→ 平均 {}bp、独立客户 {}、权重 {}bp；自评按 {} 拒绝",
            summary.average_bp,
            summary.distinct_clients,
            reputation_weight_bp(&summary),
            refusal.code.as_str()
        ))
    }));

    checks.push(check("routing.fail_closed_first", || {
        let table = RoutingTable::default_table();
        let payer = did_of(31);
        let payee = did_of(32);
        let request = SettlementRequest::new(&payer, &payee, 1_000).map_err(|e| e.to_string())?;
        let normal = routing::route(&request, &table, true).map_err(|e| e.to_string())?;
        if normal.action != RouteAction::Onchain || normal.rail != Rail::X402 {
            return Err(format!("最便宜轨应为 x402，实际 {normal:?}"));
        }
        if normal.fee != Credits(6) || normal.net != Credits(994) {
            return Err(format!("费用/到账不符：fee={} net={}", normal.fee, normal.net));
        }
        // 双轨不一致 → 一律缓办（即使请求本身完全合格）。
        let blocked = routing::route(&request, &table, false).map_err(|e| e.to_string())?;
        if blocked.action != RouteAction::Defer || blocked.reason != RouteReason::ReconciliationFailed {
            return Err(format!("fail-closed 闸门失效：{blocked:?}"));
        }
        // 真正动账也走同一条闸门。
        let mut ledger = Ledger::new();
        ledger.mint(&payer, Credits(5_000)).map_err(|e| e.to_string())?;
        let mut book = BridgeBook::new();
        let outcome = routing::settle(&mut ledger, &mut book, &request, &table)
            .map_err(|e| format!("结算失败：{e}"))?;
        if outcome.moved != Credits(1_000) || !outcome.reconciliation.consistent {
            return Err(format!("结算结果不符：{outcome:?}"));
        }
        ledger.check_conservation().map_err(|e| e.to_string())?;
        book.require_consistent(&ledger).map_err(|e| e.to_string())?;
        Ok(format!(
            "1000 → 最便宜轨 {} 费用 {} 到账 {}；不一致时 {}；实付 1000 后托管 {} == 链上表示 {}",
            normal.rail.as_str(),
            normal.fee,
            normal.net,
            blocked.reason.as_str(),
            book.escrowed(),
            book.chain_supply()
        ))
    }));

    checks.push(check("reputation.non_transferable_and_bounded", || {
        let who = did_of(41);
        let mut net = Testnet::new(ChainId::EthLocal, 1);
        net.mine_to(10);
        let mut bridge = ReputationBridge::new();
        // 反馈 9000bp（权重 6000）→ 质量维度 +2000（单步上限），可靠性不动。
        let feedback =
            ChainReputationEvent::new(&who, ReputationEventKind::Feedback, 9_000, 2)
                .map_err(|e| e.to_string())?;
        let after = bridge
            .apply_event(&net, &feedback)
            .map_err(|r| r.detail.clone())?;
        if after.quality_bp != 7_000 || after.reliability_bp != NEUTRAL_BP {
            return Err(format!("有界更新不符：{after:?}"));
        }
        // 不可转让：按名字拒绝。
        let refusal = bridge.execute("reputation.transfer").unwrap_err();
        if refusal.code != RefusalCode::PolicyDenied || refusal.op != "reputation.transfer" {
            return Err(format!("转让信誉竟然没被拒绝：{refusal:?}"));
        }
        // 未最终化的事件不改变信誉。
        let mut fresh_net = Testnet::new(ChainId::EthLocal, 5);
        fresh_net.mine_to(1);
        let early = ChainReputationEvent::new(&who, ReputationEventKind::Feedback, 1, 2)
            .map_err(|e| e.to_string())?;
        let early_err = bridge.apply_event(&fresh_net, &early).unwrap_err();
        if early_err.code != RefusalCode::Timeout {
            return Err(format!("未最终化事件未被拒绝：{early_err:?}"));
        }
        Ok(format!(
            "反馈 9000bp → 质量 {} / 可靠性 {}（单步上限 {}）；转让信誉 → {}；未最终化事件 → {}",
            after.quality_bp,
            after.reliability_bp,
            MAX_STEP_BP,
            refusal.code.as_str(),
            early_err.code.as_str()
        ))
    }));

    checks.push(check("chain.dual_track_conservation", || {
        let who = did_of(2);
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(1_000)).map_err(|e| e.to_string())?;
        let mut book = BridgeBook::new();
        book.bridge_out(&mut ledger, &who, Credits(300), "rgb:USDT", "rgb")
            .map_err(|e| e.to_string())?;
        let report = book
            .require_consistent(&ledger)
            .map_err(|e| format!("双轨对账失败：{e}"))?;
        if !report.consistent || report.escrowed != report.chain_supply {
            return Err(format!("双轨不一致：{report:?}"));
        }
        Ok(format!(
            "桥出 300：本地锁定 {} == 链上表示 {}（账本守恒 = {}，fail_closed = {}）",
            report.escrowed, report.chain_supply, report.ledger_conserved, report.fail_closed
        ))
    }));

    checks.push(check("chain.fail_closed", || {
        let who = did_of(3);
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(500)).map_err(|e| e.to_string())?;
        let mut book = BridgeBook::new();
        book.bridge_out(&mut ledger, &who, Credits(100), "rgb:USDT", "rgb")
            .map_err(|e| e.to_string())?;
        // 人为制造不一致：链上侧多报（故障注入，仅审计用）。
        let mut broken = book.clone();
        broken.force_chain_supply(Credits(7_777));
        let report = reconcile(&ledger, &broken).map_err(|e| e.to_string())?;
        if report.consistent {
            return Err("不一致的台账竟然对账通过".to_string());
        }
        if broken.require_consistent(&ledger).is_ok() {
            return Err("fail-closed 闸门未拦住不一致".to_string());
        }
        if ledger.balance(&who).locked != Credits(100) {
            return Err("账本被链上数字改写了（违反「账本是真相」）".to_string());
        }
        Ok(format!(
            "链上侧多报 7777 vs 托管 100 → 对账 consistent=false，require_consistent 拒绝，账本锁定仍为 {}",
            ledger.balance(&who).locked
        ))
    }));

    checks
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    let checks = self_check();
    let passed = checks.iter().filter(|c| c.passed).count();
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "checks_total": checks.len(),
        "checks_passed": passed,
        "all_passed": au4a_core::all_passed(&checks),
        "chains": ChainId::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
        "grade": ONCHAIN_GRADE.as_str(),
        "real_network": false,
        "modules": ["testnet", "bridge", "rgb"],
    }))
}

/// 幂等注册：共享内核里可能已有别的轨道注册过同一个种子。
fn ensure_registered(
    kernel: &mut Kernel,
    keys: &AgentKeys,
    display: &str,
    skills: &[&str],
    stake: Credits,
) -> CoreResult<Did> {
    let did = keys.did();
    if kernel.card(&did).is_some() {
        return Ok(did);
    }
    let min_stake = kernel.config().min_stake;
    let stake = if stake < min_stake { min_stake } else { stake };
    match kernel.register(keys, display, skills, stake) {
        Ok(card) => Ok(card.did),
        Err(CoreError::DuplicateAgent) => Ok(did),
        Err(err) => Err(err),
    }
}

/// 适配器结果是 [`Result<_, ChainRefusal>`]：场景里失败必须**留痕**，而不是把原因丢掉。
///
/// 拒绝会带着真实的 `op` 与 `RefusalCode` 记进内核（观察层可见），场景本身返回
/// [`CoreError::InvalidKind`]（`CoreError` 冻结、没有链上专用变体；映射写在文档里）。
fn adapter_ok<T>(
    kernel: &mut Kernel,
    who: &Did,
    result: Result<T, ChainRefusal>,
) -> CoreResult<T> {
    match result {
        Ok(value) => Ok(value),
        Err(refusal) => {
            kernel.refuse(
                who,
                refusal.code,
                format!("{}: {}", refusal.op, refusal.detail),
            );
            Err(CoreError::InvalidKind)
        }
    }
}

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!("{TITLE} {RANGE}： RGB + Taproot + ERC-8004 + x402 + 路由 + 信誉桥接（v1.8.6）"),
    );

    let alice_keys = agent(81);
    let bob_keys = agent(82);
    let alice = ensure_registered(kernel, &alice_keys, "chain.alice", &["settle.btc"], Credits(100))?;
    let bob = ensure_registered(kernel, &bob_keys, "chain.bob", &["settle.eth"], Credits(100))?;
    kernel.emit("chain.registered", "2 个 Agent 完成注册（质押自带）");

    // 双轨桥出：本地积分锁定 → 链上表示发行。
    let mut book = BridgeBook::new();
    let bridged = Credits(400);
    let event = book.bridge_out(
        kernel.ledger_mut(),
        &alice,
        bridged,
        "rgb:USDT-RGB",
        "rgb",
    )?;
    kernel.emit(
        "chain.bridge_out",
        format!("桥出 {bridged}（本地锁定 == 链上表示），事件 {}", au4a_core::short_id(&event.id)),
    );

    // RGB 契约：创世发行与密封转移。
    let mut net = Testnet::new(ChainId::BtcRegtest, 2);
    let mut rgb = RgbAdapter::new();
    let issue_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.issue",
        &alice,
        1,
        json!({ "ticker": "USDT-RGB", "supply": bridged }),
    )?;
    let issued = adapter_ok(kernel, &alice, rgb.execute(&mut net, &issue_tx))?;
    net.mine();
    let genesis_seal = Seal::new(issue_tx.id.clone(), 0);
    let bundle = TransferBundle::new(
        &rgb.contract()?.asset_id,
        vec![genesis_seal.clone()],
        vec![
            (Seal::new("alice-anchor", 0), Credits(250)),
            (Seal::new("bob-anchor", 1), Credits(150)),
        ],
        "blind-au4a-1",
    )?;
    let transfer_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.transfer",
        &alice,
        2,
        serde_json::to_value(&bundle).map_err(|_| CoreError::Encoding)?,
    )?;
    let transferred = adapter_ok(kernel, &alice, rgb.execute(&mut net, &transfer_tx))?;
    net.mine();
    kernel.emit(
        "chain.rgb_transfer",
        format!(
            "RGB 密封转移 400 → 250/150，承诺 {}（等级 {}）",
            au4a_core::short_id(&bundle.commitment),
            ONCHAIN_GRADE.as_str()
        ),
    );

    // 客户端验证：包体校验 + 总量守恒。
    let verify_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.verify",
        &bob,
        1,
        serde_json::to_value(&bundle).map_err(|_| CoreError::Encoding)?,
    )?;
    let verified = adapter_ok(kernel, &bob, rgb.execute(&mut net, &verify_tx))?;

    // 最终化：先尝试在最终性之前最终化（必须被拒），叠够深度后再最终化。
    let early = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.finalize",
        &alice,
        3,
        json!({ "bundle_id": bundle.commitment, "height": 2 }),
    )?;
    let early_refusal = rgb.execute(&mut net, &early).unwrap_err();
    kernel.refuse(
        &alice,
        early_refusal.code,
        format!("{}: {}", early_refusal.op, early_refusal.detail),
    );
    net.mine_to(net.height() + net.finality_depth());
    let final_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.finalize",
        &alice,
        4,
        json!({ "bundle_id": bundle.commitment, "height": 2 }),
    )?;
    let finalized = adapter_ok(kernel, &alice, rgb.execute(&mut net, &final_tx))?;

    // 具名拒绝：不支持的操作（原子交换）必须被拒并留痕。
    let unsupported = ChainTx::new(ChainId::BtcRegtest, "rgb.atomic_swap", &alice, 5, json!({}))?;
    let refusal = rgb.execute(&mut net, &unsupported).unwrap_err();
    kernel.refuse(
        &alice,
        RefusalCode::Unsupported,
        format!("{}: {}", refusal.op, refusal.detail),
    );

    // Taproot 侧：把 250/150 锚定进一个 BTC 输出承诺，并用 Merkle 证明验证（v1.8.2）。
    let mut tapnet = Testnet::new(ChainId::BtcRegtest, 1);
    let mut taproot = TaprootAdapter::new();
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
    let anchored = adapter_ok(&mut *kernel, &bob, taproot.execute(&mut tapnet, &anchor_tx))?;
    // 最终性：锚定块之上还要再叠 `finality_depth` 个块。
    let anchor_height = anchored
        .receipt
        .as_ref()
        .map(|r| r.height)
        .ok_or(CoreError::InvalidKind)?;
    let tap_final_height = anchor_height + tapnet.finality_depth();
    tapnet.mine_to(tap_final_height);
    let anchor = taproot
        .anchor_of("tap:USDT-RGB")
        .ok_or(CoreError::UnknownAgent)?
        .clone();
    let tap_leaf = anchor.leaves[1].clone();
    let tap_proof = anchor.proof_for(1)?;
    let tap_verify_tx = ChainTx::new(
        ChainId::BtcRegtest,
        "taproot.verify",
        &bob,
        2,
        json!({ "asset_id": "tap:USDT-RGB", "leaf": tap_leaf, "proof": tap_proof }),
    )?;
    let tap_verified = adapter_ok(&mut *kernel, &bob, taproot.execute(&mut tapnet, &tap_verify_tx))?;
    if taproot.anchored_total()? != bridged {
        return Err(CoreError::InvalidKind);
    }
    kernel.emit(
        "chain.taproot_anchor",
        format!(
            "锚定 {} 总量 {}（输出键 {}）并完成 Merkle 验证",
            anchored.detail,
            taproot.anchored_total()?,
            au4a_core::short_id(&anchor.output_key)
        ),
    );

    // ETH 侧：ERC-8004 身份与信誉注册表（v1.8.3）。
    let mut ethnet = Testnet::new(ChainId::EthLocal, 1);
    let mut erc = Erc8004Adapter::new();
    let mut x402 = X402Adapter::new();
    for (keys, nonce) in [(&alice_keys, 1u64), (&bob_keys, 1)] {
        let tx = ChainTx::new(ChainId::EthLocal, "erc8004.register", &keys.did(), nonce, json!({}))?;
        adapter_ok(&mut *kernel, &keys.did(), erc.execute(&mut ethnet, &tx))?;
    }
    let alice_id = erc
        .identity_of_did(&alice)
        .ok_or(CoreError::UnknownAgent)?
        .agent_id
        .clone();
    let feedback_tx = ChainTx::new(
        ChainId::EthLocal,
        "erc8004.give_feedback",
        &bob,
        2,
        json!({ "agent_id": alice_id, "score_bp": 8_500, "tag": "settlement" }),
    )?;
    adapter_ok(&mut *kernel, &bob, erc.execute(&mut ethnet, &feedback_tx))?;
    let self_tx = ChainTx::new(
        ChainId::EthLocal,
        "erc8004.give_feedback",
        &alice,
        2,
        json!({ "agent_id": alice_id, "score_bp": 10_000 }),
    )?;
    let self_result = erc.execute(&mut ethnet, &self_tx);
    let self_code = match self_result {
        Err(refusal) => {
            kernel.refuse(
                &alice,
                refusal.code,
                format!("{}: {}", refusal.op, refusal.detail),
            );
            refusal.code
        }
        Ok(_) => return Err(CoreError::InvalidKind),
    };
    let reputation = erc.summary(&alice_id)?;
    kernel.emit(
        "chain.erc8004",
        format!(
            "身份 {} 收到 1 条反馈：平均 {}bp、权重 {}bp；自评被拒（{}）",
            au4a_core::short_id(&alice_id),
            reputation.average_bp,
            reputation_weight_bp(&reputation),
            self_code.as_str()
        ),
    );

    // ETH 侧：x402 发票 → 支付 → 最终性 → 领取（v1.8.4）。
    let expires_at = kernel.now() + 50;
    let invoice_tx = ChainTx::new(
        ChainId::EthLocal,
        "x402.invoice",
        &alice,
        3,
        json!({ "amount": 100, "asset": "usdc-eth", "expires_at": expires_at, "memo": "settlement" }),
    )?;
    let invoiced = adapter_ok(&mut *kernel, &alice, x402.execute(&mut ethnet, &invoice_tx))?;
    // 发票 id 由适配器按交易内容计算，这里从适配器取回（不自己算）。
    let invoice = x402
        .last_invoice()
        .cloned()
        .ok_or(CoreError::UnknownAgent)?;
    let pay_tx = ChainTx::new(
        ChainId::EthLocal,
        "x402.pay",
        &bob,
        3,
        json!({ "invoice_id": invoice.id, "amount": 100, "now": kernel.now() }),
    )?;
    let paid = adapter_ok(&mut *kernel, &bob, x402.execute(&mut ethnet, &pay_tx))?;
    let escrow_before = book.bridge_out(
        kernel.ledger_mut(),
        &bob,
        Credits(100),
        "usdc-eth",
        "x402",
    )?;
    let pay_height = paid
        .receipt
        .as_ref()
        .map(|r| r.height)
        .ok_or(CoreError::InvalidKind)?;
    ethnet.mine_to(pay_height + ethnet.finality_depth());
    let claim_tx = ChainTx::new(
        ChainId::EthLocal,
        "x402.claim",
        &alice,
        4,
        json!({ "invoice_id": invoice.id }),
    )?;
    let claim_result = x402.claim(&mut ethnet, kernel.ledger_mut(), &mut book, &claim_tx);
    let claimed = adapter_ok(&mut *kernel, &alice, claim_result)?;
    kernel.emit(
        "chain.x402",
        format!(
            "x402：发票 {} → 支付 100 → 托管 {} → 最终性后领取；已结清 = {}",
            au4a_core::short_id(&invoice.id),
            escrow_before.amount,
            x402.is_settled(&invoice.id)
        ),
    );

    // 结算路由（v1.8.5）：金额/时效/费用阈值选轨；fail-closed 闸门优先。
    let routing_table = RoutingTable::default_table();
    let route_req = SettlementRequest::new(&bob, &alice, 400)?;
    let live_reconciliation = bridge::reconcile(kernel.ledger(), &book)?;
    let route_decision = routing::route(&route_req, &routing_table, live_reconciliation.consistent)?;
    // 把「双轨不一致」这个输入显式喂给决策表，验证它第一优先级就缓办（纯函数，不动账）。
    let blocked_decision = routing::route(&route_req, &routing_table, false)?;
    if blocked_decision.action != RouteAction::Defer {
        return Err(CoreError::InvalidKind);
    }
    kernel.emit(
        "chain.routed",
        format!(
            "400 → {} 走 {}（费用 {} 到账 {}，eta {} tick）；双轨不一致时决策 = {}",
            route_decision.action.as_str(),
            route_decision.rail.as_str(),
            route_decision.fee,
            route_decision.net,
            route_decision.eta_ticks,
            blocked_decision.reason.as_str()
        ),
    );

    // 信誉桥接（v1.8.6）：把链上事件映射进本地 4 维信誉；不可转让、只认最终化事件。
    let mut reputation_bridge = ReputationBridge::new();
    let rgb_event = ChainReputationEvent::new(
        &alice,
        ReputationEventKind::SettlementFinal,
        9_000,
        anchor.height,
    )?;
    let trust_a = adapter_ok(
        &mut *kernel,
        &alice,
        reputation_bridge.apply_event(&tapnet, &rgb_event),
    )?;
    let feedback_event = ChainReputationEvent::new(
        &alice,
        ReputationEventKind::Feedback,
        reputation.average_bp,
        1, // ERC-8004 反馈在 ethnet 的第 1 个块里，已随最终性敲定
    )?;
    let trust_b = adapter_ok(
        &mut *kernel,
        &alice,
        reputation_bridge.apply_event(&ethnet, &feedback_event),
    )?;
    let transfer_refusal = reputation_bridge.execute("reputation.transfer").unwrap_err();
    kernel.refuse(
        &alice,
        transfer_refusal.code,
        format!("{}: {}", transfer_refusal.op, transfer_refusal.detail),
    );
    kernel.emit(
        "chain.reputation_bridged",
        format!(
            "信誉桥接：{} 个事件 → 可靠性 {} / 质量 {} / 诚实 {} / 可用性 {}（综合 {}bp）；转让信誉被 {} 拒绝",
            reputation_bridge.applied_events(),
            trust_b.reliability_bp,
            trust_b.quality_bp,
            trust_b.honesty_bp,
            trust_b.availability_bp,
            trust_b.overall_bp(),
            transfer_refusal.code.as_str()
        ),
    );

    // 双轨对账（fail-closed）：本地托管 == 链上表示 == RGB 流通量 == Taproot 锚定总量。
    let report = book.require_consistent(kernel.ledger())?;
    rgb.contract()?.check_supply_conservation()?;
    kernel.emit(
        "chain.reconciled",
        format!(
            "对账：托管 {} == 链上表示 {} == RGB 流通量 {}",
            report.escrowed,
            report.chain_supply,
            rgb.contract()?.circulating()?
        ),
    );

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "scenario": "v1.8.6 四条轨 + 结算路由 + 跨链信誉桥接（确定性测试网）",
        "agents": [alice.as_str(), bob.as_str()],
        "bridge": {
            "out": bridged,
            "escrowed": report.escrowed,
            "chain_supply": report.chain_supply,
            "consistent": report.consistent,
            "fail_closed": report.fail_closed,
        },
        "rgb": {
            "asset_id": rgb.contract()?.asset_id,
            "supply": rgb.contract()?.supply,
            "circulating": rgb.contract()?.circulating()?,
            "seals": rgb.contract()?.allocations.len(),
            "spent": rgb.contract()?.spent.len(),
            "finalized": rgb.contract()?.finalized.len(),
            "issue": issued.detail,
            "transfer": transferred.detail,
            "verify": verified.detail,
            "finalize": finalized.detail,
        },
        "taproot": {
            "asset_id": anchor.asset_id,
            "internal_key": anchor.internal_key,
            "merkle_root": anchor.merkle_root,
            "output_key": anchor.output_key,
            "amount_total": anchor.amount_total,
            "leaves": anchor.leaves.len(),
            "annotated_height": anchor.height,
            "proof_steps": tap_proof.len(),
            "anchor": anchored.detail,
            "verify": tap_verified.detail,
            "verified_count": taproot.verified_count(),
            "anchored_total": taproot.anchored_total()?,
        },
        "erc8004": {
            "identities": erc.identity_count(),
            "agent_id": alice_id,
            "feedback_count": reputation.count,
            "average_bp": reputation.average_bp,
            "weight_bp": reputation_weight_bp(&reputation),
            "self_feedback_code": self_code.as_str(),
            "summary": reputation.to_json(),
        },
        "x402": {
            "invoice_id": invoice.id,
            "amount": invoice.amount,
            "asset": invoice.asset,
            "invoice": invoiced.detail,
            "pay": paid.detail,
            "escrow_before_claim": escrow_before.amount,
            "claim": claimed.detail,
            "settled": x402.is_settled(&invoice.id),
            "escrow_after_claim": book.escrowed(),
            "chain_supply_after_claim": book.chain_supply(),
        },
        "routing": {
            "table": routing_table,
            "decision": route_decision.to_json(),
            "blocked_decision": blocked_decision.to_json(),
            "reconciliation": live_reconciliation,
        },
        "reputation": {
            "applied_events": reputation_bridge.applied_events(),
            "alice": trust_b.to_json(),
            "after_settlement_event": trust_a.to_json(),
            "transfer_refusal_code": transfer_refusal.code.as_str(),
            "bridge": reputation_bridge.to_json(),
        },
        "refusals": [
            { "op": early_refusal.op, "code": early_refusal.code.as_str(), "detail": early_refusal.detail },
            { "op": refusal.op, "code": refusal.code.as_str(), "detail": refusal.detail },
        ],
        "testnet": net.to_json(),
        "grade": ONCHAIN_GRADE.as_str(),
        "real_network": false,
        "conservation": {
            "ok": true,
            "minted": kernel.ledger().minted(),
            "slashed": kernel.ledger().slashed(),
            "locked": kernel.ledger().balance(&alice).locked,
        },
        "events": kernel.observe().progress.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_kernel::KernelConfig;

    #[test]
    fn the_scenario_is_reproducible_and_never_claims_a_real_chain() {
        let mut first = Kernel::new(KernelConfig::default());
        let a = match scenario(&mut first) {
            Ok(value) => value,
            Err(err) => panic!("场景失败：{err}；内核拒绝记录：{:?}", first.refusals()),
        };
        let mut second = Kernel::new(KernelConfig::default());
        let b = scenario(&mut second).unwrap();
        assert_eq!(
            au4a_core::canonicalize(&a).unwrap(),
            au4a_core::canonicalize(&b).unwrap()
        );
        assert_eq!(a["real_network"], json!(false));
        assert_eq!(a["grade"], json!("cpu-proto"));
        assert_eq!(a["testnet"]["real_network"], json!(false));
        assert_eq!(a["bridge"]["consistent"], json!(true));
        assert_eq!(a["rgb"]["circulating"], json!(400));
        assert_eq!(a["rgb"]["supply"], json!(400));
        assert_eq!(a["rgb"]["finalized"], json!(1));
        // Taproot：锚定 400（250+150），Merkle 验证通过。
        assert_eq!(a["taproot"]["amount_total"], json!(400));
        assert_eq!(a["taproot"]["leaves"], json!(2));
        assert_eq!(a["taproot"]["verified_count"], json!(1));
        assert_eq!(a["taproot"]["anchored_total"], json!(400));
        // ERC-8004：2 个身份、1 条反馈 8500bp、权重 8500 × 2000 / 10000 = 1700bp，自评被 policy_denied 拒绝。
        assert_eq!(a["erc8004"]["identities"], json!(2));
        assert_eq!(a["erc8004"]["feedback_count"], json!(1));
        assert_eq!(a["erc8004"]["average_bp"], json!(8_500));
        assert_eq!(a["erc8004"]["weight_bp"], json!(1_700));
        assert_eq!(a["erc8004"]["self_feedback_code"], json!("policy_denied"));
        // x402：发票 100 → 支付 → 托管 100 → 最终性后领取 → 托管回到 400（链上表示销毁）。
        assert_eq!(a["x402"]["amount"], json!(100));
        assert_eq!(a["x402"]["settled"], json!(true));
        assert_eq!(a["x402"]["escrow_before_claim"], json!(100));
        assert_eq!(a["x402"]["escrow_after_claim"], json!(400));
        assert_eq!(a["x402"]["chain_supply_after_claim"], json!(400));
        // 结算路由：400 → x402（60bp → 费用 2、到账 398、eta 12）；双轨不一致 → 缓办。
        assert_eq!(a["routing"]["decision"]["action"], json!("onchain"));
        assert_eq!(a["routing"]["decision"]["rail"], json!("x402"));
        assert_eq!(a["routing"]["decision"]["fee"], json!(2));
        assert_eq!(a["routing"]["decision"]["net"], json!(398));
        assert_eq!(a["routing"]["decision"]["eta_ticks"], json!(12));
        assert_eq!(a["routing"]["blocked_decision"]["action"], json!("defer"));
        assert_eq!(
            a["routing"]["blocked_decision"]["reason"],
            json!("reconciliation_failed")
        );
        // 信誉桥接：结算事件 9000（权重 4000）→ 可靠性/可用性 6600；反馈 8500（权重 6000，
        // 位移 2100 被单步上限 2000 截断）→ 质量 7000；诚实保持 5000；综合 6300；不可转让。
        assert_eq!(a["reputation"]["applied_events"], json!(2));
        assert_eq!(a["reputation"]["alice"]["reliability_bp"], json!(6_600));
        assert_eq!(a["reputation"]["alice"]["quality_bp"], json!(7_000));
        assert_eq!(a["reputation"]["alice"]["honesty_bp"], json!(5_000));
        assert_eq!(a["reputation"]["alice"]["availability_bp"], json!(6_600));
        assert_eq!(a["reputation"]["alice"]["overall_bp"], json!(6_300));
        assert_eq!(a["reputation"]["transfer_refusal_code"], json!("policy_denied"));
        assert_eq!(a["reputation"]["bridge"]["transferable"], json!(false));
        // 锁定 = 注册质押 100 + 桥出托管 400 = 500。
        assert_eq!(a["conservation"]["locked"], json!(500));
        first.ledger().check_conservation().unwrap();
        second.ledger().check_conservation().unwrap();
    }

    #[test]
    fn self_check_and_results_are_wired_for_the_node() {
        let checks = self_check();
        assert!(au4a_core::all_passed(&checks), "{checks:?}");
        assert!(checks.len() >= 5);
        let results = results_json().unwrap();
        assert_eq!(results["grade"], json!("cpu-proto"));
        assert_eq!(results["real_network"], json!(false));
        assert_eq!(results["all_passed"], json!(true));
        assert_eq!(results["chains"].as_array().map(Vec::len), Some(2));
    }
}
