//! v1.1.3 —— 广播协议。
//!
//! 能力通告走的是 PMB（`au4a-core::msg`）——全系统唯一的 Agent-to-Agent 信道。
//! 关键性质（不是「设计意图」，是测试里断言的东西）：
//!
//! 1. **先签名后发送**：`Envelope::seal` 用发送方私钥签规范 JSON 字节，`verify()` 逐字节重算。
//!    改动 body 里任何一个数字 → `InvalidSignature`。
//! 2. **两层签名**：信封签名绑定「谁在这条信道上说了话」，内层声明签名绑定
//!    「这份能力清单是谁说的」。两层都验，且必须指向同一个 Agent——
//!    否则就是有人替别人做了陈述，这属于身份不成立（`unauthorized`）。
//! 3. **广播 = `to: None`**：与 `Kernel` 的投递语义一致，人类观察层是旁路订阅，不参与投递。
//! 4. **收到什么就判什么**：信封层的失败（未签名/篡改/类型不符）与声明层的失败
//!    （版本陈旧、内容冲突）都返回**类型化拒绝**，由内核记录、由观察层只读展示。
//!
//! 诚实边界：本 crate 不做网络 I/O（轨道契约禁止），因此「投递」在本 crate 内是
//! 共享 `Kernel` 队列上的本地传递。签名、验签、分帧、内容寻址全部是真实的；
//! 跨进程/跨机器的真实传输属于 `au4a-node` 与轨道 1.9，标注为 cpu-proto。

use au4a_core::{CoreError, CoreResult, Did, Envelope, RefusalCode};
use serde_json::{json, Value};

use crate::capability::SkillId;
use crate::declaration::{Declaration, SignedDeclaration};
use crate::graph::{AgentCapabilityGraph, DeclareOutcome};

/// 通告协议版本。跨版本不兼容时靠它拒绝，而不是靠字段猜测。
pub const PROTOCOL: &str = "au4a.capgraph/1";
/// 广播能力通告的消息类型。
pub const KIND_ANNOUNCE: &str = "capgraph.announce";
/// 单播询问某个技能的消息类型。
pub const KIND_REQUEST: &str = "capgraph.request";

/// 一条能力通告的语义（解析后的形状，不是 wire 形状）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    pub from: Did,
    pub declaration: Declaration,
    /// 声明内容指纹（不含签名）。
    pub fingerprint: String,
}

/// 信封层的接收结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ingest {
    /// 信封与声明都通过，结果由图给出（Applied / Unchanged / Rejected）。
    Routed {
        about: Did,
        outcome: DeclareOutcome,
    },
    /// 在信封层就被拒：未签名、被篡改、类型不符、身份不符、协议版本不符。
    Refused {
        from: Did,
        code: RefusalCode,
        reason: String,
    },
}

impl Ingest {
    fn refused(from: &Did, code: RefusalCode, reason: impl Into<String>) -> Self {
        Self::Refused {
            from: from.clone(),
            code,
            reason: reason.into(),
        }
    }

    pub fn is_routed(&self) -> bool {
        matches!(self, Self::Routed { .. })
    }

    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Routed { outcome, .. } if outcome.is_applied())
    }

    /// 任何一层拒绝都归一到一个类型化拒绝码。
    pub fn refusal(&self) -> Option<RefusalCode> {
        match self {
            Self::Refused { code, .. } => Some(*code),
            Self::Routed { outcome, .. } => outcome.refusal(),
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Self::Routed { about, outcome } => json!({
                "layer": "graph",
                "about": about.as_str(),
                "outcome": outcome.to_value(),
            }),
            Self::Refused { from, code, reason } => json!({
                "layer": "envelope",
                "from": from.as_str(),
                "code": code.as_str(),
                "reason": reason,
            }),
        }
    }
}

/// 构造一条广播能力通告（`to: None`，先签名后发送）。
///
/// 只能通告自己的声明：`Declaration::sign` 在 `agent != keys.did()` 时返回 `InvalidSignature`。
pub fn announce(keys: &au4a_core::AgentKeys, declaration: &Declaration, ts: u64) -> CoreResult<Envelope> {
    let signed = declaration.clone().sign(keys)?;
    Envelope::new(
        keys.did(),
        None,
        KIND_ANNOUNCE,
        ts,
        None,
        announce_body(&signed)?,
    )?
    .seal(keys)
}

/// 构造一条单播能力通告（回应询问，或定向同步）。
pub fn announce_to(
    keys: &au4a_core::AgentKeys,
    declaration: &Declaration,
    to: Did,
    ts: u64,
    in_reply_to: Option<String>,
) -> CoreResult<Envelope> {
    let signed = declaration.clone().sign(keys)?;
    Envelope::new(
        keys.did(),
        Some(to),
        KIND_ANNOUNCE,
        ts,
        in_reply_to,
        announce_body(&signed)?,
    )?
    .seal(keys)
}

fn announce_body(signed: &SignedDeclaration) -> CoreResult<Value> {
    Ok(json!({
        "protocol": PROTOCOL,
        "declaration": signed.to_value()?,
    }))
}

/// 构造一条「谁提供技能 X」的询问（广播）。
pub fn query_skill(
    keys: &au4a_core::AgentKeys,
    skill: &SkillId,
    ts: u64,
    in_reply_to: Option<String>,
) -> CoreResult<Envelope> {
    Envelope::new(
        keys.did(),
        None,
        KIND_REQUEST,
        ts,
        in_reply_to,
        json!({"protocol": PROTOCOL, "skill": skill.as_str()}),
    )?
    .seal(keys)
}

/// 解析并校验一条能力通告信封。**这是唯一进入图的门**。
pub fn parse_announcement(env: &Envelope) -> Result<Announcement, (RefusalCode, String)> {
    // 1) 信封签名与内容寻址 id（篡改在这里被抓住）。
    if let Err(err) = env.verify() {
        let code = match err {
            CoreError::NotSealed | CoreError::InvalidSignature => RefusalCode::Unauthorized,
            _ => RefusalCode::Malformed,
        };
        return Err((code, format!("envelope rejected: {err}")));
    }
    // 2) 消息类型。
    if env.kind.as_str() != KIND_ANNOUNCE {
        return Err((
            RefusalCode::Unsupported,
            format!("kind {} is not {KIND_ANNOUNCE}", env.kind.as_str()),
        ));
    }
    // 3) 协议版本。
    let protocol = env.body.get("protocol").and_then(Value::as_str);
    if protocol != Some(PROTOCOL) {
        return Err((
            RefusalCode::Unsupported,
            format!("protocol {:?} != {PROTOCOL}", protocol),
        ));
    }
    // 4) 内层声明签名。
    let payload = env
        .body
        .get("declaration")
        .cloned()
        .ok_or((RefusalCode::Malformed, "missing declaration".to_string()))?;
    let signed = SignedDeclaration::from_value(&payload)
        .map_err(|err| (RefusalCode::Malformed, format!("declaration encoding: {err}")))?;
    signed
        .verify()
        .map_err(|err| (RefusalCode::Unauthorized, format!("declaration signature: {err}")))?;
    // 5) 信封发送者必须就是声明者：信道上的身份与陈述的身份必须一致。
    if signed.declaration.agent != env.from {
        return Err((
            RefusalCode::Unauthorized,
            format!(
                "envelope from {} carries a declaration by {}",
                au4a_core::short_id(env.from.as_str()),
                au4a_core::short_id(signed.declaration.agent.as_str())
            ),
        ));
    }
    let fingerprint = signed
        .content_fingerprint()
        .map_err(|err| (RefusalCode::Malformed, format!("fingerprint: {err}")))?;
    Ok(Announcement {
        from: env.from.clone(),
        declaration: signed.declaration,
        fingerprint,
    })
}

/// 解析一条技能询问，返回（询问者，技能名）。
pub fn parse_query(env: &Envelope) -> Result<(Did, SkillId), (RefusalCode, String)> {
    if let Err(err) = env.verify() {
        let code = match err {
            CoreError::NotSealed | CoreError::InvalidSignature => RefusalCode::Unauthorized,
            _ => RefusalCode::Malformed,
        };
        return Err((code, format!("envelope rejected: {err}")));
    }
    if env.kind.as_str() != KIND_REQUEST {
        return Err((
            RefusalCode::Unsupported,
            format!("kind {} is not {KIND_REQUEST}", env.kind.as_str()),
        ));
    }
    if env.body.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
        return Err((RefusalCode::Unsupported, "protocol mismatch".to_string()));
    }
    let raw = env
        .body
        .get("skill")
        .and_then(Value::as_str)
        .ok_or((RefusalCode::Malformed, "missing skill".to_string()))?;
    let skill = SkillId::new(raw).map_err(|err| (RefusalCode::Malformed, format!("skill: {err}")))?;
    Ok((env.from.clone(), skill))
}

/// 把一条通告信封交给图：先解析（信封层），再应用（图声明层）。
///
/// 注意这里**不做**版本比较——那是图自己的职责（v1.1.7 起有完整语义），
/// 广播层只负责「这条消息在信道与身份上是否成立」。
pub fn ingest(graph: &mut AgentCapabilityGraph, env: &Envelope, now: u64) -> Ingest {
    match parse_announcement(env) {
        Ok(announcement) => {
            let about = announcement.declaration.agent.clone();
            let outcome = graph.apply(
                &SignedDeclaration {
                    declaration: announcement.declaration,
                    sig: env
                        .body
                        .get("declaration")
                        .and_then(|d| d.get("sig"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
                now,
            );
            Ingest::Routed { about, outcome }
        }
        Err((code, reason)) => Ingest::refused(&env.from, code, reason),
    }
}

/// 一次 `pump` 的结果：谁被路由、多少条被转发、多少条被拒。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PumpReport {
    /// 交给本图处理的通告条数。
    pub routed: usize,
    /// 其中真正改变了图的条数。
    pub applied: usize,
    /// 被拒的条数（信封层或声明层）。
    pub refused: usize,
    /// 不属于本轨道的信封：**转发回队列**，不吞掉别人的消息。
    pub forwarded: usize,
    /// 转发失败（例如发送方未注册）而丢弃的条数。
    pub dropped: usize,
}

impl PumpReport {
    pub fn to_value(&self) -> Value {
        json!({
            "routed": self.routed,
            "applied": self.applied,
            "refused": self.refused,
            "forwarded": self.forwarded,
            "dropped": self.dropped,
        })
    }
}

/// 处理内核队列里当前的全部信封。
///
/// 只有 `capgraph.*` 类型的信封会被本图消费；其余信封**原样转发回队列**
/// （重新 `send` 同一个已签名信封，id 不变），因为一个能力图没有资格吞掉别的轨道的消息。
/// 每一条拒绝都同时写进内核的拒绝记录：这样观察层看到的「谁因为什么被拒」是完整的。
pub fn pump(kernel: &mut au4a_kernel::Kernel, graph: &mut AgentCapabilityGraph, now: u64) -> PumpReport {
    let inbox = kernel.drain();
    let mut report = PumpReport::default();
    for env in inbox {
        if !env.kind.as_str().starts_with("capgraph.") {
            match kernel.send(&env) {
                Ok(_) => report.forwarded += 1,
                Err(_) => report.dropped += 1,
            }
            continue;
        }
        report.routed += 1;
        let outcome = ingest(graph, &env, now);
        if outcome.is_applied() {
            report.applied += 1;
        }
        if let Some(code) = outcome.refusal() {
            report.refused += 1;
            let reason = match &outcome {
                Ingest::Refused { reason, .. } => reason.clone(),
                Ingest::Routed { outcome, .. } => outcome.to_value().to_string(),
            };
            kernel.refuse(&env.from, code, reason);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;
    use crate::graph::CapGraphConfig;
    use au4a_core::{kinds, AgentKeys, Credits};
    use au4a_kernel::{Kernel, KernelConfig};

    fn keys(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn declaration(keys: &AgentKeys, epoch: u64, skills: &[&str]) -> Declaration {
        let caps: Vec<Capability> = skills
            .iter()
            .map(|s| Capability::new(SkillId::new(s).expect("valid"), Credits(3)))
            .collect();
        Declaration::new(keys.did(), epoch, epoch * 10, caps).expect("coherent")
    }

    #[test]
    fn an_announcement_round_trips_through_the_wire() {
        let a = keys(1);
        let env = announce(&a, &declaration(&a, 1, &["translate.en-zh"]), 7).expect("announce");
        env.verify().expect("sealed");
        assert_eq!(env.to, None, "能力通告是广播");
        assert_eq!(env.kind.as_str(), KIND_ANNOUNCE);
        let frame = au4a_core::encode_frame(&env).expect("frames");
        let decoded = au4a_core::decode_frame(&frame).expect("decodes");
        assert_eq!(decoded, env);
        let parsed = parse_announcement(&decoded).expect("parses");
        assert_eq!(parsed.from, a.did());
        assert_eq!(parsed.declaration.capabilities.len(), 1);
    }

    #[test]
    fn tampering_with_the_announcement_is_refused() {
        let a = keys(2);
        let mut env = announce(&a, &declaration(&a, 1, &["x"]), 3).expect("announce");
        env.body["declaration"]["declaration"]["capabilities"][0]["price_per_unit"] =
            json!(0);
        let (code, _) = parse_announcement(&env).expect_err("tampered");
        assert_eq!(code, RefusalCode::Unauthorized);
        assert!(code.is_misconduct());
    }

    #[test]
    fn a_declaration_smuggled_under_another_sender_is_refused() {
        let a = keys(3);
        let b = keys(4);
        // B 把 A 的合法声明放进自己的信封：内容是真的，但信道身份与陈述身份不一致。
        let body = announce_body(
            &declaration(&a, 1, &["x"])
                .sign(&a)
                .expect("A signs"),
        )
        .expect("body");
        let env = Envelope::new(b.did(), None, KIND_ANNOUNCE, 1, None, body)
            .expect("envelope")
            .seal(&b)
            .expect("sealed");
        env.verify().expect("envelope signature is genuinely B's");
        let (code, reason) = parse_announcement(&env).expect_err("identity mismatch");
        assert_eq!(code, RefusalCode::Unauthorized);
        assert!(reason.contains("carries a declaration by"), "{reason}");
    }

    #[test]
    fn wrong_kind_and_wrong_protocol_are_refused_without_touching_the_graph() {
        let a = keys(5);
        let b = keys(6);
        let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        let wrong_kind = Envelope::new(
            b.did(),
            None,
            kinds::AGENT_CARD,
            1,
            None,
            json!({"protocol": PROTOCOL}),
        )
        .expect("envelope")
        .seal(&b)
        .expect("sealed");
        let outcome = ingest(&mut graph, &wrong_kind, 0);
        assert_eq!(outcome.refusal(), Some(RefusalCode::Unsupported));
        assert_eq!(graph.neighbor_count(), 0);

        let wrong_protocol = Envelope::new(
            b.did(),
            None,
            KIND_ANNOUNCE,
            1,
            None,
            json!({"protocol": "au4a.capgraph/0"}),
        )
        .expect("envelope")
        .seal(&b)
        .expect("sealed");
        assert_eq!(
            ingest(&mut graph, &wrong_protocol, 0).refusal(),
            Some(RefusalCode::Unsupported)
        );
    }

    #[test]
    fn pump_delivers_announcements_and_records_refusals_in_the_kernel() {
        let a = keys(7);
        let b = keys(8);
        let mut kernel = Kernel::new(KernelConfig::default());
        kernel
            .register(&a, "alice", &["x"], Credits(20))
            .expect("register a");
        kernel
            .register(&b, "bob", &["x"], Credits(20))
            .expect("register b");

        let good = announce(&b, &declaration(&b, 1, &["sentiment.analyze"]), 1).expect("announce");
        let mut bad = announce(&b, &declaration(&b, 2, &["x"]), 2).expect("announce");
        bad.body["declaration"]["declaration"]["epoch"] = json!(99);
        kernel.send(&good).expect("accepted");
        assert!(kernel.send(&bad).is_err(), "篡改信封连内核都进不去");

        let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        let report = pump(&mut kernel, &mut graph, 1);
        assert_eq!(report.routed, 1);
        assert_eq!(report.applied, 1);
        assert_eq!(report.refused, 0);
        assert_eq!(graph.neighbor_count(), 1);
        assert_eq!(graph.capability_count(), 1);
    }

    #[test]
    fn pump_forwards_envelopes_belonging_to_other_tracks() {
        let a = keys(9);
        let b = keys(10);
        let mut kernel = Kernel::new(KernelConfig::default());
        kernel.register(&a, "a", &["x"], Credits(20)).expect("register");
        kernel.register(&b, "b", &["y"], Credits(20)).expect("register");
        let foreign = Envelope::new(b.did(), None, kinds::AGENT_CARD, 1, None, json!({"hi": 1}))
            .expect("envelope")
            .seal(&b)
            .expect("sealed");
        kernel.send(&foreign).expect("accepted");

        let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        let report = pump(&mut kernel, &mut graph, 1);
        assert_eq!(report.forwarded, 1);
        assert_eq!(report.routed, 0);
        assert_eq!(kernel.queue_len(), 1, "别人的消息必须还在队列里");
        let left = kernel.drain();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, foreign.id, "转发不改变内容寻址 id");
    }

    #[test]
    fn a_broadcast_query_can_only_be_parsed_with_the_right_kind() {
        let a = keys(11);
        let skill = SkillId::new("translate.en-zh").expect("valid");
        let env = query_skill(&a, &skill, 4, None).expect("query");
        let (from, parsed) = parse_query(&env).expect("parses");
        assert_eq!(from, a.did());
        assert_eq!(parsed, skill);
        let announce_env = announce(&a, &declaration(&a, 1, &["x"]), 5).expect("announce");
        assert_eq!(
            parse_query(&announce_env).expect_err("wrong kind").0,
            RefusalCode::Unsupported
        );
    }
}
