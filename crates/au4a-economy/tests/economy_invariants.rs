//! v1.4.7 集成测试：跨模块不变量与性质测试（无外部依赖，确定性伪随机）。
//!
//! 这里的每一条都不是「示例」，而是**性质**：
//!
//! * 任意操作序列下守恒都成立，且序列可重放（同种子同结果）；
//! * 定价在 0..=10000 全域上单调、可复现、无浮点；
//! * 分成严格分完（Σ == 总额，且每份与理想值偏差 < 1 微积分）；
//! * 罚没永不超过锁定余额；
//! * 质押簿声称的锁定量永不超过账本实际锁定量；
//! * 被拒绝的操作**一个字节都不改账本**。

use au4a_core::{AgentKeys, CoreError, Credits, Did, EvidenceGrade, Ledger};
use au4a_economy::arbitration::{ArbitrationTerms, Court};
use au4a_economy::balance::{
    autostake, check_spend, spend, BalanceManager, BalancePolicy, SpendVerdict,
};
use au4a_economy::fx::{route, ExchangeBook, ExchangeRequest, RouteAction, RouteTable, Urgency};
use au4a_economy::pricing::{quote, unit_price, PriceInputs, PriceKnobs};
use au4a_economy::settlement::{split_weights, Beneficiary, DecisionRights, Receipt, RevenueBook};
use au4a_economy::stake::{StakeBook, StakeTerms};

/// 确定性线性同余伪随机数：不引入依赖，重放结果完全一致。
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

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[i + 30; 32]).did())
        .collect()
}

/// 递归断言：投影里没有任何浮点数（字符串里的点号是允许的，数字里的不是）。
fn assert_no_float(value: &serde_json::Value) {
    match value {
        serde_json::Value::Number(n) => assert!(
            n.as_i64().is_some() || n.as_u64().is_some(),
            "投影里出现了浮点数：{n}"
        ),
        serde_json::Value::Array(items) => items.iter().for_each(assert_no_float),
        serde_json::Value::Object(map) => map.values().for_each(assert_no_float),
        _ => {}
    }
}

fn sample_policy() -> BalancePolicy {
    BalancePolicy {
        reserve_floor: Credits(10),
        stake_target_bp: 3_000,
        max_spend_bp: 9_000,
    }
}

fn sample_terms() -> StakeTerms {
    StakeTerms {
        min_stake: Credits(5),
        cooldown_ticks: 2,
        max_stake_bp: 9_000,
    }
}

/// 跑一遍 400 步的混合操作序列，返回最终账本视图的规范 JSON 与操作计数。
fn run_operation_sequence(seed: u64) -> (String, u64, u64) {
    let agents = dids(4);
    let mut ledger = Ledger::new();
    let mut balances = BalanceManager::new();
    let mut stakes = StakeBook::new();
    let policy = sample_policy();
    let terms = sample_terms();
    for who in &agents {
        balances.declare(who, policy).unwrap();
        ledger.mint(who, Credits(1_000)).unwrap();
    }
    let mut rng = Lcg::new(seed);
    let mut now = 0u64;
    let mut accepted = 0u64;
    let mut refused = 0u64;
    for _ in 0..400 {
        let i = rng.below(agents.len() as u64) as usize;
        let j = rng.below(agents.len() as u64) as usize;
        let amount = Credits((rng.below(300) + 1) as i64);
        let result = match rng.below(7) {
            0 => ledger.mint(&agents[i], amount),
            1 => ledger.transfer(&agents[i], &agents[j], amount),
            2 => balances
                .spend(&mut ledger, &agents[i], &agents[j], amount)
                .map(|_| ()),
            3 => balances
                .autostake(&mut ledger, &agents[i], amount)
                .map(|_| ()),
            4 => stakes
                .stake(&mut ledger, &terms, &agents[i], amount)
                .map(|_| ()),
            5 => {
                now += 1;
                let locked = ledger.balance(&agents[i]).locked;
                let take = if locked < amount { locked } else { amount };
                stakes
                    .request_unstake(&terms, &agents[i], take, now)
                    .map(|_| ())
            }
            _ => {
                now += 1;
                stakes
                    .release_matured(&mut ledger, &agents[i], now)
                    .map(|_| ())
            }
        };
        if result.is_ok() {
            accepted += 1;
        } else {
            refused += 1;
        }
        // 每一步之后：守恒 + 质押簿不得多记。
        ledger.check_conservation().unwrap();
        stakes.assert_consistent(&ledger).unwrap();
    }
    let view = serde_json::to_value(ledger.view()).unwrap();
    (au4a_core::canonicalize(&view).unwrap(), accepted, refused)
}

#[test]
fn conservation_and_the_stake_invariant_survive_a_long_operation_sequence() {
    let (canonical, accepted, refused) = run_operation_sequence(20_261_004);
    assert_eq!(accepted + refused, 400, "每一步都必须有确定的结果");
    assert!(accepted > 0 && refused > 0, "序列应同时覆盖成功与拒绝路径");
    assert!(canonical.contains("\"minted\""));
}

#[test]
fn the_operation_sequence_replays_byte_for_byte() {
    let a = run_operation_sequence(7);
    let b = run_operation_sequence(7);
    assert_eq!(a.0, b.0, "同种子的两次运行必须逐字节一致");
    assert_eq!((a.1, a.2), (b.1, b.2));
    let other = run_operation_sequence(8);
    assert_ne!(
        a.0, other.0,
        "不同种子应给出不同结果（否则测试没在测随机性）"
    );
}

#[test]
fn pricing_is_monotone_over_the_whole_input_domain() {
    let knobs = PriceKnobs::DEFAULT;
    for base in [1i64, 7, 999, 1_000_000] {
        let mut prev_load = unit_price(&PriceInputs::idle(Credits(base)), &knobs).unwrap();
        let mut prev_rep = prev_load;
        for bp in (0..=10_000i64).step_by(100) {
            let with_load = unit_price(
                &PriceInputs {
                    base_price: Credits(base),
                    reputation_bp: 0,
                    scarcity_bp: 0,
                    load_bp: bp,
                },
                &knobs,
            )
            .unwrap();
            assert!(
                with_load >= prev_load,
                "base={base} load={bp} 价格下降：{prev_load} → {with_load}"
            );
            prev_load = with_load;
            let with_rep = unit_price(
                &PriceInputs {
                    base_price: Credits(base),
                    reputation_bp: bp,
                    scarcity_bp: 0,
                    load_bp: 0,
                },
                &knobs,
            )
            .unwrap();
            assert!(
                with_rep <= prev_rep,
                "base={base} reputation={bp} 价格上升：{prev_rep} → {with_rep}"
            );
            prev_rep = with_rep;
            assert!(with_rep >= Credits(1), "价格永不为 0");
        }
    }
}

#[test]
fn every_quote_is_reproducible_and_integer_only() {
    let mut rng = Lcg::new(31_337);
    for _ in 0..500 {
        let inputs = PriceInputs {
            base_price: Credits((rng.below(1_000_000) + 1) as i64),
            reputation_bp: rng.below(10_001) as i64,
            scarcity_bp: rng.below(10_001) as i64,
            load_bp: rng.below(10_001) as i64,
        };
        let a = quote(&inputs, &PriceKnobs::DEFAULT).unwrap();
        let b = quote(&inputs, &PriceKnobs::DEFAULT).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
        let value = serde_json::to_value(a).unwrap();
        au4a_core::canonicalize(&value).unwrap();
        assert_no_float(&value);
    }
}

#[test]
fn the_split_always_adds_up_and_stays_within_one_micro_credit_of_the_ideal() {
    let mut rng = Lcg::new(4_242);
    for _ in 0..300 {
        let parts = 2 + rng.below(4) as usize;
        let mut weights = vec![0i64; parts];
        let mut left = 10_000i64;
        for k in 0..parts - 1 {
            let room = (left / (parts - k) as i64) + 1;
            let w = rng.below(room.max(1) as u64) as i64;
            weights[k] = w.min(left);
            left -= weights[k];
        }
        weights[parts - 1] = left;
        let total = Credits((rng.below(10_000) + 1) as i64);
        let amounts = split_weights(total, &weights).unwrap();
        let sum: i64 = amounts.iter().map(|c| c.get()).sum();
        assert_eq!(sum, total.get(), "分成必须严格分完：weights={weights:?}");
        for (amount, w) in amounts.iter().zip(weights.iter()) {
            let ideal = total.get() * w;
            assert!(
                (amount.get() * 10_000 - ideal).abs() < 10_000,
                "每份与理想值偏差必须小于 1 微积分：{amount} vs {w}bp of {total}"
            );
        }
    }
}

#[test]
fn penalties_never_exceed_the_locked_balance_across_many_cases() {
    let mut rng = Lcg::new(555);
    for round in 0..40 {
        let respondent = AgentKeys::from_seed(&[round as u8 + 100; 32]).did();
        let claimant = AgentKeys::from_seed(&[round as u8 + 200; 32]).did();
        let locked = (rng.below(500) + 1) as i64;
        let claimed = (rng.below(100_000) + 1) as i64;
        let cpu_proto = round % 3 == 0;
        let evidence = if cpu_proto {
            EvidenceGrade::CpuProto
        } else {
            EvidenceGrade::Verified
        };
        let mut ledger = Ledger::new();
        ledger.mint(&respondent, Credits(10_000_000)).unwrap();
        ledger.lock(&respondent, Credits(locked)).unwrap();
        let mut court = Court::new();
        let case = court
            .open(&claimant, &respondent, Credits(claimed), evidence, 1)
            .unwrap();
        court
            .vote(
                &case.id,
                &AgentKeys::from_seed(&[round as u8 + 1; 32]).did(),
                true,
                6_000,
            )
            .unwrap();
        court
            .vote(
                &case.id,
                &AgentKeys::from_seed(&[round as u8 + 2; 32]).did(),
                true,
                4_000,
            )
            .unwrap();
        let terms = ArbitrationTerms::DEFAULT;
        let ruling = court.rule(&mut ledger, &case.id, &terms, 2).unwrap();
        assert!(
            ruling.slashed <= Credits(locked),
            "罚没 {} 超过锁定余额 {locked}",
            ruling.slashed
        );
        assert_eq!(ledger.slashed(), ruling.slashed);
        assert_eq!(
            ledger.balance(&respondent).locked,
            Credits(locked - ruling.slashed.get())
        );
        if cpu_proto {
            assert!(ruling.slashed <= terms.cpu_proto_cap);
        }
        ledger.check_conservation().unwrap();
    }
}

#[test]
fn refusals_never_touch_the_ledger() {
    let agents = dids(3);
    let mut ledger = Ledger::new();
    for who in &agents {
        ledger.mint(who, Credits(1_000)).unwrap();
    }
    let policy = BalancePolicy {
        reserve_floor: Credits(500),
        stake_target_bp: 0,
        max_spend_bp: 1_000,
    };
    let mut stakes = StakeBook::new();
    let terms = sample_terms();

    let before = ledger.view();
    // 穿过运营底线。
    assert_eq!(
        spend(&mut ledger, &agents[0], &agents[1], Credits(600), &policy),
        Err(CoreError::InsufficientFunds)
    );
    assert_eq!(
        check_spend(&ledger, &agents[0], Credits(600), &policy).unwrap(),
        SpendVerdict::BelowReserve
    );
    // 零额。
    assert_eq!(
        spend(&mut ledger, &agents[0], &agents[1], Credits::ZERO, &policy),
        Err(CoreError::ZeroAmount)
    );
    // 质押低于准入线。
    let strict = StakeTerms {
        min_stake: Credits(100),
        ..sample_terms()
    };
    assert!(stakes
        .stake(&mut ledger, &strict, &agents[0], Credits(50))
        .is_err());
    // 无头寸解质押。
    assert!(stakes
        .request_unstake(&terms, &agents[2], Credits(10), 1)
        .is_err());
    // 自动质押无缺口。
    assert_eq!(
        autostake(&mut ledger, &agents[0], &policy, Credits(100)),
        Err(CoreError::ZeroAmount)
    );
    assert_eq!(ledger.view(), before, "被拒绝的操作不得改动账本");
    ledger.check_conservation().unwrap();
}

#[test]
fn exchange_decisions_are_pure_and_never_claim_chain_success() {
    let mut rng = Lcg::new(9_001);
    let who = dids(1)[0].clone();
    for _ in 0..200 {
        let request = ExchangeRequest {
            from: who.clone(),
            amount: Credits((rng.below(5_000) + 1) as i64),
            urgency: Urgency::Standard,
            slack_ticks: rng.below(80),
            preferred: None,
        };
        let a = route(&request, &RouteTable::DEFAULT).unwrap();
        let b = route(&request, &RouteTable::DEFAULT).unwrap();
        assert_eq!(a, b, "路由决策必须是纯函数");
        assert!(!a.chain_execution.executed());
        if a.action == RouteAction::RouteOnchain {
            assert_eq!(a.fee.get() + a.net.get(), a.amount().get());
        }
        let value = serde_json::to_value(a).unwrap();
        au4a_core::canonicalize(&value).unwrap();
        assert_no_float(&value);
    }
    let book = ExchangeBook::new();
    let value = book.to_json();
    assert_eq!(value["chain_executed"], serde_json::json!(false));
    assert_eq!(value["pending"], serde_json::json!(0));
}

#[test]
fn revenue_book_counts_only_real_receipts_and_keeps_humans_read_only() {
    let human = Beneficiary::human_operator(dids(1)[0].clone());
    let agent = Beneficiary::agent(dids(2)[1].clone());
    assert_eq!(human.decision_rights(), DecisionRights::IncomeOnly);
    assert_eq!(agent.decision_rights(), DecisionRights::Agent);

    let mut book = RevenueBook::new();
    book.record_receipt(Receipt {
        beneficiary: human.did.as_str().to_string(),
        amount: Credits(60),
        share_bp: 3_000,
        role: au4a_economy::ProviderRole::Data,
        kind: au4a_economy::BeneficiaryKind::HumanOperator,
        at: 3,
    });
    book.record_receipt(Receipt {
        beneficiary: agent.did.as_str().to_string(),
        amount: Credits(140),
        share_bp: 7_000,
        role: au4a_economy::ProviderRole::Compute,
        kind: au4a_economy::BeneficiaryKind::Agent,
        at: 3,
    });
    assert_eq!(book.earned(&human.did).unwrap(), Credits(60));
    assert_eq!(book.earned(&agent.did).unwrap(), Credits(140));
    assert_eq!(book.total_earned().unwrap(), Credits(200));
    let value = book.to_json();
    au4a_core::canonicalize(&value).unwrap();
    assert_no_float(&value);
}

#[test]
fn every_public_projection_is_canonical_and_float_free() {
    let results = au4a_economy::results_json().unwrap();
    au4a_core::canonicalize(&results).unwrap();
    assert_no_float(&results);
    assert_eq!(results["track"], serde_json::json!("1.4"));

    let checks = au4a_economy::self_check();
    assert!(au4a_core::all_passed(&checks), "自检必须全绿：{checks:?}");
    for check in &checks {
        assert!(
            !check.detail.is_empty(),
            "自检 {} 必须有真实细节",
            check.name
        );
    }
}
