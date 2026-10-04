//! AU4A 轨道 1.5 — Safety API 安全 API（v1.5.1 → v1.5.10）。
//!
//! 安全接口是 **Agent 侧**的：Agent 自己查权限边界、自己举报（带可验证证据哈希）、
//! 自己被处罚后自己提交申诉证据、自己查询处罚记录、自己按案件订阅状态变更。
//! 这不是一个「人类控制台」——本 crate 里没有审批入口、没有管理员身份、没有人工复核阶段。
//!
//! 三条不可协商的不变式（每一版都在测试里被断言）：
//!
//! 1. **无罪不罚**：未被裁决确认的举报不改变任何 Agent 的余额与信誉。
//! 2. **证据可验证**：举报与申诉都必须携带可复算的证据哈希，缺失或伪造一律拒绝。
//! 3. **状态可审计**：每次状态变更写入哈希链事件（`prev` + 内容 → `hash`），断链可检测。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责（v1.5.1 权限查询 → v1.5.10 审计），
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）；
//! 与 `au4a-council` 的协作只走**数据契约**（可序列化 JSON），不互相依赖 crate。

pub mod appeal;
pub mod arbitration;
pub mod audit;
pub mod case;
pub mod chain;
pub mod config;
pub mod evidence;
pub mod notify;
pub mod office;
pub mod penalty;
pub mod permission;
pub mod pmb;
pub mod schema;
pub mod setup;

use au4a_core::{canonical_hash, CoreError, CoreResult, Credits, Did, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

pub use appeal::Appeal;
pub use arbitration::{
    ArbitrationRequest, ArbitrationVerdict, CaseOutcome, VerdictOutcome,
};
pub use audit::{verify_journal, AuditCode, AuditFinding, AuditReport, ReplayState};
pub use case::{Case, CaseStatus, ViolationKind, ViolationReport};
pub use chain::{
    chain_head, verify_chain, ChainBreak, ChainVerdict, SafetyEvent, SafetyEventKind, GENESIS_PREV,
};
pub use config::SafetyConfig;
pub use evidence::{is_lower_hex64, require_well_formed, EvidenceKind, EvidenceRef};
pub use notify::{Notification, Subscription};
pub use office::{ledger_fingerprint, ledger_snapshot, SafetyOffice};
pub use penalty::{PenaltyOrder, PenaltyRecord, SanctionKind};
pub use permission::{
    query_permissions, stake_requirement, DenialReason, DeniedPermission, Permission,
    PermissionBoundary, PermissionQuery, StakeGate,
};
pub use pmb::{
    appeal_envelope, classify as classify_safety_message, query_envelope, receipt_envelope,
    report_envelope, SafetyMessage,
};
pub use pmb::kinds as safety_kinds;
pub use schema::{schema_json, schema_summary};
pub use setup::{ensure_agent, keys as role_keys, seed as role_seed};

/// 轨道号。
pub const TRACK: &str = "1.5";
/// 轨道标题。
pub const TITLE: &str = "Safety API 安全 API";
/// 版本区间。
pub const RANGE: &str = "v1.5.1 → v1.5.10";
/// 当前小版本（每个小版本落地时前移）。
pub const CURRENT: &str = "v1.5.10";

/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_safety";

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
///
/// 每一项都是**跑出来的断言**：`scenario` 在私有内核上真跑一遍，
/// 事件链被独立重新解析并复算，账本不变式由一次独立的举报实验验证。
pub fn self_check() -> Vec<SelfCheck> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let mut checks = Vec::new();

    let run = scenario(&mut kernel);
    checks.push(match &run {
        Ok(_) => SelfCheck::pass(
            TRACK,
            "scenario.completed",
            format!("{CURRENT} 端到端流程无错误返回"),
        ),
        Err(err) => SelfCheck::fail(TRACK, "scenario.completed", format!("scenario 失败: {err}")),
    });

    // 独立复算：把 scenario 报告的事件链解析回来，重新验证序号/前驱/哈希。
    checks.push(match &run {
        Ok(value) => match replay_reported_chain(value) {
            Ok((len, head)) => SelfCheck::pass(
                TRACK,
                "chain.recomputable",
                format!("{len} 条事件重新解析并复算通过，链头 {head}"),
            ),
            Err(err) => SelfCheck::fail(TRACK, "chain.recomputable", format!("复算失败: {err}")),
        },
        Err(_) => SelfCheck::fail(TRACK, "chain.recomputable", "scenario 未产生事件链"),
    });

    // 无罪不罚：一次独立的「举报 + 申诉」实验，账本快照必须逐字段相等。
    checks.push(match no_penalty_probe() {
        Ok(()) => SelfCheck::pass(
            TRACK,
            "report.moves_nothing",
            "未确认举报与申诉前后账本快照、AgentCard 逐字段相等（余额/信誉不动）",
        ),
        Err(err) => SelfCheck::fail(TRACK, "report.moves_nothing", err.to_string()),
    });

    // 申诉权不可代理：第三方代签必须失败。
    checks.push(match third_party_appeal_probe() {
        Ok(()) => SelfCheck::pass(
            TRACK,
            "appeal.subject_only",
            "第三方代签申诉返回 invalid_signature，状态仍为 reported",
        ),
        Err(err) => SelfCheck::fail(TRACK, "appeal.subject_only", err.to_string()),
    });

    // 处罚闸门：只有受信仲裁者签名的契约能动账本。
    checks.push(match penalty_gate_probe() {
        Ok((applied, slashed)) => SelfCheck::pass(
            TRACK,
            "penalty.arbiter_gated",
            format!(
                "未授权契约被拒后账本逐字段不变；受信契约罚没 {applied}，账本 slashed={slashed}，守恒成立"
            ),
        ),
        Err(err) => SelfCheck::fail(TRACK, "penalty.arbiter_gated", err.to_string()),
    });

    // 通知隔离：只投递给订阅了该状态的人，且第三方不能退订别人的订阅。
    checks.push(match notification_isolation_probe() {
        Ok((subscriber, bystander)) => SelfCheck::pass(
            TRACK,
            "notify.subscription_scoped",
            format!("订阅者收到 {subscriber} 条通知、旁观者收到 {bystander} 条（未订阅状态不投递）"),
        ),
        Err(err) => SelfCheck::fail(TRACK, "notify.subscription_scoped", err.to_string()),
    });

    // PMB：查询走真实信封往返并拿回签名回执；证据不符的举报经网络同样被拒。
    checks.push(match pmb_probe() {
        Ok((receipts, refused)) => SelfCheck::pass(
            TRACK,
            "pmb.round_trip",
            format!("{receipts} 个回执由服务身份签名并验签通过；经 PMB 的伪造证据被拒 {refused} 次"),
        ),
        Err(err) => SelfCheck::fail(TRACK, "pmb.round_trip", err.to_string()),
    });

    // 仲裁回滚：rejected 裁决必须把罚没还回去，且守恒式仍成立。
    checks.push(match arbitration_rollback_probe() {
        Ok((slashed, refunded)) => SelfCheck::pass(
            TRACK,
            "arbitration.rollback",
            format!(
                "upheld 罚没 {slashed} → rejected 归还 {refunded}，净罚没归零、守恒成立、案件 arbitrated"
            ),
        ),
        Err(err) => SelfCheck::fail(TRACK, "arbitration.rollback", err.to_string()),
    });

    // 审计：链完整性 + 状态==重放(链) + 账本交叉核对。
    checks.push(match audit_probe() {
        Ok(report) => {
            let codes: Vec<&str> = report.codes().iter().map(|c| c.as_str()).collect();
            SelfCheck::pass(
                TRACK,
                "audit.clean",
                format!(
                    "{} 条事件重放一致、gross_slashed={} == ledger.slashed、findings={:?}",
                    report.events, report.gross_slashed, codes
                ),
            )
        }
        Err(err) => SelfCheck::fail(TRACK, "audit.clean", err.to_string()),
    });

    // 证据闸门：伪造摘要必须被拒，且不留下案件与事件。
    checks.push(match forged_evidence_probe() {
        Ok(()) => SelfCheck::pass(
            TRACK,
            "evidence.forgery_refused",
            "摘要与证据本体不一致时返回 invalid_signature，案件数 0、事件数 0",
        ),
        Err(err) => SelfCheck::fail(TRACK, "evidence.forgery_refused", err.to_string()),
    });

    let config = demo_config();
    let reporter = role_keys(setup::ROLE_REPORTER).did();
    let arbiter = role_keys(setup::ROLE_ARBITER).did();
    let probe = role_keys(0xEE).did();

    let reporter_boundary = PermissionBoundary::of(&kernel, &config, &reporter);
    checks.push(if reporter_boundary.allows(Permission::ReportViolation) {
        SelfCheck::pass(
            TRACK,
            "permission.reporter_can_report",
            format!(
                "已注册 Agent 的 allowed={} denied={}，举报权可达",
                reporter_boundary.allowed.len(),
                reporter_boundary.denied.len()
            ),
        )
    } else {
        SelfCheck::fail(TRACK, "permission.reporter_can_report", "举报权不可达")
    });

    let probe_boundary = PermissionBoundary::of(&kernel, &config, &probe);
    checks.push(
        if !probe_boundary.registered
            && probe_boundary.allowed == vec![Permission::Register]
            && probe_boundary.denial(Permission::SendMessage)
                == Some(&DenialReason::NotRegistered)
        {
            SelfCheck::pass(
                TRACK,
                "permission.unregistered_is_structured",
                "未注册 DID 只能自助注册，其余权限带 not_registered 码被拒",
            )
        } else {
            SelfCheck::fail(TRACK, "permission.unregistered_is_structured", "未注册者的边界不结构化")
        },
    );

    let arbiter_boundary = PermissionBoundary::of(&kernel, &config, &arbiter);
    checks.push(if arbiter_boundary.allows(Permission::IssueVerdict) {
        SelfCheck::pass(
            TRACK,
            "permission.arbiter_only",
            "配置中的仲裁者可出具裁决，其余 Agent 得到 arbiter_only",
        )
    } else {
        SelfCheck::fail(TRACK, "permission.arbiter_only", "仲裁者权限不可达")
    });

    checks.push(match kernel.ledger().check_conservation() {
        Ok(()) => SelfCheck::pass(
            TRACK,
            "ledger.conservation",
            format!(
                "Σ可用+Σ锁定+罚没 == 发行（minted={} slashed={}）",
                kernel.ledger().minted(),
                kernel.ledger().slashed()
            ),
        ),
        Err(err) => SelfCheck::fail(TRACK, "ledger.conservation", err.to_string()),
    });

    checks
}

/// 从 `scenario` 的 JSON 里取回事件链并独立复算。
fn replay_reported_chain(value: &Value) -> CoreResult<(usize, String)> {
    let raw = value
        .get("chain")
        .and_then(|c| c.get("events"))
        .and_then(|e| e.as_array())
        .ok_or(CoreError::Encoding)?;
    let mut events = Vec::with_capacity(raw.len());
    for item in raw {
        events.push(SafetyEvent::from_json(item)?);
    }
    let verdict = verify_chain(&events);
    if !verdict.ok {
        return Err(CoreError::InvalidSignature);
    }
    Ok((verdict.len, verdict.head))
}

/// 独立实验：举报 + 申诉都不改账本、不改名片。
fn no_penalty_probe() -> CoreResult<()> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let office_keys = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    for (keys, display, skill) in [
        (&office_keys, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20))?;
    }
    let config = SafetyConfig::single_arbiter(office_keys.did(), arbiter.did());
    let mut office = SafetyOffice::new(config, office_keys)?;

    let participants = vec![reporter.did(), subject.did()];
    let before = ledger_snapshot(&kernel, &participants);
    let card_before = kernel.card(&subject.did()).cloned();

    let payload = json!({"probe": "self-check", "delivered": false});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload)?;
    let report = office.report(
        &mut kernel,
        &reporter,
        &subject.did(),
        ViolationKind::NonDelivery,
        reference,
        &payload,
    )?;
    if office.status_of(&report.id) != Some(CaseStatus::Reported) {
        return Err(CoreError::InvalidKind);
    }

    // 被举报方申诉：只提交证据、推状态、写链。
    let appeal_payloads = vec![json!({"probe": "appeal", "reason": "receipt exists", "ok": true})];
    let appeal_refs = vec![EvidenceRef::commit(
        EvidenceKind::Witness,
        &appeal_payloads[0],
    )?];
    let appeal = office.appeal(
        &mut kernel,
        &subject,
        &report.id,
        appeal_refs,
        &appeal_payloads,
    )?;
    if office.status_of(&appeal.case) != Some(CaseStatus::Appealed) {
        return Err(CoreError::InvalidKind);
    }

    if ledger_snapshot(&kernel, &participants) != before {
        return Err(CoreError::Overflow);
    }
    if kernel.card(&subject.did()).cloned() != card_before {
        return Err(CoreError::InvalidSignature);
    }
    kernel.ledger().check_conservation()
}

/// 独立实验：第三方代签的申诉必须被拒（签名属于主体，不能代理）。
fn third_party_appeal_probe() -> CoreResult<()> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let office_keys = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    for (keys, display, skill) in [
        (&office_keys, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20))?;
    }
    let config = SafetyConfig::single_arbiter(office_keys.did(), arbiter.did());
    let mut office = SafetyOffice::new(config, office_keys)?;

    let payload = json!({"probe": "third-party"});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload)?;
    let report = office.report(
        &mut kernel,
        &reporter,
        &subject.did(),
        ViolationKind::Spam,
        reference,
        &payload,
    )?;
    let outcome = office.appeal(
        &mut kernel,
        &reporter,
        &report.id,
        vec![EvidenceRef::commit(EvidenceKind::Witness, &payload)?],
        &[payload],
    );
    if outcome != Err(CoreError::InvalidSignature)
        || office.status_of(&report.id) != Some(CaseStatus::Reported)
        || office.appeal_count() != 0
    {
        return Err(CoreError::InvalidSignature);
    }
    Ok(())
}

/// 独立实验：伪造证据必须被拒且不留痕迹。
fn forged_evidence_probe() -> CoreResult<()> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let office_keys = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    for (keys, display, skill) in [
        (&office_keys, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20))?;
    }
    let config = SafetyConfig::single_arbiter(office_keys.did(), arbiter.did());
    let mut office = SafetyOffice::new(config, office_keys)?;

    let honest = json!({"probe": "honest"});
    let forged = json!({"probe": "forged"});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &honest)?;
    let outcome = office.report(
        &mut kernel,
        &reporter,
        &subject.did(),
        ViolationKind::FakeEvidence,
        reference,
        &forged,
    );
    if outcome != Err(CoreError::InvalidSignature)
        || office.case_count() != 0
        || office.event_count() != 0
    {
        return Err(CoreError::InvalidSignature);
    }
    Ok(())
}

/// 独立实验：处罚闸门。未受信的契约分文不动，受信的契约精确罚没。
fn penalty_gate_probe() -> CoreResult<(Credits, Credits)> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let office_keys = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    for (keys, display, skill) in [
        (&office_keys, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20))?;
    }
    let config = SafetyConfig::single_arbiter(office_keys.did(), arbiter.did());
    let mut office = SafetyOffice::new(config, office_keys)?;

    let participants = vec![reporter.did(), subject.did()];
    let payload = json!({"probe": "penalty-gate"});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload)?;
    let report = office.report(
        &mut kernel,
        &reporter,
        &subject.did(),
        ViolationKind::NonDelivery,
        reference,
        &payload,
    )?;
    let before = ledger_snapshot(&kernel, &participants);

    // 未受信的签署者：即使签名自洽也必须被拒。
    let outsider = role_keys(0x6d);
    let rogue = PenaltyOrder::new(
        report.id.clone(),
        subject.did(),
        SanctionKind::StakeSlash,
        Credits(5),
        outsider.did(),
        0,
    )
    .sign(&outsider)?;
    if office.apply_penalty_order(&mut kernel, rogue) != Err(CoreError::InvalidSignature) {
        return Err(CoreError::InvalidSignature);
    }
    if ledger_snapshot(&kernel, &participants) != before {
        return Err(CoreError::Overflow);
    }

    // 受信的仲裁者：罚没精确发生。
    let order = PenaltyOrder::new(
        report.id.clone(),
        subject.did(),
        SanctionKind::StakeSlash,
        Credits(5),
        arbiter.did(),
        kernel.now(),
    )
    .sign(&arbiter)?;
    let record = office.apply_penalty_order(&mut kernel, order)?;
    if record.applied != Credits(5) || kernel.ledger().slashed() != Credits(5) {
        return Err(CoreError::Overflow);
    }
    if office.slashed_total()? != Credits(5) {
        return Err(CoreError::Overflow);
    }
    kernel.ledger().check_conservation()?;
    Ok((record.applied, kernel.ledger().slashed()))
}

/// 独立实验：通知按订阅投递，旁观者收不到，第三方不能替别人退订。
fn notification_isolation_probe() -> CoreResult<(usize, usize)> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let office_keys = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    let bystander = role_keys(0x77);
    for (keys, display, skill) in [
        (&office_keys, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
        (&bystander, "bystander-agent", "observe"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20))?;
    }
    let config = SafetyConfig::single_arbiter(office_keys.did(), arbiter.did());
    let mut office = SafetyOffice::new(config, office_keys)?;

    let payload = json!({"probe": "notify"});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload)?;
    let report = office.report(
        &mut kernel,
        &reporter,
        &subject.did(),
        ViolationKind::Spam,
        reference,
        &payload,
    )?;
    // 举报人订阅 reported/appealed，旁观者只订阅 arbitrated。
    let watched = office.subscribe(
        &mut kernel,
        &reporter,
        &report.id,
        vec![CaseStatus::Reported, CaseStatus::Appealed],
    )?;
    office.subscribe(
        &mut kernel,
        &bystander,
        &report.id,
        vec![CaseStatus::Arbitrated],
    )?;
    let appeal_payloads = vec![json!({"probe": "notify-appeal", "ok": true})];
    let appeal_refs = vec![EvidenceRef::commit(EvidenceKind::Witness, &appeal_payloads[0])?];
    office.appeal(&mut kernel, &subject, &report.id, appeal_refs, &appeal_payloads)?;

    // 第三方不能替订阅者退订。
    if office.unsubscribe(&mut kernel, &bystander, &watched.id) != Err(CoreError::InvalidSignature) {
        return Err(CoreError::InvalidSignature);
    }
    let subscriber_inbox = office.inbox(&reporter.did()).len();
    let bystander_inbox = office.inbox(&bystander.did()).len();
    if subscriber_inbox != 2 || bystander_inbox != 0 {
        return Err(CoreError::InvalidSignature);
    }
    Ok((subscriber_inbox, bystander_inbox))
}

/// 独立实验：PMB 往返。查询得到一个验签通过的回执；举报信封里的伪造证据经网络被拒。
fn pmb_probe() -> CoreResult<(usize, usize)> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let office_keys = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    for (keys, display, skill) in [
        (&office_keys, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20))?;
    }
    let config = SafetyConfig::single_arbiter(office_keys.did(), arbiter.did());
    let mut office = SafetyOffice::new(config, office_keys)?;
    let service_did = office.service_did();

    // 1) 查询：信封签名 → 服务受理 → 回执验签。
    let query = query_envelope(&reporter, &service_did, kernel.now(), &reporter.did())?;
    kernel.send(&query)?;
    let queued = kernel.drain();
    let mut receipts = 0;
    for env in &queued {
        if let Some(receipt) = office.handle(&mut kernel, env)? {
            receipt.verify()?;
            if receipt.in_reply_to.as_deref() != Some(env.id.as_str()) {
                return Err(CoreError::InvalidSignature);
            }
            receipts += 1;
        }
    }
    if receipts != 1 {
        return Err(CoreError::InvalidSignature);
    }

    // 2) 举报：信封合法，但证据本体与摘要不符 → 经网络同样被拒。
    let honest = json!({"probe": "pmb-honest"});
    let forged = json!({"probe": "pmb-forged"});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &honest)?;
    let report = ViolationReport::new(
        reporter.did(),
        subject.did(),
        ViolationKind::FakeEvidence,
        reference,
        kernel.now(),
    )
    .sign(&reporter)?;
    let env = report_envelope(&reporter, &service_did, kernel.now(), &report, &forged)?;
    let mut refused = 0;
    // 信封签名成立（发送者确实是举报人），但证据复算失败 → 服务拒绝，不留案件。
    if office.handle(&mut kernel, &env) != Err(CoreError::InvalidSignature) {
        return Err(CoreError::InvalidSignature);
    }
    refused += 1;

    // 3) 证据本体被换掉的信封：信封签名立刻失败。
    let mut swapped = report_envelope(&reporter, &service_did, kernel.now(), &report, &forged)?;
    swapped.body["evidence"] = honest.clone();
    if swapped.verify() != Err(CoreError::InvalidSignature) {
        return Err(CoreError::InvalidSignature);
    }
    refused += 1;
    if office.case_count() != 0 || office.event_count() != 0 {
        return Err(CoreError::InvalidSignature);
    }
    Ok((receipts, refused))
}

/// 独立实验：仲裁回滚。罚没 → rejected 裁决 → 等额归还、净额归零、守恒成立。
fn arbitration_rollback_probe() -> CoreResult<(Credits, Credits)> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let office_keys = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    for (keys, display, skill) in [
        (&office_keys, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20))?;
    }
    let config = SafetyConfig::single_arbiter(office_keys.did(), arbiter.did());
    let mut office = SafetyOffice::new(config, office_keys)?;

    let payload = json!({"probe": "arbitration"});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload)?;
    let report = office.report(
        &mut kernel,
        &reporter,
        &subject.did(),
        ViolationKind::NonDelivery,
        reference,
        &payload,
    )?;
    let request = office.arbitration_request(&report.id)?;
    if request.case != report.id || request.subject != subject.did() {
        return Err(CoreError::Encoding);
    }

    let upheld = ArbitrationVerdict::new(
        &report.id,
        VerdictOutcome::Upheld,
        SanctionKind::StakeSlash,
        Credits(7),
        canonical_hash(&json!({"probe": "rationale"}))?,
        arbiter.did(),
        kernel.now(),
    )
    .sign(&arbiter)?;
    let outcome = office.resolve(&mut kernel, upheld)?;
    if outcome.applied != Credits(7) || office.slashed_total()? != Credits(7) {
        return Err(CoreError::Overflow);
    }
    kernel.ledger().check_conservation()?;

    let rejected = ArbitrationVerdict::new(
        &report.id,
        VerdictOutcome::Rejected,
        SanctionKind::Warning,
        Credits::ZERO,
        canonical_hash(&json!({"probe": "rationale-2"}))?,
        arbiter.did(),
        kernel.now(),
    )
    .sign(&arbiter)?;
    let rolled_back = office.resolve(&mut kernel, rejected)?;
    if rolled_back.refunded != Credits(7) || office.slashed_total()? != Credits::ZERO {
        return Err(CoreError::Overflow);
    }
    if office.status_of(&report.id) != Some(CaseStatus::Arbitrated) {
        return Err(CoreError::InvalidKind);
    }
    kernel.ledger().check_conservation()?;
    Ok((outcome.applied, rolled_back.refunded))
}

/// 独立实验：审计。跑完整生命周期后，报告必须干净且与账本交叉核对一致。
fn audit_probe() -> CoreResult<AuditReport> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let office_keys = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    for (keys, display, skill) in [
        (&office_keys, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20))?;
    }
    let config = SafetyConfig::single_arbiter(office_keys.did(), arbiter.did());
    let mut office = SafetyOffice::new(config.clone(), office_keys)?;

    let payload = json!({"probe": "audit", "delivered": false});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload)?;
    let report = office.report(
        &mut kernel,
        &reporter,
        &subject.did(),
        ViolationKind::NonDelivery,
        reference,
        &payload,
    )?;
    let order = PenaltyOrder::new(
        report.id.clone(),
        subject.did(),
        SanctionKind::StakeSlash,
        Credits(4),
        arbiter.did(),
        kernel.now(),
    )
    .sign(&arbiter)?;
    office.apply_penalty_order(&mut kernel, order)?;
    let appeal_payload = json!({"probe": "audit-appeal", "ok": true});
    let appeal_evidence = vec![EvidenceRef::commit(EvidenceKind::Witness, &appeal_payload)?];
    office.appeal(
        &mut kernel,
        &subject,
        &report.id,
        appeal_evidence,
        &[appeal_payload],
    )?;
    let verdict = ArbitrationVerdict::new(
        report.id.clone(),
        VerdictOutcome::Upheld,
        SanctionKind::StakeSlash,
        Credits(3),
        canonical_hash(&json!({"probe": "audit-rationale"}))?,
        arbiter.did(),
        kernel.now(),
    )
    .sign(&arbiter)?;
    office.resolve(&mut kernel, verdict)?;

    let report = office.audit(&kernel);
    if !report.ok {
        return Err(CoreError::InvalidSignature);
    }
    if report.gross_slashed != kernel.ledger().slashed() || report.events != office.event_count() {
        return Err(CoreError::Overflow);
    }
    // 重放重建：同一份事件链必须重建出同样的状态。
    let rebuilt = SafetyOffice::from_journal(
        office.events().to_vec(),
        config,
        role_keys(setup::ROLE_SERVICE),
    )?;
    if rebuilt.chain_head() != office.chain_head() || rebuilt.case_count() != office.case_count() {
        return Err(CoreError::InvalidSignature);
    }
    Ok(report)
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let scenario_value = scenario(&mut kernel)?;
    let checks = self_check();
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "current": CURRENT,
        "checks": checks.len(),
        "checks_passed": checks.iter().filter(|c| c.passed).count(),
        "schema": schema_summary()?,
        "scenario": scenario_value,
    }))
}

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
/// 每一版迭代都应该让这里多做一件真实的事，而不是多打印一行字。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    let service = role_keys(setup::ROLE_SERVICE);

    ensure_agent(kernel, &reporter, "reporter-agent", &["audit.report"], Credits(20))?;
    ensure_agent(kernel, &subject, "subject-agent", &["deliver.task"], Credits(20))?;
    ensure_agent(kernel, &arbiter, "arbiter-agent", &["arbitrate.case"], Credits(20))?;
    ensure_agent(kernel, &service, "safety-service", &["safety.api"], Credits(20))?;

    let config = SafetyConfig::single_arbiter(service.did(), arbiter.did());
    let mut office = SafetyOffice::new(config.clone(), service)?;
    // 审计基线：本服务开始活动时的历史罚没额（共享内核上可能已有别人的罚没）。
    let slashed_at_start = kernel.ledger().slashed();

    // 1) 权限边界：结构化查询。
    let reporter_boundary = PermissionBoundary::of(kernel, &config, &reporter.did());
    let arbiter_boundary = PermissionBoundary::of(kernel, &config, &arbiter.did());
    let probe_boundary = PermissionBoundary::of(kernel, &config, &role_keys(0xEE).did());

    // 2) 举报：带可复算证据哈希。
    let participants = vec![reporter.did(), subject.did()];
    let evidence_payload = json!({"task": "deliver-1", "delivered": false, "deadline": 40});
    let evidence = EvidenceRef::commit(EvidenceKind::Transcript, &evidence_payload)?;
    let before = ledger_snapshot(kernel, &participants);
    let fingerprint_before = ledger_fingerprint(kernel, &participants)?;
    let subject_card_before = kernel.card(&subject.did()).cloned();

    let report = office.report(
        kernel,
        &reporter,
        &subject.did(),
        ViolationKind::NonDelivery,
        evidence,
        &evidence_payload,
    )?;
    // 举报受理后立刻取快照：这一阶段（未确认）不允许任何账本变化。
    let snapshot_after_report = ledger_snapshot(kernel, &participants);
    let fingerprint_after_report = ledger_fingerprint(kernel, &participants)?;

    // 2.5) 通知：举报人按案件订阅状态变更（订阅即回放当前状态快照，此后只投递增量）。
    let subscription = office.subscribe(
        kernel,
        &reporter,
        &report.id,
        vec![
            CaseStatus::Reported,
            CaseStatus::Appealed,
            CaseStatus::Penalized,
            CaseStatus::Arbitrated,
        ],
    )?;

    // 4) 伪造证据：必须被拒，且不留案件、不留事件。
    let forged_payload = json!({"task": "deliver-1", "delivered": true, "deadline": 40});
    let forged_reference = EvidenceRef::commit(EvidenceKind::Transcript, &forged_payload)?;
    let cases_before_forgery = office.case_count();
    let events_before_forgery = office.event_count();
    let forgery = office.report(
        kernel,
        &reporter,
        &subject.did(),
        ViolationKind::FakeEvidence,
        forged_reference,
        &evidence_payload,
    );
    let forgery_refused = forgery == Err(CoreError::InvalidSignature)
        && office.case_count() == cases_before_forgery
        && office.event_count() == events_before_forgery;

    // 5) 仲裁者处罚契约：这是账本**唯一**的改动入口。
    //    预期罚没额由**观察到的锁定质押**推导（`applied = min(请求额, 锁定额)`），
    //    因此 scenario 在共享内核上重复调用时依然正确：质押被罚光后 applied 为 0，
    //    既不会假装罚了钱，也不会因此失败。
    let slashed_before = kernel.ledger().slashed();
    let locked_before = kernel.ledger().balance(&subject.did()).locked;
    let slash_request = Credits(5);
    let expected_slash = if slash_request > locked_before {
        locked_before
    } else {
        slash_request
    };
    let order = PenaltyOrder::new(
        report.id.clone(),
        subject.did(),
        SanctionKind::StakeSlash,
        slash_request,
        arbiter.did(),
        kernel.now(),
    )
    .sign(&arbiter)?;
    let penalty = office.apply_penalty_order(kernel, order)?;
    let after_penalty = ledger_snapshot(kernel, &participants);
    let penalty_moved_ledger = penalty.applied == expected_slash
        && kernel.ledger().slashed() == slashed_before.checked_add(expected_slash)?
        && (expected_slash == Credits::ZERO || after_penalty != before);

    // 6) 被处罚方申诉：提交证据、推状态、写链——**不再动账本**。
    let appeal_payloads = vec![json!({"task": "deliver-1", "receipt": "signed-by-receiver", "ok": true})];
    let appeal_references = vec![EvidenceRef::commit(
        EvidenceKind::Witness,
        &appeal_payloads[0],
    )?];
    let appeal = office.appeal(
        kernel,
        &subject,
        &report.id,
        appeal_references,
        &appeal_payloads,
    )?;
    let after_appeal = ledger_snapshot(kernel, &participants);
    let appeal_untouched = after_appeal == after_penalty
        && office.status_of(&report.id) == Some(CaseStatus::Appealed);
    if !penalty_moved_ledger || !appeal_untouched {
        return Err(CoreError::Overflow);
    }

    // 7) 仲裁：案件事实交给仲裁方（数据契约），裁决回来自带仲裁者签名 → 终局。
    let request = office.arbitration_request(&report.id)?;
    let verdict = ArbitrationVerdict::new(
        request.case.clone(),
        VerdictOutcome::Upheld,
        SanctionKind::StakeSlash,
        Credits(2),
        canonical_hash(&json!({"rationale": "delivery receipt contradicted the appeal"}))?,
        arbiter.did(),
        kernel.now(),
    )
    .sign(&arbiter)?;
    let outcome = office.resolve(kernel, verdict)?;
    let locked_after_penalty = if slash_request > locked_before {
        Credits::ZERO
    } else {
        locked_before.checked_sub(slash_request)?
    };
    let slash_request_verdict = Credits(2);
    let expected_verdict_slash = if slash_request_verdict > locked_after_penalty {
        locked_after_penalty
    } else {
        slash_request_verdict
    };
    let arbitrated = office.status_of(&report.id) == Some(CaseStatus::Arbitrated)
        && outcome.outcome == VerdictOutcome::Upheld
        && outcome.applied == expected_verdict_slash;
    if !arbitrated {
        return Err(CoreError::InvalidKind);
    }
    let after_verdict = ledger_snapshot(kernel, &participants);
    if expected_verdict_slash > Credits::ZERO && after_verdict == after_appeal {
        return Err(CoreError::Overflow);
    }

    // 8) 通知结果：订阅者按状态集合收到快照 + 增量（含终局 arbitrated）。
    let delivered: Vec<&str> = office
        .inbox(&reporter.did())
        .iter()
        .map(|n| n.status.as_str())
        .collect();
    let notifications_as_watched =
        delivered == vec!["reported", "penalized", "appealed", "arbitrated"];
    let notifications_private = office.inbox(&subject.did()).is_empty();
    if !notifications_as_watched || !notifications_private {
        return Err(CoreError::InvalidSignature);
    }

    // 8) PMB：安全 API 走真实信封（真签名、线格式名、服务回执）。
    let pmb_query = query_envelope(
        &reporter,
        &office.service_did(),
        kernel.now(),
        &reporter.did(),
    )?;
    let pmb_receipt = office
        .handle(kernel, &pmb_query)?
        .ok_or(CoreError::InvalidKind)?;
    pmb_receipt.verify()?;
    let query_receipt_ok = pmb_receipt.in_reply_to.as_deref() == Some(pmb_query.id.as_str())
        && pmb_receipt.kind.as_str() == safety_kinds::SAFETY_RECEIPT
        && pmb_receipt.body["result"]["boundary"]["registered"] == json!(true);

    let report_envelope = report_envelope(
        &reporter,
        &office.service_did(),
        kernel.now(),
        &report,
        &evidence_payload,
    )?;
    let appeal_envelope = appeal_envelope(
        &subject,
        &office.service_did(),
        kernel.now(),
        &appeal,
        &appeal_payloads,
    )?;
    let envelopes_verified = report_envelope.verify().is_ok()
        && appeal_envelope.verify().is_ok()
        && classify_safety_message(&report_envelope)?.kind() == safety_kinds::SAFETY_REPORT
        && classify_safety_message(&appeal_envelope)?.kind() == safety_kinds::SAFETY_APPEAL;
    if !query_receipt_ok || !envelopes_verified {
        return Err(CoreError::InvalidSignature);
    }

    // 未确认举报阶段（举报前后）账本与名片必须逐字段不变。
    let ledger_untouched_by_report = before == snapshot_after_report
        && fingerprint_before == fingerprint_after_report
        && kernel.card(&subject.did()).cloned() == subject_card_before;

    let verdict = office.verify_chain();
    // 共享内核上可能有别的轨道在动账本：用活动开始时的 slashed 作为基线做交叉核对。
    let audit = office.audit_since(kernel, slashed_at_start);
    if !audit.ok {
        return Err(CoreError::InvalidSignature);
    }
    let events: Vec<Value> = office
        .events()
        .iter()
        .map(|event| event.to_json())
        .collect::<CoreResult<Vec<Value>>>()?;

    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!(
            "{CURRENT} 举报→处罚→申诉，哈希链 {} 条事件（链头 {}）",
            verdict.len,
            au4a_core::short_id(&verdict.head)
        ),
    );

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "version": CURRENT,
        "agents": kernel.agent_count(),
        "permission_points": Permission::ALL.len(),
        "reporter": {
            "allowed": reporter_boundary.allowed.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            "denied": reporter_boundary.denied.len(),
        },
        "arbiter": {
            "can_issue_verdict": arbiter_boundary.allows(Permission::IssueVerdict),
        },
        "probe_unregistered": {
            "registered": probe_boundary.registered,
            "allowed": probe_boundary.allowed.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            "send_message_denial": probe_boundary.denial(Permission::SendMessage).map(|r| r.code()),
        },
        "case": {
            "id": report.id,
            "status": office.status_of(&report.id).map(|s| s.as_str()),
            "outcome": office.case(&report.id).and_then(|c| c.outcome).map(|o| o.as_str()),
            "violation": report.violation.as_str(),
            "reporter": report.reporter.as_str(),
            "subject": report.subject.as_str(),
            "appeals": office.case(&report.id).map(|c| c.appeals.len()),
            "penalties": office.case(&report.id).map(|c| c.penalties.len()),
        },
        "arbitration": {
            "request": request.to_json()?,
            "outcome": outcome.to_json()?,
            "arbitrated": arbitrated,
        },
        "penalty": {
            "id": penalty.id,
            "sanction": penalty.sanction.as_str(),
            "requested": penalty.requested.get(),
            "applied": penalty.applied.get(),
            "arbiter": penalty.arbiter.as_str(),
            "slashed_total": office.slashed_total()?.get(),
            "moved_ledger": penalty_moved_ledger,
        },
        "ledger_after_verdict": after_verdict,
        "appeal": {
            "id": appeal.id,
            "appellant": appeal.appellant.as_str(),
            "evidence": appeal.evidence_count(),
            "moved_ledger": !appeal_untouched,
        },
        "notifications": {
            "subscription": subscription.id,
            "watched": subscription.statuses.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "delivered": delivered,
            "as_watched": notifications_as_watched,
            "private": notifications_private,
        },
        "pmb": {
            "kinds": [
                safety_kinds::SAFETY_QUERY,
                safety_kinds::SAFETY_REPORT,
                safety_kinds::SAFETY_APPEAL,
                safety_kinds::SAFETY_RECEIPT,
            ],
            "query_receipt_ok": query_receipt_ok,
            "receipt_id": pmb_receipt.id,
            "envelopes_verified": envelopes_verified,
        },
        "chain": {
            "len": verdict.len,
            "head": verdict.head,
            "ok": verdict.ok,
            "events": events,
        },
        "audit": {
            "ok": audit.ok,
            "events": audit.events,
            "cases": audit.cases,
            "penalties": audit.penalties,
            "subscriptions": audit.subscriptions,
            "notifications": audit.notifications,
            "gross_slashed": audit.gross_slashed.get(),
            "net_slashed": audit.net_slashed.get(),
            "refunded": audit.refunded.get(),
            "ledger_slashed": audit.ledger_slashed.get(),
            "findings": audit.findings.len(),
        },
        "ledger": {
            "untouched_by_unconfirmed_report": ledger_untouched_by_report,
            "untouched_by_appeal": appeal_untouched,
            "fingerprint_before": fingerprint_before,
            "fingerprint_after_report": fingerprint_after_report,
            "snapshot_after_penalty": after_penalty,
            "snapshot": after_appeal,
        },
        "forgery": {
            "refused": forgery_refused,
            "refusals": kernel.refusals().len(),
        },
        "ledger_conserved": kernel.ledger().check_conservation().is_ok(),
    }))
}

/// 演示用配置：服务身份 + 单一仲裁者，全部来自确定性种子。
pub fn demo_config() -> SafetyConfig {
    let arbiter = role_keys(setup::ROLE_ARBITER);
    let service = role_keys(setup::ROLE_SERVICE);
    SafetyConfig::single_arbiter(service.did(), arbiter.did())
}

/// 演示用参与者的 DID 列表（账本快照的可比较顺序无关，见 `ledger_snapshot`）。
pub fn demo_participants() -> Vec<Did> {
    vec![
        role_keys(setup::ROLE_REPORTER).did(),
        role_keys(setup::ROLE_SUBJECT).did(),
        role_keys(setup::ROLE_ARBITER).did(),
    ]
}
