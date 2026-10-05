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
pub mod cache;
pub mod capability;
pub mod declaration;
pub mod graph;
pub mod index;
pub mod perf;
pub mod planner;
pub mod version;

pub use broadcast::{
    announce, announce_to, ingest, parse_announcement, parse_query, pump, query_skill,
    Announcement, Ingest, PumpReport, KIND_ANNOUNCE, KIND_REQUEST, PROTOCOL,
};
pub use cache::{CacheInsert, CacheStats, CapabilityCache};
pub use capability::{
    Capability, Constraints, FormatId, SkillId, BP_SCALE, MAX_FORMAT_LEN, MAX_SKILL_LEN,
};
pub use declaration::{Declaration, SignedDeclaration};
pub use graph::{AgentCapabilityGraph, CapGraphConfig, DeclareOutcome, NeighborRecord};
pub use index::{
    CapKey, CapabilityIndex, CapabilityMatch, CapabilityQuery, QueryResult, QueryStats,
};
pub use perf::{rank_top_k, QueryCache, QueryCacheStats, QueryPerf};
pub use planner::{
    plan, verify_pipeline, NoPath, NoPathReason, Pipeline, PipelineNode, PipelineRequest,
    PipelineStep, PlanCost, PlanOutcome, SearchStats,
};
pub use version::{VersionHistory, VersionRecord, HISTORY_CAPACITY};

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

/// v1.1.4：容量上限是硬的、淘汰是确定性的、失效是显式的。
fn checks_v114() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.4.capacity_is_a_hard_bound", || {
        let mut cache = CapabilityCache::new(3, 0);
        let mut peak = 0usize;
        for seed in 0..10u8 {
            let did = AgentKeys::from_seed(&[seed; 32]).did();
            cache.insert(
                NeighborRecord {
                    did,
                    epoch: 1,
                    at: 0,
                    fingerprint: format!("fp{seed}"),
                    capabilities: vec![sample_capability()?],
                },
                0,
            );
            peak = peak.max(cache.len());
        }
        if peak > 3 || cache.len() != 3 || cache.stats().evictions != 7 {
            return Err(format!(
                "容量上限被突破或淘汰计数不符：peak={peak} len={} evictions={}",
                cache.len(),
                cache.stats().evictions
            ));
        }
        Ok("断言：容量 3 的缓存写入 10 条后 len 恒 <=3、末尾 len=3、evictions=7".into())
    }));
    checks.push(verdict("1.1.4.lru_eviction_is_deterministic", || {
        let mut cache = CapabilityCache::new(2, 0);
        let d1 = AgentKeys::from_seed(&[1; 32]).did();
        let d2 = AgentKeys::from_seed(&[2; 32]).did();
        let d3 = AgentKeys::from_seed(&[3; 32]).did();
        for (did, seed) in [(&d1, 1u8), (&d2, 2)] {
            cache.insert(
                NeighborRecord {
                    did: did.clone(),
                    epoch: 1,
                    at: 0,
                    fingerprint: format!("fp{seed}"),
                    capabilities: vec![sample_capability()?],
                },
                0,
            );
        }
        // 用一下 d1，使 d2 成为最久未用。
        if cache.get(&d1, 0).is_none() {
            return Err("命中失败".into());
        }
        let evicted = cache.insert(
            NeighborRecord {
                did: d3.clone(),
                epoch: 1,
                at: 0,
                fingerprint: "fp3".into(),
                capabilities: vec![sample_capability()?],
            },
            0,
        );
        if evicted.evicted != vec![d2.clone()] || cache.peek(&d1).is_none() {
            return Err(format!("淘汰对象错误：{:?}", evicted.evicted));
        }
        Ok("断言：命中 d1 后再写入 → 淘汰最久未用的 d2（(last_used,did) 定序），d1 仍在".into())
    }));
    checks.push(verdict("1.1.4.ttl_uses_logical_ticks", || {
        let mut cache = CapabilityCache::new(4, 10);
        let did = AgentKeys::from_seed(&[7; 32]).did();
        cache.insert(
            NeighborRecord {
                did: did.clone(),
                epoch: 1,
                at: 100,
                fingerprint: "fp".into(),
                capabilities: vec![sample_capability()?],
            },
            100,
        );
        let alive_at_110 = cache.is_live(&did, 110);
        let alive_at_111 = cache.is_live(&did, 111);
        let hit = cache.get(&did, 111).is_some();
        if !alive_at_110 || alive_at_111 || hit || cache.stats().expirations != 1 {
            return Err("TTL 边界不正确（应为 at+ttl 之内存活、之外失效）".into());
        }
        Ok("断言：at=100、ttl=10 时 110 刻存活、111 刻失效并计入 expirations（只用逻辑刻）".into())
    }));
    checks.push(verdict("1.1.4.graph_evicts_instead_of_refusing", || {
        let owner = AgentKeys::from_seed(&[41; 32]);
        let graph_config = CapGraphConfig {
            neighbor_capacity: 2,
            cache_ttl_ticks: 0,
            ..CapGraphConfig::default()
        };
        let mut graph = AgentCapabilityGraph::new(owner.did(), graph_config);
        let mut applied = 0;
        for seed in [42u8, 43, 44] {
            let peer = AgentKeys::from_seed(&[seed; 32]);
            let signed = Declaration::new(peer.did(), 1, 1, vec![sample_capability()?])
                .map_err(show)?
                .sign(&peer)
                .map_err(show)?;
            if graph.apply(&signed, 1).is_applied() {
                applied += 1;
            }
        }
        if applied != 3 || graph.neighbor_count() != 2 || graph.cache_stats().evictions != 1 {
            return Err(format!(
                "满容量后的行为不对：applied={applied} neighbors={} evictions={}",
                graph.neighbor_count(),
                graph.cache_stats().evictions
            ));
        }
        Ok("断言：容量 2 的图接受 3 个邻居声明全部 applied，最终只留 2 条、淘汰 1 条（不拒绝新信息）".into())
    }));
    checks
}

/// v1.1.5：查询走索引、过滤是硬条件、排序确定、索引与图始终一致。
fn checks_v115() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.5.query_does_not_scan_the_graph", || {
        let owner = AgentKeys::from_seed(&[100; 32]);
        let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
        // 60 个邻居 × 2 条能力 = 120 条能力入图。
        for seed in 0..60u8 {
            let peer = AgentKeys::from_seed(&[seed; 32]);
            let caps = vec![
                Capability::new(SkillId::new("translate.en-zh").map_err(show)?, Credits(3)),
                Capability::new(SkillId::new("sentiment.analyze").map_err(show)?, Credits(2)),
            ];
            let signed = Declaration::new(peer.did(), 1, 1, caps)
                .map_err(show)?
                .sign(&peer)
                .map_err(show)?;
            graph.apply(&signed, 1);
        }
        let result = graph.query(
            &CapabilityQuery::new(SkillId::new("translate.en-zh").map_err(show)?).with_limit(3),
            1,
        );
        let stats = result.stats;
        if stats.candidates != 60 || stats.scanned != 60 || stats.nodes_total != 120 {
            return Err(format!("索引扫描量不符：{}", stats.to_value()));
        }
        if stats.scanned >= stats.nodes_total {
            return Err("查询退化成全图扫描".into());
        }
        Ok(format!(
            "断言：视图 120 条能力，按技能查询只扫 {} 条（nodes_total={}，省掉 {} 条），返回前 3 名",
            stats.scanned, stats.nodes_total, stats.nodes_total - stats.scanned
        ))
    }));
    checks.push(verdict("1.1.5.filters_are_hard_conditions", || {
        let owner = AgentKeys::from_seed(&[101; 32]);
        let peer = AgentKeys::from_seed(&[102; 32]);
        let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
        let pricey = Capability::new(SkillId::new("translate.en-zh").map_err(show)?, Credits(99));
        let signed = Declaration::new(peer.did(), 1, 1, vec![pricey])
            .map_err(show)?
            .sign(&peer)
            .map_err(show)?;
        graph.apply(&signed, 1);
        let skill = SkillId::new("translate.en-zh").map_err(show)?;
        let unfiltered = graph.query(&CapabilityQuery::new(skill.clone()), 1);
        let filtered = graph.query(
            &CapabilityQuery::new(skill).with_max_price(Credits(10)),
            1,
        );
        if unfiltered.matches.len() != 1 || !filtered.is_empty() {
            return Err("价格上限没有生效".into());
        }
        if filtered.refusal_code() != Some(au4a_core::RefusalCode::Unsupported) {
            return Err("「查不到」应给出类型化结局 unsupported".into());
        }
        Ok("断言：99 微积分的报价在 max_price=10 的查询下被过滤，空结果给出 unsupported".into())
    }));
    checks.push(verdict("1.1.5.index_stays_consistent_through_eviction", || {
        let owner = AgentKeys::from_seed(&[103; 32]);
        let mut graph = AgentCapabilityGraph::new(
            owner.did(),
            CapGraphConfig {
                neighbor_capacity: 2,
                cache_ttl_ticks: 0,
                ..CapGraphConfig::default()
            },
        );
        for seed in [104u8, 105, 106] {
            let peer = AgentKeys::from_seed(&[seed; 32]);
            let signed = Declaration::new(peer.did(), 1, 1, vec![sample_capability()?])
                .map_err(show)?
                .sign(&peer)
                .map_err(show)?;
            graph.apply(&signed, 1);
        }
        if !graph.index_consistent() || graph.index_summary()["entries"].as_u64() != Some(2) {
            return Err(format!("淘汰后索引与图不一致：{}", graph.index_summary()));
        }
        Ok("断言：容量 2 的图接受 3 个邻居后 index.indexed_entries()==2 且每个键都能解析回同技能能力".into())
    }));
    checks.push(verdict("1.1.5.ranking_is_deterministic", || {
        let build = || -> Result<QueryResult, String> {
            let owner = AgentKeys::from_seed(&[107; 32]);
            let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
            for (seed, price, reliability) in [(108u8, 5i64, 9_000u16), (109, 5, 9_000), (110, 4, 8_000)] {
                let peer = AgentKeys::from_seed(&[seed; 32]);
                let cap = Capability::new(SkillId::new("translate.en-zh").map_err(show)?, Credits(price))
                    .with_reliability_bp(reliability);
                let signed = Declaration::new(peer.did(), 1, 1, vec![cap])
                    .map_err(show)?
                    .sign(&peer)
                    .map_err(show)?;
                graph.apply(&signed, 1);
            }
            Ok(graph.query(&CapabilityQuery::new(SkillId::new("translate.en-zh").map_err(show)?), 1))
        };
        let first = build()?;
        let second = build()?;
        if first.matches != second.matches || first.matches.len() != 3 {
            return Err("两次相同查询得到不同顺序".into());
        }
        let prices: Vec<i64> = first.matches.iter().map(|m| m.capability.price_per_unit.get()).collect();
        if prices != vec![5, 5, 4] {
            return Err(format!("排序不符（可靠度优先）：{prices:?}"));
        }
        Ok("断言：可靠度 9000/9000/8000 的三条报价排序后为 5,5,4，两次查询逐位相同".into())
    }));
    checks
}

/// 用演示角色搭一张 alice 视角的图（不经过内核），供自检与场景复用。
fn demo_graph() -> Result<AgentCapabilityGraph, String> {
    let agents = demo_agents().map_err(show)?;
    let mut graph = AgentCapabilityGraph::new(agents[0].keys.did(), CapGraphConfig::default());
    let own = Declaration::new(agents[0].keys.did(), 1, 0, agents[0].capabilities.clone())
        .map_err(show)?
        .sign(&agents[0].keys)
        .map_err(show)?;
    graph.apply(&own, 0);
    for agent in &agents[1..] {
        let signed = Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone())
            .map_err(show)?
            .sign(&agent.keys)
            .map_err(show)?;
        graph.apply(&signed, 0);
    }
    Ok(graph)
}

/// 演示请求：`text/plain` 进，先英译中再情感分析。
fn demo_request() -> Result<PipelineRequest, String> {
    Ok(PipelineRequest::new(
        FormatId::new("text/plain").map_err(show)?,
        vec![
            PipelineStep::new(SkillId::new("translate.en-zh").map_err(show)?),
            PipelineStep::new(SkillId::new("sentiment.analyze").map_err(show)?),
        ],
    ))
}

/// v1.1.6：真图搜索，能跑通的链、贪心的反例、无路径的类型化原因。
fn checks_v116() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.6.format_compatible_path", || {
        let mut graph = demo_graph()?;
        let request = demo_request()?;
        let outcome = graph.plan(&request, &PlanCost::default(), 0);
        let pipeline = outcome.pipeline().ok_or_else(|| format!("{:?}", outcome.to_value()))?;
        let agents = demo_agents().map_err(show)?;
        let first = pipeline.nodes.first().ok_or("流水线为空")?;
        let second = pipeline.nodes.get(1).ok_or("流水线只有一步")?;
        if first.did != agents[1].keys.did() || second.did != agents[2].keys.did() {
            return Err("规划给出的不是 bob → carol".into());
        }
        if pipeline.total_price != Credits(5) {
            return Err(format!("总价应为 5，实测 {}", pipeline.total_price));
        }
        Ok("断言：text/plain 进、translate+sentiment 两步 → bob(→application/json) → carol，总价 5 微积分".into())
    }));
    checks.push(verdict("1.1.6.search_beats_greedy", || {
        let mut graph = demo_graph()?;
        let request = demo_request()?;
        let outcome = graph.plan(&request, &PlanCost::default(), 0);
        let pipeline = outcome.pipeline().ok_or("应有可行路径")?;
        let agents = demo_agents().map_err(show)?;
        // 贪心会选 dave（1 微积分）——他的产出是 text/html，接不上任何情感分析。
        let cheapest = agents[3].keys.did();
        if pipeline.nodes[0].did == cheapest {
            return Err("规划选了贪心的死路".into());
        }
        let dave = &agents[3].capabilities[0];
        let continuations = graph
            .query(
                &CapabilityQuery::new(SkillId::new("sentiment.analyze").map_err(show)?).with_limit(0),
                0,
            )
            .matches
            .iter()
            .filter(|m| dave.handoff_format(&m.capability).is_some())
            .count();
        if continuations != 0 {
            return Err("dave 的产出竟然能接上后续步骤".into());
        }
        Ok("断言：最便宜的第一步（dave，1 微积分）产出 text/html 无任何后续可接，规划改选 bob（3 微积分）".into())
    }));
    checks.push(verdict("1.1.6.no_path_is_typed", || {
        let mut graph = demo_graph()?;
        let reversed = PipelineRequest::new(
            FormatId::new("text/plain").map_err(show)?,
            vec![
                PipelineStep::new(SkillId::new("sentiment.analyze").map_err(show)?),
                PipelineStep::new(SkillId::new("translate.en-zh").map_err(show)?),
            ],
        );
        match graph.plan(&reversed, &PlanCost::default(), 0) {
            PlanOutcome::NoPath(no_path) => {
                let ok = matches!(no_path.reason, NoPathReason::NoCompatibleFormat { step: 1, .. })
                    && no_path.refusal_code() == au4a_core::RefusalCode::Unsupported;
                if !ok {
                    return Err(format!("无路径原因不精确：{:?}", no_path.reason));
                }
                Ok("断言：情感分析→翻译 无路径，原因精确到 step 1 的格式接不上，拒绝码 unsupported".into())
            }
            PlanOutcome::Path(pipeline) => Err(format!("不该有路径：{:?}", pipeline.to_value())),
        }
    }));
    checks.push(verdict("1.1.6.budget_and_deadline_are_distinguishable", || {
        let mut graph = demo_graph()?;
        let request = demo_request()?;
        let poor = request.clone().with_budget(Credits(4));
        let slow = request.clone().with_deadline_ms(150);
        let poor_reason = match graph.plan(&poor, &PlanCost::default(), 0) {
            PlanOutcome::NoPath(no_path) => no_path.reason,
            PlanOutcome::Path(p) => return Err(format!("预算 4 不该可行：{:?}", p.to_value())),
        };
        let slow_reason = match graph.plan(&slow, &PlanCost::default(), 0) {
            PlanOutcome::NoPath(no_path) => no_path.reason,
            PlanOutcome::Path(p) => return Err(format!("150ms 不该可行：{:?}", p.to_value())),
        };
        if poor_reason != (NoPathReason::BudgetExceeded { allowed: 4 }) {
            return Err(format!("预算原因不精确：{poor_reason:?}"));
        }
        if slow_reason != (NoPathReason::DeadlineExceeded { allowed_ms: 150 }) {
            return Err(format!("期限原因不精确：{slow_reason:?}"));
        }
        Ok("断言：预算 4 → BudgetExceeded{4}；期限 150ms → DeadlineExceeded{150}（两者可区分）".into())
    }));
    checks
}

/// v1.1.7：版本随内容自动递增；陈旧与冲突在缓存失效后依然能被历史抓住。
fn checks_v117() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.7.version_auto_increments", || {
        let alice = AgentKeys::from_seed(&[51; 32]);
        let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
        let first = graph
            .declare(&alice, vec![sample_capability()?], 0)
            .map_err(show)?;
        let second = graph
            .declare(
                &alice,
                vec![sample_capability()?.with_price(Credits(9))],
                1,
            )
            .map_err(show)?;
        let same_again = graph
            .declare(
                &alice,
                vec![sample_capability()?.with_price(Credits(9))],
                2,
            )
            .map_err(show)?;
        if !first.is_applied() || !second.is_applied() {
            return Err("内容变化必须被接受".into());
        }
        if graph.own_epoch() != 2 {
            return Err(format!("版本号应为 2，实测 {}", graph.own_epoch()));
        }
        if !matches!(same_again, DeclareOutcome::Unchanged { .. }) {
            return Err("内容相同的重发不该推进版本".into());
        }
        Ok("断言：两次内容变更 → 版本 1→2；第三次内容相同 → Unchanged 且版本仍为 2".into())
    }));
    checks.push(verdict("1.1.7.history_is_content_addressed", || {
        let alice = AgentKeys::from_seed(&[52; 32]);
        let owner = alice.did();
        let mut graph = AgentCapabilityGraph::new(owner.clone(), CapGraphConfig::default());
        graph
            .declare(&alice, vec![sample_capability()?], 0)
            .map_err(show)?;
        let declaration =
            Declaration::new(owner.clone(), 1, 0, vec![sample_capability()?]).map_err(show)?;
        let expected = declaration.capabilities_fingerprint().map_err(show)?;
        let recorded = graph
            .history()
            .hash_of(&owner, 1)
            .map(str::to_string)
            .ok_or("历史里没有 v1 记录")?;
        if recorded != expected {
            return Err("历史哈希与声明指纹不一致".into());
        }
        Ok("断言：历史记录里的 hash == Declaration::fingerprint()（内容寻址，不可伪造）".into())
    }));
    checks.push(verdict("1.1.7.history_survives_cache_expiry", || {
        let owner = AgentKeys::from_seed(&[53; 32]);
        let peer = AgentKeys::from_seed(&[54; 32]);
        let mut graph = AgentCapabilityGraph::new(
            owner.did(),
            CapGraphConfig {
                cache_ttl_ticks: 5,
                ..CapGraphConfig::default()
            },
        );
        let version_three_a = Declaration::new(peer.did(), 3, 0, vec![sample_capability()?])
            .map_err(show)?
            .sign(&peer)
            .map_err(show)?;
        graph.apply(&version_three_a, 100);
        // TTL 过后缓存里已经没有这条记录了。
        let _ = graph.expire_neighbors(1_000);
        // 同一个版本号但内容不同 → 仍然必须判 conflict（靠历史，不靠缓存）。
        let version_three_b = Declaration::new(
            peer.did(),
            3,
            0,
            vec![sample_capability()?.with_price(Credits(77))],
        )
        .map_err(show)?
        .sign(&peer)
        .map_err(show)?;
        let conflict = graph.apply(&version_three_b, 1_001);
        // 比历史更旧的版本 → stale_epoch。
        let version_two = Declaration::new(peer.did(), 2, 0, vec![sample_capability()?])
            .map_err(show)?
            .sign(&peer)
            .map_err(show)?;
        let stale = graph.apply(&version_two, 1_002);
        if conflict.refusal() != Some(au4a_core::RefusalCode::Conflict) {
            return Err(format!("缓存过期后未判 conflict：{:?}", conflict.to_value()));
        }
        if stale.refusal() != Some(au4a_core::RefusalCode::StaleEpoch) {
            return Err(format!("缓存过期后未判 stale_epoch：{:?}", stale.to_value()));
        }
        Ok("断言：缓存过期后，同版本异内容仍判 conflict、更旧版本仍判 stale_epoch（历史兜底）".into())
    }));
    checks.push(verdict("1.1.7.history_is_bounded", || {
        let alice = AgentKeys::from_seed(&[55; 32]);
        let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
        for price in 0..(HISTORY_CAPACITY as i64 + 5) {
            let capability = sample_capability()?.with_price(Credits(price));
            graph.declare(&alice, vec![capability], price as u64).map_err(show)?;
        }
        if graph.history().len() != HISTORY_CAPACITY {
            return Err(format!("历史长度应为 {HISTORY_CAPACITY}，实测 {}", graph.history().len()));
        }
        if graph.own_epoch() != HISTORY_CAPACITY as u64 + 4 {
            return Err(format!("版本号应为 {}，实测 {}", HISTORY_CAPACITY + 4, graph.own_epoch()));
        }
        Ok(format!(
            "断言：{} 次内容变更后历史长度为 {HISTORY_CAPACITY}（有界），版本号为 {}",
            HISTORY_CAPACITY + 5,
            graph.own_epoch()
        ))
    }));
    checks
}

/// v1.1.8：性能用操作计数证明——增量索引不重建、有界 top-k 与全排序等价、
/// 查询缓存按修订号失效、同样的负载得到同样的计数（不依赖墙钟）。
fn checks_v118() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.8.index_updates_are_incremental", || {
        let owner = AgentKeys::from_seed(&[61; 32]);
        let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
        for seed in 0..100u8 {
            let peer = AgentKeys::from_seed(&[seed; 32]);
            let caps = vec![
                Capability::new(SkillId::new("translate.en-zh").map_err(show)?, Credits(3)),
                Capability::new(SkillId::new("sentiment.analyze").map_err(show)?, Credits(2)),
            ];
            let signed = Declaration::new(peer.did(), 1, 1, caps)
                .map_err(show)?
                .sign(&peer)
                .map_err(show)?;
            graph.apply(&signed, 1);
        }
        let perf = graph.query_perf();
        let index = graph.index_summary();
        if perf.index_rebuilds != 0 {
            return Err(format!("正常读写路径出现了整表重建：{}", perf.to_value()));
        }
        if index["entries"].as_u64() != Some(200) || !graph.index_consistent() {
            return Err(format!("索引与图不一致：{index}"));
        }
        Ok("断言：100 个邻居 × 2 条能力增量入图，index_rebuilds=0、indexed_entries=200、索引自洽"
            .to_string())
    }));
    checks.push(verdict("1.1.8.bounded_top_k_equals_full_sort", || {
        let owner = AgentKeys::from_seed(&[62; 32]);
        let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
        let skill = SkillId::new("translate.en-zh").map_err(show)?;
        for seed in 0..64u8 {
            let peer = AgentKeys::from_seed(&[seed; 32]);
            let capability = Capability::new(skill.clone(), Credits(3))
                .with_reliability_bp(9_000 - u16::from(seed))
                .with_latency(u32::from(seed) + 10, u32::from(seed) + 100);
            let signed = Declaration::new(peer.did(), 1, 1, vec![capability])
                .map_err(show)?
                .sign(&peer)
                .map_err(show)?;
            graph.apply(&signed, 1);
        }
        let bounded = graph.query(&CapabilityQuery::new(skill.clone()).with_limit(3), 1);
        let before = graph.query_perf();
        let full = graph.query(&CapabilityQuery::new(skill.clone()).with_limit(0), 1);
        let top3: Vec<&str> = full.matches.iter().take(3).map(|m| m.did.as_str()).collect();
        let bounded_ids: Vec<&str> = bounded.matches.iter().map(|m| m.did.as_str()).collect();
        if !bounded.stats.bounded_selection || bounded_ids != top3 {
            return Err(format!(
                "有界 top-k 与全排序不一致：bounded={bounded_ids:?} full={top3:?}"
            ));
        }
        if before.bounded_selections != 1 || before.full_sorts != 0 {
            return Err(format!("计数不符：{}", before.to_value()));
        }
        Ok("断言：64 条候选中取前 3 → bounded_selections=1、full_sorts=0，且与全排序前 3 名逐位相同"
            .to_string())
    }));
    checks.push(verdict("1.1.8.query_cache_is_revision_bound", || {
        let owner = AgentKeys::from_seed(&[63; 32]);
        let peer = AgentKeys::from_seed(&[64; 32]);
        let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
        let skill = SkillId::new("translate.en-zh").map_err(show)?;
        let signed = Declaration::new(peer.did(), 1, 1, vec![sample_capability()?])
            .map_err(show)?
            .sign(&peer)
            .map_err(show)?;
        graph.apply(&signed, 1);
        let query = CapabilityQuery::new(skill);
        let first = graph.query_cached(&query, 1);
        let second = graph.query_cached(&query, 1);
        if first != second || graph.query_cache_stats().hits != 1 {
            return Err(format!(
                "缓存未命中或结果不一致：{}",
                graph.query_cache_stats().to_value()
            ));
        }
        let revision_before = graph.revision();
        graph
            .declare(&owner, vec![sample_capability()?], 2)
            .map_err(show)?;
        if graph.revision() <= revision_before {
            return Err("数据变更没有推进修订号".into());
        }
        let third = graph.query_cached(&query, 2);
        let stats = graph.query_cache_stats();
        if third != first || stats.invalidated != 1 {
            return Err(format!("缓存未按修订号失效：{}", stats.to_value()));
        }
        Ok("断言：同一查询第二次命中（hits=1）；声明变更推进修订号后旧条目作废（invalidated=1）且结果不变"
            .to_string())
    }));
    checks.push(verdict("1.1.8.perf_evidence_is_deterministic", || {
        let workload = || -> Result<String, String> {
            let owner = AgentKeys::from_seed(&[65; 32]);
            let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
            for seed in 0..32u8 {
                let peer = AgentKeys::from_seed(&[seed; 32]);
                let signed = Declaration::new(peer.did(), 1, 1, vec![sample_capability()?])
                    .map_err(show)?
                    .sign(&peer)
                    .map_err(show)?;
                graph.apply(&signed, 1);
            }
            let skill = SkillId::new("translate.en-zh").map_err(show)?;
            for _ in 0..5 {
                let _ = graph.query_cached(&CapabilityQuery::new(skill.clone()).with_limit(2), 1);
            }
            graph.reindex();
            Ok(graph.perf_summary().to_string())
        };
        let first = workload()?;
        let second = workload()?;
        if first != second {
            return Err("同样的负载得到不同的性能计数（说明证据依赖了非确定性来源）".into());
        }
        Ok(format!("断言：同一负载两次运行得到逐字节相同的计数（无墙钟参与）：{first}"))
    }));
    checks
}

/// v1.1.9：独立校验器（收到方案的一方必须能自己验），以及场景输出的逐版覆盖。
fn checks_v119() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.push(verdict("1.1.9.plan_is_independently_verifiable", || {
        let mut graph = demo_graph()?;
        let request = demo_request()?;
        let outcome = graph.plan(&request, &PlanCost::default(), 0);
        let pipeline = outcome.pipeline().ok_or("应有可行路径")?.clone();
        verify_pipeline(&graph, &request, &PlanCost::default(), &pipeline)
            .map_err(|err| format!("校验器拒绝了合法方案：{err}"))?;
        Ok("断言：规划结果能被 verify_pipeline 逐项验通（步数/格式链/度量/代价/约束）".into())
    }));
    checks.push(verdict("1.1.9.verifier_rejects_tampered_plan", || {
        let mut graph = demo_graph()?;
        let request = demo_request()?;
        let pipeline = graph
            .plan(&request, &PlanCost::default(), 0)
            .pipeline()
            .ok_or("应有可行路径")?
            .clone();
        let mut wrong_price = pipeline.clone();
        wrong_price.nodes[0].price_per_unit = Credits(1);
        let mut wrong_format = pipeline.clone();
        wrong_format.nodes[1].input_format = FormatId::new("text/html").map_err(show)?;
        let mut short = pipeline.clone();
        short.nodes.pop();
        let mut forged = pipeline.clone();
        forged.nodes[0].did = AgentKeys::from_seed(&[250; 32]).did();
        let cases = [
            ("改价", &wrong_price),
            ("改格式", &wrong_format),
            ("少一步", &short),
            ("换 DID", &forged),
        ];
        for (name, candidate) in cases {
            if verify_pipeline(&graph, &request, &PlanCost::default(), candidate).is_ok() {
                return Err(format!("校验器接受了被篡改的方案：{name}"));
            }
        }
        Ok("断言：改价 / 改格式 / 少一步 / 换 DID 四种篡改方案全部被校验器拒绝".into())
    }));
    checks.push(verdict("1.1.9.scenario_covers_every_version", || {
        let mut kernel = au4a_kernel::Kernel::new(au4a_kernel::KernelConfig::default());
        let value = scenario(&mut kernel).map_err(show)?;
        let required = [
            ("/graph/owner", "v1.1.1-2 图投影"),
            ("/announcements_sent", "v1.1.3 广播"),
            ("/bounded_cache/evictions", "v1.1.4 缓存"),
            ("/index_consistent", "v1.1.5 索引"),
            ("/plan/outcome", "v1.1.6 规划"),
            ("/versions/own", "v1.1.7 版本化"),
            ("/perf/queries/index_rebuilds", "v1.1.8 性能"),
            ("/query_cache_hit", "v1.1.8 查询缓存"),
        ];
        for (pointer, label) in required {
            if value.pointer(pointer).is_none() {
                return Err(format!("scenario 缺少 {label} 的证据字段 {pointer}"));
            }
        }
        if value.pointer("/plan/outcome").and_then(Value::as_str) != Some("path") {
            return Err("scenario 没有给出流水线".into());
        }
        Ok(format!(
            "断言：scenario 输出同时覆盖 8 个版本的证据字段（{} 个必填指针全部存在）",
            required.len()
        ))
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
    checks.extend(checks_v114());
    checks.extend(checks_v115());
    checks.extend(checks_v116());
    checks.extend(checks_v117());
    checks.extend(checks_v118());
    checks.extend(checks_v119());
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
        "versions": [
            "v1.1.1", "v1.1.2", "v1.1.3", "v1.1.4", "v1.1.5",
            "v1.1.6", "v1.1.7", "v1.1.8", "v1.1.9",
        ],
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

/// 端到端自有流程：五个 Agent 各自签声明并**通过 PMB 广播**，alice 收下邻居通告、
/// 查询索引、规划流水线，并演示版本化与无路径结论。
pub fn scenario(kernel: &mut au4a_kernel::Kernel) -> CoreResult<Value> {
    let agents = demo_agents()?;
    let newly_registered = ensure_registered(kernel, &agents)?;
    let alice = &agents[0];

    // v1.1.7：自有能力用自动递增版本的入口声明（内容变则版本 +1）。
    let mut graph = AgentCapabilityGraph::new(alice.keys.did(), CapGraphConfig::default());
    let own_outcome = graph.declare(&alice.keys, alice.capabilities.clone(), 0)?;
    let bumped = graph.declare(
        &alice.keys,
        vec![Capability::new(SkillId::new("summarize.zh")?, Credits(6))
            .with_formats(&["text/plain"], &["text/plain"])?
            .with_latency(200, 600)],
        1,
    )?;
    let idempotent = graph.declare(
        &alice.keys,
        vec![Capability::new(SkillId::new("summarize.zh")?, Credits(6))
            .with_formats(&["text/plain"], &["text/plain"])?
            .with_latency(200, 600)],
        2,
    )?;

    // 每个邻居把自己的完整声明作为广播通告送进 PMB。ts 用固定值，保证可重放。
    let mut sent = 0usize;
    let mut send_failures = 0usize;
    let mut envelopes = Vec::new();
    for agent in &agents[1..] {
        let declaration = Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone())?;
        let env = announce(&agent.keys, &declaration, 1)?;
        match kernel.send(&env) {
            Ok(_) => {
                sent += 1;
                envelopes.push(env);
            }
            Err(_) => send_failures += 1,
        }
    }

    // 收到的通告进图；不属于本轨道的信封被原样转发回队列。
    let pump_report = pump(kernel, &mut graph, 1);

    // 容量与失效的现场演示：另开一个容量 2、TTL 30 的图，喂同样 4 条通告。
    let observer = AgentKeys::from_seed(&[16; 32]);
    let mut bounded = AgentCapabilityGraph::new(
        observer.did(),
        CapGraphConfig {
            neighbor_capacity: 2,
            cache_ttl_ticks: 30,
            ..CapGraphConfig::default()
        },
    );
    for env in &envelopes {
        let _ = ingest(&mut bounded, env, 1);
    }
    let bounded_retained = bounded.neighbor_count();
    let bounded_stats = bounded.cache_stats();
    let bounded_expired = bounded.expire_neighbors(100).len();

    // v1.1.5：alice 用索引查询「谁能翻译」「谁能做情感分析」并报告扫描量。
    // v1.1.8：translate 的查询走结果缓存（第二次命中），另跑一次 limit=1 的查询走有界 top-k。
    let translate = SkillId::new("translate.en-zh")?;
    let sentiment = SkillId::new("sentiment.analyze")?;
    let translate_query = CapabilityQuery::new(translate.clone());
    let translate_hits = graph.query_cached(&translate_query, 1);
    let translate_hits_again = graph.query_cached(&translate_query, 1);
    let cache_hit = translate_hits == translate_hits_again;
    let best_only = graph.query_cached(&translate_query.clone().with_limit(1), 1);
    let sentiment_hits = graph.query(&CapabilityQuery::new(sentiment.clone()), 1);
    let best_translate = best_only
        .best()
        .map(|m| m.did.as_str().to_string())
        .unwrap_or_default();
    let best_sentiment = sentiment_hits
        .best()
        .map(|m| m.did.as_str().to_string())
        .unwrap_or_default();
    let index_consistent = graph.index_consistent();

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

    // v1.1.6：alice 用索引查询 + 图搜索规划出流水线。
    let request = PipelineRequest::new(
        FormatId::new("text/plain")?,
        vec![
            PipelineStep::new(translate.clone()),
            PipelineStep::new(sentiment.clone()),
        ],
    );
    let outcome = graph.plan(&request, &PlanCost::default(), 1);
    let pipeline = outcome.pipeline().cloned();

    // 无路径的现场演示：把两步调换，格式接不上（情感分析产出 json，翻译只吃 text/plain）。
    let reversed = PipelineRequest::new(
        FormatId::new("text/plain")?,
        vec![
            PipelineStep::new(sentiment.clone()),
            PipelineStep::new(translate.clone()),
        ],
    );
    let reversed_outcome = graph.plan(&reversed, &PlanCost::default(), 1);

    let pipeline_agents: Vec<String> = pipeline
        .as_ref()
        .map(|p| p.agents().iter().map(|d| d.as_str().to_string()).collect())
        .unwrap_or_default();
    let pipeline_price = pipeline.as_ref().map(|p| p.total_price.get()).unwrap_or(0);

    let rejected = usize::from(!own_outcome.is_applied());

    // v1.1.7 版本化演示：陈旧通告（epoch 0 < 已知 1）必须被拒，且不经过内核（不污染共享拒绝记录）。
    let stale = graph.apply(
        &Declaration::new(agents[2].keys.did(), 0, 0, agents[2].capabilities.clone())?
            .sign(&agents[2].keys)?,
        2,
    );
    let stale_rejected = stale.refusal() == Some(au4a_core::RefusalCode::StaleEpoch);
    let idempotent_ignored = matches!(idempotent, DeclareOutcome::Unchanged { .. });
    let version_summary = graph.version_summary();

    // v1.1.8：性能证据（全部是操作计数，没有墙钟）。
    let perf_summary = graph.perf_summary();
    let no_path_code = reversed_outcome
        .refusal_code()
        .map(|c| c.as_str().to_string())
        .unwrap_or_default();
    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!(
            "v1.1.7 版本化：alice v{}（+{} 次变更，历史 {} 条）、陈旧通告被拒={stale_rejected}；流水线 {pipeline_agents:?} 总价 {pipeline_price}；反向请求无路径（{no_path_code}）",
            graph.own_epoch(),
            graph.history().bumps(),
            graph.history().len()
        ),
    );
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "version": "v1.1.8",
        "agents_in_kernel": kernel.agent_count(),
        "newly_registered": newly_registered,
        "announcements_sent": sent,
        "send_failures": send_failures,
        "pump": pump_report.to_value(),
        "tampered_rejected": tampered_rejected,
        "graph": graph.to_value(),
        "perf": perf_summary,
        "query_cache_hit": cache_hit,
        "versions": version_summary,
        "version_bump_applied": bumped.is_applied(),
        "idempotent_ignored": idempotent_ignored,
        "stale_rejected": stale_rejected,
        "queries": [
            {"skill": "translate.en-zh", "best": best_translate, "result": translate_hits.to_value()},
            {"skill": "sentiment.analyze", "best": best_sentiment, "result": sentiment_hits.to_value()},
        ],
        "index_consistent": index_consistent,
        "plan": outcome.to_value(),
        "no_path_demo": reversed_outcome.to_value(),
        "bounded_cache": {
            "capacity": 2,
            "ttl_ticks": 30,
            "retained": bounded_retained,
            "evictions": bounded_stats.evictions,
            "expired_at_tick_100": bounded_expired,
            "retained_after_expiry": bounded.neighbor_count(),
        },
        "rejected": rejected,
        "events": 1,
    }))
}
