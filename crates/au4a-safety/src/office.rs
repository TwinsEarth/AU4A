//! v1.5.2 安全服务（`SafetyOffice`）：受理举报，并把每次状态变更写进哈希链。
//!
//! **无罪不罚在这里是结构性保证，不是纪律要求**：`report()` 路径上不存在任何
//! 账本写入调用。信誉同理——`AgentCard` 只读不写。要动账本，必须有一条带仲裁者
//! 签名的裁决（v1.5.4 / v1.5.7），这条路径在 `report()` 里根本不可达。
//!
//! 服务自身也是一个 Agent（`service` 密钥）：它与其它 Agent 用同一套签名原语，
//! 没有「运营方」这种超越身份的角色。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Credits, Did, Envelope, RefusalCode};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

use crate::appeal::Appeal;
use crate::case::{Case, CaseStatus, ViolationKind, ViolationReport};
use crate::chain::{chain_head, verify_chain, ChainVerdict, SafetyEvent, SafetyEventKind};
use crate::config::SafetyConfig;
use crate::evidence::EvidenceRef;
use crate::notify::{Notification, Subscription};
use crate::penalty::{PenaltyOrder, PenaltyRecord};
use crate::permission::PermissionBoundary;
use crate::pmb::{self, SafetyMessage};

/// 安全服务：案件登记处 + 变更日志。
pub struct SafetyOffice {
    config: SafetyConfig,
    service: AgentKeys,
    events: Vec<SafetyEvent>,
    cases: BTreeMap<String, Case>,
    appeals: BTreeMap<String, Appeal>,
    penalties: Vec<PenaltyRecord>,
    subscriptions: BTreeMap<String, Subscription>,
    notifications: Vec<Notification>,
}

impl SafetyOffice {
    /// 构造服务。服务密钥必须与配置中的 `service` DID 一致——
    /// 否则任何人都能拿别的密钥冒充安全服务签发回执。
    pub fn new(config: SafetyConfig, service: AgentKeys) -> CoreResult<Self> {
        if service.did() != config.service {
            return Err(CoreError::InvalidSignature);
        }
        Ok(Self {
            config,
            service,
            events: Vec::new(),
            cases: BTreeMap::new(),
            appeals: BTreeMap::new(),
            penalties: Vec::new(),
            subscriptions: BTreeMap::new(),
            notifications: Vec::new(),
        })
    }

    pub fn config(&self) -> &SafetyConfig {
        &self.config
    }

    /// 服务身份（它也是一个 Agent）。DID 直接由持有的密钥派生，
    /// 因此「谁是安全服务」永远等于「谁持有能签发回执的那把密钥」。
    pub fn service_did(&self) -> Did {
        self.service.did()
    }

    pub fn events(&self) -> &[SafetyEvent] {
        &self.events
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// 当前链头（空链为创世前驱）。
    pub fn chain_head(&self) -> String {
        chain_head(&self.events)
    }

    pub fn verify_chain(&self) -> ChainVerdict {
        verify_chain(&self.events)
    }

    pub fn case(&self, id: &str) -> Option<&Case> {
        self.cases.get(id)
    }

    pub fn case_count(&self) -> usize {
        self.cases.len()
    }

    pub fn cases(&self) -> impl Iterator<Item = &Case> {
        self.cases.values()
    }

    /// 追加一条链上事件。序号、时间、前驱全部由链自身决定，调用方无法伪造。
    pub(crate) fn append(
        &mut self,
        kernel: &mut Kernel,
        kind: SafetyEventKind,
        payload: Value,
    ) -> CoreResult<()> {
        let at = kernel.tick();
        let seq = self.events.len() as u64;
        let prev = chain_head(&self.events);
        let event = SafetyEvent::seal(seq, at, kind, payload, &prev)?;
        self.events.push(event);
        Ok(())
    }

    /// 受理一次举报。**不触碰账本，不触碰信誉。**
    ///
    /// 证据本体随举报提交：安全服务必须能复算摘要，否则「带哈希」只是装饰。
    pub fn report(
        &mut self,
        kernel: &mut Kernel,
        keys: &AgentKeys,
        subject: &Did,
        violation: ViolationKind,
        evidence: EvidenceRef,
        payload: &Value,
    ) -> CoreResult<ViolationReport> {
        let at = kernel.tick();
        let report = ViolationReport::new(
            keys.did(),
            subject.clone(),
            violation,
            evidence,
            at,
        )
        .sign(keys)?;
        self.accept_report(kernel, &report, payload)?;
        Ok(report)
    }

    /// 受理一份**已经签名**的举报。本地 API 与 PMB `safety.report` 路径共用这一套检查，
    /// 因此「走网络」不会绕开任何一条不变式。
    pub fn accept_report(
        &mut self,
        kernel: &mut Kernel,
        report: &ViolationReport,
        payload: &Value,
    ) -> CoreResult<()> {
        let reporter = report.reporter.clone();
        let subject = report.subject.clone();

        if kernel.card(&reporter).is_none() {
            kernel.refuse(
                &reporter,
                RefusalCode::Unauthorized,
                "report from an unregistered did",
            );
            return Err(CoreError::UnknownAgent);
        }
        if reporter == subject {
            kernel.refuse(
                &reporter,
                RefusalCode::PolicyDenied,
                "a self-report is not a violation report",
            );
            return Err(CoreError::InvalidKind);
        }
        if kernel.card(&subject).is_none() {
            kernel.refuse(
                &reporter,
                RefusalCode::StaleEpoch,
                "report subject is not a registered agent",
            );
            return Err(CoreError::UnknownAgent);
        }
        if !report.evidence.is_well_formed() {
            kernel.refuse(
                &reporter,
                RefusalCode::PolicyDenied,
                "evidence digest is missing or malformed",
            );
            return Err(CoreError::InvalidSignature);
        }
        if payload.is_null() {
            kernel.refuse(
                &reporter,
                RefusalCode::PolicyDenied,
                "evidence payload is missing so the digest cannot be recomputed",
            );
            return Err(CoreError::Encoding);
        }
        if report.evidence.verify(payload).is_err() {
            kernel.refuse(
                &reporter,
                RefusalCode::PolicyDenied,
                "evidence digest does not match the submitted payload",
            );
            return Err(CoreError::InvalidSignature);
        }
        report.verify()?;

        let at = report.at;
        let case = Case::from_report(report);
        let case_id = case.id.clone();
        self.cases.insert(case_id.clone(), case);
        self.append(
            kernel,
            SafetyEventKind::Reported,
            json!({"case": case_id, "report": report.to_json()?}),
        )?;
        // 未确认状态的变更同样要通知订阅者（状态已变，只是账本没动）。
        self.deliver(&case_id, CaseStatus::Reported, 0, at);
        kernel.emit(
            &format!("{}.reported", crate::TRACK),
            format!(
                "{} 举报 {}（{}）",
                au4a_core::short_id(reporter.as_str()),
                au4a_core::short_id(subject.as_str()),
                report.violation.as_str()
            ),
        );
        Ok(())
    }

    /// 案件的处理状态（未确认 == `reported`）。
    pub fn status_of(&self, case_id: &str) -> Option<CaseStatus> {
        self.cases.get(case_id).map(|c| c.status)
    }

    pub fn appeal_by_id(&self, id: &str) -> Option<&Appeal> {
        self.appeals.get(id)
    }

    pub fn appeal_count(&self) -> usize {
        self.appeals.len()
    }

    /// 某案件的申诉（按提交顺序）。
    pub fn appeals_of(&self, case_id: &str) -> Vec<&Appeal> {
        self.cases
            .get(case_id)
            .map(|case| {
                case.appeals
                    .iter()
                    .filter_map(|id| self.appeals.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 受理一次申诉。**只有案件主体本人**能申诉；申诉只提交证据、推状态、写链，
    /// 不触碰账本——罚没与归还永远只发生在裁决路径上。
    ///
    /// 申诉权不被终局性剥夺：已 `arbitrated` 的案件收到新证据后回到 `appealed`，
    /// 由下一次裁决重新处理。
    pub fn appeal(
        &mut self,
        kernel: &mut Kernel,
        keys: &AgentKeys,
        case_id: &str,
        evidence: Vec<EvidenceRef>,
        payloads: &[Value],
    ) -> CoreResult<Appeal> {
        let at = kernel.tick();
        let appeal = Appeal::new(case_id.to_string(), keys.did(), evidence, at).sign(keys)?;
        self.accept_appeal(kernel, &appeal, payloads)?;
        Ok(appeal)
    }

    /// 受理一份**已经签名**的申诉。本地 API 与 PMB `safety.appeal` 路径共用这一套检查。
    pub fn accept_appeal(
        &mut self,
        kernel: &mut Kernel,
        appeal: &Appeal,
        payloads: &[Value],
    ) -> CoreResult<()> {
        let appellant = appeal.appellant.clone();
        let case_id = appeal.case.clone();

        let subject = match self.cases.get(&case_id) {
            Some(case) => case.subject.clone(),
            None => {
                kernel.refuse(
                    &appellant,
                    RefusalCode::StaleEpoch,
                    "appeal references an unknown case",
                );
                return Err(CoreError::UnknownAgent);
            }
        };
        if appellant != subject {
            kernel.refuse(
                &appellant,
                RefusalCode::Unauthorized,
                "only the case subject may appeal; a third party cannot sign for them",
            );
            return Err(CoreError::InvalidSignature);
        }
        if appeal.evidence.is_empty() || appeal.evidence.len() != payloads.len() {
            kernel.refuse(
                &appellant,
                RefusalCode::PolicyDenied,
                "appeal evidence is missing or its payloads do not line up",
            );
            return Err(CoreError::InvalidSignature);
        }
        for (reference, payload) in appeal.evidence.iter().zip(payloads.iter()) {
            if payload.is_null() || reference.verify(payload).is_err() {
                kernel.refuse(
                    &appellant,
                    RefusalCode::PolicyDenied,
                    "appeal evidence digest does not match its payload",
                );
                return Err(CoreError::InvalidSignature);
            }
        }
        appeal.verify()?;

        let at = appeal.at;
        let seq = self.events.len() as u64;
        if let Some(case) = self.cases.get_mut(&case_id) {
            case.status = CaseStatus::Appealed;
            case.status_seq = seq;
            case.appeals.push(appeal.id.clone());
        }
        self.appeals.insert(appeal.id.clone(), appeal.clone());
        self.append(
            kernel,
            SafetyEventKind::Appealed,
            json!({"case": case_id, "appeal": appeal.to_json()?}),
        )?;
        self.deliver(&case_id, CaseStatus::Appealed, seq, at);
        kernel.emit(
            &format!("{}.appealed", crate::TRACK),
            format!(
                "案件 {} 收到申诉（证据 {} 条）",
                au4a_core::short_id(&case_id),
                appeal.evidence_count()
            ),
        );
        Ok(())
    }

    // ---- v1.5.4 处罚 ----

    pub fn penalty_count(&self) -> usize {
        self.penalties.len()
    }

    pub fn penalty(&self, id: &str) -> Option<&PenaltyRecord> {
        self.penalties.iter().find(|record| record.id == id)
    }

    /// 处罚查询：按主体返回记录（按执行顺序）。
    pub fn penalties_for(&self, did: &Did) -> Vec<&PenaltyRecord> {
        self.penalties
            .iter()
            .filter(|record| &record.subject == did)
            .collect()
    }

    /// 全部处罚记录（按执行顺序）。
    pub fn penalties(&self) -> &[PenaltyRecord] {
        &self.penalties
    }

    /// 至今净罚没额（Σ执行 − Σ已回滚）。
    pub fn slashed_total(&self) -> CoreResult<Credits> {
        let mut total = Credits::ZERO;
        for record in &self.penalties {
            if !record.reversed {
                total = total.checked_add(record.applied)?;
            }
        }
        Ok(total)
    }

    /// 执行一份仲裁者签名的处罚契约。**这是账本唯一的改动入口。**
    ///
    /// 罚没额按锁定余额封顶：记录里同时保留「请求额」与「实际执行额」，
    /// 因此请求 100、只有 20 可罚时，记录不会假装罚了 100。
    pub fn apply_penalty_order(
        &mut self,
        kernel: &mut Kernel,
        order: PenaltyOrder,
    ) -> CoreResult<PenaltyRecord> {
        if order.verify_against(&self.config).is_err() {
            kernel.refuse(
                &order.arbiter,
                RefusalCode::Unauthorized,
                "penalty order is not signed by a trusted arbiter",
            );
            return Err(CoreError::InvalidSignature);
        }
        let case = match self.cases.get(&order.case) {
            Some(case) => case.clone(),
            None => {
                kernel.refuse(
                    &order.arbiter,
                    RefusalCode::StaleEpoch,
                    "penalty order references an unknown case",
                );
                return Err(CoreError::UnknownAgent);
            }
        };
        if case.subject != order.subject {
            kernel.refuse(
                &order.arbiter,
                RefusalCode::PolicyDenied,
                "penalty order subject does not match the case subject",
            );
            return Err(CoreError::InvalidSignature);
        }

        let at = kernel.tick();
        let mut applied = Credits::ZERO;
        if order.sanction.moves_ledger() {
            let locked = kernel.ledger().balance(&order.subject).locked;
            let take = if order.amount > locked {
                locked
            } else {
                order.amount
            };
            if take > Credits::ZERO {
                kernel.ledger_mut().slash(&order.subject, take)?;
                applied = take;
            }
        }

        let record = PenaltyRecord {
            id: order.id.clone(),
            case: order.case.clone(),
            subject: order.subject.clone(),
            sanction: order.sanction,
            requested: order.amount,
            applied,
            arbiter: order.arbiter.clone(),
            at,
            reversed: false,
        };

        let seq = self.events.len() as u64;
        if let Some(target) = self.cases.get_mut(&order.case) {
            target.status = CaseStatus::Penalized;
            target.status_seq = seq;
            target.penalties.push(record.id.clone());
        }
        self.penalties.push(record.clone());
        self.append(
            kernel,
            SafetyEventKind::Penalized,
            json!({"case": order.case, "penalty": record.to_json()?}),
        )?;
        self.deliver(&record.case, CaseStatus::Penalized, seq, at);
        kernel.emit(
            &format!("{}.penalized", crate::TRACK),
            format!(
                "案件 {} 执行 {}（请求 {}，实际 {}）",
                au4a_core::short_id(&record.case),
                record.sanction.as_str(),
                record.requested,
                record.applied
            ),
        );
        Ok(record)
    }

    // ---- v1.5.5 通知 ----

    pub fn subscription(&self, id: &str) -> Option<&Subscription> {
        self.subscriptions.get(id)
    }

    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// 某案件的订阅（按订阅 id 升序，确定性）。
    pub fn subscriptions_of(&self, case_id: &str) -> Vec<&Subscription> {
        self.subscriptions
            .values()
            .filter(|sub| sub.case == case_id)
            .collect()
    }

    pub fn notification_count(&self) -> usize {
        self.notifications.len()
    }

    /// 某 Agent 的通知收件箱（按投递顺序）。
    pub fn inbox(&self, did: &Did) -> Vec<&Notification> {
        self.notifications
            .iter()
            .filter(|n| &n.to == did)
            .collect()
    }

    pub fn notifications_of_case(&self, case_id: &str) -> Vec<&Notification> {
        self.notifications
            .iter()
            .filter(|n| n.case == case_id)
            .collect()
    }

    fn notify_one(
        &mut self,
        subscription: &str,
        to: &Did,
        case_id: &str,
        status: CaseStatus,
        seq: u64,
        at: u64,
    ) {
        self.notifications.push(Notification {
            subscription: subscription.to_string(),
            to: to.clone(),
            case: case_id.to_string(),
            status,
            seq,
            at,
        });
    }

    /// 把一次状态变更投递给所有匹配的订阅（派生投影，不写链）。
    fn deliver(&mut self, case_id: &str, status: CaseStatus, seq: u64, at: u64) {
        let targets: Vec<(String, Did)> = self
            .subscriptions
            .values()
            .filter(|sub| sub.case == case_id && sub.watches(status))
            .map(|sub| (sub.id.clone(), sub.subscriber.clone()))
            .collect();
        for (subscription, to) in targets {
            self.notify_one(&subscription, &to, case_id, status, seq, at);
        }
    }

    /// 建立一个订阅。**订阅即回放当前状态快照**，但不投递订阅前的历史。
    pub fn subscribe(
        &mut self,
        kernel: &mut Kernel,
        keys: &AgentKeys,
        case_id: &str,
        statuses: Vec<CaseStatus>,
    ) -> CoreResult<Subscription> {
        let subscriber = keys.did();
        let (status, status_seq) = match self.cases.get(case_id) {
            Some(case) => (case.status, case.status_seq),
            None => {
                kernel.refuse(
                    &subscriber,
                    RefusalCode::StaleEpoch,
                    "subscription references an unknown case",
                );
                return Err(CoreError::UnknownAgent);
            }
        };
        if statuses.is_empty() {
            kernel.refuse(
                &subscriber,
                RefusalCode::PolicyDenied,
                "a subscription without statuses does not say what to watch",
            );
            return Err(CoreError::InvalidKind);
        }
        let at = kernel.tick();
        let subscription =
            Subscription::new(case_id.to_string(), subscriber.clone(), statuses, at).sign(keys)?;
        subscription.verify()?;
        self.subscriptions
            .insert(subscription.id.clone(), subscription.clone());
        self.append(
            kernel,
            SafetyEventKind::Subscribed,
            json!({"case": case_id, "subscription": subscription.to_json()?}),
        )?;
        if subscription.watches(status) {
            self.notify_one(&subscription.id, &subscriber, case_id, status, status_seq, at);
        }
        kernel.emit(
            &format!("{}.subscribed", crate::TRACK),
            format!(
                "{} 订阅案件 {} 的 {} 个状态",
                au4a_core::short_id(subscriber.as_str()),
                au4a_core::short_id(case_id),
                subscription.statuses.len()
            ),
        );
        Ok(subscription)
    }

    /// 退订。只有订阅者本人可以撤销自己的订阅。
    pub fn unsubscribe(
        &mut self,
        kernel: &mut Kernel,
        keys: &AgentKeys,
        subscription_id: &str,
    ) -> CoreResult<Subscription> {
        let requester = keys.did();
        let subscription = match self.subscriptions.get(subscription_id) {
            Some(subscription) => subscription.clone(),
            None => {
                kernel.refuse(
                    &requester,
                    RefusalCode::StaleEpoch,
                    "unsubscribe references an unknown subscription",
                );
                return Err(CoreError::UnknownAgent);
            }
        };
        if subscription.subscriber != requester {
            kernel.refuse(
                &requester,
                RefusalCode::Unauthorized,
                "only the subscriber may cancel their own subscription",
            );
            return Err(CoreError::InvalidSignature);
        }
        self.subscriptions.remove(subscription_id);
        self.append(
            kernel,
            SafetyEventKind::Unsubscribed,
            json!({"case": subscription.case, "subscription": subscription_id}),
        )?;
        kernel.emit(
            &format!("{}.unsubscribed", crate::TRACK),
            format!(
                "{} 退订 {}",
                au4a_core::short_id(requester.as_str()),
                au4a_core::short_id(subscription_id)
            ),
        );
        Ok(subscription)
    }

    // ---- v1.5.6 PMB ----

    /// 处理一个进来的 PMB 信封。
    ///
    /// * `Ok(None)`：不是发给本服务的（原样忽略，不做任何状态变更）。
    /// * `Ok(Some(receipt))`：受理成功；回执由**服务身份**签名并带 `in_reply_to`。
    /// * `Err(_)`：格式、签名、身份或证据不成立。拒绝不产生回执，但服务侧会留下
    ///   一条类型化拒绝记录（`kernel.refusals()`），因此拒绝本身同样可审计。
    pub fn handle(
        &mut self,
        kernel: &mut Kernel,
        env: &Envelope,
    ) -> CoreResult<Option<Envelope>> {
        if let Some(to) = &env.to {
            if to != &self.service.did() {
                return Ok(None);
            }
        }
        let message = pmb::classify(env)?;
        let in_reply_kind = message.kind();
        let at = kernel.tick();
        let result = match message {
            SafetyMessage::Query(query) => {
                let boundary = PermissionBoundary::of(kernel, &self.config, &query.about);
                json!({
                    "query": "permissions",
                    "about": query.about.as_str(),
                    "boundary": boundary.to_json()?,
                    "cases": self.case_count(),
                    "chain_head": self.chain_head(),
                })
            }
            SafetyMessage::Report { report, evidence } => {
                self.accept_report(kernel, &report, &evidence)?;
                json!({
                    "accepted": true,
                    "case": report.id,
                    "status": self.status_of(&report.id).map(|s| s.as_str()),
                    "chain_head": self.chain_head(),
                })
            }
            SafetyMessage::Appeal { appeal, evidence } => {
                self.accept_appeal(kernel, &appeal, &evidence)?;
                json!({
                    "accepted": true,
                    "case": appeal.case,
                    "appeal": appeal.id,
                    "status": self.status_of(&appeal.case).map(|s| s.as_str()),
                    "chain_head": self.chain_head(),
                })
            }
        };
        let receipt =
            pmb::receipt_envelope(&self.service, &env.from, at, &env.id, in_reply_kind, result)?;
        kernel.emit(
            &format!("{}.handled", crate::TRACK),
            format!(
                "{} → 回执 {}",
                in_reply_kind,
                au4a_core::short_id(&receipt.id)
            ),
        );
        Ok(Some(receipt))
    }
}

/// 账本快照：参与者逐字段 + 全局发行/罚没。用于「未确认不动账本」的可比较断言。
///
/// 账户按 DID 排序，因此同一状态永远得到同一份投影。
pub fn ledger_snapshot(kernel: &Kernel, dids: &[Did]) -> Value {
    let mut accounts: Vec<Value> = dids
        .iter()
        .map(|did| {
            let account = kernel.ledger().balance(did);
            json!({
                "did": did.as_str(),
                "available": account.available.get(),
                "locked": account.locked.get(),
            })
        })
        .collect();
    accounts.sort_by(|a, b| a["did"].as_str().cmp(&b["did"].as_str()));
    json!({
        "minted": kernel.ledger().minted().get(),
        "slashed": kernel.ledger().slashed().get(),
        "accounts": accounts,
    })
}

/// 账本指纹：快照的规范哈希，便于在 JSON 证据里做等值断言。
pub fn ledger_fingerprint(kernel: &Kernel, dids: &[Did]) -> CoreResult<String> {
    canonical_hash(&ledger_snapshot(kernel, dids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::EvidenceKind;
    use crate::penalty::SanctionKind;
    use crate::setup;
    use au4a_core::Credits;
    use au4a_kernel::KernelConfig;

    struct World {
        kernel: Kernel,
        office: SafetyOffice,
        reporter: AgentKeys,
        subject: AgentKeys,
        arbiter: AgentKeys,
        participants: Vec<Did>,
    }

    fn world() -> World {
        let mut kernel = Kernel::new(KernelConfig::default());
        let service = setup::keys(setup::ROLE_SERVICE);
        let reporter = setup::keys(setup::ROLE_REPORTER);
        let subject = setup::keys(setup::ROLE_SUBJECT);
        let arbiter = setup::keys(setup::ROLE_ARBITER);
        for (keys, display, skill) in [
            (&service, "safety-service", "safety.api"),
            (&reporter, "reporter-agent", "audit.report"),
            (&subject, "subject-agent", "deliver.task"),
        ] {
            setup::ensure_agent(&mut kernel, keys, display, &[skill], Credits(20)).unwrap();
        }
        let config = SafetyConfig::single_arbiter(service.did(), arbiter.did());
        let office = SafetyOffice::new(config, service).unwrap();
        let participants = vec![reporter.did(), subject.did()];
        World {
            kernel,
            office,
            reporter,
            subject,
            arbiter,
            participants,
        }
    }

    fn evidence(tag: &str) -> (EvidenceRef, Value) {
        let payload = json!({"tag": tag, "delivered": false});
        let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload).unwrap();
        (reference, payload)
    }

    #[test]
    fn a_report_opens_an_unconfirmed_case_and_grows_the_chain() {
        let mut w = world();
        let (reference, payload) = evidence("case-1");
        let report = w
            .office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::NonDelivery,
                reference,
                &payload,
            )
            .unwrap();

        assert_eq!(w.office.case_count(), 1);
        let case = w.office.case(&report.id).unwrap();
        assert_eq!(case.status, CaseStatus::Reported);
        assert_eq!(case.reporter, w.reporter.did());
        assert_eq!(case.subject, w.subject.did());

        assert_eq!(w.office.event_count(), 1);
        assert_eq!(w.office.events()[0].kind, SafetyEventKind::Reported);
        let verdict = w.office.verify_chain();
        assert!(verdict.ok, "{verdict:?}");
        assert_eq!(verdict.head, w.office.chain_head());
        assert_eq!(verdict.head.len(), 64);
    }

    #[test]
    fn an_unconfirmed_report_moves_no_balance_and_no_card() {
        let mut w = world();
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let before_fingerprint = ledger_fingerprint(&w.kernel, &w.participants).unwrap();
        let subject_card_before = w.kernel.card(&w.subject.did()).cloned();
        let reporter_card_before = w.kernel.card(&w.reporter.did()).cloned();

        let (reference, payload) = evidence("case-2");
        let report = w
            .office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Fraud,
                reference,
                &payload,
            )
            .unwrap();

        // 无罪不罚：逐字段相等，而不是「差值很小」。
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(
            ledger_fingerprint(&w.kernel, &w.participants).unwrap(),
            before_fingerprint
        );
        assert_eq!(w.kernel.card(&w.subject.did()).cloned(), subject_card_before);
        assert_eq!(
            w.kernel.card(&w.reporter.did()).cloned(),
            reporter_card_before
        );
        assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
        w.kernel.ledger().check_conservation().unwrap();
        // 案件仍未被确认。
        assert_eq!(w.office.status_of(&report.id), Some(CaseStatus::Reported));
    }

    #[test]
    fn forged_evidence_is_refused_and_leaves_no_trace() {
        let mut w = world();
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let (honest, _) = evidence("case-3");
        let (_, other_payload) = evidence("case-3-forged");

        let result = w.office.report(
            &mut w.kernel,
            &w.reporter,
            &w.subject.did(),
            ViolationKind::FakeEvidence,
            honest,
            &other_payload,
        );
        assert_eq!(result, Err(CoreError::InvalidSignature));
        assert_eq!(w.office.case_count(), 0);
        assert_eq!(w.office.event_count(), 0);
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::PolicyDenied);
        assert!(refusal.reason.contains("does not match"));
    }

    #[test]
    fn missing_or_malformed_evidence_is_refused() {
        let mut w = world();
        // 摘要不成形（缺失）。
        let malformed = EvidenceRef {
            kind: EvidenceKind::Transcript,
            digest: String::new(),
        };
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Spam,
                malformed,
                &json!({"x": 1}),
            ),
            Err(CoreError::InvalidSignature)
        );
        // 摘要成形但证据本体缺失：无法复算。
        let (good, _) = evidence("case-4");
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Spam,
                good,
                &Value::Null,
            ),
            Err(CoreError::Encoding)
        );
        assert_eq!(w.office.event_count(), 0);
        assert_eq!(w.office.case_count(), 0);
    }

    #[test]
    fn self_reports_and_unknown_subjects_are_refused() {
        let mut w = world();
        let (reference, payload) = evidence("case-5");
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &w.reporter,
                &w.reporter.did(),
                ViolationKind::Spam,
                reference,
                &payload,
            ),
            Err(CoreError::InvalidKind)
        );
        let (reference, payload) = evidence("case-6");
        let ghost = AgentKeys::from_seed(&[0x9e; 32]).did();
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &w.reporter,
                &ghost,
                ViolationKind::Spam,
                reference,
                &payload,
            ),
            Err(CoreError::UnknownAgent)
        );
        assert_eq!(w.office.event_count(), 0);
        let codes: Vec<RefusalCode> = w.kernel.refusals().iter().map(|(_, r)| r.code).collect();
        assert_eq!(
            codes,
            vec![RefusalCode::PolicyDenied, RefusalCode::StaleEpoch]
        );
    }

    #[test]
    fn an_unregistered_agent_cannot_report() {
        let mut w = world();
        let outsider = AgentKeys::from_seed(&[0x5a; 32]);
        let (reference, payload) = evidence("case-7");
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &outsider,
                &w.subject.did(),
                ViolationKind::Spam,
                reference,
                &payload,
            ),
            Err(CoreError::UnknownAgent)
        );
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::Unauthorized);
        assert_eq!(w.office.event_count(), 0);
    }

    #[test]
    fn consecutive_reports_link_into_one_chain() {
        let mut w = world();
        let third = setup::keys(setup::ROLE_ARBITER);
        setup::ensure_agent(&mut w.kernel, &third, "third", &["x"], Credits(20)).unwrap();
        for (tag, subject) in [("c-1", w.subject.did()), ("c-2", third.did())] {
            let (reference, payload) = evidence(tag);
            w.office
                .report(
                    &mut w.kernel,
                    &w.reporter,
                    &subject,
                    ViolationKind::NonDelivery,
                    reference,
                    &payload,
                )
                .unwrap();
        }
        assert_eq!(w.office.event_count(), 2);
        let events = w.office.events();
        assert_eq!(events[1].prev, events[0].hash);
        assert_eq!(w.office.verify_chain().ok, true);
    }

    #[test]
    fn a_tampered_copy_of_the_chain_is_detected() {
        let mut w = world();
        let (reference, payload) = evidence("case-8");
        w.office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Collusion,
                reference,
                &payload,
            )
            .unwrap();
        let mut tampered = w.office.events().to_vec();
        tampered[0].payload = json!({"case": "forged", "report": {}});
        let verdict = verify_chain(&tampered);
        assert!(!verdict.ok);
        assert_eq!(verdict.broken_at, Some(0));
        // 服务自身的链不受影响（篡改的是副本）。
        assert!(w.office.verify_chain().ok);
    }

    #[test]
    fn the_service_identity_must_match_the_configured_did() {
        let service = setup::keys(setup::ROLE_SERVICE);
        let service_did = service.did();
        let impostor = setup::keys(0x7f);
        let config = SafetyConfig::single_arbiter(service_did.clone(), setup::keys(setup::ROLE_ARBITER).did());
        assert_eq!(
            SafetyOffice::new(config.clone(), impostor).err(),
            Some(CoreError::InvalidSignature)
        );
        let office = SafetyOffice::new(config, service).unwrap();
        assert_eq!(office.service_did(), service_did);
    }

    #[test]
    fn ledger_snapshot_is_order_independent_and_fingerprinted() {
        let mut w = world();
        let (reference, payload) = evidence("case-9");
        w.office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Spam,
                reference,
                &payload,
            )
            .unwrap();
        let forward = ledger_snapshot(&w.kernel, &w.participants);
        let mut reversed = w.participants.clone();
        reversed.reverse();
        assert_eq!(ledger_snapshot(&w.kernel, &reversed), forward);
        assert_eq!(
            ledger_fingerprint(&w.kernel, &w.participants).unwrap(),
            ledger_fingerprint(&w.kernel, &reversed).unwrap()
        );
    }

    // ---- v1.5.3 申诉 ----

    fn appeal_evidence(tag: &str) -> (Vec<EvidenceRef>, Vec<Value>) {
        let payloads = vec![
            json!({"tag": tag, "kind": "delivery-receipt", "delivered": true}),
            json!({"tag": tag, "kind": "witness-statement", "witness": "peer"}),
        ];
        let references = payloads
            .iter()
            .map(|p| EvidenceRef::commit(EvidenceKind::Witness, p).unwrap())
            .collect();
        (references, payloads)
    }

    fn open_case(w: &mut World, tag: &str) -> String {
        let (reference, payload) = evidence(tag);
        w.office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::NonDelivery,
                reference,
                &payload,
            )
            .unwrap()
            .id
    }

    #[test]
    fn the_subject_can_appeal_and_the_case_becomes_appealed() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-1");
        let (references, payloads) = appeal_evidence("appeal-1");
        let appeal = w
            .office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();

        assert_eq!(appeal.appellant, w.subject.did());
        assert_eq!(appeal.case, case_id);
        assert_eq!(appeal.evidence_count(), 2);
        appeal.verify().unwrap();

        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Appealed));
        assert_eq!(
            w.office.case(&case_id).unwrap().appeals,
            vec![appeal.id.clone()]
        );
        assert_eq!(w.office.appeal_count(), 1);
        assert_eq!(w.office.event_count(), 2);
        assert_eq!(w.office.events()[1].kind, SafetyEventKind::Appealed);
        assert!(w.office.verify_chain().ok);
    }

    #[test]
    fn an_appeal_moves_no_ledger_entry_and_no_card() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-2");
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let fingerprint_before = ledger_fingerprint(&w.kernel, &w.participants).unwrap();
        let subject_card_before = w.kernel.card(&w.subject.did()).cloned();

        let (references, payloads) = appeal_evidence("appeal-2");
        w.office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();

        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(
            ledger_fingerprint(&w.kernel, &w.participants).unwrap(),
            fingerprint_before
        );
        assert_eq!(w.kernel.card(&w.subject.did()).cloned(), subject_card_before);
        assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
        w.kernel.ledger().check_conservation().unwrap();
    }

    #[test]
    fn a_third_party_cannot_appeal_on_behalf_of_the_subject() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-3");
        let (references, payloads) = appeal_evidence("appeal-3");
        // 举报人替被举报人「代为申诉」：签名不是主体的，密码学上就不成立。
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.reporter, &case_id, references, &payloads),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Reported));
        assert_eq!(w.office.appeal_count(), 0);
        assert_eq!(w.office.event_count(), 1);
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::Unauthorized);
    }

    #[test]
    fn an_appeal_for_an_unknown_case_is_refused() {
        let mut w = world();
        let (references, payloads) = appeal_evidence("appeal-4");
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.subject, "no-such-case", references, &payloads),
            Err(CoreError::UnknownAgent)
        );
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::StaleEpoch);
        assert_eq!(w.office.event_count(), 0);
    }

    #[test]
    fn forged_appeal_evidence_is_refused_without_state_change() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-5");
        let (references, _) = appeal_evidence("appeal-5");
        let other_payloads = vec![json!({"tag": "appeal-5", "forged": true})];
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.subject, &case_id, references, &other_payloads),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Reported));
        assert_eq!(w.office.appeal_count(), 0);
        assert_eq!(w.office.event_count(), 1);
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::PolicyDenied);
    }

    #[test]
    fn an_appeal_without_evidence_or_with_mismatched_payloads_is_refused() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-6");
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.subject, &case_id, Vec::new(), &[]),
            Err(CoreError::InvalidSignature)
        );
        let (references, payloads) = appeal_evidence("appeal-6");
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads[..1]),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(w.office.appeal_count(), 0);
    }

    #[test]
    fn appeals_are_repeatable_with_new_evidence() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-7");
        let mut ids = Vec::new();
        for tag in ["appeal-7-a", "appeal-7-b"] {
            let (references, payloads) = appeal_evidence(tag);
            let appeal = w
                .office
                .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
                .unwrap();
            ids.push(appeal.id);
        }
        // 终局性属于裁决，不属于申诉人：新证据可以再次申诉。
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Appealed));
        assert_eq!(w.office.case(&case_id).unwrap().appeals, ids);
        let indexed: Vec<String> = w
            .office
            .appeals_of(&case_id)
            .iter()
            .map(|a| a.id.clone())
            .collect();
        assert_eq!(indexed, w.office.case(&case_id).unwrap().appeals);
        assert_eq!(w.office.event_count(), 3);
        assert!(w.office.verify_chain().ok);
    }

    // ---- v1.5.4 处罚 ----

    fn slash_order(case_id: &str, subject: &Did, amount: i64, arbiter: &AgentKeys) -> PenaltyOrder {
        PenaltyOrder::new(
            case_id,
            subject.clone(),
            SanctionKind::StakeSlash,
            Credits(amount),
            arbiter.did(),
            0,
        )
    }

    #[test]
    fn an_arbiter_order_slashes_the_stake_and_is_recorded() {
        let mut w = world();
        let case_id = open_case(&mut w, "penalty-1");
        let subject_did = w.subject.did();
        let locked_before = w.kernel.ledger().balance(&subject_did).locked;

        let order = slash_order(&case_id, &subject_did, 5, &w.arbiter)
            .sign(&w.arbiter)
            .unwrap();
        let record = w.office.apply_penalty_order(&mut w.kernel, order).unwrap();

        assert_eq!(record.requested, Credits(5));
        assert_eq!(record.applied, Credits(5));
        assert_eq!(record.sanction, SanctionKind::StakeSlash);
        assert_eq!(record.arbiter, w.arbiter.did());
        assert!(!record.reversed);

        assert_eq!(
            w.kernel.ledger().balance(&subject_did).locked,
            locked_before.checked_sub(Credits(5)).unwrap()
        );
        assert_eq!(w.kernel.ledger().slashed(), Credits(5));
        w.kernel.ledger().check_conservation().unwrap();

        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Penalized));
        assert_eq!(
            w.office.case(&case_id).unwrap().penalties,
            vec![record.id.clone()]
        );
        assert_eq!(w.office.penalty_count(), 1);
        assert_eq!(w.office.penalty(&record.id), Some(&record));
        assert_eq!(w.office.slashed_total().unwrap(), Credits(5));
        assert_eq!(w.office.events().last().unwrap().kind, SafetyEventKind::Penalized);
        assert!(w.office.verify_chain().ok);
    }

    #[test]
    fn an_order_from_an_untrusted_arbiter_is_refused_and_nothing_moves() {
        let mut w = world();
        let case_id = open_case(&mut w, "penalty-2");
        let subject_did = w.subject.did();
        let before = ledger_snapshot(&w.kernel, &w.participants);

        let outsider = AgentKeys::from_seed(&[0x66; 32]);
        let order = slash_order(&case_id, &subject_did, 5, &outsider)
            .sign(&outsider)
            .unwrap();
        assert_eq!(
            w.office.apply_penalty_order(&mut w.kernel, order),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Reported));
        assert_eq!(w.office.penalty_count(), 0);
        let (_, refusal) = w.kernel.refusals().last().unwrap();
        assert_eq!(refusal.code, RefusalCode::Unauthorized);
    }

    #[test]
    fn a_tampered_order_is_refused_and_nothing_moves() {
        let mut w = world();
        let case_id = open_case(&mut w, "penalty-3");
        let subject_did = w.subject.did();
        let before = ledger_snapshot(&w.kernel, &w.participants);

        let mut order = slash_order(&case_id, &subject_did, 5, &w.arbiter)
            .sign(&w.arbiter)
            .unwrap();
        order.amount = Credits(19);
        assert_eq!(
            w.office.apply_penalty_order(&mut w.kernel, order),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(w.office.penalty_count(), 0);
    }

    #[test]
    fn a_slash_larger_than_the_stake_is_capped_and_conservation_holds() {
        let mut w = world();
        let case_id = open_case(&mut w, "penalty-4");
        let subject_did = w.subject.did();
        let locked_before = w.kernel.ledger().balance(&subject_did).locked;

        let order = slash_order(&case_id, &subject_did, 10_000, &w.arbiter)
            .sign(&w.arbiter)
            .unwrap();
        let record = w.office.apply_penalty_order(&mut w.kernel, order).unwrap();
        assert_eq!(record.requested, Credits(10_000));
        assert_eq!(record.applied, locked_before);
        assert_eq!(w.kernel.ledger().balance(&subject_did).locked, Credits::ZERO);
        assert_eq!(w.kernel.ledger().slashed(), locked_before);
        assert_eq!(w.office.slashed_total().unwrap(), locked_before);
        w.kernel.ledger().check_conservation().unwrap();
    }

    #[test]
    fn an_order_naming_the_wrong_subject_or_an_unknown_case_is_refused() {
        let mut w = world();
        let case_id = open_case(&mut w, "penalty-5");
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let reporter_did = w.reporter.did();

        let wrong_subject = slash_order(&case_id, &reporter_did, 5, &w.arbiter)
            .sign(&w.arbiter)
            .unwrap();
        assert_eq!(
            w.office.apply_penalty_order(&mut w.kernel, wrong_subject),
            Err(CoreError::InvalidSignature)
        );
        let (_, refusal) = w.kernel.refusals().last().unwrap();
        assert_eq!(refusal.code, RefusalCode::PolicyDenied);

        let unknown_case = slash_order("no-such-case", &w.subject.did(), 5, &w.arbiter)
            .sign(&w.arbiter)
            .unwrap();
        assert_eq!(
            w.office.apply_penalty_order(&mut w.kernel, unknown_case),
            Err(CoreError::UnknownAgent)
        );
        let (_, refusal) = w.kernel.refusals().last().unwrap();
        assert_eq!(refusal.code, RefusalCode::StaleEpoch);
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(w.office.penalty_count(), 0);
    }

    #[test]
    fn a_warning_is_recorded_without_moving_the_ledger() {
        let mut w = world();
        let case_id = open_case(&mut w, "penalty-6");
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let order = PenaltyOrder::new(
            &case_id,
            w.subject.did(),
            SanctionKind::Warning,
            Credits::ZERO,
            w.arbiter.did(),
            0,
        )
        .sign(&w.arbiter)
        .unwrap();
        let record = w.office.apply_penalty_order(&mut w.kernel, order).unwrap();
        assert_eq!(record.applied, Credits::ZERO);
        assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Penalized));
        w.kernel.ledger().check_conservation().unwrap();
    }

    #[test]
    fn penalty_queries_are_scoped_to_the_subject() {
        let mut w = world();
        let third = setup::keys(0x31);
        setup::ensure_agent(&mut w.kernel, &third, "third", &["x"], Credits(20)).unwrap();

        let case_one = open_case(&mut w, "penalty-7a");
        let (reference, payload) = evidence("penalty-7b");
        let case_two = w
            .office
            .report(
                &mut w.kernel,
                &w.reporter,
                &third.did(),
                ViolationKind::Fraud,
                reference,
                &payload,
            )
            .unwrap()
            .id;

        let first = w
            .office
            .apply_penalty_order(
                &mut w.kernel,
                slash_order(&case_one, &w.subject.did(), 4, &w.arbiter)
                    .sign(&w.arbiter)
                    .unwrap(),
            )
            .unwrap();
        let second = w
            .office
            .apply_penalty_order(
                &mut w.kernel,
                slash_order(&case_two, &third.did(), 6, &w.arbiter)
                    .sign(&w.arbiter)
                    .unwrap(),
            )
            .unwrap();

        let subject_records = w.office.penalties_for(&w.subject.did());
        assert_eq!(subject_records.len(), 1);
        assert_eq!(subject_records[0].id, first.id);
        let third_records = w.office.penalties_for(&third.did());
        assert_eq!(third_records.len(), 1);
        assert_eq!(third_records[0].id, second.id);
        assert_eq!(w.office.penalties().len(), 2);
        assert_eq!(w.office.slashed_total().unwrap(), Credits(10));
        assert_eq!(w.kernel.ledger().slashed(), Credits(10));
        w.kernel.ledger().check_conservation().unwrap();
        assert!(w.office.verify_chain().ok);
    }

    // ---- v1.5.5 通知 ----

    #[test]
    fn subscribing_replays_the_current_status_then_delivers_increments() {
        let mut w = world();
        let case_id = open_case(&mut w, "notify-1");
        let statuses = vec![CaseStatus::Reported, CaseStatus::Appealed];
        let subscription = w
            .office
            .subscribe(&mut w.kernel, &w.reporter, &case_id, statuses)
            .unwrap();

        // 订阅即回放当前状态快照：seq 指向造成该状态的事件序号。
        let inbox = w.office.inbox(&w.reporter.did());
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].status, CaseStatus::Reported);
        assert_eq!(inbox[0].seq, 0);
        assert_eq!(inbox[0].subscription, subscription.id);

        // 后续增量：主体申诉 → appealed。
        let (references, payloads) = appeal_evidence("notify-1");
        w.office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();
        let inbox = w.office.inbox(&w.reporter.did());
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox[1].status, CaseStatus::Appealed);
        assert_eq!(inbox[1].seq, 2, "seq 指向 appealed 事件（0=reported,1=subscribed,2=appealed）");
        assert_eq!(w.office.subscription_count(), 1);
        assert!(w.office.verify_chain().ok);
    }

    #[test]
    fn only_watched_statuses_are_delivered() {
        let mut w = world();
        let case_id = open_case(&mut w, "notify-2");
        // 只关心 arbitrated：当前是 reported（不投递快照），后续 appealing/penalized 也不投递。
        w.office
            .subscribe(&mut w.kernel, &w.reporter, &case_id, vec![CaseStatus::Arbitrated])
            .unwrap();
        assert_eq!(w.office.inbox(&w.reporter.did()).len(), 0);

        let (references, payloads) = appeal_evidence("notify-2");
        w.office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();
        w.office
            .apply_penalty_order(
                &mut w.kernel,
                slash_order(&case_id, &w.subject.did(), 3, &w.arbiter)
                    .sign(&w.arbiter)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            w.office.inbox(&w.reporter.did()).len(),
            0,
            "未订阅的状态不得投递"
        );
    }

    #[test]
    fn unsubscribing_stops_further_delivery_and_is_recorded() {
        let mut w = world();
        let case_id = open_case(&mut w, "notify-3");
        let subscription = w
            .office
            .subscribe(
                &mut w.kernel,
                &w.reporter,
                &case_id,
                vec![CaseStatus::Reported, CaseStatus::Appealed],
            )
            .unwrap();
        assert_eq!(w.office.inbox(&w.reporter.did()).len(), 1);

        w.office
            .unsubscribe(&mut w.kernel, &w.reporter, &subscription.id)
            .unwrap();
        assert_eq!(w.office.subscription_count(), 0);
        assert_eq!(
            w.office.events().last().unwrap().kind,
            SafetyEventKind::Unsubscribed
        );

        let (references, payloads) = appeal_evidence("notify-3");
        w.office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();
        assert_eq!(
            w.office.inbox(&w.reporter.did()).len(),
            1,
            "退订后不再投递"
        );
    }

    #[test]
    fn a_third_party_cannot_cancel_someone_elses_subscription() {
        let mut w = world();
        let case_id = open_case(&mut w, "notify-4");
        let subscription = w
            .office
            .subscribe(&mut w.kernel, &w.reporter, &case_id, vec![CaseStatus::Appealed])
            .unwrap();
        assert_eq!(
            w.office
                .unsubscribe(&mut w.kernel, &w.subject, &subscription.id),
            Err(CoreError::InvalidSignature)
        );
        let (_, refusal) = w.kernel.refusals().last().unwrap();
        assert_eq!(refusal.code, RefusalCode::Unauthorized);
        assert_eq!(w.office.subscription_count(), 1);

        // 订阅仍然有效：主体申诉后订阅者照样收到通知。
        let (references, payloads) = appeal_evidence("notify-4");
        w.office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();
        assert_eq!(w.office.inbox(&w.reporter.did()).len(), 1);
    }

    #[test]
    fn unknown_cases_and_unknown_subscriptions_are_refused() {
        let mut w = world();
        assert_eq!(
            w.office
                .subscribe(&mut w.kernel, &w.reporter, "no-such-case", vec![CaseStatus::Reported]),
            Err(CoreError::UnknownAgent)
        );
        let (_, refusal) = w.kernel.refusals().last().unwrap();
        assert_eq!(refusal.code, RefusalCode::StaleEpoch);

        assert_eq!(
            w.office
                .unsubscribe(&mut w.kernel, &w.reporter, "no-such-subscription"),
            Err(CoreError::UnknownAgent)
        );
        assert_eq!(w.office.event_count(), 0);
    }

    #[test]
    fn an_empty_status_set_is_not_a_subscription() {
        let mut w = world();
        let case_id = open_case(&mut w, "notify-5");
        assert_eq!(
            w.office.subscribe(&mut w.kernel, &w.reporter, &case_id, Vec::new()),
            Err(CoreError::InvalidKind)
        );
        let (_, refusal) = w.kernel.refusals().last().unwrap();
        assert_eq!(refusal.code, RefusalCode::PolicyDenied);
        assert_eq!(w.office.subscription_count(), 0);
    }

    #[test]
    fn notifications_are_private_to_their_subscriber() {
        let mut w = world();
        let third = setup::keys(0x51);
        setup::ensure_agent(&mut w.kernel, &third, "watcher", &["x"], Credits(20)).unwrap();
        let case_id = open_case(&mut w, "notify-6");
        let first = w
            .office
            .subscribe(&mut w.kernel, &w.reporter, &case_id, vec![CaseStatus::Appealed])
            .unwrap();
        let second = w
            .office
            .subscribe(&mut w.kernel, &third, &case_id, vec![CaseStatus::Appealed])
            .unwrap();
        assert_ne!(first.id, second.id);

        let (references, payloads) = appeal_evidence("notify-6");
        w.office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();

        let reporter_inbox = w.office.inbox(&w.reporter.did());
        let watcher_inbox = w.office.inbox(&third.did());
        assert_eq!(reporter_inbox.len(), 1);
        assert_eq!(watcher_inbox.len(), 1);
        assert_eq!(reporter_inbox[0].subscription, first.id);
        assert_eq!(watcher_inbox[0].subscription, second.id);
        assert_eq!(w.office.inbox(&w.subject.did()).len(), 0);
        assert_eq!(w.office.notifications_of_case(&case_id).len(), 2);
        assert_eq!(w.office.subscriptions_of(&case_id).len(), 2);
    }

    #[test]
    fn a_late_subscription_gets_only_the_current_status_not_the_history() {
        let mut w = world();
        let case_id = open_case(&mut w, "notify-7");
        let (references, payloads) = appeal_evidence("notify-7");
        w.office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();
        // 订阅发生在 reported 之后：只回放当前状态 appealed，不泄露订阅前的历史。
        w.office
            .subscribe(
                &mut w.kernel,
                &w.reporter,
                &case_id,
                vec![CaseStatus::Reported, CaseStatus::Appealed],
            )
            .unwrap();
        let inbox = w.office.inbox(&w.reporter.did());
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].status, CaseStatus::Appealed);
        assert_eq!(inbox[0].seq, 1);
    }

    #[test]
    fn subscribing_and_unsubscribing_do_not_move_the_ledger() {
        let mut w = world();
        let case_id = open_case(&mut w, "notify-8");
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let subscription = w
            .office
            .subscribe(
                &mut w.kernel,
                &w.reporter,
                &case_id,
                vec![CaseStatus::Reported],
            )
            .unwrap();
        w.office
            .unsubscribe(&mut w.kernel, &w.reporter, &subscription.id)
            .unwrap();
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        w.kernel.ledger().check_conservation().unwrap();
    }

    // ---- v1.5.6 PMB ----

    #[test]
    fn a_query_over_pmb_gets_a_signed_receipt_with_the_boundary() {
        let mut w = world();
        let service_did = w.office.service_did();
        let request = crate::pmb::query_envelope(
            &w.reporter,
            &service_did,
            w.kernel.now(),
            &w.reporter.did(),
        )
        .unwrap();
        w.kernel.send(&request).unwrap();
        let queued = w.kernel.drain();
        assert_eq!(queued.len(), 1);

        let receipt = w.office.handle(&mut w.kernel, &queued[0]).unwrap().unwrap();
        receipt.verify().unwrap();
        assert_eq!(receipt.from, service_did);
        assert_eq!(receipt.to, Some(w.reporter.did()));
        assert_eq!(receipt.kind.as_str(), crate::pmb::kinds::SAFETY_RECEIPT);
        assert_eq!(receipt.in_reply_to.as_deref(), Some(request.id.as_str()));
        assert_eq!(
            receipt.body["in_reply_kind"],
            json!(crate::pmb::kinds::SAFETY_QUERY)
        );
        assert_eq!(receipt.body["result"]["boundary"]["registered"], json!(true));
        assert_eq!(
            receipt.body["result"]["boundary"]["allowed"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
        // 查询不改变任何案件状态。
        assert_eq!(w.office.event_count(), 0);
    }

    #[test]
    fn a_report_over_pmb_opens_an_unconfirmed_case() {
        let mut w = world();
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let (reference, payload) = evidence("pmb-report");
        let report = ViolationReport::new(
            w.reporter.did(),
            w.subject.did(),
            ViolationKind::NonDelivery,
            reference,
            w.kernel.now(),
        )
        .sign(&w.reporter)
        .unwrap();
        let env = crate::pmb::report_envelope(
            &w.reporter,
            &w.office.service_did(),
            w.kernel.now(),
            &report,
            &payload,
        )
        .unwrap();
        w.kernel.send(&env).unwrap();
        let queued = w.kernel.drain();

        let receipt = w.office.handle(&mut w.kernel, &queued[0]).unwrap().unwrap();
        receipt.verify().unwrap();
        assert_eq!(w.office.status_of(&report.id), Some(CaseStatus::Reported));
        assert_eq!(w.office.event_count(), 1);
        assert_eq!(receipt.body["result"]["accepted"], json!(true));
        assert_eq!(receipt.body["result"]["case"], json!(report.id));
        // 走网络不会让「未确认」变成「已处罚」。
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
    }

    #[test]
    fn an_appeal_over_pmb_flips_the_case_status() {
        let mut w = world();
        let case_id = open_case(&mut w, "pmb-appeal");
        let payloads = vec![json!({"receipt": "signed"})];
        let references = vec![EvidenceRef::commit(EvidenceKind::Witness, &payloads[0]).unwrap()];
        let appeal = Appeal::new(case_id.clone(), w.subject.did(), references, w.kernel.now())
            .sign(&w.subject)
            .unwrap();
        let env = crate::pmb::appeal_envelope(
            &w.subject,
            &w.office.service_did(),
            w.kernel.now(),
            &appeal,
            &payloads,
        )
        .unwrap();
        w.kernel.send(&env).unwrap();
        let queued = w.kernel.drain();

        let receipt = w.office.handle(&mut w.kernel, &queued[0]).unwrap().unwrap();
        receipt.verify().unwrap();
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Appealed));
        assert_eq!(receipt.body["result"]["appeal"], json!(appeal.id));
        assert!(w.office.verify_chain().ok);
    }

    #[test]
    fn a_forged_report_over_pmb_is_refused_and_recorded() {
        let mut w = world();
        let (reference, _) = evidence("pmb-forged");
        let honest = json!({"pmb": "honest"});
        let report = ViolationReport::new(
            w.reporter.did(),
            w.subject.did(),
            ViolationKind::FakeEvidence,
            reference,
            w.kernel.now(),
        )
        .sign(&w.reporter)
        .unwrap();
        // 信封里塞进与摘要不符的证据本体：信封签名成立，但证据复算失败。
        let env = crate::pmb::report_envelope(
            &w.reporter,
            &w.office.service_did(),
            w.kernel.now(),
            &report,
            &honest,
        )
        .unwrap();
        let mut tampered = env.clone();
        tampered.body["evidence"] = json!({"pmb": "forged"});
        assert_eq!(tampered.verify(), Err(CoreError::InvalidSignature));

        assert_eq!(
            w.office.handle(&mut w.kernel, &env),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(w.office.case_count(), 0);
        assert_eq!(w.office.event_count(), 0);
        let (_, refusal) = w.kernel.refusals().last().unwrap();
        assert_eq!(refusal.code, RefusalCode::PolicyDenied);
    }

    #[test]
    fn envelopes_addressed_elsewhere_are_ignored() {
        let mut w = world();
        let someone_else = setup::keys(0x5c).did();
        let env = crate::pmb::query_envelope(
            &w.reporter,
            &someone_else,
            w.kernel.now(),
            &w.reporter.did(),
        )
        .unwrap();
        assert_eq!(w.office.handle(&mut w.kernel, &env), Ok(None));
        assert_eq!(w.office.event_count(), 0);
        assert!(w.kernel.refusals().is_empty());
    }

    #[test]
    fn a_tampered_envelope_is_refused_by_the_kernel_and_leaves_misconduct_evidence() {
        let mut w = world();
        let mut env = crate::pmb::query_envelope(
            &w.reporter,
            &w.office.service_did(),
            w.kernel.now(),
            &w.reporter.did(),
        )
        .unwrap();
        env.body = json!({"about": setup::keys(0x5d).did()});
        assert_eq!(w.kernel.send(&env), Err(CoreError::InvalidSignature));
        let (_, refusal) = w.kernel.refusals().last().unwrap();
        assert_eq!(refusal.code, RefusalCode::Unauthorized);
        assert!(refusal.code.is_misconduct(), "篡改信封是单次即恶意的拒绝码");
    }
}
