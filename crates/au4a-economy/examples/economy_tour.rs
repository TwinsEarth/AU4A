//! v1.4.9 示例：经济自主九步走（可运行、确定性、无网络 / 无文件 / 无墙钟）。
//!
//! 运行：
//!
//! ```powershell
//! $env:CARGO_TARGET_DIR = "E:\DS\_forangent\target\au4a-economy"
//! cargo run -p au4a-economy --example economy_tour
//! ```
//!
//! 每一步都直接调用本 crate 的 public API 并打印**真实数字**（不是说明文字），
//! 最后跑一遍 `scenario` 输出完整 JSON 摘要，并用只读收益面板（v1.4.10）收尾。
//! 同一个种子永远给出同样的输出。

use au4a_core::{AgentKeys, Credits, Did, EvidenceGrade, Ledger};
use au4a_economy::arbitration::{ArbitrationTerms, Court};
use au4a_economy::balance::{check_spend, spend, BalanceManager, BalancePolicy};
use au4a_economy::fx::{route, ExchangeRequest, RouteTable, Urgency};
use au4a_economy::pricing::{quote, PriceInputs, PriceKnobs};
use au4a_economy::settlement::{
    split_weights, Beneficiary, BeneficiaryKind, ProviderRole, Receipt, RevenueBook, RevenueShare,
};
use au4a_economy::stake::{StakeBook, StakeTerms};
use au4a_kernel::{Kernel, KernelConfig};

fn did(seed: u8) -> Did {
    AgentKeys::from_seed(&[seed; 32]).did()
}

fn main() -> au4a_core::CoreResult<()> {
    println!("AU4A 轨道 1.4 — Economic Autonomy 经济自主（v1.4.1 → v1.4.10）");
    println!("说明：所有金额是整数微积分，所有比率是基点（1bp = 0.01%），无浮点。\n");

    // ── 第 1 步（v1.4.1）余额管理：Agent 自己申报策略，自己判定、自己支出 ──────────────
    let alice = did(1);
    let bob = did(2);
    let mut ledger = Ledger::new();
    ledger.mint(&alice, Credits(1_000))?;
    let mut balances = BalanceManager::new();
    let policy = BalancePolicy {
        reserve_floor: Credits(100),
        stake_target_bp: 2_000,
        max_spend_bp: 5_000,
    };
    balances.declare(&alice, policy)?;
    let verdict = check_spend(&ledger, &alice, Credits(600), &policy)?;
    let left = spend(&mut ledger, &alice, &bob, Credits(300), &policy)?;
    let locked = balances.autostake(&mut ledger, &alice, Credits(1_000))?;
    println!(
        "[1/9 v1.4.1 余额] 支出 600 判定={}（越单笔上限）；支出 300 成功，剩余额度 {left}，自主质押 {locked}",
        verdict.as_str()
    );

    // ── 第 2 步（v1.4.2）自主定价：信誉 / 稀缺度 / 负载 ────────────────────────────
    let base = Credits(400);
    let idle = quote(&PriceInputs::idle(base), &PriceKnobs::DEFAULT)?;
    let busy = quote(
        &PriceInputs {
            base_price: base,
            reputation_bp: 8_500,
            scarcity_bp: 3_000,
            load_bp: 2_000,
        },
        &PriceKnobs::DEFAULT,
    )?;
    println!(
        "[2/9 v1.4.2 定价] 空闲={} → 高信誉+稀缺+半载={}（乘数 {}bp：折扣 {} / 稀缺 +{} / 负载 +{}）",
        idle.unit_price,
        busy.unit_price,
        busy.multiplier_bp,
        busy.components.reputation_discount_bp,
        busy.components.scarcity_premium_bp,
        busy.components.load_premium_bp
    );

    // ── 第 3 步（v1.4.3）兑换路由：金额 / 时效 / 费用阈值，绝不伪造链上成功 ──────────
    let table = RouteTable::DEFAULT;
    let ask = |amount: i64, slack: u64| ExchangeRequest {
        from: alice.clone(),
        amount: Credits(amount),
        urgency: Urgency::Standard,
        slack_ticks: slack,
        preferred: None,
    };
    let small = route(&ask(150, 50), &table)?;
    let tight = route(&ask(400, 12), &table)?;
    let ok = route(&ask(400, 30), &table)?;
    println!(
        "[3/9 v1.4.3 兑换] 150→{}（{}）；400/剩余12→{}（{}）；400/剩余30→{} 走 {} 费用 {} 到账 {}，链上已执行={}",
        small.action.as_str(),
        small.reason.as_str(),
        tight.action.as_str(),
        tight.reason.as_str(),
        ok.action.as_str(),
        ok.venue.as_str(),
        ok.fee,
        ok.net,
        ok.chain_execution.executed()
    );

    // ── 第 4 步（v1.4.4）质押：自主质押 → 解质押 → 冷静期到点释放 ──────────────────
    let terms = StakeTerms::DEFAULT;
    let mut stakes = StakeBook::new();
    stakes.adopt(&ledger, &terms, &alice, Credits(100))?;
    let unbond = stakes.request_unstake(&terms, &alice, Credits(40), 10)?;
    let early = stakes.release_matured(&mut ledger, &alice, unbond.release_at - 1)?;
    let released = stakes.release_matured(&mut ledger, &alice, unbond.release_at)?;
    stakes.assert_consistent(&ledger)?;
    println!(
        "[4/9 v1.4.4 质押] 解质押 40（{} → 到点 {}）：提前释放 {early}、到点释放 {released}，质押簿与账本一致",
        unbond.requested_at, unbond.release_at
    );

    // ── 第 5 步（v1.4.5）争议仲裁：罚没上限 = 锁定余额，且可申诉 ──────────────────
    let respondent = did(3);
    let mut court = Court::new();
    let mut case_ledger = Ledger::new();
    case_ledger.mint(&respondent, Credits(1_000))?;
    case_ledger.lock(&respondent, Credits(50))?;
    let case = court.open(
        &bob,
        &respondent,
        Credits(1_000_000),
        EvidenceGrade::Verified,
        1,
    )?;
    court.vote(&case.id, &did(4), true, 6_000)?;
    court.vote(&case.id, &did(5), true, 4_000)?;
    let arbi_terms = ArbitrationTerms::DEFAULT;
    let ruling = court.rule(&mut case_ledger, &case.id, &arbi_terms, 2)?;
    let appeal = court.appeal(&case.id, &respondent, "new evidence", &arbi_terms, 3)?;
    println!(
        "[5/9 v1.4.5 仲裁] 索赔 1000000、锁定 50 → 罚没 {}（上限来源 {}，支持票权 {}bp）；败方申诉第 {} 轮",
        ruling.slashed,
        ruling.cap.as_str(),
        ruling.uphold_bp,
        appeal.round
    );

    // ── 第 6 步（v1.4.6）结算路由与收益归属：整数最大余数法分成 ────────────────────
    let parts = split_weights(Credits(10), &[3_333, 3_333, 3_334])?;
    let provider = did(6);
    let owner = Beneficiary::human_operator(did(7));
    let mut revenue = RevenueBook::new();
    revenue.record_receipt(Receipt {
        beneficiary: owner.did.as_str().to_string(),
        amount: Credits(60),
        share_bp: 3_000,
        role: ProviderRole::Data,
        kind: BeneficiaryKind::HumanOperator,
        at: 1,
    });
    let shares = [
        RevenueShare {
            beneficiary: provider.clone(),
            share_bp: 7_000,
            role: ProviderRole::Compute,
            kind: BeneficiaryKind::Agent,
        },
        RevenueShare {
            beneficiary: owner.did.clone(),
            share_bp: 3_000,
            role: ProviderRole::Data,
            kind: BeneficiaryKind::HumanOperator,
        },
    ];
    println!(
        "[6/9 v1.4.6 结算] 10 按 3333/3333/3334 拆成 {:?}；分成表 {} 条；人类操作者决策权={:?}、已收 {}",
        parts.iter().map(|c| c.get()).collect::<Vec<_>>(),
        shares.len(),
        owner.decision_rights(),
        revenue.earned(&owner.did)?
    );

    // ── 第 7 步（v1.4.7）不变量：账本守恒 + 质押簿不变式 ───────────────────────────
    ledger.check_conservation()?;
    case_ledger.check_conservation()?;
    println!(
        "[7/9 v1.4.7 不变量] 演示账本 Σ可用={} 罚没={} 发行={}；仲裁账本罚没={}；两本账均守恒",
        ledger.total()?,
        ledger.slashed(),
        ledger.minted(),
        case_ledger.slashed()
    );

    // ── 第 8 步（v1.4.8 文档 + 自检）：节点 verify 聚合的自检项 ────────────────────
    let checks = au4a_economy::self_check();
    let passed = checks.iter().filter(|c| c.passed).count();
    println!(
        "[8/9 v1.4.8 自检] {} 项自检 {passed} 项通过（{}）",
        checks.len(),
        au4a_economy::TITLE
    );
    for check in &checks {
        println!("        - {:<28} {}", check.name, check.detail);
    }

    let mut kernel = Kernel::new(KernelConfig::default());
    let summary = au4a_economy::scenario(&mut kernel)?;

    // ── 第 9 步（v1.4.10）监控：只读收益面板（节点 /api/revenue 的超集） ──────────
    let panel = au4a_economy::monitor::revenue_panel(&kernel, &revenue, &summary)?;
    println!(
        "[9/9 v1.4.10 监控] 只读面板：账户 {} 个、注册 Agent {} 个、发行 {}、罚没 {}、收益 {}、守恒={}",
        panel["ledger"]["account_count"],
        panel["agents"],
        panel["ledger"]["minted"],
        panel["ledger"]["slashed"],
        panel["total_earned"],
        panel["ledger"]["conservation_ok"]
    );
    println!(
        "        账本：Σ可用={} Σ锁定={} 罚没={} ⇒ 发行={}（read_only={}）",
        panel["ledger"]["available"],
        panel["ledger"]["locked"],
        panel["ledger"]["slashed"],
        panel["ledger"]["minted"],
        panel["read_only"]
    );
    for check in au4a_economy::monitor::monitor_checks(&kernel, &revenue) {
        println!("        - {:<28} {}", check.name, check.detail);
    }

    println!("\n场景摘要 JSON：");
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).unwrap_or_default()
    );

    Ok(())
}
