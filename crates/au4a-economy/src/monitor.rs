//! v1.4.10 只读监控：收益面板数据源。
//!
//! 人类只有三个只读投影：进度 / 结果 / **收益**。这个模块就是「收益」的数据源，
//! 它对外只暴露**读**：所有入口都接 `&Kernel` 与 `&RevenueBook`，返回一个 `Value`，
//! 没有任何配套的写方法；[`READ_ONLY`] 恒为 `true`。
//!
//! 面板的账本部分是节点 `/api/revenue` 的**超集**：保留 `minted` / `slashed` / `total` /
//! `accounts.<did>.{available,locked}` 这些字段名，另外补上 `earned` / `kind` / `display` /
//! `account_count` / `conservation_ok`，因此既能被既有前端直接读，也能让人看出「收益归谁」。
//!
//! 收益只统计**真实到账凭证**（[`RevenueBook`] 里的 `Receipt`）：拒付与托管都不算收益，
//! 争议罚没只会体现在 `slashed` 里，不会变成任何人的收入。

use au4a_core::{CoreResult, Credits, Did, Ledger, SelfCheck};
use au4a_kernel::Kernel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::balance::assert_conserved;
use crate::settlement::{BeneficiaryKind, RevenueBook};
use crate::TRACK;

/// 监控面板是只读的——这个常量参与自检，也出现在 JSON 里。
pub const READ_ONLY: bool = true;

/// 账本总量（面板 `ledger` 段的第一层）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerTotals {
    pub minted: Credits,
    pub slashed: Credits,
    pub total: Credits,
    pub available: Credits,
    pub locked: Credits,
    pub accounts: usize,
    pub conservation_ok: bool,
}

/// 收益面板的一行（一个账户）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevenueRow {
    pub did: String,
    /// 注册 Agent 的显示名；未注册的收益主体显示为 `human-operator`。
    pub display: String,
    pub kind: BeneficiaryKind,
    pub available: Credits,
    pub locked: Credits,
    /// 累计真实到账收益（来自凭证，不含拒付与托管）。
    pub earned: Credits,
}

/// 收益面板。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevenuePanel {
    pub at: u64,
    /// 注册 Agent 数量（不含只收收益的人类操作者账户）。
    pub agents: usize,
    pub rows: Vec<RevenueRow>,
    pub ledger: LedgerTotals,
    pub total_earned: Credits,
    /// 附加上下文（场景摘要片段；只做透传，不参与计算）。
    pub context: Value,
}

impl RevenuePanel {
    /// 守恒断言是否成立（面板自己先说清楚，别让读者猜）。
    pub fn conservation_ok(&self) -> bool {
        self.ledger.conservation_ok
            && self.ledger.total
                == Credits(
                    self.ledger
                        .available
                        .get()
                        .saturating_add(self.ledger.locked.get()),
                )
    }

    /// 行内收益之和必须等于总收益——面板不能自己制造收益。
    pub fn rows_earned_total(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for row in &self.rows {
            sum = sum.checked_add(row.earned)?;
        }
        Ok(sum)
    }

    /// 只读 JSON 投影（与节点 `/api/revenue` 字段对齐的超集）。
    pub fn to_json(&self) -> Value {
        let mut accounts = Map::new();
        for row in &self.rows {
            accounts.insert(
                row.did.clone(),
                json!({
                    "available": row.available,
                    "locked": row.locked,
                    "earned": row.earned,
                    "kind": row.kind,
                    "display": row.display,
                }),
            );
        }
        json!({
            "track": TRACK,
            "panel": "revenue",
            "read_only": READ_ONLY,
            "at": self.at,
            "agents": self.agents,
            "rows": self.rows,
            "ledger": {
                "minted": self.ledger.minted,
                "slashed": self.ledger.slashed,
                "total": self.ledger.total,
                "available": self.ledger.available,
                "locked": self.ledger.locked,
                "accounts": accounts,
                "account_count": self.ledger.accounts,
                "conservation_ok": self.ledger.conservation_ok,
            },
            "total_earned": self.total_earned,
            "context": self.context,
        })
    }
}

/// 账本总量：`Σ可用 + Σ锁定 + 罚没 == 发行` 的只读快照。
pub fn ledger_totals(ledger: &Ledger) -> CoreResult<LedgerTotals> {
    let view = ledger.view();
    let mut available = Credits::ZERO;
    let mut locked = Credits::ZERO;
    for account in view.accounts.values() {
        available = available.checked_add(account.available)?;
        locked = locked.checked_add(account.locked)?;
    }
    Ok(LedgerTotals {
        minted: view.minted,
        slashed: view.slashed,
        total: view.total,
        available,
        locked,
        accounts: view.accounts.len(),
        conservation_ok: ledger.check_conservation().is_ok(),
    })
}

/// 构造收益面板（纯读：不写账本、不写内核、不读文件/网络/墙钟）。
///
/// 未注册为 Agent 的账户按 [`BeneficiaryKind::HumanOperator`] 处理——它们只收收益，
/// 面板上也不会给它们任何决策能力（`may_decide()` 这类入口在 [`crate::settlement::Beneficiary`] 上恒为 false）。
pub fn panel(kernel: &Kernel, revenue: &RevenueBook, context: &Value) -> CoreResult<RevenuePanel> {
    let view = kernel.observe();
    let ledger = ledger_totals(kernel.ledger())?;
    let mut rows = Vec::new();
    for (key, account) in &view.ledger.accounts {
        let did = Did::parse(key)?;
        let (display, kind) = match kernel.card(&did) {
            Some(card) => (card.display.clone(), BeneficiaryKind::Agent),
            None => ("human-operator".to_string(), BeneficiaryKind::HumanOperator),
        };
        rows.push(RevenueRow {
            did: key.clone(),
            display,
            kind,
            available: account.available,
            locked: account.locked,
            earned: revenue.earned(&did)?,
        });
    }
    Ok(RevenuePanel {
        at: view.now,
        agents: view.agents.len(),
        rows,
        ledger,
        total_earned: revenue.total_earned()?,
        context: context.clone(),
    })
}

/// 只读 JSON 面板（`panel` 的便捷包装，供节点与示例直接使用）。
pub fn revenue_panel(kernel: &Kernel, revenue: &RevenueBook, context: &Value) -> CoreResult<Value> {
    Ok(panel(kernel, revenue, context)?.to_json())
}

/// 不带上下文的极简面板（节点只想要账本与收益时用）。
pub fn monitor_json(kernel: &Kernel, revenue: &RevenueBook) -> CoreResult<Value> {
    revenue_panel(kernel, revenue, &Value::Null)
}

/// 监控自检项：守恒、收益归属、只读性——都是**真跑一遍**再看断言。
pub fn monitor_checks(kernel: &Kernel, revenue: &RevenueBook) -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    let double = |ok: bool, name: &str, detail: String| {
        if ok {
            SelfCheck::pass(TRACK, name, detail)
        } else {
            SelfCheck::fail(TRACK, name, detail)
        }
    };

    checks.push(match ledger_totals(kernel.ledger()) {
        Ok(totals) => double(
            totals.conservation_ok
                && totals.total == Credits(totals.available.get() + totals.locked.get()),
            "monitor.ledger_conservation",
            format!(
                "发行 {} = Σ可用 {} + Σ锁定 {} + 罚没 {}（账户 {} 个），conservation_ok = {}",
                totals.minted,
                totals.available,
                totals.locked,
                totals.slashed,
                totals.accounts,
                totals.conservation_ok
            ),
        ),
        Err(err) => SelfCheck::fail(TRACK, "monitor.ledger_conservation", err.to_string()),
    });

    checks.push(assert_conserved(kernel.ledger()).map_or_else(
        |err| SelfCheck::fail(TRACK, "monitor.conservation_gate", err.to_string()),
        |()| {
            SelfCheck::pass(
                TRACK,
                "monitor.conservation_gate",
                format!(
                    "check_conservation() 通过：罚没 {}、发行 {}",
                    kernel.ledger().slashed(),
                    kernel.ledger().minted()
                ),
            )
        },
    ));

    let first = panel(kernel, revenue, &Value::Null);
    let second = panel(kernel, revenue, &Value::Null);
    checks.push(match (&first, &second) {
        (Ok(a), Ok(b)) => double(
            a == b && a.rows_earned_total().map(|s| s == b.total_earned).unwrap_or(false),
            "monitor.read_only",
            format!(
                "两次只读投影逐字段相同（账户 {} 个、注册 Agent {} 个），行内收益合计 = 总收益 {}",
                a.rows.len(),
                a.agents,
                a.total_earned
            ),
        ),
        (Err(err), _) | (_, Err(err)) => {
            SelfCheck::fail(TRACK, "monitor.read_only", err.to_string())
        }
    });

    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settlement::{Beneficiary, ProviderRole, Receipt};
    use au4a_core::{AgentKeys, EvidenceGrade};
    use au4a_kernel::KernelConfig;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn kernel_with_two_agents() -> (Kernel, Did, Did) {
        let mut kernel = Kernel::new(KernelConfig::default());
        let a = AgentKeys::from_seed(&[1; 32]);
        let b = AgentKeys::from_seed(&[2; 32]);
        kernel.register(&a, "provider", &["compute"], Credits(10)).unwrap();
        kernel.register(&b, "consumer", &["buy"], Credits(10)).unwrap();
        let seller = a.did();
        let buyer = b.did();
        (kernel, seller, buyer)
    }

    fn receipt(who: &Did, amount: i64) -> Receipt {
        Receipt {
            beneficiary: who.as_str().to_string(),
            amount: Credits(amount),
            share_bp: 10_000,
            role: ProviderRole::Compute,
            kind: BeneficiaryKind::Agent,
            at: 1,
        }
    }

    #[test]
    fn totals_match_the_ledger() {
        let mut ledger = Ledger::new();
        let a = did(3);
        let b = did(4);
        ledger.mint(&a, Credits(1_000)).unwrap();
        ledger.mint(&b, Credits(500)).unwrap();
        ledger.lock(&a, Credits(200)).unwrap();
        ledger.transfer(&a, &b, Credits(100)).unwrap();
        ledger.slash(&a, Credits(50)).unwrap();
        let totals = ledger_totals(&ledger).unwrap();
        assert_eq!(totals.minted, Credits(1_500));
        assert_eq!(totals.slashed, Credits(50));
        assert_eq!(totals.locked, Credits(150));
        assert_eq!(totals.available, Credits(1_300));
        assert_eq!(totals.total, Credits(1_450));
        assert_eq!(totals.accounts, 2);
        assert!(totals.conservation_ok);
        assert_eq!(totals.total, Credits(totals.available.get() + totals.locked.get()));
    }

    #[test]
    fn the_panel_lists_every_account_and_counts_only_real_receipts() {
        let (mut kernel, seller, buyer) = kernel_with_two_agents();
        // 人类操作者的收益账户：不注册为 Agent，只通过分成收钱。
        let owner = Beneficiary::human_operator(did(5));
        kernel
            .settle(&buyer, &seller, Credits(140), EvidenceGrade::Verified)
            .unwrap();
        kernel
            .settle(&buyer, &owner.did, Credits(60), EvidenceGrade::Verified)
            .unwrap();
        let mut revenue = RevenueBook::new();
        revenue.record_receipt(receipt(&seller, 140));
        revenue.record_receipt(Receipt {
            beneficiary: owner.did.as_str().to_string(),
            amount: Credits(60),
            share_bp: 3_000,
            role: ProviderRole::Data,
            kind: BeneficiaryKind::HumanOperator,
            at: 2,
        });
        let value = revenue_panel(&kernel, &revenue, &json!({"scenario": "unit-test"})).unwrap();
        assert_eq!(value["read_only"], json!(true));
        assert_eq!(value["agents"], json!(2));
        assert_eq!(value["ledger"]["account_count"], json!(3));
        assert_eq!(value["total_earned"], json!(200));
        // 账本是节点 /api/revenue 的超集：字段名保持不变。
        let seller_key = seller.as_str();
        assert_eq!(value["ledger"]["accounts"][seller_key]["available"], json!(1_130));
        assert_eq!(value["ledger"]["accounts"][seller_key]["earned"], json!(140));
        assert_eq!(value["ledger"]["accounts"][seller_key]["kind"], json!("agent"));
        let owner_key = owner.did.as_str();
        assert_eq!(value["ledger"]["accounts"][owner_key]["kind"], json!("human_operator"));
        assert_eq!(value["ledger"]["accounts"][owner_key]["display"], json!("human-operator"));
        assert_eq!(value["ledger"]["accounts"][owner_key]["earned"], json!(60));
        assert_eq!(value["ledger"]["accounts"][owner_key]["locked"], json!(0));
        assert_eq!(value["context"]["scenario"], json!("unit-test"));
    }

    #[test]
    fn the_panel_is_read_only_and_deterministic() {
        let (kernel, _seller, buyer) = kernel_with_two_agents();
        let mut revenue = RevenueBook::new();
        revenue.record_receipt(receipt(&buyer, 10));
        let before = kernel.ledger().view();
        let first = revenue_panel(&kernel, &revenue, &Value::Null).unwrap();
        let second = revenue_panel(&kernel, &revenue, &Value::Null).unwrap();
        assert_eq!(first, second, "只读投影必须逐字段稳定");
        assert_eq!(kernel.ledger().view(), before, "只读投影不得改动账本");
        assert!(READ_ONLY);
        let panel = super::panel(&kernel, &revenue, &Value::Null).unwrap();
        assert!(panel.conservation_ok());
        assert_eq!(panel.rows_earned_total().unwrap(), panel.total_earned);
    }

    #[test]
    fn the_panel_json_is_float_free() {
        let (kernel, _seller, buyer) = kernel_with_two_agents();
        let mut revenue = RevenueBook::new();
        revenue.record_receipt(receipt(&buyer, 25));
        let value = monitor_json(&kernel, &revenue).unwrap();
        au4a_core::canonicalize(&value).unwrap();
        assert!(value["ledger"]["minted"].is_i64());
        assert!(value["ledger"]["slashed"].is_i64());
        assert!(value["ledger"]["total"].is_i64());
        assert_eq!(value["context"], Value::Null);
    }

    #[test]
    fn monitor_checks_are_real_and_all_pass() {
        let (kernel, _seller, buyer) = kernel_with_two_agents();
        let mut revenue = RevenueBook::new();
        revenue.record_receipt(receipt(&buyer, 5));
        let checks = monitor_checks(&kernel, &revenue);
        assert_eq!(checks.len(), 3);
        for check in &checks {
            assert!(check.passed, "{}: {}", check.name, check.detail);
            assert_eq!(check.track, "1.4");
            assert!(!check.detail.is_empty());
        }
        assert!(au4a_core::all_passed(&checks));
    }

    #[test]
    fn withheld_and_escrowed_money_is_never_counted_as_income() {
        let (mut kernel, seller, buyer) = kernel_with_two_agents();
        // 一次被拒付的结算尝试：账本不动，收益更不该出现。
        let outcome = crate::settlement::pay_split(
            &mut kernel,
            &crate::settlement::SettlementRequest {
                payer: buyer.clone(),
                payee: seller.clone(),
                total: Credits(50),
                evidence: EvidenceGrade::Unverified,
                dispute_open: false,
                at: 1,
            },
            &[crate::settlement::RevenueShare {
                beneficiary: seller.clone(),
                share_bp: 10_000,
                role: ProviderRole::Compute,
                kind: BeneficiaryKind::Agent,
            }],
            &crate::settlement::SettlementPolicy::DEFAULT,
        )
        .unwrap();
        assert!(matches!(
            outcome,
            crate::settlement::SettlementOutcome::Withheld(_)
        ));
        let revenue = RevenueBook::new();
        let panel = super::panel(&kernel, &revenue, &Value::Null).unwrap();
        assert_eq!(panel.total_earned, Credits::ZERO);
        let seller_row = panel
            .rows
            .iter()
            .find(|r| r.did == seller.as_str())
            .expect("卖方必须有行");
        assert_eq!(seller_row.earned, Credits::ZERO);
        assert_eq!(seller_row.available, Credits(990), "拒付不得改动余额");
    }
}
