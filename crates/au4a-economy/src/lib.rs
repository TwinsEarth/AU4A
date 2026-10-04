//! AU4A 轨道 1.4 — Economic Autonomy 经济自主（v1.4.1 → v1.4.10）。
//!
//! 经济自主：Agent 自己管理余额、自己定价、自己质押、自己参与争议仲裁、自己决定兑换路由，
//! 收益归属资源提供者（人类只收收益、不决策），监控是只读投影。
//!
//! 三条贯穿全轨道的规则：
//!
//! 1. **一切账务走 [`au4a_core::Ledger`]**，每个写路径末尾断言
//!    `Σ可用 + Σ锁定 + 罚没 == 发行`（[`au4a_core::Ledger::check_conservation`]）。
//! 2. **一切金额与比率是整数**（微积分 / 基点），规范 JSON 拒绝浮点，因此结果可复现。
//! 3. **证据分级是结算闸门**：[`au4a_core::EvidenceGrade::Unverified`] 永远不可结算，
//!    链上执行属于 v1.8，本轨道只做**路由决策与账务**，绝不伪造链上成功。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责，并留下自检与证据。
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）。

pub mod balance;

use au4a_core::{AgentKeys, CoreError, CoreResult, Credits, Did, Ledger, RefusalCode, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

pub use balance::{AccountDelta, BalanceManager, BalancePolicy, BalanceReport, SpendVerdict};

/// 轨道号。
pub const TRACK: &str = "1.4";
/// 轨道标题。
pub const TITLE: &str = "Economic Autonomy 经济自主";
/// 版本区间。
pub const RANGE: &str = "v1.4.1 → v1.4.10";
/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_economy";

/// 场景/自检用的确定性身份种子（同一个种子永远给出同一把密钥与同一个 DID）。
fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

/// 把一次判定转成带**真实数字**的自检项：失败时写明期望值与实际值，而不是「出错了」。
fn check(name: &str, f: impl FnOnce() -> Result<String, String>) -> SelfCheck {
    match f() {
        Ok(detail) => SelfCheck::pass(TRACK, name, detail),
        Err(detail) => SelfCheck::fail(TRACK, name, detail),
    }
}

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
///
/// 每一项都是**真跑一遍**：在临时账本上执行真实写路径，然后断言具体数值与守恒。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = vec![SelfCheck::pass(
        TRACK,
        "track.wired",
        format!("{TITLE} {RANGE} 已接入 au4a-node"),
    )];

    checks.push(check("balance.conservation", || {
        let (a, b) = (agent(1).did(), agent(2).did());
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).map_err(|e| e.to_string())?;
        let policy = BalancePolicy {
            reserve_floor: Credits(100),
            stake_target_bp: 2_000,
            max_spend_bp: 5_000,
        };
        let left = balance::spend(&mut ledger, &a, &b, Credits(300), &policy)
            .map_err(|e| format!("支出 300 被拒绝：{e}"))?;
        let locked = balance::autostake(&mut ledger, &a, &policy, Credits(1_000))
            .map_err(|e| format!("自主质押被拒绝：{e}"))?;
        balance::assert_conserved(&ledger).map_err(|e| format!("守恒断言失败：{e}"))?;
        Ok(format!(
            "支出 300 后 a 可用额度 {left}，自主锁定 {locked}；Σ可用 {} + 罚没 {} == 发行 {}",
            ledger.total().map_err(|e| e.to_string())?,
            ledger.slashed(),
            ledger.minted()
        ))
    }));

    checks.push(check("balance.reserve_floor", || {
        let a = agent(3).did();
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(150)).map_err(|e| e.to_string())?;
        let policy = BalancePolicy {
            reserve_floor: Credits(100),
            stake_target_bp: 0,
            max_spend_bp: 10_000,
        };
        let verdict = balance::check_spend(&ledger, &a, Credits(60), &policy)
            .map_err(|e| e.to_string())?;
        if verdict != SpendVerdict::BelowReserve {
            return Err(format!("期望 below_reserve，实际 {}", verdict.as_str()));
        }
        let refused = balance::spend(&mut ledger, &a, &did_of(4), Credits(60), &policy);
        if refused.is_ok() {
            return Err("穿过运营底线的支出被错误地放行".to_string());
        }
        if ledger.balance(&a).available != Credits(150) {
            return Err(format!(
                "拒绝后账本被改动：可用 = {}",
                ledger.balance(&a).available
            ));
        }
        balance::assert_conserved(&ledger).map_err(|e| e.to_string())?;
        Ok(format!(
            "可用 150、底线 100：支出 60 判定 {}，拒绝后可用仍为 {}，守恒成立",
            verdict.as_str(),
            ledger.balance(&a).available
        ))
    }));

    checks
}

/// 自检内部使用的小工具：从种子得到 DID（不暴露为公共 API）。
fn did_of(seed: u8) -> Did {
    agent(seed).did()
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
        "modules": ["balance"],
        "invariants": [
            "Σ可用 + Σ锁定 + 罚没 == 发行",
            "整数微积分与基点运算，规范 JSON 禁浮点",
            "Unverified 证据永不结算",
        ],
    }))
}

/// 幂等注册：共享内核里可能已有别的轨道注册过同一个种子，重复注册不算失败。
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

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
/// 每一版迭代都应该让这里多做一件真实的事，而不是多打印一行字。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!("{TITLE} {RANGE}：余额管理（v1.4.1）"),
    );

    let seller_keys = agent(41);
    let buyer_keys = agent(42);
    let seller = ensure_registered(
        kernel,
        &seller_keys,
        "economy.seller",
        &["translate.en-zh"],
        Credits(100),
    )?;
    let buyer = ensure_registered(
        kernel,
        &buyer_keys,
        "economy.buyer",
        &["consume.translation"],
        Credits(100),
    )?;
    kernel.emit("economy.registered", "2 个 Agent 自证身份并自带质押完成注册");

    // 每个 Agent 自己申报策略——没有任何外部设定入口。
    let mut book = BalanceManager::new();
    let policy = BalancePolicy {
        reserve_floor: Credits(100),
        stake_target_bp: 2_000,
        max_spend_bp: 5_000,
    };
    book.declare(&seller, policy)?;
    book.declare(&buyer, policy)?;

    let before = kernel.ledger().view();
    let price = Credits(300);
    let buyer_left = book.spend(kernel.ledger_mut(), &buyer, &seller, price)?;
    kernel.emit(
        "economy.paid",
        format!("买方自主支付 {price} 微积分，剩余可用额度 {buyer_left}"),
    );

    // 被拒绝的一步：穿过运营底线的支出必须被挡住，并留下类型化拒绝记录。
    let overreach = Credits(900);
    let verdict = book.check(kernel.ledger(), &buyer, overreach)?;
    if verdict.allowed() {
        return Err(CoreError::InvalidKind);
    }
    kernel.refuse(
        &buyer,
        RefusalCode::PolicyDenied,
        format!("支出 {overreach} 被余额策略拒绝：{}", verdict.as_str()),
    );
    kernel.emit("economy.refused", format!("{}：{}", verdict.as_str(), overreach));

    // 自主补足质押：不穿过底线，也不超过预算。
    let locked = book.autostake(kernel.ledger_mut(), &seller, Credits(1_000))?;
    kernel.emit("economy.staked", format!("卖方自主锁定 {locked} 微积分质押"));

    // 守恒断言 + 余额变动投影。
    balance::assert_conserved(kernel.ledger())?;
    let after = kernel.ledger().view();
    let row = |did: &Did| -> CoreResult<Value> {
        let key = did.as_str().to_string();
        let b = before.accounts.get(&key).cloned().unwrap_or_default();
        let a = after.accounts.get(&key).cloned().unwrap_or_default();
        let d = balance::delta(&b, &a);
        Ok(json!({
            "did": key,
            "available": a.available,
            "locked": a.locked,
            "delta": { "available": d.available, "locked": d.locked, "total": d.total },
        }))
    };

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "scenario": "v1.4.1 余额管理",
        "agents": [row(&seller)?, row(&buyer)?],
        "settled": [{ "from": buyer.as_str(), "to": seller.as_str(), "amount": price, "gate": "ledger-direct" }],
        "refusals": [{ "code": RefusalCode::PolicyDenied.as_str(), "verdict": verdict.as_str(), "amount": overreach }],
        "staked": locked,
        "conservation": {
            "ok": true,
            "minted": after.minted,
            "slashed": after.slashed,
            "total": after.total,
        },
        "events": kernel.observe().progress.len(),
    }))
}
