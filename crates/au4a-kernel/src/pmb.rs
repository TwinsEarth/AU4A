//! PMB 协议扩展（v1.0.6）：把「谁在什么时候能给谁发什么」变成可校验的**路由语义**。
//!
//! 线格式与签名完全沿用冻结基元层：所有消息都是 [`au4a_core::Envelope`]，
//! **先签名后发送**，4 字节大端长度前缀，1 MiB 上限，超限在分配前拒绝。
//! 本模块只加语义，不加私有格式：
//!
//! * [`classify_kind`]：把消息类型归到五类语义（通告 / 协商 / 结算 / 治理 / 遥测）；
//! * [`PmbRouter::admit`]：准入与路由判定——发送者必须在册、收件人必须在册、
//!   治理消息不得广播、逻辑时钟不得倒退、**同一 `id` 不得重复投递（重放保护）**；
//! * 每条拒绝都带类型化 [`RefusalCode`]，并保持「恶意 2 码 vs 竞争 8 码」：
//!   签名/身份不成立 → `unauthorized`（恶意，单次即成立）；
//!   未知类型 → `unsupported`、重放 → `conflict`、时钟落后 → `stale_epoch`（都是竞争，单次只记警告）。
//!
//! 路由器是**纯判定**：它不改内核、不写队列；判定通过后由调用方 `Kernel::send`。

use std::collections::BTreeMap;

use au4a_core::{
    decode_frame, encode_frame, AgentKeys, CoreError, CoreResult, Credits, Did, Envelope,
    EvidenceGrade, RefusalCode, MAX_FRAME,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{AgentCard, Kernel};

/// 本轨道扩展的消息类型（基元层的 `kinds::*` 仍然有效，两者可以共存）。
pub mod kinds_ext {
    /// 能力通告（广播）。
    pub const AGENT_CARD: &str = "agent.card";
    /// 报价（点对点）。
    pub const NEGOTIATE_OFFER: &str = "negotiate.offer";
    /// 拒绝报价（点对点），拒绝也必须是消息而不是沉默。
    pub const NEGOTIATE_DECLINE: &str = "negotiate.decline";
    /// 进度事件（广播或点对点）。
    pub const PROGRESS_EVENT: &str = "progress.event";
    /// 结算请求（点对点）。
    pub const SETTLE_REQUEST: &str = "settle.request";
    /// 委员会动议（点对点）。
    pub const COUNCIL_MOTION: &str = "council.motion";
    /// 委员会选票（点对点）。
    pub const COUNCIL_VOTE: &str = "council.vote";
    /// 自治心跳（广播）。
    pub const AUTONOMY_HEARTBEAT: &str = "autonomy.heartbeat";
}

/// 消息的语义分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageClass {
    /// 能力通告：允许广播。
    Announce,
    /// 协商：必须点对点。
    Negotiation,
    /// 结算：必须点对点。
    Settlement,
    /// 治理：必须点对点（动议与选票不接受广播，避免舆论化裁决）。
    Governance,
    /// 遥测：允许广播。
    Telemetry,
    /// 未知类型：拒绝（`unsupported`），而不是猜。
    Unknown,
}

impl MessageClass {
    pub const ALL: [MessageClass; 6] = [
        MessageClass::Announce,
        MessageClass::Negotiation,
        MessageClass::Settlement,
        MessageClass::Governance,
        MessageClass::Telemetry,
        MessageClass::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MessageClass::Announce => "announce",
            MessageClass::Negotiation => "negotiation",
            MessageClass::Settlement => "settlement",
            MessageClass::Governance => "governance",
            MessageClass::Telemetry => "telemetry",
            MessageClass::Unknown => "unknown",
        }
    }

    /// 这一类是否允许广播（`to = None`）。
    pub fn broadcastable(self) -> bool {
        matches!(self, MessageClass::Announce | MessageClass::Telemetry)
    }
}

/// 消息类型 → 语义分类。
pub fn classify_kind(kind: &str) -> MessageClass {
    match kind {
        kinds_ext::AGENT_CARD | kinds_ext::AUTONOMY_HEARTBEAT => MessageClass::Announce,
        kinds_ext::NEGOTIATE_OFFER | kinds_ext::NEGOTIATE_DECLINE => MessageClass::Negotiation,
        kinds_ext::SETTLE_REQUEST => MessageClass::Settlement,
        kinds_ext::COUNCIL_MOTION | kinds_ext::COUNCIL_VOTE => MessageClass::Governance,
        kinds_ext::PROGRESS_EVENT | au4a_core::kinds::SAFETY_REPORT => MessageClass::Telemetry,
        _ => MessageClass::Unknown,
    }
}

/// 允许的逻辑时钟落后量：超过它才按 `stale_epoch` 拒绝。
pub const MAX_LAG: u64 = 64;

/// 一次路由判定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawRouteDecision", into = "RawRouteDecision")]
pub struct RouteDecision {
    pub id: String,
    pub kind: String,
    pub class: MessageClass,
    pub accepted: bool,
    /// 广播时展开出的收件人（不含发送者自己）；点对点时只有一个。
    pub recipients: Vec<String>,
    pub code: Option<RefusalCode>,
    pub reason: String,
}

/// 线上形态（字段一一对应，JSON 形状不变）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawRouteDecision {
    pub id: String,
    pub kind: String,
    pub class: MessageClass,
    pub accepted: bool,
    pub recipients: Vec<String>,
    pub code: Option<RefusalCode>,
    pub reason: String,
}

/// v2.4.0 新增（P1）：**路由判定记录必须自洽**。
///
/// 修复前 `RouteDecision` 是纯 `derive(Deserialize)`，外部 JSON 可以造出
/// `{"accepted": true, "code": "unauthorized"}`（既声称通过、又附一个恶意拒绝码）、
/// 或 `accepted: true` 但 `recipients: []`（声称投递成功却没有收件人）。
/// 判定记录是投递侧的**账**，被伪造的"已投递"会污染审计与观察面板。
impl TryFrom<RawRouteDecision> for RouteDecision {
    type Error = CoreError;

    fn try_from(raw: RawRouteDecision) -> CoreResult<Self> {
        if raw.id.trim().is_empty() || raw.reason.trim().is_empty() {
            return Err(CoreError::Encoding);
        }
        if raw.class != classify_kind(&raw.kind) {
            return Err(CoreError::InvalidKind);
        }
        if raw.accepted {
            if raw.code.is_some() || raw.recipients.is_empty() {
                return Err(CoreError::Encoding);
            }
        } else if raw.code.is_none() {
            return Err(CoreError::Encoding);
        }
        Ok(RouteDecision {
            id: raw.id,
            kind: raw.kind,
            class: raw.class,
            accepted: raw.accepted,
            recipients: raw.recipients,
            code: raw.code,
            reason: raw.reason,
        })
    }
}

impl From<RouteDecision> for RawRouteDecision {
    fn from(d: RouteDecision) -> Self {
        RawRouteDecision {
            id: d.id,
            kind: d.kind,
            class: d.class,
            accepted: d.accepted,
            recipients: d.recipients,
            code: d.code,
            reason: d.reason,
        }
    }
}

impl RouteDecision {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "class": self.class.as_str(),
            "accepted": self.accepted,
            "recipients": self.recipients.len(),
            "code": self.code.map(|c| c.as_str()),
            "reason": self.reason,
        })
    }
}

/// 路由统计（观察层「结果」面板可以展示投递质量）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterStats {
    pub admitted: u64,
    pub refused: u64,
    pub replays: u64,
    pub broadcasts: u64,
    pub direct: u64,
    pub by_class: BTreeMap<String, u64>,
}

/// PMB 路由器：重放保护 + 准入 + 收件人展开。
#[derive(Clone, Debug, Default)]
pub struct PmbRouter {
    /// 见过的 `id` → 首次投递的逻辑时刻。
    seen: BTreeMap<String, u64>,
    stats: RouterStats,
}

impl PmbRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> &RouterStats {
        &self.stats
    }

    pub fn seen_count(&self) -> usize {
        self.seen.len()
    }

    pub fn has_seen(&self, id: &str) -> bool {
        self.seen.contains_key(id)
    }

    /// v2.4.0 修复（P1）：**回收已不可能再被判为"重放"的旧 id**。
    ///
    /// 修复前 `seen` 只增不减：长跑节点被"大量合法且各不相同的消息"持续撑大内存（呼吸式 DoS）。
    /// 回收是安全的：任何 `ts < now - MAX_LAG` 的信封在准入第 5 步就会被判 `stale_epoch`，
    /// 因此"不记得它"不会放过任何真正的重放。
    fn evict_stale(&mut self, now: u64) {
        let horizon = now.saturating_sub(MAX_LAG);
        // 用 `>=`：准入的迟到判定是严格不等式（`ts + MAX_LAG < now`），
        // 因此 `ts == now - MAX_LAG` 仍然可准入，不能回收（否则重放保护会被自己的回收绕过）。
        self.seen.retain(|_, ts| *ts >= horizon);
    }

    /// 准入判定。纯函数（除了记录「见过哪些 id」这一个必要状态）。
    pub fn admit(&mut self, kernel: &Kernel, env: &Envelope) -> RouteDecision {
        // 先回收，再判定：保证 `seen` 的规模只与"滞后视野内的信封数"有关，与总消息量无关。
        self.evict_stale(kernel.now());
        let class = classify_kind(env.kind.as_str());
        let mut decision = RouteDecision {
            id: env.id.clone(),
            kind: env.kind.as_str().to_string(),
            class,
            accepted: false,
            recipients: Vec::new(),
            code: None,
            reason: String::new(),
        };

        // 1. 先验签：签名（或身份）不成立是恶意，单次即成立。
        if let Err(err) = env.verify() {
            return self.refuse(
                decision,
                RefusalCode::Unauthorized,
                format!("信封不可信：{err}"),
            );
        }

        // 2. 发送者必须在册。
        if !kernel.is_registered(&env.from) {
            return self.refuse(
                decision,
                RefusalCode::Unauthorized,
                "发送者不在册：自证身份未通过准入",
            );
        }

        // 3. 重放保护：同一内容寻址 id 只投递一次；重复投递按竞争（重传）处理。
        if self.seen.contains_key(&env.id) {
            self.stats.replays += 1;
            return self.refuse(
                decision,
                RefusalCode::Conflict,
                "同一信封 id 已投递过：按重传竞争处理，不按恶意处理",
            );
        }

        // 4. 类型必须已知。
        if class == MessageClass::Unknown {
            return self.refuse(
                decision,
                RefusalCode::Unsupported,
                format!("未知消息类型 {}：拒绝而不是猜", env.kind.as_str()),
            );
        }

        // 5. 时钟不得严重落后（逻辑时钟，不读墙钟）。
        if env.ts.saturating_add(MAX_LAG) < kernel.now() {
            return self.refuse(
                decision,
                RefusalCode::StaleEpoch,
                format!(
                    "信封时刻 {} 落后当前 {} 超过 {MAX_LAG}",
                    env.ts,
                    kernel.now()
                ),
            );
        }

        // 5b. **时钟不得超前**（v2.4.0 修复 P0：对称界）。
        //
        // 修复前只查"落后"，于是 `ts = u64::MAX` 的合法信封能通过准入；而 `Kernel::send`
        // 会用 `clock.observe(env.ts)` 把本地逻辑时钟取 max —— 一条消息就能把时钟顶到天花板，
        // 此后任何正常信封都满足 `ts + MAX_LAG < now` 而被判 stale_epoch：
        // **远端可触发、单条消息、节点不会自愈的拒绝服务。**
        if env.ts > kernel.now().saturating_add(MAX_LAG) {
            return self.refuse(
                decision,
                RefusalCode::StaleEpoch,
                format!(
                    "信封时刻 {} 超前当前 {} 超过 {MAX_LAG}",
                    env.ts,
                    kernel.now()
                ),
            );
        }

        // 6. 线格式上限：编码后在 1 MiB 之内（超限在分配前就被基元层拒绝）。
        match encode_frame(env) {
            Ok(frame) if frame.len() > MAX_FRAME + 4 => {
                return self.refuse(
                    decision,
                    RefusalCode::ResourceExhausted,
                    "信封超过 1 MiB 上限",
                );
            }
            Ok(_) => {}
            Err(err) => {
                return self.refuse(
                    decision,
                    RefusalCode::Malformed,
                    format!("信封无法编码为线格式：{err}"),
                );
            }
        }

        // 7. 收件人展开 + 治理消息不得广播。
        match &env.to {
            Some(to) => {
                if !kernel.is_registered(to) {
                    return self.refuse(
                        decision,
                        RefusalCode::StaleEpoch,
                        "收件人不在册（对端可能已离开）：这是竞争，不是恶意",
                    );
                }
                decision.recipients = vec![to.as_str().to_string()];
                self.stats.direct += 1;
            }
            None => {
                if !class.broadcastable() {
                    return self.refuse(
                        decision,
                        RefusalCode::PolicyDenied,
                        format!("{} 类消息必须点对点，不接受广播", class.as_str()),
                    );
                }
                decision.recipients = kernel
                    .agents()
                    .filter(|card| card.did != env.from)
                    .map(|card| card.did.as_str().to_string())
                    .collect();
                self.stats.broadcasts += 1;
            }
        }

        decision.accepted = true;
        decision.reason = format!("{} 类消息已准入", class.as_str());
        self.seen.insert(env.id.clone(), env.ts);
        self.stats.admitted += 1;
        *self
            .stats
            .by_class
            .entry(class.as_str().to_string())
            .or_insert(0) += 1;
        decision
    }

    fn refuse(
        &mut self,
        mut decision: RouteDecision,
        code: RefusalCode,
        reason: impl Into<String>,
    ) -> RouteDecision {
        decision.accepted = false;
        decision.code = Some(code);
        decision.reason = reason.into();
        self.stats.refused += 1;
        decision
    }
}

/// 构造并签名一个能力通告（广播）。
pub fn announce_card(keys: &AgentKeys, card: &AgentCard, ts: u64) -> CoreResult<Envelope> {
    Envelope::new(
        keys.did(),
        None,
        kinds_ext::AGENT_CARD,
        ts,
        None,
        json!({
            "display": card.display,
            "skills": card.skills,
            "stake": card.stake,
            "evidence": card.evidence.as_str(),
        }),
    )?
    .seal(keys)
}

/// 构造并签名一个进度事件。
pub fn progress(
    keys: &AgentKeys,
    to: Option<Did>,
    ts: u64,
    label: &str,
    detail: Value,
) -> CoreResult<Envelope> {
    Envelope::new(
        keys.did(),
        to,
        kinds_ext::PROGRESS_EVENT,
        ts,
        None,
        json!({"label": label, "detail": detail}),
    )?
    .seal(keys)
}

/// 构造并签名一个结算请求。
pub fn settle_request(
    keys: &AgentKeys,
    to: Did,
    ts: u64,
    amount: Credits,
    grade: EvidenceGrade,
) -> CoreResult<Envelope> {
    Envelope::new(
        keys.did(),
        Some(to),
        kinds_ext::SETTLE_REQUEST,
        ts,
        None,
        json!({"amount": amount, "grade": grade.as_str()}),
    )?
    .seal(keys)
}

/// 构造并签名一张委员会选票。
pub fn council_vote(
    keys: &AgentKeys,
    to: Did,
    ts: u64,
    motion: &str,
    ballot: &str,
) -> CoreResult<Envelope> {
    Envelope::new(
        keys.did(),
        Some(to),
        kinds_ext::COUNCIL_VOTE,
        ts,
        None,
        json!({"motion": motion, "ballot": ballot}),
    )?
    .seal(keys)
}

/// 解码线格式并**验签**：任何解码/验签失败的帧都不进入路由。
pub fn decode_and_verify(frame: &[u8]) -> CoreResult<Envelope> {
    let env = decode_frame(frame)?;
    env.verify()?;
    Ok(env)
}

/// 把信封编码成线格式并校验往返一致（投递前的自检）。
pub fn encode_checked(env: &Envelope) -> CoreResult<Vec<u8>> {
    let frame = encode_frame(env)?;
    let back = decode_and_verify(&frame)?;
    if back.id != env.id {
        return Err(CoreError::Encoding);
    }
    Ok(frame)
}

/// 编译期证据：扩展后的协议仍然只使用冻结基元的信封与分帧。
const _: fn(&Envelope) -> CoreResult<Vec<u8>> = encode_frame;
const _: fn(&[u8]) -> CoreResult<Envelope> = decode_frame;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KernelConfig;
    use au4a_core::Credits;

    fn keys(tag: u8) -> AgentKeys {
        AgentKeys::from_seed(&[tag; 32])
    }

    fn kernel_with(n: u8) -> Kernel {
        let mut k = Kernel::new(KernelConfig::default());
        for i in 0..n {
            k.register(
                &keys(120 + i),
                format!("agent-{i}"),
                &["skill.a"],
                Credits(20),
            )
            .unwrap();
        }
        k
    }

    #[test]
    fn kinds_classify_into_five_semantic_classes() {
        assert_eq!(classify_kind(kinds_ext::AGENT_CARD), MessageClass::Announce);
        assert_eq!(
            classify_kind(kinds_ext::NEGOTIATE_OFFER),
            MessageClass::Negotiation
        );
        assert_eq!(
            classify_kind(kinds_ext::NEGOTIATE_DECLINE),
            MessageClass::Negotiation
        );
        assert_eq!(
            classify_kind(kinds_ext::SETTLE_REQUEST),
            MessageClass::Settlement
        );
        assert_eq!(
            classify_kind(kinds_ext::COUNCIL_MOTION),
            MessageClass::Governance
        );
        assert_eq!(
            classify_kind(kinds_ext::COUNCIL_VOTE),
            MessageClass::Governance
        );
        assert_eq!(
            classify_kind(kinds_ext::PROGRESS_EVENT),
            MessageClass::Telemetry
        );
        assert_eq!(classify_kind("totally.made.up"), MessageClass::Unknown);
        assert!(MessageClass::Announce.broadcastable());
        assert!(!MessageClass::Governance.broadcastable());
        assert_eq!(MessageClass::ALL.len(), 6);
    }

    #[test]
    fn a_valid_direct_message_is_admitted_once() {
        let k = kernel_with(2);
        let mut router = PmbRouter::new();
        let env = settle_request(
            &keys(120),
            keys(121).did(),
            0,
            Credits(7),
            EvidenceGrade::CpuProto,
        )
        .unwrap();
        let first = router.admit(&k, &env);
        assert!(first.accepted);
        assert_eq!(first.class, MessageClass::Settlement);
        assert_eq!(first.recipients, vec![keys(121).did().as_str().to_string()]);
        assert_eq!(router.stats().admitted, 1);
        assert_eq!(router.stats().direct, 1);

        // 重放：同一个 id 第二次必须被拒，且按竞争（conflict）而不是恶意。
        let second = router.admit(&k, &env);
        assert!(!second.accepted);
        assert_eq!(second.code, Some(RefusalCode::Conflict));
        assert!(!RefusalCode::Conflict.is_misconduct());
        assert_eq!(router.stats().replays, 1);
        assert!(router.has_seen(&env.id));
    }

    #[test]
    fn future_timestamp_cannot_poison_the_clock() {
        // P0 回归：修复前只查"落后"不查"超前"，于是 `ts = u64::MAX` 的合法信封能通过准入，
        // 而 `Kernel::send` 会把本地逻辑时钟取 max 顶到天花板 →
        // 之后所有正常信封都被判 stale_epoch → **远端一条消息即可让节点永久拒收**。
        let mut k = kernel_with(2);
        let mut router = PmbRouter::new();
        let poisoned = settle_request(
            &keys(120),
            keys(121).did(),
            u64::MAX,
            Credits(7),
            EvidenceGrade::CpuProto,
        )
        .unwrap();

        let decision = router.admit(&k, &poisoned);
        assert!(!decision.accepted, "超前信封必须被拒");
        assert_eq!(decision.code, Some(RefusalCode::StaleEpoch));
        assert!(
            !RefusalCode::StaleEpoch.is_misconduct(),
            "超前是竞争语义，不是单次即成立的恶意"
        );

        // 关键：被拒之后，正常信封仍然可以准入（时钟没被顶走）。
        let normal = settle_request(
            &keys(120),
            keys(121).did(),
            k.now(),
            Credits(7),
            EvidenceGrade::CpuProto,
        )
        .unwrap();
        assert!(
            router.admit(&k, &normal).accepted,
            "被拒的超前信封不得影响后续正常投递"
        );

        // 内核层纵深防御：即使绕过路由直接 send，时钟也只能前进 MAX_LAG 步。
        let before = k.now();
        let _ = k.send(&poisoned);
        assert!(
            k.now() <= before.saturating_add(MAX_LAG),
            "时钟不得被单条远端信封推进超过 MAX_LAG（{before} -> {}）",
            k.now()
        );
    }

    #[test]
    fn replay_table_is_bounded_by_the_lag_horizon() {
        // P1 回归：修复前 `seen` 只增不减——长跑节点会被"大量合法且各不相同的消息"撑爆内存。
        let mut k = kernel_with(2);
        let mut router = PmbRouter::new();
        // 要触发回收，时钟必须越过 MAX_LAG 的滞后视野：投递 100 条 ts 递增的信封，
        // 每条都把逻辑时钟推进一步。
        for ts in 0..100u64 {
            let env = settle_request(
                &keys(120),
                keys(121).did(),
                ts,
                Credits(7),
                EvidenceGrade::CpuProto,
            )
            .unwrap();
            let _ = k.send(&env); // 推进逻辑时钟（有界）
            let _ = router.admit(&k, &env); // 记录 id
        }
        let seen = router.seen_count();
        assert!(
            seen < 100,
            "重放表必须随滞后视野回收，实际保留 {seen} 条 / 共投递 100 条"
        );
        assert!(
            seen as u64 <= MAX_LAG + 4,
            "保留量应与 MAX_LAG 同量级，实际 {seen}"
        );
        // 回收不等于失忆：视野内的重复投递仍然会被判重放。
        let fresh = settle_request(
            &keys(120),
            keys(121).did(),
            k.now(),
            Credits(7),
            EvidenceGrade::CpuProto,
        )
        .unwrap();
        assert!(router.admit(&k, &fresh).accepted);
        let again = router.admit(&k, &fresh);
        assert!(!again.accepted);
        assert_eq!(again.code, Some(RefusalCode::Conflict));
    }

    #[test]
    fn forged_or_unknown_senders_are_misconduct() {
        let k = kernel_with(1);
        let mut router = PmbRouter::new();

        // 篡改 body → 验签失败 → unauthorized（恶意）。
        let mut tampered = progress(&keys(120), None, 0, "ready", json!({"p": 1})).unwrap();
        tampered.body = json!({"p": 2});
        let decision = router.admit(&k, &tampered);
        assert!(!decision.accepted);
        assert_eq!(decision.code, Some(RefusalCode::Unauthorized));
        assert!(decision.code.map(|c| c.is_misconduct()).unwrap_or(false));

        // 未在册的发送者 → unauthorized（恶意）。
        let outsider = progress(&keys(200), None, 0, "hello", json!({})).unwrap();
        let decision = router.admit(&k, &outsider);
        assert_eq!(decision.code, Some(RefusalCode::Unauthorized));
        assert!(router.stats().refused >= 2);
    }

    #[test]
    fn competition_failures_stay_competitive() {
        let k = kernel_with(2);
        let mut router = PmbRouter::new();

        // 未知类型 → unsupported。
        let unknown = Envelope::new(
            keys(120).did(),
            Some(keys(121).did()),
            "made.up.kind",
            0,
            None,
            json!({}),
        )
        .unwrap()
        .seal(&keys(120))
        .unwrap();
        let decision = router.admit(&k, &unknown);
        assert_eq!(decision.code, Some(RefusalCode::Unsupported));
        assert!(!decision.code.map(|c| c.is_misconduct()).unwrap_or(true));

        // 收件人不在册 → stale_epoch。
        let ghost = progress(&keys(120), Some(keys(220).did()), 0, "x", json!({})).unwrap();
        let decision = router.admit(&k, &ghost);
        assert_eq!(decision.code, Some(RefusalCode::StaleEpoch));
        assert!(!decision.code.map(|c| c.is_misconduct()).unwrap_or(true));

        // 时钟落后 → stale_epoch（内核时钟被推高）。
        let mut k = kernel_with(2);
        k.clock_advance_for_test(500);
        let stale = progress(&keys(120), None, 0, "late", json!({})).unwrap();
        let decision = router.admit(&k, &stale);
        assert_eq!(decision.code, Some(RefusalCode::StaleEpoch));

        // 治理消息广播 → policy_denied。
        let motion = Envelope::new(
            keys(120).did(),
            None,
            kinds_ext::COUNCIL_MOTION,
            500,
            None,
            json!({"motion": "m1"}),
        )
        .unwrap()
        .seal(&keys(120))
        .unwrap();
        let decision = router.admit(&k, &motion);
        assert_eq!(decision.code, Some(RefusalCode::PolicyDenied));
        assert!(!decision.code.map(|c| c.is_misconduct()).unwrap_or(true));
    }

    #[test]
    fn broadcasts_expand_to_every_registered_peer_but_the_sender() {
        let k = kernel_with(4);
        let mut router = PmbRouter::new();
        let env = progress(&keys(120), None, 0, "heartbeat", json!({"ok": true})).unwrap();
        let decision = router.admit(&k, &env);
        assert!(decision.accepted);
        assert_eq!(decision.recipients.len(), 3);
        assert!(!decision
            .recipients
            .contains(&keys(120).did().as_str().to_string()));
        assert_eq!(router.stats().broadcasts, 1);
        assert_eq!(router.stats().by_class.get("telemetry").copied(), Some(1));
    }

    #[test]
    fn wire_format_roundtrips_through_the_frozen_frame() {
        let card = AgentCard {
            did: keys(120).did(),
            display: "announcer".to_string(),
            skills: vec!["skill.a".to_string()],
            stake: Credits(20),
            evidence: EvidenceGrade::Verified,
        };
        let env = announce_card(&keys(120), &card, 3).unwrap();
        let frame = encode_checked(&env).unwrap();
        assert!(frame.len() > 4);
        let back = decode_and_verify(&frame).unwrap();
        assert_eq!(back, env);
        assert_eq!(back.id, env.id);

        // 截断帧与超限帧被基元层拒绝。
        assert_eq!(
            decode_and_verify(&frame[..2]),
            Err(CoreError::FrameTruncated)
        );
        let mut oversized = Vec::from((MAX_FRAME as u32 + 1).to_be_bytes());
        oversized.extend_from_slice(b"{}");
        assert_eq!(decode_and_verify(&oversized), Err(CoreError::FrameTooLarge));
    }

    #[test]
    fn routing_is_deterministic_and_stats_add_up() {
        let run = || {
            let mut k = kernel_with(3);
            let mut router = PmbRouter::new();
            let decisions = vec![
                router.admit(&k, &progress(&keys(120), None, 0, "a", json!({})).unwrap()),
                router.admit(
                    &k,
                    &settle_request(
                        &keys(121),
                        keys(120).did(),
                        0,
                        Credits(3),
                        EvidenceGrade::Verified,
                    )
                    .unwrap(),
                ),
                router.admit(
                    &k,
                    &council_vote(&keys(122), keys(121).did(), 0, "m1", "uphold").unwrap(),
                ),
            ];
            k.tick();
            (decisions, router.stats().clone())
        };
        let a = run();
        let b = run();
        assert_eq!(a, b);
        assert_eq!(
            a.1.admitted + a.1.refused,
            a.0.len() as u64,
            "每个判定都要落进统计"
        );
        assert_eq!(a.1.direct + a.1.broadcasts, a.1.admitted);
    }
}
