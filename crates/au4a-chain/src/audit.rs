//! v1.8.10 安全审计：风险清单 + **真跑一遍**的攻击测试。
//!
//! 这一版不写「我们的系统是安全的」这种话，而是把**真实风险**列出来，逐条给出：
//! 状态（已防护 / 部分防护 / **未防护**）、防护手段、以及**残余风险**。
//! 未防护的条目必须留在清单里，不许因为难看就删掉——审计的价值就在这里。
//!
//! [`RISKS`] 是风险登记表，[`attack_suite`] 是能真实执行的攻击用例：每条攻击都断言「被拦住」，
//! 拦不住就在返回值里体现 `blocked: false`（自检会因此变红）。
//!
//! 库代码不 `unwrap`：所有可能失败的步骤都走 `Result<_, String>`，失败会以 `blocked: false`
//! 的形式出现在结果里，而不是 panic。

use au4a_core::{AgentKeys, CoreResult, Credits, Did, Ledger, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bridge::BridgeBook;
use crate::erc8004::Erc8004Adapter;
use crate::reputation::{ChainReputationEvent, ReputationBridge, ReputationEventKind};
use crate::rgb::{RgbAdapter, Seal, TransferBundle};
use crate::routing::{route, RouteAction, RoutingTable, SettlementRequest};
use crate::testnet::{ChainId, ChainTx, Testnet};
use crate::x402::X402Adapter;

/// 风险状态：**未防护也要写出来**。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskStatus {
    /// 有代码防护，并且有测试钉住。
    Protected,
    /// 只挡住了一部分（残余风险明确写在 `residual` 里）。
    PartiallyProtected,
    /// 本轨道没有防护（写清楚为什么、以及真实实现需要什么）。
    Unprotected,
}

impl RiskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RiskStatus::Protected => "protected",
            RiskStatus::PartiallyProtected => "partially_protected",
            RiskStatus::Unprotected => "unprotected",
        }
    }
}

/// 一条风险。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Risk {
    pub id: &'static str,
    pub name: &'static str,
    pub status: RiskStatus,
    /// 代码里的防护手段（没有就写「无」）。
    pub mitigation: &'static str,
    /// 残余风险：防护挡不住的部分，必须诚实写出来。
    pub residual: &'static str,
}

/// 风险登记表（真实清单，含未防护项）。
pub const RISKS: [Risk; 9] = [
    Risk {
        id: "R1",
        name: "重放（replay）",
        status: RiskStatus::Protected,
        mitigation: "交易内容寻址 id + applied∪pending 集合 + 每提交者 nonce 单调；重组后按「链上∪待打包」重建",
        residual: "只在本地测试网生效；跨链重放（同一条消息在两条链上各执行一次）需要域分离与链 ID，本轨道未实现",
    },
    Risk {
        id: "R2",
        name: "双花（double-spend）",
        status: RiskStatus::Protected,
        mitigation: "RGB 封印单次使用（spent 集合）+ 客户端验证 Σ输入==Σ输出；测试网重放检查兜底",
        residual: "封印集合是本地视图；真实 RGB 需要真实 UTXO 集与链上承诺，本原型没有",
    },
    Risk {
        id: "R3",
        name: "最终性回滚（reorg）",
        status: RiskStatus::PartiallyProtected,
        mitigation: "未达最终性不认账（结算/领取/最终化/信誉都检查 is_final）；最终化高度单调冻结，越过即拒绝重组",
        residual: "最终性深度是硬编码参数（BTC 6 / ETH 12），没有真实确认数、没有重组概率模型；真实链的最终性只是概率",
    },
    Risk {
        id: "R4",
        name: "验证数据丢失（validation data loss）",
        status: RiskStatus::Unprotected,
        mitigation: "无（本原型把 leaves / finalized / allocations 放在内存里）",
        residual: "这是 RGB / Taproot Assets 的真实痛点：客户端验证数据一旦丢失，资产就花不出去。需要备份/恢复协议与见证存储，本轨道没有实现，也不声称有",
    },
    Risk {
        id: "R5",
        name: "双轨记账漂移（bridge drift）",
        status: RiskStatus::Protected,
        mitigation: "托管量 == 链上表示总量；require_consistent 在 fail-closed 第一优先级；故障注入测试证明不一致会拦住结算且账本不被改写",
        residual: "托管与链上表示假设 1:1，未处理手续费/部分成交/多资产混合池",
    },
    Risk {
        id: "R6",
        name: "未最终化事件污染信誉",
        status: RiskStatus::Protected,
        mitigation: "信誉桥只接受 is_final 的链上事件，未最终化 → Timeout；同一事件不可重复计入",
        residual: "最终性参数本身是本地参数；测试网一改深度，历史判断就跟着变",
    },
    Risk {
        id: "R7",
        name: "女巫反馈（sybil feedback）",
        status: RiskStatus::PartiallyProtected,
        mitigation: "自评拒绝（policy_denied）、反馈方必须已注册身份、同一客户对同一身份只有一条有效反馈、验证者不可重复背书",
        residual: "注册身份没有成本：攻击者仍可批量注册身份刷分。真实系统需要质押/费用/信誉门槛，本轨道未实现",
    },
    Risk {
        id: "R8",
        name: "签名与私钥（交易授权）",
        status: RiskStatus::Unprotected,
        mitigation: "无：ChainTx 只有内容哈希，没有真实签名验证",
        residual: "真实链上必须验证签名（Ed25519/ECDSA/Schnorr）并支付 gas；本轨道不做签名层，因此「谁有权提交」在本 crate 里不成立",
    },
    Risk {
        id: "R9",
        name: "伪造链上成功",
        status: RiskStatus::Protected,
        mitigation: "证据等级恒为 cpu-proto（收据字段写死）、所有投影 real_network=false、集成测试递归检查每个 grade 字段",
        residual: "如果有人把本 crate 的输出当成真实链上凭证引用（绕过类型），代码层面拦不住——这属于流程风险",
    },
];

/// 风险登记表（切片形式）。
pub fn risk_register() -> &'static [Risk] {
    &RISKS
}

/// 未防护 / 部分防护的风险（审计里最该被看见的部分）。
pub fn unresolved_risks() -> Vec<Risk> {
    RISKS
        .iter()
        .copied()
        .filter(|r| r.status != RiskStatus::Protected)
        .collect()
}

/// 攻击用例结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttackOutcome {
    pub name: String,
    /// `true` = 攻击被拦住（这是期望值）。
    pub blocked: bool,
    pub code: String,
    pub detail: String,
}

fn did(seed: u8) -> Did {
    AgentKeys::from_seed(&[seed; 32]).did()
}

/// 一次「期望被拒绝」的攻击：被拒绝 → 判定是否命中期望拒绝码；竟然成功 → `blocked: false`。
fn expect_refusal<T>(
    name: &str,
    result: Result<T, crate::testnet::ChainRefusal>,
    expected: RefusalCode,
) -> AttackOutcome {
    match result {
        Ok(_) => AttackOutcome {
            name: name.to_string(),
            blocked: false,
            code: "none".to_string(),
            detail: "攻击竟然成功了（这是缺陷，不是防御）".to_string(),
        },
        Err(refusal) => AttackOutcome {
            name: name.to_string(),
            blocked: refusal.code == expected,
            code: refusal.code.as_str().to_string(),
            detail: refusal.detail,
        },
    }
}

/// 准备阶段失败（不是攻击结果，但必须显式暴露，不许 panic）。
fn setup_failed(name: &str, why: String) -> AttackOutcome {
    AttackOutcome {
        name: name.to_string(),
        blocked: false,
        code: "setup_error".to_string(),
        detail: why,
    }
}

/// A1/A2/A3：测试网层面的重放、nonce 回退、篡改。
fn attacks_on_the_testnet() -> Result<Vec<AttackOutcome>, String> {
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let tx = ChainTx::new(ChainId::BtcRegtest, "rgb.issue", &did(1), 1, json!({ "a": 1 }))
        .map_err(|e| e.to_string())?;
    net.accept(&tx).map_err(|r| r.detail)?;

    let mut out = Vec::new();
    out.push(expect_refusal(
        "replay_same_tx",
        net.accept(&tx),
        RefusalCode::Conflict,
    ));
    let stale = ChainTx::new(ChainId::BtcRegtest, "rgb.issue", &did(1), 1, json!({ "a": 2 }))
        .map_err(|e| e.to_string())?;
    out.push(expect_refusal(
        "nonce_rollback",
        net.accept(&stale),
        RefusalCode::StaleEpoch,
    ));
    let mut tampered = tx.clone();
    tampered.payload = json!({ "a": 999 });
    out.push(expect_refusal(
        "tampered_tx",
        net.accept(&tampered),
        RefusalCode::Malformed,
    ));
    Ok(out)
}

/// A4：RGB 封印双花。
fn attack_rgb_double_spend() -> Result<AttackOutcome, String> {
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let mut rgb = RgbAdapter::new();
    let issue = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.issue",
        &did(2),
        1,
        json!({ "ticker": "T", "supply": 100 }),
    )
    .map_err(|e| e.to_string())?;
    rgb.execute(&mut net, &issue).map_err(|r| r.detail)?;
    net.mine();
    let asset_id = rgb
        .contract()
        .map_err(|e| e.to_string())?
        .asset_id
        .clone();
    let seal = Seal::new(issue.id.clone(), 0);
    let first = TransferBundle::new(
        &asset_id,
        vec![seal.clone()],
        vec![(Seal::new("o1", 0), Credits(100))],
        "b",
    )
    .map_err(|e| e.to_string())?;
    let t1 = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.transfer",
        &did(2),
        2,
        serde_json::to_value(&first).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    rgb.execute(&mut net, &t1).map_err(|r| r.detail)?;
    net.mine();
    let second = TransferBundle::new(
        &asset_id,
        vec![seal],
        vec![(Seal::new("o2", 0), Credits(100))],
        "b",
    )
    .map_err(|e| e.to_string())?;
    let t2 = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.transfer",
        &did(2),
        3,
        serde_json::to_value(&second).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(expect_refusal(
        "rgb_seal_double_spend",
        rgb.execute(&mut net, &t2),
        RefusalCode::Conflict,
    ))
}

/// A5：回滚已最终化的块。
fn attack_reorg_below_finality() -> Result<AttackOutcome, String> {
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let tx = ChainTx::new(ChainId::BtcRegtest, "rgb.transfer", &did(3), 1, json!({}))
        .map_err(|e| e.to_string())?;
    net.accept(&tx).map_err(|r| r.detail)?;
    net.mine();
    let target = net.height() + net.finality_depth();
    net.mine_to(target);
    Ok(expect_refusal(
        "reorg_below_finality",
        net.reorg(5),
        RefusalCode::Conflict,
    ))
}

/// A6：双轨不一致时硬走结算（决策层 + 执行层双重检查）。
fn attack_settle_on_inconsistent_tracks() -> Result<AttackOutcome, String> {
    let who = did(4);
    let mut ledger = Ledger::new();
    ledger.mint(&who, Credits(5_000)).map_err(|e| e.to_string())?;
    let mut book = BridgeBook::new();
    book.bridge_out(&mut ledger, &who, Credits(50), "rail:rgb", "rgb")
        .map_err(|e| e.to_string())?;
    book.force_chain_supply(Credits(123_456)); // 攻击：链上侧多报
    let table = RoutingTable::default_table();
    let request =
        SettlementRequest::new(&who, &did(5), 1_000).map_err(|e| e.to_string())?;
    let gate_deferred = route(&request, &table, false)
        .map(|d| d.action == RouteAction::Defer)
        .unwrap_or(false);
    let settle_refused = crate::routing::settle(&mut ledger, &mut book, &request, &table).is_err();
    Ok(AttackOutcome {
        name: "settle_on_inconsistent_tracks".to_string(),
        blocked: gate_deferred && settle_refused,
        code: "invalid_kind".to_string(),
        detail: format!(
            "决策层缓办 = {gate_deferred}，执行层拒绝 = {settle_refused}，账本锁定仍为 {}",
            ledger.balance(&who).locked
        ),
    })
}

/// A7：自评刷分。
fn attack_self_feedback() -> Result<AttackOutcome, String> {
    let mut net = Testnet::new(ChainId::EthLocal, 1);
    let mut erc = Erc8004Adapter::new();
    for (seed, nonce) in [(6u8, 1u64), (7, 1)] {
        let t = ChainTx::new(ChainId::EthLocal, "erc8004.register", &did(seed), nonce, json!({}))
            .map_err(|e| e.to_string())?;
        erc.execute(&mut net, &t).map_err(|r| r.detail)?;
    }
    let id = erc
        .identity_of_did(&did(6))
        .map(|i| i.agent_id.clone())
        .ok_or_else(|| "身份注册后查不到".to_string())?;
    let selfish = ChainTx::new(
        ChainId::EthLocal,
        "erc8004.give_feedback",
        &did(6),
        2,
        json!({ "agent_id": id, "score_bp": 10_000 }),
    )
    .map_err(|e| e.to_string())?;
    Ok(expect_refusal(
        "self_feedback_sybil",
        erc.execute(&mut net, &selfish),
        RefusalCode::PolicyDenied,
    ))
}

/// A8/A9：未最终化事件污染信誉、信誉转让。
fn attacks_on_reputation() -> Result<Vec<AttackOutcome>, String> {
    let mut fresh = Testnet::new(ChainId::EthLocal, 5);
    fresh.mine_to(1);
    let mut trust = ReputationBridge::new();
    let event = ChainReputationEvent::new(&did(8), ReputationEventKind::Feedback, 10_000, 9)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    out.push(expect_refusal(
        "non_final_event_into_reputation",
        trust.apply_event(&fresh, &event),
        RefusalCode::Timeout,
    ));
    out.push(expect_refusal::<()>(
        "transfer_reputation",
        trust.execute("reputation.transfer"),
        RefusalCode::PolicyDenied,
    ));
    Ok(out)
}

/// A10：x402 重复支付。
fn attack_x402_double_payment() -> Result<AttackOutcome, String> {
    let mut net = Testnet::new(ChainId::EthLocal, 1);
    let mut x402 = X402Adapter::new();
    let invoice = ChainTx::new(
        ChainId::EthLocal,
        "x402.invoice",
        &did(9),
        1,
        json!({ "amount": 10, "asset": "usdc", "expires_at": 100 }),
    )
    .map_err(|e| e.to_string())?;
    x402.execute(&mut net, &invoice).map_err(|r| r.detail)?;
    let invoice_id = x402
        .last_invoice()
        .map(|i| i.id.clone())
        .ok_or_else(|| "开票后查不到发票".to_string())?;
    let pay = ChainTx::new(
        ChainId::EthLocal,
        "x402.pay",
        &did(10),
        1,
        json!({ "invoice_id": invoice_id, "amount": 10, "now": 1 }),
    )
    .map_err(|e| e.to_string())?;
    x402.execute(&mut net, &pay).map_err(|r| r.detail)?;
    Ok(expect_refusal(
        "x402_double_payment",
        x402.execute(&mut net, &pay),
        RefusalCode::Conflict,
    ))
}

/// 真跑一遍攻击：每一条都尝试一次真实攻击，记录是否被拦住。
///
/// 只要有一条 `blocked == false`（或准备阶段出错），[`crate::self_check`] 的
/// `audit.attack_suite` 就会失败。库代码不 panic：准备失败会变成 `setup_error` 结果。
pub fn attack_suite() -> Vec<AttackOutcome> {
    let mut out = Vec::new();
    push_results(&mut out, attacks_on_the_testnet(), "attacks_on_the_testnet");
    push_one(&mut out, attack_rgb_double_spend(), "rgb_seal_double_spend");
    push_one(&mut out, attack_reorg_below_finality(), "reorg_below_finality");
    push_one(
        &mut out,
        attack_settle_on_inconsistent_tracks(),
        "settle_on_inconsistent_tracks",
    );
    push_one(&mut out, attack_self_feedback(), "self_feedback_sybil");
    push_results(&mut out, attacks_on_reputation(), "attacks_on_reputation");
    push_one(&mut out, attack_x402_double_payment(), "x402_double_payment");
    out
}

fn push_results(
    out: &mut Vec<AttackOutcome>,
    results: Result<Vec<AttackOutcome>, String>,
    what: &str,
) {
    match results {
        Ok(items) => out.extend(items),
        Err(why) => out.push(setup_failed(what, why)),
    }
}

fn push_one(out: &mut Vec<AttackOutcome>, result: Result<AttackOutcome, String>, what: &str) {
    match result {
        Ok(item) => out.push(item),
        Err(why) => out.push(setup_failed(what, why)),
    }
}

/// 审计报告：风险登记表 + 攻击结果 + 未防护清单。
pub fn audit_report() -> Value {
    let attacks = attack_suite();
    let all_blocked = attacks.iter().all(|a| a.blocked) && !attacks.is_empty();
    json!({
        "risks": RISKS,
        "unresolved": unresolved_risks(),
        "attacks": attacks,
        "all_attacks_blocked": all_blocked,
        "unresolved_count": unresolved_risks().len(),
        "grade": crate::testnet::ONCHAIN_GRADE.as_str(),
        "scope": "本审计只覆盖 au4a-chain 的确定性测试网原型；不含真实链、真实签名、真实网络",
    })
}

/// 审计报告（`CoreResult` 形态，方便自检直接 `?`）。
pub fn audit_report_result() -> CoreResult<Value> {
    Ok(audit_report())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_risk_register_names_real_risks_and_keeps_unprotected_ones() {
        let risks = risk_register();
        assert!(risks.len() >= 8);
        let names: Vec<&str> = risks.iter().map(|r| r.name).collect();
        for needle in ["重放", "双花", "最终性", "验证数据丢失", "女巫", "签名"] {
            assert!(
                names.iter().any(|n| n.contains(needle)),
                "风险清单必须包含 {needle}：{names:?}"
            );
        }
        // 未防护 / 部分防护必须留在清单里（不许粉饰）。
        let unresolved = unresolved_risks();
        assert!(unresolved.len() >= 3, "未防护项应如实保留：{unresolved:?}");
        assert!(unresolved
            .iter()
            .any(|r| r.name.contains("验证数据丢失") && r.status == RiskStatus::Unprotected));
        assert!(unresolved
            .iter()
            .any(|r| r.name.contains("签名") && r.status == RiskStatus::Unprotected));
        // 每条都要有可读的残余风险说明。
        for r in risks {
            assert!(!r.id.is_empty() && !r.mitigation.is_empty() && !r.residual.is_empty());
        }
    }

    #[test]
    fn every_attack_in_the_suite_is_blocked() {
        let attacks = attack_suite();
        assert!(attacks.len() >= 9, "攻击用例应覆盖主要风险面");
        for attack in &attacks {
            assert!(
                attack.blocked,
                "攻击 {} 未被拦住（code={}，detail={}）",
                attack.name, attack.code, attack.detail
            );
            assert!(!attack.detail.is_empty());
        }
    }

    #[test]
    fn the_attack_suite_is_deterministic() {
        let a = attack_suite();
        let b = attack_suite();
        assert_eq!(a, b, "同一套攻击必须给出同样的结果");
    }

    #[test]
    fn the_report_is_float_free_and_declares_its_scope() {
        let report = audit_report();
        au4a_core::canonicalize(&report).unwrap();
        assert_eq!(report["all_attacks_blocked"], json!(true));
        assert_eq!(report["grade"], json!("cpu-proto"));
        assert!(report["scope"].as_str().unwrap_or_default().contains("确定性测试网"));
        assert!(report["unresolved_count"].as_u64().unwrap_or(0) >= 3);
        assert!(audit_report_result().is_ok());
    }

    #[test]
    fn risk_statuses_round_trip_through_names() {
        for status in [
            RiskStatus::Protected,
            RiskStatus::PartiallyProtected,
            RiskStatus::Unprotected,
        ] {
            let text = serde_json::to_string(&status).unwrap();
            let back: RiskStatus = serde_json::from_str(&text).unwrap();
            assert_eq!(back, status);
            assert!(!status.as_str().is_empty());
        }
    }
}
