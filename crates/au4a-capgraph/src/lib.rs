//! AU4A 轨道 1.1 — Capability Graph 能力图（v1.1.1 → v1.1.10）。
//!
//! 能力图回答一个所有多 Agent 系统都必须回答、但参考项目都跳过了的问题：
//! **「谁会做什么、多快、多可靠、多少钱、能接上谁」是不是一份 Agent 自己能读写的数据？**
//! 参考仓库里能力是一张扁平的表（`nau-plugin` 的 21 个 `Capability`）或一段自我介绍
//! （`AgentCard.skills`），没有能力到能力的边，没有衰减，没有格式兼容性，没有代价。
//! 这条轨道把「能力」做成一张**可广播、可缓存、可索引、可规划路径、可版本化**的图。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责，并留下自检与证据。
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）。
//!
//! 版本地图：
//!
//! | 版本 | 职责 | 模块 |
//! |---|---|---|
//! | v1.1.1 | 数据结构（整数度量 + 硬约束） | [`capability`] |
//! | v1.1.2 | 声明 API（自签声明，不能替他人声明） | 规划中 |
//! | v1.1.3 | 广播协议（PMB `Envelope` + 真验签） | 规划中 |
//! | v1.1.4 | 邻居缓存（容量上限 + 失效） | 规划中 |
//! | v1.1.5 | 查询接口（倒排索引，不全图扫描） | 规划中 |
//! | v1.1.6 | 路径规划（图搜索 + 格式兼容 + 代价最小） | 规划中 |
//! | v1.1.7 | 版本化（变更自动递增 + 陈旧/冲突检测） | 规划中 |
//! | v1.1.8 | 性能优化（增量索引 + 有界 top-k + 查询缓存） | 规划中 |
//! | v1.1.9 | 测试（端到端 + 不变式 + 对抗用例） | `tests/` |
//! | v1.1.10 | 文档与证据 | `docs/tracks/1.1.md` |

pub mod broadcast;
pub mod capability;
pub mod declaration;
pub mod graph;

pub use broadcast::{
    announce, announce_to, ingest, parse_announcement, parse_query, pump, query_skill,
    Announcement, Ingest, PumpReport, KIND_ANNOUNCE, KIND_REQUEST, PROTOCOL,
};
pub use capability::{
    Capability, Constraints, FormatId, SkillId, BP_SCALE, MAX_FORMAT_LEN, MAX_SKILL_LEN,
};
pub use declaration::{Declaration, SignedDeclaration};
pub use graph::{AgentCapabilityGraph, CapGraphConfig, DeclareOutcome, NeighborRecord};

use au4a_core::{AgentKeys, CoreResult, Credits, SelfCheck};
use serde_json::{json, Value};

/// 轨道号。
pub const TRACK: &str = "1.1";
/// 轨道标题。
pub const TITLE: &str = "Capability Graph 能力图";
/// 版本区间。
pub const RANGE: &str = "v1.1.1 → v1.1.10";
/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_capgraph";

/// 把一次检查的 `Result` 变成自检项：成功给出**具体断言了什么**，失败给出原因。
fn verdict(name: &str, check: impl FnOnce() -> Result<String, String>) -> SelfCheck {
    match check() {
        Ok(detail) => SelfCheck::pass(TRACK, name, detail),
        Err(detail) => SelfCheck::fail(TRACK, name, detail),
    }
}

/// 把任意错误渲染成自检的失败说明。
fn show<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

/// v1.1.1：度量是整数、校验会拒绝、内容可寻址、格式可接续。
fn checks_v111() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.1.integer_metrics", || {
        let cap = Capability::new(SkillId::new("translate.en-zh").map_err(show)?, Credits(7));
        let canonical = au4a_core::canonicalize(&cap.to_value().map_err(show)?).map_err(show)?;
        if canonical.contains('.') {
            return Err(format!("规范 JSON 出现小数字符：{canonical}"));
        }
        Ok(format!("9 个度量字段全部整数，规范 JSON 无小数点：{canonical}"))
    }));
    checks.push(verdict("1.1.1.validation_rejects", || {
        let base = Capability::new(SkillId::new("x").map_err(show)?, Credits(1));
        let bad_latency = base.clone().with_latency(500, 100).validate().is_err();
        let bad_load = base.clone().with_load_bp(10_001).validate().is_err();
        let bad_reliability = base.clone().with_reliability_bp(10_001).validate().is_err();
        if !(bad_latency && bad_load && bad_reliability) {
            return Err("p99<p50 / 负载>10000 / 可靠度>10000 中至少一个未被拒绝".into());
        }
        Ok("断言 p99<p50、load>10000bp、reliability>10000bp 三类不合法能力均返回 Err".into())
    }));
    checks.push(verdict("1.1.1.load_weighted_math", || {
        let idle = Capability::new(SkillId::new("x").map_err(show)?, Credits(1))
            .with_latency(100, 300)
            .with_load_bp(5_000);
        let latency = idle.effective_latency_ms();
        let reliability = idle.effective_reliability_bp();
        if latency != 200 || reliability != 4_500 {
            return Err(format!("期望 200ms/4500bp，实测 {latency}ms/{reliability}bp"));
        }
        Ok("断言 p50=100,p99=300,负载=5000bp 时加权延迟=200ms、加权可靠度=4500bp（整数运算）".into())
    }));
    checks.push(verdict("1.1.1.content_addressed", || {
        let a = Capability::new(SkillId::new("x").map_err(show)?, Credits(5));
        let b = Capability::new(SkillId::new("x").map_err(show)?, Credits(5));
        let c = a.clone().with_price(Credits(6));
        if a.fingerprint().map_err(show)? != b.fingerprint().map_err(show)? {
            return Err("相同能力得到不同指纹".into());
        }
        if a.fingerprint().map_err(show)? == c.fingerprint().map_err(show)? {
            return Err("改价后指纹未变化".into());
        }
        Ok("断言同能力同指纹、改价后指纹改变（SHA-256 内容寻址）".into())
    }));
    checks.push(verdict("1.1.1.format_handoff", || {
        let producer = Capability::new(SkillId::new("producer").map_err(show)?, Credits(1))
            .with_formats(&["text/plain"], &["text/plain", "application/json"])
            .map_err(show)?;
        let consumer = Capability::new(SkillId::new("consumer").map_err(show)?, Credits(1))
            .with_formats(&["application/json"], &["application/json"])
            .map_err(show)?;
        let audio = Capability::new(SkillId::new("audio").map_err(show)?, Credits(1))
            .with_formats(&["audio/wav"], &["audio/wav"])
            .map_err(show)?;
        let join = producer.handoff_format(&consumer).map(|f| f.as_str().to_string());
        if join.as_deref() != Some("application/json") || producer.handoff_format(&audio).is_some() {
            return Err(format!("格式接续判断错误：join={join:?}"));
        }
        Ok("断言产出∩接受={application/json} 可接续；产出∩接受=∅ 不可接续".into())
    }));
    checks
}

/// v1.1.2：声明必须自签、内容必须自洽、拒绝必须有类型、重放必须幂等。
fn checks_v112() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.2.self_signed_only", || {
        let a = AgentKeys::from_seed(&[21; 32]);
        let b = AgentKeys::from_seed(&[22; 32]);
        let declaration = Declaration::new(a.did(), 1, 1, vec![sample_capability()?]).map_err(show)?;
        let forged = declaration.clone().sign(&b);
        if forged != Err(au4a_core::CoreError::InvalidSignature) {
            return Err("用 B 的私钥替 A 声明竟然成功了".into());
        }
        let honest = declaration.sign(&a).map_err(show)?;
        honest.verify().map_err(show)?;
        Ok("断言：替他人声明返回 InvalidSignature；自签声明验签通过".into())
    }));
    checks.push(verdict("1.1.2.tampering_is_unauthorized", || {
        let a = AgentKeys::from_seed(&[23; 32]);
        let b = AgentKeys::from_seed(&[24; 32]);
        let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        let signed = Declaration::new(b.did(), 1, 1, vec![sample_capability()?])
            .map_err(show)?
            .sign(&b)
            .map_err(show)?;
        let mut value = signed.to_value().map_err(show)?;
        value["declaration"]["capabilities"][0]["throughput_per_min"] = json!(1);
        let tampered = SignedDeclaration::from_value(&value).map_err(show)?;
        let outcome = graph.apply(&tampered, 1);
        if outcome.refusal() != Some(au4a_core::RefusalCode::Unauthorized) || graph.neighbor_count() != 0
        {
            return Err(format!("篡改未被判 unauthorized：{:?}", outcome.to_value()));
        }
        Ok("断言：改动签名后的声明被判 unauthorized，且邻居视图保持为空".into())
    }));
    checks.push(verdict("1.1.2.epoch_monotonic", || {
        let a = AgentKeys::from_seed(&[25; 32]);
        let b = AgentKeys::from_seed(&[26; 32]);
        let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        let mut declare = |epoch: u64, skill: &str| -> Result<DeclareOutcome, String> {
            let cap = Capability::new(SkillId::new(skill).map_err(show)?, Credits(3));
            let signed = Declaration::new(b.did(), epoch, epoch, vec![cap])
                .map_err(show)?
                .sign(&b)
                .map_err(show)?;
            Ok(graph.apply(&signed, epoch))
        };
        let applied = declare(5, "translate.en-zh")?;
        let stale = declare(4, "translate.en-zh")?;
        let conflict = declare(5, "sentiment.analyze")?;
        if !applied.is_applied()
            || stale.refusal() != Some(au4a_core::RefusalCode::StaleEpoch)
            || conflict.refusal() != Some(au4a_core::RefusalCode::Conflict)
        {
            return Err("陈旧/冲突检测不正确".into());
        }
        Ok("断言：epoch 回退→stale_epoch；同 epoch 异内容→conflict；图停留在 epoch=5".into())
    }));
    checks
}

/// v1.1.3：通告走 PMB、两层签名都验、篡改与身份错配被拒、别人的消息不被吞。
fn checks_v113() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.3.announce_seals_and_verifies", || {
        let a = AgentKeys::from_seed(&[31; 32]);
        let declaration = Declaration::new(a.did(), 1, 1, vec![sample_capability()?]).map_err(show)?;
        let env = announce(&a, &declaration, 7).map_err(show)?;
        env.verify().map_err(show)?;
        let frame = au4a_core::encode_frame(&env).map_err(show)?;
        let decoded = au4a_core::decode_frame(&frame).map_err(show)?;
        if decoded != env || env.to.is_some() || env.kind.as_str() != KIND_ANNOUNCE {
            return Err("信封形状不是广播，或分帧往返不一致".into());
        }
        let parsed = parse_announcement(&decoded).map_err(|(_, r)| r)?;
        Ok(format!(
            "断言：{KIND_ANNOUNCE} 信封验签通过、to=None、4 字节大端分帧往返一致，解析出 {} 条能力",
            parsed.declaration.capabilities.len()
        ))
    }));
    checks.push(verdict("1.1.3.tamper_is_unauthorized", || {
        let a = AgentKeys::from_seed(&[32; 32]);
        let mut env = announce(
            &a,
            &Declaration::new(a.did(), 1, 1, vec![sample_capability()?]).map_err(show)?,
            3,
        )
        .map_err(show)?;
        env.body["declaration"]["declaration"]["capabilities"][0]["price_per_unit"] = json!(0);
        match parse_announcement(&env) {
            Err((code, _)) if code == au4a_core::RefusalCode::Unauthorized => {
                Ok("断言：改动 body 后信封验签失败，判 unauthorized（一次即恶意，不可重试）".into())
            }
            other => Err(format!("篡改未被判 unauthorized：{other:?}")),
        }
    }));
    checks.push(verdict("1.1.3.identity_mismatch_refused", || {
        let a = AgentKeys::from_seed(&[33; 32]);
        let b = AgentKeys::from_seed(&[34; 32]);
        let signed = Declaration::new(a.did(), 1, 1, vec![sample_capability()?])
            .map_err(show)?
            .sign(&a)
            .map_err(show)?;
        let body = json!({"protocol": PROTOCOL, "declaration": signed.to_value().map_err(show)?});
        let env = au4a_core::Envelope::new(b.did(), None, KIND_ANNOUNCE, 1, None, body)
            .map_err(show)?
            .seal(&b)
            .map_err(show)?;
        match parse_announcement(&env) {
            Err((code, _)) if code == au4a_core::RefusalCode::Unauthorized => {
                Ok("断言：B 的信封携带 A 的合法声明 → 两层签名都对但身份错配，判 unauthorized".into())
            }
            other => Err(format!("身份错配未被拒：{other:?}")),
        }
    }));
    checks.push(verdict("1.1.3.pump_routes_and_forwards", || {
        let a = AgentKeys::from_seed(&[35; 32]);
        let b = AgentKeys::from_seed(&[36; 32]);
        let mut kernel = au4a_kernel::Kernel::new(au4a_kernel::KernelConfig::default());
        kernel
            .register(&a, "alice", &["x"], Credits(20))
            .map_err(show)?;
        kernel
            .register(&b, "bob", &["x"], Credits(20))
            .map_err(show)?;
        let announcement = announce(
            &b,
            &Declaration::new(b.did(), 1, 1, vec![sample_capability()?]).map_err(show)?,
            1,
        )
        .map_err(show)?;
        kernel.send(&announcement).map_err(show)?;
        let foreign = au4a_core::Envelope::new(
            b.did(),
            None,
            au4a_core::kinds::AGENT_CARD,
            1,
            None,
            json!({"other_track": true}),
        )
        .map_err(show)?
        .seal(&b)
        .map_err(show)?;
        kernel.send(&foreign).map_err(show)?;

        let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        let report = pump(&mut kernel, &mut graph, 1);
        if report.routed != 1 || report.applied != 1 || report.forwarded != 1 || kernel.queue_len() != 1
        {
            return Err(format!("pump 统计不符：{}", report.to_value()));
        }
        Ok("断言：1 条能力通告被路由入图、1 条他轨道信封被转发回队列（id 不变、不吞消息）".into())
    }));
    checks
}

/// 自检用的一条合法能力。
fn sample_capability() -> Result<Capability, String> {
    Ok(Capability::new(SkillId::new("translate.en-zh").map_err(show)?, Credits(4)))
}

/// 演示用 Agent：确定性种子 → 确定性 DID → 确定性场景。
///
/// 五个角色是刻意设计的，不是随手凑数：
///
/// * `alice` 需要「英译中 + 情感分析」的流水线（输入 `text/plain`）。
/// * `bob` 提供 `translate.en-zh`，产出 `application/json`。
/// * `carol` 提供 `sentiment.analyze`，接受 `application/json`。→ `bob → carol` 是可行解。
/// * `dave` 提供**更便宜**的 `translate.en-zh`，但产出 `text/html`：贪心选它会走进死路。
/// * `erin` 提供**更便宜**的 `sentiment.analyze`，但只接受 `text/plain`：同样对不上。
///
/// 于是「路径规划必须是真搜索」这件事有了可证伪的用例（见 v1.1.6 测试）：
/// 按每步最便宜贪心会失败，只有搜索才能找到 `bob → carol`。
pub struct DemoAgent {
    pub keys: AgentKeys,
    pub display: &'static str,
    pub capabilities: Vec<Capability>,
}

/// 构造演示 Agent 集合（顺序固定：alice, bob, carol, dave, erin）。
pub fn demo_agents() -> CoreResult<Vec<DemoAgent>> {
    let translate_bob = Capability::new(SkillId::new("translate.en-zh")?, Credits(3))
        .with_formats(&["text/plain"], &["application/json"])?
        .with_latency(90, 240)
        .with_reliability_bp(9_600);
    let sentiment_carol = Capability::new(SkillId::new("sentiment.analyze")?, Credits(2))
        .with_formats(&["application/json"], &["application/json"])?
        .with_latency(110, 300)
        .with_reliability_bp(9_700);
    let translate_dave = Capability::new(SkillId::new("translate.en-zh")?, Credits(1))
        .with_formats(&["text/plain"], &["text/html"])?
        .with_latency(60, 150)
        .with_reliability_bp(8_000);
    let sentiment_erin = Capability::new(SkillId::new("sentiment.analyze")?, Credits(1))
        .with_formats(&["text/plain"], &["application/json"])?
        .with_latency(50, 120)
        .with_reliability_bp(7_500);
    let summarize_alice = Capability::new(SkillId::new("summarize.zh")?, Credits(5))
        .with_formats(&["text/plain"], &["text/plain"])?
        .with_latency(200, 600);

    let mut agents = Vec::new();
    for (seed, display, capabilities) in [
        (11u8, "alice", vec![summarize_alice]),
        (12, "bob", vec![translate_bob]),
        (13, "carol", vec![sentiment_carol]),
        (14, "dave", vec![translate_dave]),
        (15, "erin", vec![sentiment_erin]),
    ] {
        agents.push(DemoAgent {
            keys: AgentKeys::from_seed(&[seed; 32]),
            display,
            capabilities,
        });
    }
    Ok(agents)
}

/// 在共享内核里注册演示 Agent（已注册就跳过：内核是共享资源，本轨道不重复注册）。
fn ensure_registered(kernel: &mut au4a_kernel::Kernel, agents: &[DemoAgent]) -> CoreResult<usize> {
    let mut registered = 0;
    for agent in agents {
        if kernel.card(&agent.keys.did()).is_none() {
            kernel.register(&agent.keys, agent.display, &[], Credits(20))?;
            registered += 1;
        }
    }
    Ok(registered)
}

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
///
/// 每一条都是**运行中的真实断言**，不是版本号回显。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = vec![SelfCheck::pass(
        TRACK,
        "track.wired",
        format!("{TITLE} {RANGE} 已接入 au4a-node"),
    )];
    checks.extend(checks_v111());
    checks.extend(checks_v112());
    checks.extend(checks_v113());
    checks
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    let checks = self_check();
    let passed = checks.iter().filter(|c| c.passed).count();
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "versions": ["v1.1.1", "v1.1.2", "v1.1.3"],
        "checks": checks.len(),
        "checks_passed": passed,
        "schema": {
            "capability_fields": [
                "skill", "latency_p50_ms", "latency_p99_ms", "throughput_per_min",
                "current_load_bp", "price_per_unit", "reliability_bp",
                "supported_formats", "produced_formats", "constraints"
            ],
            "units": "所有度量与价格均为整数（毫秒 / 每分钟 / 万分比 / 微积分）",
        },
    }))
}

/// v1.1.3 的自有流程：五个 Agent 各自签声明并**通过 PMB 广播**，alice 收下邻居通告。
pub fn scenario(kernel: &mut au4a_kernel::Kernel) -> CoreResult<Value> {
    let agents = demo_agents()?;
    let newly_registered = ensure_registered(kernel, &agents)?;
    let alice = &agents[0];

    let mut graph = AgentCapabilityGraph::new(alice.keys.did(), CapGraphConfig::default());
    let own = Declaration::new(alice.keys.did(), 1, 0, alice.capabilities.clone())?.sign(&alice.keys)?;
    let own_outcome = graph.apply(&own, 0);

    // 每个邻居把自己的完整声明作为广播通告送进 PMB。ts 用固定值，保证可重放。
    let mut sent = 0usize;
    let mut send_failures = 0usize;
    for agent in &agents[1..] {
        let declaration = Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone())?;
        let env = announce(&agent.keys, &declaration, 1)?;
        match kernel.send(&env) {
            Ok(_) => sent += 1,
            Err(_) => send_failures += 1,
        }
    }

    // 收到的通告进图；不属于本轨道的信封被原样转发回队列。
    let pump_report = pump(kernel, &mut graph, 1);

    // 篡改演示：改动已签通告里的一个数字，验签必然失败（不经过内核，避免污染共享拒绝记录）。
    let tampered_rejected = {
        let mut env = announce(
            &agents[2].keys,
            &Declaration::new(agents[2].keys.did(), 1, 0, agents[2].capabilities.clone())?,
            2,
        )?;
        env.body["declaration"]["declaration"]["capabilities"][0]["price_per_unit"] = json!(0);
        parse_announcement(&env)
            .err()
            .map(|(code, _)| code == au4a_core::RefusalCode::Unauthorized)
            .unwrap_or(false)
    };

    let rejected = usize::from(!own_outcome.is_applied());
    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!(
            "v1.1.3 广播协议：{sent} 条通告入队、{} 条被路由入图、{rejected} 条自有声明被拒、篡改拒绝={tampered_rejected}",
            pump_report.routed
        ),
    );
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "version": "v1.1.3",
        "agents_in_kernel": kernel.agent_count(),
        "newly_registered": newly_registered,
        "announcements_sent": sent,
        "send_failures": send_failures,
        "pump": pump_report.to_value(),
        "tampered_rejected": tampered_rejected,
        "graph": graph.to_value(),
        "rejected": rejected,
        "events": 1,
    }))
}
