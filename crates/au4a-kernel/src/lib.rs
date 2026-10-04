//! AU4A 宿主内核（轨道 v1.0 · Autonomy）。
//!
//! 内核只做四件事，而且每件都必须是「Agent 能自己发起」的：
//!
//! 1. **自主注册**：Agent 生成密钥、自带质押、自己声明能力，内核不要求人类账户。
//! 2. **PMB 投递**：签名信封入队/出队，广播与单播都走同一条路径。
//! 3. **策略与拒绝**：用类型化拒绝记录「谁因为什么被拒绝」，恶意与竞争分开计。
//! 4. **只读观察投影**：`observe()` 是人类的**唯一**入口，它返回一个值，不提供任何写路径。
//!
//! `Kernel` 不读墙钟、不做 I/O：时间是 [`LogicalClock`]，网络与面板在 `au4a-node`。

use au4a_core::{
    all_passed, canonical_hash, AgentKeys, CoreError, CoreResult, Credits, Did, Envelope,
    EvidenceGrade, Ledger, LedgerView, LogicalClock, Refusal, RefusalCode, SelfCheck,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub mod audit;
pub mod autonomy;
pub mod registry;

pub use audit::{audit_kernel, AuditFinding, HostAudit};
pub use autonomy::{
    classify_error, AgentContext, AutonomyLayer, AutonomyPolicy, AutonomyState, AutonomyTurn,
    Intent, IntentKind, IntentRecord,
};
pub use registry::{AgentRegistry, RegistrySnapshot};

/// 内核配置。人类可以设定初始资源与安全底线，但不能设定「谁做什么」。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelConfig {
    /// 网络标识（进入签名载荷，跨网络重放无效）。
    pub network_id: String,
    /// 准入质押下限。
    pub min_stake: Credits,
    /// 新 Agent 的创世额度（由网络规则发放，不由人类逐次批准）。
    pub genesis_mint: Credits,
    /// `cpu-proto` 证据等级的结算上限。
    pub cpu_proto_settle_cap: Credits,
    /// 每账户可携带的最大能力声明数（防止公告轰炸）。
    pub max_skills: usize,
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            network_id: "au4a-local".to_string(),
            min_stake: Credits(10),
            genesis_mint: Credits(1_000),
            cpu_proto_settle_cap: Credits(100),
            max_skills: 64,
        }
    }
}

/// Agent 的能力名片。由 Agent 自己构造并签名，内核只做准入检查。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCard {
    pub did: Did,
    pub display: String,
    pub skills: Vec<String>,
    pub stake: Credits,
    pub evidence: EvidenceGrade,
}

/// 进度事件——观察层「看进度」的数据来源。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub at: u64,
    pub kind: String,
    pub detail: String,
}

/// 投递结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReport {
    pub id: String,
    pub accepted: bool,
    pub queue_len: usize,
}

/// 观察层里的 Agent 行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRow {
    pub did: String,
    pub display: String,
    pub skills: Vec<String>,
    pub stake: Credits,
    pub evidence: String,
    pub available: Credits,
    pub locked: Credits,
}

/// 人类能看到的全部内容（进度 + 结果 + 收益）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObserverView {
    pub network_id: String,
    pub now: u64,
    pub agents: Vec<AgentRow>,
    pub messages_delivered: u64,
    pub refusal_count: u64,
    pub ledger: LedgerView,
    pub progress: Vec<ProgressEvent>,
}

/// 宿主内核。
#[derive(Clone, Debug)]
pub struct Kernel {
    config: KernelConfig,
    clock: LogicalClock,
    ledger: Ledger,
    agents: AgentRegistry,
    queue: Vec<Envelope>,
    delivered: u64,
    refusals: Vec<(Did, Refusal)>,
    progress: Vec<ProgressEvent>,
}

impl Kernel {
    pub fn new(config: KernelConfig) -> Self {
        let mut kernel = Self {
            config,
            clock: LogicalClock::new(),
            ledger: Ledger::new(),
            agents: AgentRegistry::new(),
            queue: Vec::new(),
            delivered: 0,
            refusals: Vec::new(),
            progress: Vec::new(),
        };
        kernel.emit("network.genesis", format!("network_id={}", kernel.config.network_id));
        kernel
    }

    pub fn config(&self) -> &KernelConfig {
        &self.config
    }

    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    pub fn tick(&mut self) -> u64 {
        self.clock.tick()
    }

    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    pub fn ledger_mut(&mut self) -> &mut Ledger {
        &mut self.ledger
    }

    /// 按注册序迭代名片（顺序是语义的一部分：同一串注册动作必须给出同一序列）。
    pub fn agents(&self) -> impl Iterator<Item = &AgentCard> {
        self.agents.cards()
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    pub fn card(&self, did: &Did) -> Option<&AgentCard> {
        self.agents.get(did)
    }

    /// 是否在册。只读谓词，路由与审计都用它。
    pub fn is_registered(&self, did: &Did) -> bool {
        self.agents.contains(did)
    }

    /// 声明了某能力的 Agent（按注册序）——能力路由的第一跳。
    pub fn agents_with_skill(&self, skill: &str) -> Vec<&AgentCard> {
        self.agents.agents_with_skill(skill)
    }

    /// 注册表指纹：同一串注册动作 → 同一个值（重放比对用）。
    pub fn registry_fingerprint(&self) -> CoreResult<String> {
        self.agents.fingerprint()
    }

    /// 只读的注册表投影。
    pub fn registry_snapshot(&self) -> RegistrySnapshot {
        self.agents.snapshot()
    }

    /// 注册表缺陷清单（空即自洽）：重复身份、倒排索引不忠实、同一名片重复声明能力。
    pub fn registry_defects(&self) -> Vec<String> {
        let mut defects = Vec::new();
        if !self.agents.index_is_faithful() {
            defects.push("skill index is not a faithful projection of the cards".to_string());
        }
        let dids: Vec<&Did> = self.agents.dids().collect();
        let mut unique: Vec<&Did> = dids.clone();
        unique.sort();
        unique.dedup();
        if unique.len() != dids.len() {
            defects.push("duplicate identity in registration order".to_string());
        }
        for (did, skill) in self.agents.duplicate_skills() {
            defects.push(format!("{did} declares skill {skill} more than once"));
        }
        defects
    }

    /// 待投递队列的只读视图。
    pub fn queued_envelopes(&self) -> &[Envelope] {
        &self.queue
    }

    /// 进度事件流的只读视图（人类观察层「看进度」的来源）。
    pub fn progress_events(&self) -> &[ProgressEvent] {
        &self.progress
    }

    /// 宿主审计：把内核声称的不变式变成可断言对象。
    pub fn audit(&self) -> HostAudit {
        audit_kernel(self)
    }

    /// 追加一条进度事件（任何轨道都可以发，观察层只读订阅）。
    pub fn emit(&mut self, kind: &str, detail: impl Into<String>) {
        let at = self.clock.now();
        self.progress.push(ProgressEvent {
            at,
            kind: kind.to_string(),
            detail: detail.into(),
        });
    }

    /// Agent 自主注册：自证身份 + 自带质押 + 自报能力。
    pub fn register(
        &mut self,
        keys: &AgentKeys,
        display: impl Into<String>,
        skills: &[&str],
        stake: Credits,
    ) -> CoreResult<AgentCard> {
        let did = keys.did();
        if self.agents.contains(&did) {
            return Err(CoreError::DuplicateAgent);
        }
        if stake < self.config.min_stake {
            self.refuse(&did, RefusalCode::PolicyDenied, "stake below min_stake");
            return Err(CoreError::InsufficientStake);
        }
        if skills.len() > self.config.max_skills {
            self.refuse(&did, RefusalCode::PolicyDenied, "too many skills");
            return Err(CoreError::InvalidKind);
        }
        // 创世额度：只在余额为零时发放，避免重复领取。
        if self.ledger.balance(&did).available == Credits::ZERO {
            self.ledger.mint(&did, self.config.genesis_mint)?;
        }
        self.ledger.lock(&did, stake)?;
        let card = AgentCard {
            did: did.clone(),
            display: display.into(),
            skills: skills.iter().map(|s| (*s).to_string()).collect(),
            stake,
            evidence: EvidenceGrade::Verified,
        };
        self.agents.insert(card.clone())?;
        self.clock.tick();
        self.emit(
            "agent.registered",
            format!("{} stake={} skills={}", card.display, stake, card.skills.len()),
        );
        Ok(card)
    }

    /// 投递一个信封。验签失败按 `unauthorized` 拒绝并记录（恶意证据）。
    pub fn send(&mut self, env: &Envelope) -> CoreResult<DeliveryReport> {
        if let Err(err) = env.verify() {
            let code = match &err {
                CoreError::NotSealed | CoreError::InvalidSignature => RefusalCode::Unauthorized,
                _ => RefusalCode::Malformed,
            };
            let from = env.from.clone();
            self.refuse(&from, code, format!("envelope rejected: {err}"));
            return Err(err);
        }
        if !self.agents.contains(&env.from) {
            self.refuse(&env.from, RefusalCode::Unauthorized, "sender not registered");
            return Err(CoreError::UnknownAgent);
        }
        if let Some(to) = &env.to {
            if !self.agents.contains(to) {
                self.refuse(&env.from, RefusalCode::StaleEpoch, "recipient unknown");
                return Err(CoreError::UnknownAgent);
            }
        }
        self.queue.push(env.clone());
        self.delivered += 1;
        self.clock.observe(env.ts);
        Ok(DeliveryReport {
            id: env.id.clone(),
            accepted: true,
            queue_len: self.queue.len(),
        })
    }

    /// 取出全部待投递信封（投递语义由上层决定：本地内存或真实网络）。
    pub fn drain(&mut self) -> Vec<Envelope> {
        std::mem::take(&mut self.queue)
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// 记录一次拒绝。
    pub fn refuse(&mut self, who: &Did, code: RefusalCode, reason: impl Into<String>) {
        self.refusals.push((who.clone(), Refusal::new(code, reason)));
    }

    pub fn refusals(&self) -> &[(Did, Refusal)] {
        &self.refusals
    }

    /// 对某个 Agent 的升级判定（委员会在 v1.7 消费它）。
    pub fn escalation_for(&self, who: &Did, code: RefusalCode) -> au4a_core::Escalation {
        let repeats = self
            .refusals
            .iter()
            .filter(|(d, r)| d == who && r.code == code)
            .count() as u32;
        au4a_core::refusal::escalate(code, repeats)
    }

    /// 结算：先过证据闸门，再动账本。`unverified` 永远不结算。
    pub fn settle(
        &mut self,
        from: &Did,
        to: &Did,
        amount: Credits,
        evidence: EvidenceGrade,
    ) -> CoreResult<()> {
        if !evidence.settleable(amount, self.config.cpu_proto_settle_cap) {
            self.refuse(from, RefusalCode::PolicyDenied, "evidence gate refused settlement");
            return Err(CoreError::InsufficientFunds);
        }
        self.ledger.transfer(from, to, amount)?;
        self.clock.tick();
        self.emit(
            "settlement",
            format!(
                "{} -> {} amount={} grade={}",
                au4a_core::short_id(from.as_str()),
                au4a_core::short_id(to.as_str()),
                amount,
                evidence.as_str()
            ),
        );
        Ok(())
    }

    /// 只读投影。人类的唯一入口：它返回一个值，没有任何配套的写方法。
    pub fn observe(&self) -> ObserverView {
        ObserverView {
            network_id: self.config.network_id.clone(),
            now: self.clock.now(),
            agents: self
                .agents
                .cards()
                .map(|c| {
                    let acct = self.ledger.balance(&c.did);
                    AgentRow {
                        did: c.did.as_str().to_string(),
                        display: c.display.clone(),
                        skills: c.skills.clone(),
                        stake: c.stake,
                        evidence: c.evidence.as_str().to_string(),
                        available: acct.available,
                        locked: acct.locked,
                    }
                })
                .collect(),
            messages_delivered: self.delivered,
            refusal_count: self.refusals.len() as u64,
            ledger: self.ledger.view(),
            progress: self.progress.clone(),
        }
    }

    /// 观察层的 JSON 投影（三个只读面板共用）。
    pub fn observe_json(&self) -> Value {
        serde_json::to_value(self.observe()).unwrap_or(Value::Null)
    }

    /// 内核自检：宿主审计（守恒 / 准入 / 质押覆盖 / 注册表 / 队列 / 拒绝分类 / 时钟）
    /// 加上「观察层只读」这一结构性结论。
    pub fn self_check(&self) -> Vec<SelfCheck> {
        let mut checks = self.audit().to_self_checks(TRACK);
        checks.push(SelfCheck::pass(
            TRACK,
            "observer.read_only",
            "observe() 取 &self 并返回投影值；内核不存在以人类为发起者的写入口",
        ));
        checks
    }

    /// 内核产物摘要（观察层「结果」面板）。
    pub fn results_json(&self) -> Value {
        json!({
            "agents": self.agents.len(),
            "delivered": self.delivered,
            "refusals": self.refusals.len(),
            "queue": self.queue.len(),
        })
    }
}

/// 轨道号。
pub const TRACK: &str = "1.0";
/// 轨道标题。
pub const TITLE: &str = "Autonomy 自治内核";
/// 版本区间。
pub const RANGE: &str = "v1.0.1 → v1.0.10";

/// 本轨道 Agent 的确定性种子。
///
/// 前缀刻意取 `0xA4 0x0A`（AU4A 的自指），避免与其它轨道的种子撞成同一个 DID——
/// 十条轨道共用同一个 `Kernel` 实例，身份碰撞会让注册变成 `DuplicateAgent`。
pub fn track_seed(tag: u8) -> [u8; 32] {
    let mut seed = [0xA1u8; 32];
    seed[0] = 0xA4;
    seed[1] = 0x0A;
    seed[2] = tag;
    seed
}

/// 本轨道的确定性 Agent 密钥。
pub fn track_keys(tag: u8) -> AgentKeys {
    AgentKeys::from_seed(&track_seed(tag))
}

/// 确定性引导：注册本轨道的三个 Agent（宿主 / 观察 / 结算）。
///
/// 用于 `self_check()`、`scenario()` 与集成测试。纯 CPU、无 I/O、同一配置给同一结果。
pub fn bootstrap(config: KernelConfig) -> CoreResult<Kernel> {
    let mut kernel = Kernel::new(config);
    for (tag, display, skills) in TRACK_AGENTS {
        let keys = track_keys(tag);
        if kernel.is_registered(&keys.did()) {
            continue;
        }
        kernel.register(&keys, display, skills, Credits(20))?;
    }
    Ok(kernel)
}

/// 本轨道自带的三个 Agent：(种子标签, 显示名, 能力声明)。
const TRACK_AGENTS: [(u8, &str, &[&str]); 3] = [
    (1, "kernel-host", &["kernel.host"]),
    (2, "kernel-observer", &["kernel.observe"]),
    (3, "kernel-settler", &["kernel.settle"]),
];

/// 若 Agent 已在册则复用名片；否则自主注册。让 `scenario()` 可以被重复调用而不 panic。
fn ensure_agent(
    kernel: &mut Kernel,
    tag: u8,
    display: &str,
    skills: &[&str],
) -> CoreResult<AgentCard> {
    let keys = track_keys(tag);
    if let Some(card) = kernel.card(&keys.did()) {
        return Ok(card.clone());
    }
    match kernel.register(&keys, display, skills, Credits(20)) {
        Ok(card) => Ok(card),
        // 并发场景下可能刚好被别人注册：复用而不是失败。
        Err(CoreError::DuplicateAgent) => kernel
            .card(&keys.did())
            .cloned()
            .ok_or(CoreError::UnknownAgent),
        Err(e) => Err(e),
    }
}

/// 轨道自检入口（节点聚合时使用）。每个检查都真的跑了一遍宿主闭环。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = vec![SelfCheck::pass(
        TRACK,
        "track.wired",
        format!("{TITLE} {RANGE} 已接入 au4a-node"),
    )];

    match bootstrap(KernelConfig::default()) {
        Ok(kernel) => {
            let audit = kernel.audit();
            checks.push(if audit.is_clean() {
                SelfCheck::pass(
                    TRACK,
                    "host.audit",
                    format!("{} 项宿主不变式全部成立（确定性种子引导）", audit.findings.len()),
                )
            } else {
                SelfCheck::fail(
                    TRACK,
                    "host.audit",
                    format!(
                        "{} 项不成立：{:?}",
                        audit.failures().len(),
                        audit.failures().iter().map(|f| f.name.clone()).collect::<Vec<String>>()
                    ),
                )
            });
        }
        Err(e) => checks.push(SelfCheck::fail(TRACK, "host.bootstrap", e.to_string())),
    }

    match (bootstrap(KernelConfig::default()), bootstrap(KernelConfig::default())) {
        (Ok(a), Ok(b)) => {
            let same = a.registry_fingerprint() == b.registry_fingerprint();
            checks.push(if same {
                SelfCheck::pass(
                    TRACK,
                    "registry.deterministic",
                    "两次独立引导得到同一注册表指纹（同种子 → 同结果）",
                )
            } else {
                SelfCheck::fail(TRACK, "registry.deterministic", "注册表指纹不确定")
            });
        }
        _ => checks.push(SelfCheck::fail(TRACK, "registry.deterministic", "引导失败")),
    }

    checks
}

/// 轨道级结果摘要（节点「结果」面板聚合用）。
pub fn results_json() -> CoreResult<Value> {
    let checks = self_check();
    let value = serde_json::to_value(&checks).map_err(|_| CoreError::Encoding)?;
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "checks_passed": checks.iter().filter(|c| c.passed).count(),
        "checks_total": checks.len(),
        "all_passed": all_passed(&checks),
        "self_checks": value,
    }))
}

/// 本轨道自有端到端流程：用**共享内核**走一遍「注册 → 通告 → 投递 → 结算 → 审计 → 观察」。
///
/// 返回 JSON 摘要。它不读文件、不开网络、不读墙钟，可被重复调用：
/// 已经在册的 Agent 复用名片，已经发生的事不会被伪造。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    // 1. Agent 自主注册（无需人类账户）。
    let host = ensure_agent(kernel, 1, "kernel-host", &["kernel.host"])?;
    let observer = ensure_agent(kernel, 2, "kernel-observer", &["kernel.observe"])?;
    let settler = ensure_agent(kernel, 3, "kernel-settler", &["kernel.settle"])?;

    // 2. 能力通告：广播信封（to = None），先签名后发送。
    let host_keys = track_keys(1);
    let observer_keys = track_keys(2);
    let mut announced = 0usize;
    for (keys, card) in [(&host_keys, &host), (&observer_keys, &observer)] {
        let ts = kernel.now() + 1;
        let env = Envelope::new(
            keys.did(),
            None,
            "agent.card",
            ts,
            None,
            json!({"skills": card.skills, "stake": card.stake, "evidence": card.evidence.as_str()}),
        )?
        .seal(keys)?;
        kernel.send(&env)?;
        announced += 1;
    }

    // 3. 点对点消息 + 投递。
    let ts = kernel.now() + 1;
    let env = Envelope::new(
        host_keys.did(),
        Some(observer.did.clone()),
        "progress.event",
        ts,
        None,
        json!({"kind": "host.ready", "by": "kernel-host"}),
    )?
    .seal(&host_keys)?;
    kernel.send(&env)?;
    let drained = kernel.drain().len();

    // 4. 结算：verified 通过、cpu-proto 受额度限制、unverified 永远拒绝。
    let mut settled_verified = 0usize;
    let mut settled_cpu_proto = 0usize;
    let mut refused_unverified = 0usize;
    if kernel
        .settle(&host.did, &observer.did, Credits(5), EvidenceGrade::Verified)
        .is_ok()
    {
        settled_verified += 1;
    }
    if kernel
        .settle(&host.did, &settler.did, Credits(3), EvidenceGrade::CpuProto)
        .is_ok()
    {
        settled_cpu_proto += 1;
    }
    if kernel
        .settle(&observer.did, &settler.did, Credits(1), EvidenceGrade::Unverified)
        .is_err()
    {
        refused_unverified += 1;
    }

    // 5. 自治层：host 的运行时自己决定下一步（通告 + 回应 settler 的报价请求），
    //    第二次 turn 因为心跳未到而空转——决策完全由 Agent 自己做，没有任何人工输入。
    let settler_keys = track_keys(3);
    let offer_req = Envelope::new(
        settler_keys.did(),
        Some(host.did.clone()),
        "negotiate.offer",
        kernel.now() + 1,
        None,
        json!({"skill": "kernel.host", "note": "autonomy scenario"}),
    )?
    .seal(&settler_keys)?;
    kernel.send(&offer_req)?;

    let mut host_runtime = AutonomyLayer::new(track_keys(1), AutonomyPolicy::default());
    let first_turn = host_runtime.turn(kernel)?;
    let second_turn = host_runtime.turn(kernel)?;
    let planned_first: Vec<&str> = first_turn.planned.iter().map(|k| k.as_str()).collect();
    let planned_second: Vec<&str> = second_turn.planned.iter().map(|k| k.as_str()).collect();
    let autonomy = json!({
        "planned_first": planned_first,
        "sent_first": first_turn.sent,
        "planned_second": planned_second,
        "sent_second": second_turn.sent,
        "journal": host_runtime.journal().len(),
        "announcements": host_runtime.state().announcements,
        "halted": host_runtime.state().halted,
    });

    // 6. 审计 + 只读观察。
    let audit = kernel.audit();
    let view = kernel.observe();

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "agents": {"host": host.display, "observer": observer.display, "settler": settler.display},
        "steps": ["register", "announce", "deliver", "settle", "autonomy", "audit", "observe"],
        "announced": announced,
        "drained": drained,
        "settled_verified": settled_verified,
        "settled_cpu_proto": settled_cpu_proto,
        "refused_unverified": refused_unverified,
        "autonomy": autonomy,
        "queue_len": kernel.queue_len(),
        "registry_fingerprint": kernel.registry_fingerprint()?,
        "audit": audit.to_json(),
        "observer": {
            "network_id": view.network_id,
            "now": view.now,
            "agents": view.agents.len(),
            "messages_delivered": view.messages_delivered,
            "refusal_count": view.refusal_count,
            "progress_events": view.progress.len(),
            "yield_total": view.ledger.total,
        },
    }))
}

/// 供 `canonical_hash` 复用的重导出，保证 tracks 不必各自依赖 au4a-core 的内部路径。
pub fn fingerprint(value: &Value) -> CoreResult<String> {
    canonical_hash(value)
}

/// 仅自测使用的「制造不一致」钩子：审计的价值在于能发现被篡改的状态，
/// 而这些状态无法通过公开 API 合法产生，因此只能在 crate 内部构造。
#[cfg(test)]
impl Kernel {
    pub(crate) fn replace_card_for_audit(&mut self, card: AgentCard) {
        if let Some(slot) = self.agents.card_mut(&card.did) {
            *slot = card;
        }
    }

    pub(crate) fn break_skill_index_for_audit(&mut self) {
        self.agents.break_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::{kinds, Envelope};

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    #[test]
    fn agents_register_themselves_without_a_human_account() {
        let mut k = Kernel::new(KernelConfig::default());
        let keys = agent(1);
        let card = k.register(&keys, "translator", &["translate.en-zh"], Credits(20)).unwrap();
        assert_eq!(card.did, keys.did());
        assert_eq!(k.agent_count(), 1);
        assert_eq!(k.ledger().balance(&keys.did()).locked, Credits(20));
        k.ledger().check_conservation().unwrap();
    }

    #[test]
    fn under_staked_agents_are_refused_with_a_typed_reason() {
        let mut k = Kernel::new(KernelConfig::default());
        let keys = agent(2);
        assert_eq!(
            k.register(&keys, "cheapskate", &[], Credits(1)),
            Err(CoreError::InsufficientStake)
        );
        assert_eq!(k.refusals().len(), 1);
        assert_eq!(k.refusals()[0].1.code, RefusalCode::PolicyDenied);
    }

    #[test]
    fn a_forged_envelope_is_recorded_as_misconduct() {
        let mut k = Kernel::new(KernelConfig::default());
        let a = agent(3);
        let b = agent(4);
        k.register(&a, "a", &["x"], Credits(20)).unwrap();
        k.register(&b, "b", &["y"], Credits(20)).unwrap();
        let mut env = Envelope::new(a.did(), Some(b.did()), kinds::AGENT_CARD, 1, None, json!({}))
            .unwrap()
            .seal(&a)
            .unwrap();
        env.body = json!({"tampered": true});
        assert!(k.send(&env).is_err());
        let (_, refusal) = &k.refusals()[0];
        assert!(refusal.code.is_misconduct());
        assert_eq!(k.escalation_for(&a.did(), refusal.code), au4a_core::Escalation::Quarantine);
    }

    #[test]
    fn delivery_is_content_addressed_and_drained_once() {
        let mut k = Kernel::new(KernelConfig::default());
        let a = agent(5);
        let b = agent(6);
        k.register(&a, "a", &["x"], Credits(20)).unwrap();
        k.register(&b, "b", &["y"], Credits(20)).unwrap();
        let env = Envelope::new(a.did(), Some(b.did()), kinds::PROGRESS_EVENT, 1, None, json!({"p":1}))
            .unwrap()
            .seal(&a)
            .unwrap();
        let report = k.send(&env).unwrap();
        assert!(report.accepted);
        assert_eq!(k.drain().len(), 1);
        assert_eq!(k.drain().len(), 0);
    }

    #[test]
    fn broadcast_to_null_is_allowed() {
        let mut k = Kernel::new(KernelConfig::default());
        let a = agent(7);
        k.register(&a, "a", &["x"], Credits(20)).unwrap();
        let env = Envelope::new(a.did(), None, kinds::AGENT_CARD, 1, None, json!({"announce": true}))
            .unwrap()
            .seal(&a)
            .unwrap();
        assert!(k.send(&env).unwrap().accepted);
    }

    #[test]
    fn evidence_gate_blocks_unverified_settlement() {
        let mut k = Kernel::new(KernelConfig::default());
        let a = agent(8);
        let b = agent(9);
        k.register(&a, "a", &["x"], Credits(20)).unwrap();
        k.register(&b, "b", &["y"], Credits(20)).unwrap();
        assert!(k
            .settle(&a.did(), &b.did(), Credits(10), EvidenceGrade::Unverified)
            .is_err());
        assert!(k
            .settle(&a.did(), &b.did(), Credits(10), EvidenceGrade::Verified)
            .is_ok());
        k.ledger().check_conservation().unwrap();
    }

    #[test]
    fn observer_view_is_a_value_and_has_no_write_counterpart() {
        let mut k = Kernel::new(KernelConfig::default());
        let a = agent(10);
        k.register(&a, "a", &["x"], Credits(20)).unwrap();
        let view = k.observe();
        assert_eq!(view.agents.len(), 1);
        assert_eq!(view.ledger.total, Credits(1_000));
        assert!(!view.progress.is_empty());
        assert!(au4a_core::all_passed(&k.self_check()));
    }
}
