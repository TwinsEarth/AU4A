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
pub mod fx;
pub mod pricing;

use au4a_core::{AgentKeys, CoreError, CoreResult, Credits, Did, Envelope, Ledger, RefusalCode, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

pub use balance::{AccountDelta, BalanceManager, BalancePolicy, BalanceReport, SpendVerdict};
pub use fx::{
    ChainExecution, DecisionReason, ExchangeBook, ExchangeIntent, ExchangeRequest, IntentStatus,
    RouteAction, RoutePlan, RouteTable, Urgency, Venue,
};
pub use pricing::{unit_price, PriceComponents, PriceInputs, PriceKnobs, PriceQuote};

/// 轨道号。
pub const TRACK: &str = "1.4";
/// 轨道标题。
pub const TITLE: &str = "Economic Autonomy 经济自主";
/// 版本区间。
pub const RANGE: &str = "v1.4.1 → v1.4.10";
/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_economy";

/// 报价公告的消息类型（后续轨道可扩展，冻结基元不因此改动）。
pub const PRICE_KIND: &str = "economy.price";

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

    checks.push(check("pricing.reproducible", || {
        let inputs = PriceInputs {
            base_price: Credits(1_000_000),
            reputation_bp: 8_000,
            scarcity_bp: 2_000,
            load_bp: 4_000,
        };
        let a = pricing::quote(&inputs, &PriceKnobs::DEFAULT).map_err(|e| e.to_string())?;
        let b = pricing::quote(&inputs, &PriceKnobs::DEFAULT).map_err(|e| e.to_string())?;
        if a != b || a.fingerprint().map_err(|e| e.to_string())? != b.fingerprint().map_err(|e| e.to_string())? {
            return Err("同一输入给出了不同报价".to_string());
        }
        if a.multiplier_bp != 11_000 || a.unit_price != Credits(1_100_000) {
            return Err(format!(
                "钉住的数值不符：multiplier={} price={}",
                a.multiplier_bp, a.unit_price
            ));
        }
        Ok(format!(
            "信誉 8000 / 稀缺 2000 / 负载 4000 → 乘数 {}bp、单价 {}，两次报价与内容哈希一致",
            a.multiplier_bp, a.unit_price
        ))
    }));

    checks.push(check("pricing.monotonicity", || {
        let knobs = PriceKnobs::DEFAULT;
        let base = Credits(1_000_000);
        let mut load_prev = pricing::unit_price(&PriceInputs::idle(base), &knobs).map_err(|e| e.to_string())?;
        let mut rep_prev = load_prev;
        for bp in (0..=10_000i64).step_by(250) {
            let with_load = pricing::unit_price(
                &PriceInputs {
                    base_price: base,
                    reputation_bp: 0,
                    scarcity_bp: 0,
                    load_bp: bp,
                },
                &knobs,
            )
            .map_err(|e| e.to_string())?;
            if with_load < load_prev {
                return Err(format!("负载 {bp} 时价格下降：{load_prev} → {with_load}"));
            }
            load_prev = with_load;
            let with_rep = pricing::unit_price(
                &PriceInputs {
                    base_price: base,
                    reputation_bp: bp,
                    scarcity_bp: 0,
                    load_bp: 0,
                },
                &knobs,
            )
            .map_err(|e| e.to_string())?;
            if with_rep > rep_prev {
                return Err(format!("信誉 {bp} 时价格上升：{rep_prev} → {with_rep}"));
            }
            rep_prev = with_rep;
        }
        Ok(format!(
            "负载 0→10000 单调不降（{load_prev} 为满载价），信誉 0→10000 单调不增（{rep_prev} 为满信誉价）"
        ))
    }));

    checks.push(check("fx.decision_table", || {
        let table = fx::RouteTable::DEFAULT;
        let ask = |amount: i64, slack: u64| fx::ExchangeRequest {
            from: agent(9).did(),
            amount: Credits(amount),
            urgency: fx::Urgency::Standard,
            slack_ticks: slack,
            preferred: None,
        };
        let small = fx::route(&ask(150, 100), &table).map_err(|e| e.to_string())?;
        let tight = fx::route(&ask(400, 12), &table).map_err(|e| e.to_string())?;
        let ok = fx::route(&ask(400, 30), &table).map_err(|e| e.to_string())?;
        if small.action != fx::RouteAction::KeepInternal
            || small.reason != fx::DecisionReason::BelowOnchainMinimum
        {
            return Err(format!(
                "小额应内部结算，实际 {}/{}",
                small.action.as_str(),
                small.reason.as_str()
            ));
        }
        if tight.action != fx::RouteAction::Defer
            || tight.reason != fx::DecisionReason::DeadlineTooTight
        {
            return Err(format!(
                "时效不足应缓办，实际 {}/{}",
                tight.action.as_str(),
                tight.reason.as_str()
            ));
        }
        if ok.action != fx::RouteAction::RouteOnchain
            || ok.venue != fx::Venue::Ethereum
            || ok.fee != Credits(3)
            || ok.net != Credits(397)
        {
            return Err(format!(
                "可执行路由应走 ETH（3bp 费用），实际 {}/{} fee={} net={}",
                ok.action.as_str(),
                ok.venue.as_str(),
                ok.fee,
                ok.net
            ));
        }
        Ok(format!(
            "150 → {}（{}）；400/剩余 12 → {}（{}）；400/剩余 30 → {} 走 {} 费用 {} 到账 {}",
            small.action.as_str(),
            small.reason.as_str(),
            tight.action.as_str(),
            tight.reason.as_str(),
            ok.action.as_str(),
            ok.venue.as_str(),
            ok.fee,
            ok.net
        ))
    }));

    checks.push(check("fx.no_fake_chain", || {
        let who = agent(10).did();
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(10_000)).map_err(|e| e.to_string())?;
        let plan = fx::route(
            &fx::ExchangeRequest {
                from: who.clone(),
                amount: Credits(1_000),
                urgency: fx::Urgency::Standard,
                slack_ticks: 100,
                preferred: None,
            },
            &fx::RouteTable::DEFAULT,
        )
        .map_err(|e| e.to_string())?;
        let intent = fx::escrow_for_route(&mut ledger, &plan, 1).map_err(|e| e.to_string())?;
        if intent.on_chain_success() || intent.chain_execution.executed() {
            return Err("兑换意图错误地声称链上成功".to_string());
        }
        let names: Vec<&str> = fx::ChainExecution::ALL.iter().map(|c| c.as_str()).collect();
        if names.contains(&"executed") || fx::IntentStatus::ALL.len() != 1 {
            return Err("类型层出现了「已上链」的表达".to_string());
        }
        let back = fx::cancel_intent(&mut ledger, &intent).map_err(|e| e.to_string())?;
        ledger
            .check_conservation()
            .map_err(|e| format!("守恒断言失败：{e}"))?;
        Ok(format!(
            "预留 {back} 并撤回，状态 {}，链上执行 {}（真实执行属 v1.8），全程守恒",
            intent.status.as_str(),
            intent.chain_execution.as_str()
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
        "modules": ["balance", "pricing", "fx"],
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

/// 一条公告（场景里 Agent 广播的自有报价）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Announcement {
    provider: String,
    skill: String,
    unit_price: i64,
    multiplier_bp: i64,
}

/// 校验并解析公告；不是本轨道的消息、验签失败或字段缺失一律忽略（不 panic）。
fn parse_announcement(env: &Envelope) -> Option<Announcement> {
    if env.kind.as_str() != PRICE_KIND || env.verify().is_err() {
        return None;
    }
    Some(Announcement {
        provider: env.body.get("provider")?.as_str()?.to_string(),
        skill: env.body.get("skill")?.as_str()?.to_string(),
        unit_price: env.body.get("unit_price")?.as_i64()?,
        multiplier_bp: env.body.get("multiplier_bp")?.as_i64()?,
    })
}

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
/// 每一版迭代都应该让这里多做一件真实的事，而不是多打印一行字。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!("{TITLE} {RANGE}：余额管理 + 自主定价 + 兑换路由决策（v1.4.3）"),
    );

    let seller_keys = agent(41);
    let buyer_keys = agent(42);
    let rival_keys = agent(43);
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
    let rival = ensure_registered(
        kernel,
        &rival_keys,
        "economy.rival",
        &["translate.en-zh"],
        Credits(100),
    )?;
    kernel.emit("economy.registered", "3 个 Agent 自证身份并自带质押完成注册");

    // 每个 Agent 自己申报策略——没有任何外部设定入口。
    let mut book = BalanceManager::new();
    let policy = BalancePolicy {
        reserve_floor: Credits(100),
        stake_target_bp: 2_000,
        max_spend_bp: 5_000,
    };
    book.declare(&seller, policy)?;
    book.declare(&buyer, policy)?;
    book.declare(&rival, policy)?;

    // 自主定价：两个供给方各自用自己的信誉 / 稀缺度 / 负载算出报价，然后广播。
    let skill = "translate.en-zh";
    let states: [(u8, &AgentKeys, &Did, i64, i64, i64); 2] = [
        (41, &seller_keys, &seller, 8_500, 3_000, 2_000),
        (43, &rival_keys, &rival, 4_000, 9_000, 8_000),
    ];
    let mut quotes_json = Vec::new();
    for (seed, keys, did, reputation_bp, scarcity_bp, load_bp) in states {
        let inputs = PriceInputs {
            base_price: Credits(400),
            reputation_bp,
            scarcity_bp,
            load_bp,
        };
        let quote = pricing::quote(&inputs, &PriceKnobs::DEFAULT)?;
        let envelope = Envelope::new(
            did.clone(),
            None,
            PRICE_KIND,
            kernel.tick(),
            None,
            json!({
                "provider": did.as_str(),
                "skill": skill,
                "unit_price": quote.unit_price,
                "multiplier_bp": quote.multiplier_bp,
                "components": quote.components,
                "reputation_bp": reputation_bp,
                "scarcity_bp": scarcity_bp,
                "load_bp": load_bp,
                "fingerprint": quote.fingerprint()?,
            }),
        )?
        .seal(keys)?;
        kernel.send(&envelope)?;
        kernel.emit(
            "economy.priced",
            format!(
                "种子 {seed} 的 Agent 自主定价 {}（乘数 {}bp）并广播",
                quote.unit_price, quote.multiplier_bp
            ),
        );
        quotes_json.push(json!({
            "provider": did.as_str(),
            "skill": skill,
            "unit_price": quote.unit_price,
            "multiplier_bp": quote.multiplier_bp,
            "components": quote.components,
            "fingerprint": quote.fingerprint()?,
        }));
    }

    // 发现彼此：买方取回报价公告、验签、按整数单价排序挑选最便宜的供给方。
    let mut discovered: Vec<Announcement> = kernel
        .drain()
        .iter()
        .filter_map(parse_announcement)
        .filter(|a| a.skill == skill)
        .collect();
    discovered.sort_by(|a, b| {
        (a.unit_price, a.provider.as_str()).cmp(&(b.unit_price, b.provider.as_str()))
    });
    let chosen = discovered.first().ok_or(CoreError::UnknownAgent)?;
    let price = Credits(chosen.unit_price);
    kernel.emit(
        "economy.discovered",
        format!(
            "买方发现 {} 条报价，选中 {}（单价 {}）",
            discovered.len(),
            au4a_core::short_id(&chosen.provider),
            price
        ),
    );

    let before = kernel.ledger().view();
    let buyer_left = book.spend(kernel.ledger_mut(), &buyer, &seller, price)?;
    kernel.emit(
        "economy.paid",
        format!("买方按成交价自主支付 {price} 微积分，剩余可用额度 {buyer_left}"),
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

    // 自主兑换路由决策：金额阈值 / 时效 / 费用三个条件决定是否上链。
    // 本轨道只做决策与账务预留——**绝不伪造链上成功**（真实执行属 v1.8）。
    let table = fx::RouteTable::DEFAULT;
    let ask = |amount: i64, slack: u64| fx::ExchangeRequest {
        from: seller.clone(),
        amount: Credits(amount),
        urgency: fx::Urgency::Standard,
        slack_ticks: slack,
        preferred: None,
    };
    let small = fx::route(&ask(150, 40), &table)?;
    let tight = fx::route(&ask(400, 12), &table)?;
    let executable = fx::route(&ask(400, 30), &table)?;
    let mut exchange = fx::ExchangeBook::new();
    if executable.action == fx::RouteAction::RouteOnchain {
        // 预留前先过 Agent 自己的余额策略：兑换也是支出。
        if !book.check(kernel.ledger(), &seller, executable.amount())?.allowed() {
            return Err(CoreError::InsufficientFunds);
        }
        let at = kernel.tick();
        let intent = fx::escrow_for_route(kernel.ledger_mut(), &executable, at)?;
        exchange.record(intent);
    }
    kernel.emit(
        "economy.exchange",
        format!(
            "兑换路由：{}（{}）/ {}（{}）/ {}（{}），链上成功报告 = false",
            small.action.as_str(),
            small.reason.as_str(),
            tight.action.as_str(),
            tight.reason.as_str(),
            executable.action.as_str(),
            executable.reason.as_str()
        ),
    );
    // 只有一条链上路由被预留：缓办与内部结算都不允许留下账务痕迹。
    if exchange.pending() != 1 {
        return Err(CoreError::InvalidKind);
    }

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
        "scenario": "v1.4.3 余额管理 + 自主定价 + 兑换路由决策",
        "agents": [row(&seller)?, row(&buyer)?, row(&rival)?],
        "pricing": {
            "knobs": PriceKnobs::DEFAULT,
            "quotes": quotes_json,
            "discovered": discovered.len(),
            "chosen": chosen.provider,
            "chosen_unit_price": price,
        },
        "exchange": {
            "table": table,
            "decisions": [
                { "amount": 150, "action": small.action.as_str(), "venue": small.venue.as_str(), "reason": small.reason.as_str(), "fee": small.fee, "net": small.net, "chain_execution": small.chain_execution.as_str() },
                { "amount": 400, "action": tight.action.as_str(), "venue": tight.venue.as_str(), "reason": tight.reason.as_str(), "fee": tight.fee, "net": tight.net, "chain_execution": tight.chain_execution.as_str() },
                { "amount": 400, "action": executable.action.as_str(), "venue": executable.venue.as_str(), "reason": executable.reason.as_str(), "fee": executable.fee, "net": executable.net, "chain_execution": executable.chain_execution.as_str() },
            ],
            "book": exchange.to_json(),
            "chain_executed": false,
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_kernel::KernelConfig;

    /// 同一个空内核跑两次场景：规范 JSON 必须逐字节一致（可复现）。
    #[test]
    fn the_scenario_is_reproducible_and_its_prices_are_pinned() {
        let mut first = Kernel::new(KernelConfig::default());
        let a = scenario(&mut first).unwrap();
        let mut second = Kernel::new(KernelConfig::default());
        let b = scenario(&mut second).unwrap();
        assert_eq!(
            au4a_core::canonicalize(&a).unwrap(),
            au4a_core::canonicalize(&b).unwrap()
        );
        // 卖方 408（信誉高、空闲），对手 636（信誉低、稀缺且满载）→ 买方选中卖方。
        assert_eq!(a["pricing"]["discovered"], json!(2));
        assert_eq!(a["pricing"]["quotes"][0]["unit_price"], json!(408));
        assert_eq!(a["pricing"]["quotes"][1]["unit_price"], json!(636));
        assert_eq!(a["pricing"]["chosen_unit_price"], json!(408));
        assert_eq!(a["settled"][0]["amount"], json!(408));
        assert_eq!(a["refusals"][0]["verdict"], json!("below_reserve"));
        // 兑换路由决策：小额内部、时效不足缓办、可执行则走 ETH（费率 3、到账 397）。
        assert_eq!(a["exchange"]["decisions"][0]["action"], json!("keep_internal"));
        assert_eq!(
            a["exchange"]["decisions"][0]["reason"],
            json!("below_onchain_minimum")
        );
        assert_eq!(a["exchange"]["decisions"][1]["action"], json!("defer"));
        assert_eq!(
            a["exchange"]["decisions"][1]["reason"],
            json!("deadline_too_tight")
        );
        assert_eq!(a["exchange"]["decisions"][2]["action"], json!("route_onchain"));
        assert_eq!(a["exchange"]["decisions"][2]["venue"], json!("eth"));
        assert_eq!(a["exchange"]["decisions"][2]["fee"], json!(3));
        assert_eq!(a["exchange"]["decisions"][2]["net"], json!(397));
        assert_eq!(a["exchange"]["chain_executed"], json!(false));
        assert_eq!(a["exchange"]["book"]["pending"], json!(1));
        assert_eq!(
            a["exchange"]["book"]["intents"][0]["status"],
            json!("awaiting_chain_execution")
        );
        assert_eq!(a["conservation"]["ok"], json!(true));
        assert!(first.ledger().check_conservation().is_ok());
        assert!(second.ledger().check_conservation().is_ok());
    }
}
