//! v1.8.4 x402：支付发票 / 支付 / 领取（**ETH 侧确定性测试网原型**）。
//!
//! x402 的语义是「402 Payment Required」：服务方给出付款条件，付款方付钱后带凭证重试，
//! 服务方**验证凭证**再提供服务。这里把该流程做成纯 CPU 状态机：
//!
//! ```text
//! x402.invoice  服务方开票：id = H(payee ‖ amount ‖ asset ‖ nonce)，含到期时刻（逻辑时钟）
//! x402.pay      付款方支付：金额必须与发票**完全相等**；过期 → Timeout；重复支付 → Conflict
//! x402.claim    服务方领取：必须已达最终性；领取后本地托管解锁 → 转给服务方，链上表示销毁
//! ```
//!
//! 双轨：支付时本地积分**锁定**（托管）并记链上表示；领取时链上表示**销毁**并把托管转给服务方。
//! 任何时刻 `本地托管 == 链上表示`（由 [`crate::bridge`] 的对账保证），不一致就 fail-closed。
//!
//! 具名拒绝（见 [`X402_REFUSALS`]）：部分支付、流式支付、法币入金、链下结算、无发票支付。
//! 证据等级恒为 `cpu-proto`：没有真实 HTTP、没有真实结算层。

use std::collections::BTreeMap;

use au4a_core::{CoreError, CoreResult, Credits, Did, Ledger, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bridge::BridgeBook;
use crate::testnet::{ChainRefusal, ChainTx, Receipt, RefusalSpec, Testnet, ONCHAIN_GRADE};

/// 本适配器支持的操作。
pub const X402_SUPPORTED: [&str; 3] = ["x402.invoice", "x402.pay", "x402.claim"];

/// 具名拒绝清单。
pub const X402_REFUSALS: [RefusalSpec; 5] = [
    RefusalSpec {
        op: "x402.partial_payment",
        code: RefusalCode::Unsupported,
        reason: "本适配器不支持部分支付：金额必须与发票完全相等",
    },
    RefusalSpec {
        op: "x402.stream_payment",
        code: RefusalCode::Unsupported,
        reason: "流式支付需要真实时间与通道状态，本轨道不联网也不读墙钟",
    },
    RefusalSpec {
        op: "x402.fiat_onramp",
        code: RefusalCode::PolicyDenied,
        reason: "不接受法币通道：会引入受信任第三方，破坏无信任前提",
    },
    RefusalSpec {
        op: "x402.offchain_settle",
        code: RefusalCode::Unsupported,
        reason: "链下结算需要真实 L2/通道，本轨道只有确定性测试网",
    },
    RefusalSpec {
        op: "x402.pay_without_invoice",
        code: RefusalCode::Malformed,
        reason: "没有发票就没有付款条件，拒绝",
    },
];

/// 一张发票（服务方开票）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    pub id: String,
    pub payee: String,
    pub amount: Credits,
    pub asset: String,
    pub nonce: u64,
    /// 逻辑时钟下的到期时刻（`<=` 即过期）。
    pub expires_at: u64,
    pub memo: String,
}

impl Invoice {
    pub fn new(
        payee: &Did,
        amount: Credits,
        asset: &str,
        nonce: u64,
        expires_at: u64,
        memo: &str,
    ) -> CoreResult<Self> {
        if amount == Credits::ZERO {
            return Err(CoreError::ZeroAmount);
        }
        if asset.is_empty() {
            return Err(CoreError::InvalidKind);
        }
        let id = au4a_core::canonical_hash(&json!({
            "payee": payee,
            "amount": amount,
            "asset": asset,
            "nonce": nonce,
            "expires_at": expires_at,
        }))?;
        Ok(Self {
            id,
            payee: payee.as_str().to_string(),
            amount,
            asset: asset.to_string(),
            nonce,
            expires_at,
            memo: memo.to_string(),
        })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "payee": self.payee,
            "amount": self.amount,
            "asset": self.asset,
            "nonce": self.nonce,
            "expires_at": self.expires_at,
            "memo": self.memo,
            "grade": ONCHAIN_GRADE.as_str(),
        })
    }
}

/// 一次支付（付款方 + 发票 + 链上收据）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payment {
    pub invoice_id: String,
    pub payer: String,
    pub amount: Credits,
    pub tx_id: String,
    pub height: u64,
    pub claimed: bool,
}

/// 适配器执行结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct X402Outcome {
    pub op: String,
    pub detail: String,
    pub receipt: Option<Receipt>,
    pub grade: String,
}

/// x402 适配器。
#[derive(Clone, Debug, Default)]
pub struct X402Adapter {
    invoices: BTreeMap<String, Invoice>,
    payments: BTreeMap<String, Payment>,
}

impl X402Adapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn invoice_of(&self, id: &str) -> Option<&Invoice> {
        self.invoices.get(id)
    }

    /// 最近开出的一张发票（只读；场景用它拿回适配器计算出的真实 id）。
    pub fn last_invoice(&self) -> Option<&Invoice> {
        self.invoices.values().next_back()
    }

    pub fn payment_of(&self, invoice_id: &str) -> Option<&Payment> {
        self.payments.get(invoice_id)
    }

    pub fn invoice_count(&self) -> usize {
        self.invoices.len()
    }

    /// 执行不需要账本的操作（开票 / 支付）。
    ///
    /// `x402.claim` 需要账本与双轨台账，必须调用 [`X402Adapter::claim`] 或
    /// [`X402Adapter::execute_with_ledger`]；在这里调用会得到一个**具名拒绝**，
    /// 而不是静默失败。
    pub fn execute(
        &mut self,
        net: &mut Testnet,
        tx: &ChainTx,
    ) -> Result<X402Outcome, ChainRefusal> {
        match tx.op.as_str() {
            "x402.invoice" => self.invoice(net, tx),
            "x402.pay" => self.pay(net, tx),
            "x402.claim" => {
                let refusal = ChainRefusal::new(
                    "x402.claim",
                    RefusalCode::PolicyDenied,
                    "领取必须带账本与双轨台账：请调用 claim(net, ledger, book, tx)",
                );
                net.record_refusal(refusal.clone());
                Err(refusal)
            }
            _ => {
                let refusal = match X402_REFUSALS.iter().find(|spec| spec.op == tx.op) {
                    Some(spec) => ChainRefusal::new(&tx.op, spec.code, spec.reason),
                    None => ChainRefusal::unsupported(&tx.op, &X402_REFUSALS),
                };
                net.record_refusal(refusal.clone());
                Err(refusal)
            }
        }
    }

    /// 完整调度：开票 / 支付 / 领取（领取需要 `&mut Ledger` 与 `&mut BridgeBook`）。
    pub fn execute_with_ledger(
        &mut self,
        net: &mut Testnet,
        ledger: &mut Ledger,
        book: &mut BridgeBook,
        tx: &ChainTx,
    ) -> Result<X402Outcome, ChainRefusal> {
        if tx.op == "x402.claim" {
            self.claim(net, ledger, book, tx)
        } else {
            self.execute(net, tx)
        }
    }

    fn invoice(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<X402Outcome, ChainRefusal> {
        let malformed =
            |detail: &str| ChainRefusal::new("x402.invoice", RefusalCode::Malformed, detail);
        let amount = tx
            .payload
            .get("amount")
            .and_then(Value::as_i64)
            .and_then(|v| Credits::new(v).ok())
            .ok_or_else(|| malformed("缺少 amount"))?;
        let asset = tx
            .payload
            .get("asset")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("缺少 asset"))?;
        let expires_at = tx
            .payload
            .get("expires_at")
            .and_then(Value::as_u64)
            .ok_or_else(|| malformed("缺少 expires_at"))?;
        let memo = tx.payload.get("memo").and_then(Value::as_str).unwrap_or("");
        let invoice = Invoice::new(&tx.submitter, amount, asset, tx.nonce, expires_at, memo)
            .map_err(|err| malformed(&err.to_string()))?;
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "x402");
        self.invoices.insert(invoice.id.clone(), invoice.clone());
        Ok(X402Outcome {
            op: "x402.invoice".to_string(),
            detail: format!(
                "发票 {}：{amount} {} 到期于 t={expires_at}",
                au4a_core::short_id(&invoice.id),
                invoice.asset
            ),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    fn pay(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<X402Outcome, ChainRefusal> {
        let invoice_id = tx
            .payload
            .get("invoice_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainRefusal::new("x402.pay", RefusalCode::Malformed, "缺少 invoice_id")
            })?
            .to_string();
        let amount = tx
            .payload
            .get("amount")
            .and_then(Value::as_i64)
            .and_then(|v| Credits::new(v).ok())
            .ok_or_else(|| ChainRefusal::new("x402.pay", RefusalCode::Malformed, "缺少 amount"))?;
        let now = tx.payload.get("now").and_then(Value::as_u64).unwrap_or(0);
        let invoice = self.invoices.get(&invoice_id).cloned().ok_or_else(|| {
            let refusal = ChainRefusal::new(
                "x402.pay_without_invoice",
                RefusalCode::Malformed,
                X402_REFUSALS[4].reason,
            );
            net.record_refusal(refusal.clone());
            refusal
        })?;
        // 金额必须完全相等（部分支付按具名拒绝码拒绝）。
        if amount != invoice.amount {
            let op = if amount < invoice.amount {
                "x402.partial_payment"
            } else {
                "x402.pay"
            };
            let refusal = ChainRefusal::new(
                op,
                RefusalCode::Malformed,
                format!("支付金额 {amount} 与发票金额 {} 不符", invoice.amount),
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        if now >= invoice.expires_at {
            let refusal = ChainRefusal::new(
                "x402.pay",
                RefusalCode::Timeout,
                format!("发票已于 t={} 过期（当前 t={now}）", invoice.expires_at),
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        if self.payments.contains_key(&invoice_id) {
            let refusal = ChainRefusal::new(
                "x402.pay",
                RefusalCode::Conflict,
                "该发票已经支付过（重复支付被拒）",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "x402");
        let height = receipt.height;
        self.payments.insert(
            invoice_id.clone(),
            Payment {
                invoice_id,
                payer: tx.submitter.as_str().to_string(),
                amount,
                tx_id: tx.id.clone(),
                height,
                claimed: false,
            },
        );
        Ok(X402Outcome {
            op: "x402.pay".to_string(),
            detail: format!("支付 {amount} 已入账（高度 {height}，等待最终性与领取）"),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    /// 领取：**必须已达最终性**，然后本地托管解锁并转给服务方（链上表示销毁）。
    pub fn claim(
        &mut self,
        net: &mut Testnet,
        ledger: &mut Ledger,
        book: &mut BridgeBook,
        tx: &ChainTx,
    ) -> Result<X402Outcome, ChainRefusal> {
        let invoice_id = tx
            .payload
            .get("invoice_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainRefusal::new("x402.claim", RefusalCode::Malformed, "缺少 invoice_id")
            })?
            .to_string();
        let payment = self.payments.get(&invoice_id).cloned().ok_or_else(|| {
            let refusal = ChainRefusal::new(
                "x402.claim",
                RefusalCode::Conflict,
                "该发票尚未支付，不能领取",
            );
            net.record_refusal(refusal.clone());
            refusal
        })?;
        let invoice =
            self.invoices.get(&invoice_id).cloned().ok_or_else(|| {
                ChainRefusal::new("x402.claim", RefusalCode::Conflict, "发票不存在")
            })?;
        let payer = Did::parse(&payment.payer).map_err(|_| {
            ChainRefusal::new("x402.claim", RefusalCode::Malformed, "payer DID 非法")
        })?;
        let payee = Did::parse(&invoice.payee).map_err(|_| {
            ChainRefusal::new("x402.claim", RefusalCode::Malformed, "payee DID 非法")
        })?;
        // fail-closed：未达最终性不领取。
        if !net.is_final(payment.height) {
            let refusal = ChainRefusal::new(
                "x402.claim",
                RefusalCode::Timeout,
                format!(
                    "支付高度 {} 尚未最终（当前 {}，最终化到 {}）",
                    payment.height,
                    net.height(),
                    net.finalized_height()
                ),
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        if payment.claimed {
            let refusal = ChainRefusal::new(
                "x402.claim",
                RefusalCode::Conflict,
                "该发票已经领取过（重复领取被拒）",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        // 双轨：链上表示销毁 + 本地托管 → 服务方。
        book.bridge_in(
            ledger,
            &payer,
            &payee,
            payment.amount,
            &invoice.asset,
            "x402",
        )
        .map_err(|err| {
            let refusal = ChainRefusal::new(
                "x402.claim",
                RefusalCode::Conflict,
                format!("双轨对账失败（fail-closed）：{err}"),
            );
            net.record_refusal(refusal.clone());
            refusal
        })?;
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "x402");
        if let Some(entry) = self.payments.get_mut(&invoice_id) {
            entry.claimed = true;
        }
        Ok(X402Outcome {
            op: "x402.claim".to_string(),
            detail: format!("领取 {}（托管解锁 → 服务方，链上表示销毁）", payment.amount),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    /// 只读：发票是否已结清。
    pub fn is_settled(&self, invoice_id: &str) -> bool {
        self.payments
            .get(invoice_id)
            .map(|p| p.claimed)
            .unwrap_or(false)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "invoices": self.invoices.values().map(Invoice::to_json).collect::<Vec<_>>(),
            "payments": self.payments.values().collect::<Vec<_>>(),
            "invoice_count": self.invoices.len(),
            "settled": self.payments.values().filter(|p| p.claimed).count(),
            "grade": ONCHAIN_GRADE.as_str(),
            "real_network": false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testnet::ChainId;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    struct World {
        net: Testnet,
        x402: X402Adapter,
        ledger: Ledger,
        book: BridgeBook,
        payer: Did,
        payee: Did,
        invoice_id: String,
    }

    fn world(expires_at: u64) -> World {
        let payer = did(1);
        let payee = did(2);
        let mut ledger = Ledger::new();
        ledger.mint(&payer, Credits(1_000)).unwrap();
        let mut net = Testnet::new(ChainId::EthLocal, 2);
        let mut x402 = X402Adapter::new();
        let invoice_tx = ChainTx::new(
            ChainId::EthLocal,
            "x402.invoice",
            &payee,
            1,
            json!({ "amount": 200, "asset": "usdc-eth", "expires_at": expires_at, "memo": "gpu" }),
        )
        .unwrap();
        x402.execute(&mut net, &invoice_tx).unwrap();
        let invoice_id = x402.invoices.keys().next().cloned().expect("发票已生成");
        World {
            net,
            x402,
            ledger,
            book: BridgeBook::new(),
            payer,
            payee,
            invoice_id,
        }
    }

    fn pay(world: &mut World, amount: i64, now: u64) -> Result<X402Outcome, ChainRefusal> {
        let tx = ChainTx::new(
            ChainId::EthLocal,
            "x402.pay",
            &world.payer,
            1,
            json!({ "invoice_id": world.invoice_id, "amount": amount, "now": now }),
        )
        .unwrap();
        world.x402.execute(&mut world.net, &tx)
    }

    /// 支付成功后确实需要托管：测试里显式调用桥出（与 x402 的语义一致）。
    fn escrow(world: &mut World) {
        world
            .book
            .bridge_out(
                &mut world.ledger,
                &world.payer,
                Credits(200),
                "usdc-eth",
                "x402",
            )
            .unwrap();
    }

    #[test]
    fn an_invoice_is_content_addressed_and_binds_its_terms() {
        let world = world(100);
        let invoice = world.x402.invoice_of(&world.invoice_id).unwrap();
        assert_eq!(invoice.amount, Credits(200));
        assert_eq!(invoice.asset, "usdc-eth");
        assert_eq!(invoice.id.len(), 64);
        assert_eq!(invoice.payee, world.payee.as_str());
        // 改一个条件 → 完全不同的 id（内容寻址）。
        let other = Invoice::new(&world.payee, Credits(201), "usdc-eth", 1, 100, "gpu").unwrap();
        assert_ne!(other.id, invoice.id);
        au4a_core::canonicalize(&invoice.to_json()).unwrap();
    }

    #[test]
    fn payments_must_match_the_invoice_amount_exactly() {
        let mut world = world(100);
        let under = pay(&mut world, 199, 10).unwrap_err();
        assert_eq!(under.op, "x402.partial_payment");
        assert_eq!(under.code, RefusalCode::Malformed);
        let over = pay(&mut world, 201, 10).unwrap_err();
        assert_eq!(over.code, RefusalCode::Malformed);
        assert_eq!(world.net.refusals().len(), 2);
        let ok = pay(&mut world, 200, 10).unwrap();
        assert!(ok.detail.contains("支付 200"));
    }

    #[test]
    fn expired_invoices_are_refused() {
        let mut world = world(50);
        let err = pay(&mut world, 200, 50).unwrap_err();
        assert_eq!(err.code, RefusalCode::Timeout);
        assert!(err.detail.contains("过期"));
    }

    #[test]
    fn double_payment_is_refused() {
        let mut world = world(100);
        pay(&mut world, 200, 10).unwrap();
        let err = pay(&mut world, 200, 11).unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
        assert_eq!(
            world.x402.payment_of(&world.invoice_id).unwrap().claimed,
            false
        );
    }

    #[test]
    fn claiming_requires_finality_and_then_moves_the_money() {
        let mut world = world(100);
        pay(&mut world, 200, 10).unwrap();
        escrow(&mut world);
        assert_eq!(world.book.chain_supply(), Credits(200));
        let claim_tx = ChainTx::new(
            ChainId::EthLocal,
            "x402.claim",
            &world.payee,
            2, // 服务方已用 nonce 1 开票
            json!({ "invoice_id": world.invoice_id }),
        )
        .unwrap();
        // 未达最终性 → Timeout。
        let early = world
            .x402
            .claim(
                &mut world.net,
                &mut world.ledger,
                &mut world.book,
                &claim_tx,
            )
            .unwrap_err();
        assert_eq!(early.code, RefusalCode::Timeout);
        // 叠够最终性（1 笔支付在高度 1，finality 2 → 需要高度 ≥ 3）。
        world.net.mine_to(3);
        let ok = world
            .x402
            .claim(
                &mut world.net,
                &mut world.ledger,
                &mut world.book,
                &claim_tx,
            )
            .unwrap();
        assert!(ok.detail.contains("领取 200"));
        assert!(world.x402.is_settled(&world.invoice_id));
        // 链上表示销毁、托管归零，钱到了服务方。
        assert_eq!(world.book.chain_supply(), Credits::ZERO);
        assert_eq!(world.book.escrowed(), Credits::ZERO);
        assert_eq!(world.ledger.balance(&world.payee).available, Credits(200));
        assert_eq!(world.ledger.balance(&world.payer).available, Credits(800));
        world.ledger.check_conservation().unwrap();
        world.book.require_consistent(&world.ledger).unwrap();
    }

    #[test]
    fn claiming_twice_is_refused() {
        let mut world = world(100);
        pay(&mut world, 200, 10).unwrap();
        escrow(&mut world);
        world.net.mine_to(3);
        let claim_tx = ChainTx::new(
            ChainId::EthLocal,
            "x402.claim",
            &world.payee,
            2,
            json!({ "invoice_id": world.invoice_id }),
        )
        .unwrap();
        world
            .x402
            .claim(
                &mut world.net,
                &mut world.ledger,
                &mut world.book,
                &claim_tx,
            )
            .unwrap();
        let again = world
            .x402
            .claim(
                &mut world.net,
                &mut world.ledger,
                &mut world.book,
                &claim_tx,
            )
            .unwrap_err();
        assert_eq!(again.code, RefusalCode::Conflict);
    }

    #[test]
    fn claiming_an_unpaid_invoice_is_refused() {
        let mut world = world(100);
        let claim_tx = ChainTx::new(
            ChainId::EthLocal,
            "x402.claim",
            &world.payee,
            1,
            json!({ "invoice_id": world.invoice_id }),
        )
        .unwrap();
        let err = world
            .x402
            .claim(
                &mut world.net,
                &mut world.ledger,
                &mut world.book,
                &claim_tx,
            )
            .unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
    }

    #[test]
    fn named_refusals_cover_the_x402_unsupported_set() {
        let mut world = world(100);
        for (op, code) in [
            ("x402.partial_payment", RefusalCode::Unsupported),
            ("x402.stream_payment", RefusalCode::Unsupported),
            ("x402.fiat_onramp", RefusalCode::PolicyDenied),
            ("x402.offchain_settle", RefusalCode::Unsupported),
            ("x402.unknown_call", RefusalCode::Unsupported),
        ] {
            let tx = ChainTx::new(ChainId::EthLocal, op, &world.payer, 9, json!({})).unwrap();
            let err = world.x402.execute(&mut world.net, &tx).unwrap_err();
            assert_eq!(err.op, op);
            assert_eq!(err.code, code);
        }
        // 无发票付款 → Malformed（具名拒绝码 pay_without_invoice）。
        let tx = ChainTx::new(
            ChainId::EthLocal,
            "x402.pay",
            &world.payer,
            2,
            json!({ "invoice_id": "nope", "amount": 200, "now": 1 }),
        )
        .unwrap();
        let err = world.x402.execute(&mut world.net, &tx).unwrap_err();
        assert_eq!(err.op, "x402.pay_without_invoice");
        assert_eq!(err.code, RefusalCode::Malformed);
        assert_eq!(X402_REFUSALS.len(), 5);
        assert_eq!(X402_SUPPORTED.len(), 3);
    }

    #[test]
    fn the_x402_projection_is_float_free_and_cpu_proto() {
        let mut world = world(100);
        pay(&mut world, 200, 10).unwrap();
        let value = world.x402.to_json();
        au4a_core::canonicalize(&value).unwrap();
        assert_eq!(value["grade"], json!("cpu-proto"));
        assert_eq!(value["real_network"], json!(false));
        assert_eq!(value["invoice_count"], json!(1));
        assert_eq!(value["settled"], json!(0));
    }

    #[test]
    fn a_claim_without_escrow_is_refused_by_fail_closed() {
        let mut world = world(100);
        pay(&mut world, 200, 10).unwrap();
        // 故意不托管：双轨不一致 → 领取必须被 fail-closed 拦住。
        world.net.mine_to(3);
        let claim_tx = ChainTx::new(
            ChainId::EthLocal,
            "x402.claim",
            &world.payee,
            1,
            json!({ "invoice_id": world.invoice_id }),
        )
        .unwrap();
        let err = world
            .x402
            .claim(
                &mut world.net,
                &mut world.ledger,
                &mut world.book,
                &claim_tx,
            )
            .unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
        assert!(err.detail.contains("fail-closed"));
        assert_eq!(world.ledger.balance(&world.payee).available, Credits::ZERO);
    }
}
