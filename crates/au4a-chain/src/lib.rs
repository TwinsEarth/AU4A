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
pub mod rgb;
pub mod taproot;
pub mod testnet;

use au4a_core::{AgentKeys, CoreError, CoreResult, Credits, Did, Ledger, RefusalCode, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

pub use bridge::{
    reconcile, BridgeBook, BridgeDirection, BridgeEvent, Reconciliation, BRIDGE_INCONSISTENCY_CODE,
};
pub use rgb::{RgbAdapter, RgbContract, RgbOutcome, Seal, TransferBundle, RGB_REFUSALS, RGB_SUPPORTED};
pub use taproot::{
    leaf_hash, merkle_proof, merkle_root, verify_merkle, ProofStep, TaprootAdapter, TaprootAnchor,
    TaprootOutcome, TAPROOT_REFUSALS, TAPROOT_SUPPORTED,
};
pub use testnet::{
    ChainId, ChainRefusal, ChainTx, Receipt, RefusalSpec, Testnet, ONCHAIN_GRADE,
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
        format!("{TITLE} {RANGE}：RGB 密封转移 + 双轨守恒（v1.8.1，确定性测试网）"),
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
        "scenario": "v1.8.2 RGB 集成 + Taproot Assets 锚定（确定性测试网）",
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
