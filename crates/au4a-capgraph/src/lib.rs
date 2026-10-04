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

pub mod capability;
pub mod declaration;
pub mod graph;

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

/// 自检用的一条合法能力。
fn sample_capability() -> Result<Capability, String> {
    Ok(Capability::new(SkillId::new("translate.en-zh").map_err(show)?, Credits(4)))
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
        "versions": ["v1.1.1", "v1.1.2"],
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

/// v1.1.2 的自有流程：三个 Agent 各自签一份声明，A 接受 B/C 的声明并报告结果。
///
/// 后续小版本会让这里逐步接上广播、缓存、查询与路径规划，
/// 但它从第一版起就必须做**真事**：返回值里的每个数字都来自真实计算。
pub fn scenario(kernel: &mut au4a_kernel::Kernel) -> CoreResult<Value> {
    let alice = AgentKeys::from_seed(&[11; 32]);
    let bob = AgentKeys::from_seed(&[12; 32]);
    let carol = AgentKeys::from_seed(&[13; 32]);

    let alice_caps = vec![
        Capability::new(SkillId::new("translate.en-zh")?, Credits(4))
            .with_formats(&["text/plain"], &["text/plain"])?
            .with_latency(80, 200)
            .with_reliability_bp(9_500),
        Capability::new(SkillId::new("sentiment.analyze")?, Credits(2))
            .with_formats(&["text/plain"], &["application/json"])?
            .with_latency(120, 400),
    ];
    let bob_caps = vec![Capability::new(SkillId::new("translate.en-zh")?, Credits(3))
        .with_formats(&["text/plain"], &["application/json"])?
        .with_latency(90, 240)];
    let carol_caps = vec![Capability::new(SkillId::new("sentiment.analyze")?, Credits(2))
        .with_formats(&["application/json"], &["application/json"])?
        .with_latency(110, 300)];

    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let own = Declaration::new(alice.did(), 1, 0, alice_caps)?.sign(&alice)?;
    let own_outcome = graph.apply(&own, 0);
    let bob_outcome = graph.apply(&Declaration::new(bob.did(), 1, 0, bob_caps)?.sign(&bob)?, 0);
    let carol_outcome =
        graph.apply(&Declaration::new(carol.did(), 1, 0, carol_caps)?.sign(&carol)?, 0);

    let rejected = [&own_outcome, &bob_outcome, &carol_outcome]
        .iter()
        .filter(|o| !o.is_applied())
        .count();
    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!(
            "v1.1.2 声明 API：{} 个 Agent 自签声明，{} 条能力入图，{rejected} 次拒绝",
            graph.known_agents(),
            graph.capability_count()
        ),
    );
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "version": "v1.1.2",
        "agents": graph.known_agents(),
        "capabilities": graph.capability_count(),
        "skills": graph.skills().iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "outcomes": [
            own_outcome.to_value(),
            bob_outcome.to_value(),
            carol_outcome.to_value(),
        ],
        "rejected": rejected,
        "events": 1,
    }))
}
