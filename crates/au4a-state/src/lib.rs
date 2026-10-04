//! AU4A 轨道 1.3 — Portable State 可移植状态（v1.3.1 → v1.3.10）。
//!
//! 可移植状态：快照、跨节点传输、签名验证、2PC 恢复、一致性检查、灾难恢复。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责，并留下自检与证据。
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）。
//!
//! v1.3.1 落地的是地基：**三区状态快照 + 内容寻址 + 本地状态存储**。
//! v1.3.2 让状态能**跨节点移动**：块级增量 diff（三区 set/del）+ 可续跑的本地双节点传输
//! （证据等级 `cpu-proto`：语义完整、可重放，但没有真实网络）。
//! 之后每一版都在同一 crate 内增量实现，公共 API 只增不改。

pub mod diff;
pub mod snapshot;
pub mod store;
pub mod transfer;

use au4a_core::{AgentKeys, CoreResult, Did, SelfCheck};
use serde_json::{json, Value};

pub use diff::{DelOp, DeltaChunk, DeltaOp, StateDelta};
pub use snapshot::{
    StateBlock, StateSnapshot, StateZone, MAX_BLOCKS, MAX_KEY_LEN, MAX_NODE_LEN, MAX_VALUE_LEN,
};
pub use store::{
    clear_namespace, read_snapshot, split_store_key, store_key, write_snapshot, MemoryStore,
    StateStore, StoreCounters, ZONE_SEP,
};
pub use transfer::{
    decode_frame, encode_frame, pull_into_session, send_chunks, send_delta, AcceptOutcome,
    LocalNetwork, NetworkStats, NodeId, TransferReport, TransferSession, DEFAULT_CHUNK_OPS,
    MAX_FRAME,
};

/// 轨道号。
pub const TRACK: &str = "1.3";
/// 轨道标题。
pub const TITLE: &str = "Portable State 可移植状态";
/// 版本区间。
pub const RANGE: &str = "v1.3.1 → v1.3.10";

/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_state";

/// 本轨道使用的确定性身份种子：重放需要「同样的种子给同样的结果」。
pub const AGENT_SEED: [u8; 32] = [0x13; 32];

fn track_agent() -> AgentKeys {
    AgentKeys::from_seed(&AGENT_SEED)
}

/// 造一份确定性的三区样例状态（测试、自检、演练共用，避免三处漂移）。
pub fn sample_state() -> CoreResult<Vec<StateBlock>> {
    Ok(vec![
        StateBlock::new(StateZone::Fs, "/work/notes.md", json!({"sha256": "00ff", "bytes": 42}))?,
        StateBlock::new(StateZone::Fs, "/work/plan.json", json!({"steps": [1, 2, 3]}))?,
        StateBlock::new(StateZone::Memory, "last_task", json!("translate.en-zh"))?,
        StateBlock::new(StateZone::Memory, "scratch", json!({"counter": 7}))?,
        StateBlock::new(
            StateZone::Context,
            "goal",
            json!({"text": "从节点 A 迁到节点 B", "priority": 3}),
        )?,
        StateBlock::new(StateZone::Context, "todo", json!(["capture", "sign", "transfer"]))?,
    ])
}

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
///
/// 这里的每一条都是**真实断言**：跑一段代码，看结果，再决定 passed。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = Vec::new();

    // 1) 内容寻址：插入序不影响 root。
    checks.push(check_root_is_order_independent());
    // 2) 三区齐备且可区分。
    checks.push(check_three_zones());
    // 3) 存储往返：写进去再读出来，root 必须一致。
    checks.push(check_store_roundtrip());
    // 4) 篡改必被拒。
    checks.push(check_tamper_refused());
    // 5) v1.3.2：块级增量 diff 覆盖三区 set/del，且能重建目标内容。
    checks.push(check_delta_three_zones());
    // 6) v1.3.2：断线后从断点续跑，不重头开始，也不丢块。
    checks.push(check_transfer_resumable());
    // 7) v1.3.2：base 不对（过期/被换）时拒绝应用。
    checks.push(check_stale_base_refused());

    checks
}

fn check_root_is_order_independent() -> SelfCheck {
    let name = "snapshot.content_addressed";
    let agent = track_agent();
    let ok = (|| -> CoreResult<bool> {
        let forward = StateSnapshot::capture(&agent.did(), "node-a", 1, sample_state()?)?;
        let mut reversed = sample_state()?;
        reversed.reverse();
        let backward = StateSnapshot::capture(&agent.did(), "node-a", 1, reversed)?;
        Ok(forward.root() == backward.root() && forward.verify().is_ok())
    })();
    match ok {
        Ok(true) => SelfCheck::pass(
            TRACK,
            name,
            "正序/逆序捕获得到同一 root；块摘要与快照 root 均自洽",
        ),
        Ok(false) => SelfCheck::fail(TRACK, name, "插入序改变了 root"),
        Err(e) => SelfCheck::fail(TRACK, name, format!("capture 失败: {e}")),
    }
}

fn check_three_zones() -> SelfCheck {
    let name = "snapshot.three_zones";
    let agent = track_agent();
    match sample_snapshot(&agent.did()) {
        Ok(snap) => match snap.zone_roots() {
            Ok(roots) => {
                let all_present = roots.iter().all(|(z, _)| snap.block_count(*z) > 0);
                let distinct = roots[0].1 != roots[1].1
                    && roots[1].1 != roots[2].1
                    && roots[0].1 != roots[2].1;
                if all_present && distinct {
                    SelfCheck::pass(
                        TRACK,
                        name,
                        format!("fs/memory/context 三区各有块，root={}", short(snap.root())),
                    )
                } else {
                    SelfCheck::fail(TRACK, name, "三区缺失或 zone root 不互异")
                }
            }
            Err(e) => SelfCheck::fail(TRACK, name, format!("zone_root 失败: {e}")),
        },
        Err(e) => SelfCheck::fail(TRACK, name, format!("capture 失败: {e}")),
    }
}

fn check_store_roundtrip() -> SelfCheck {
    let name = "store.roundtrip";
    let agent = track_agent();
    let mut store = MemoryStore::new();
    let result = (|| -> CoreResult<(String, String, String, String, usize)> {
        let snap = sample_snapshot(&agent.did())?;
        let written = write_snapshot(&mut store, "live:", &snap)?;
        // 同出处读回：文档 root 必须一致。
        let same = read_snapshot(
            &store,
            "live:",
            snap.agent(),
            snap.source_node(),
            snap.epoch(),
        )?;
        // 换节点读回：状态内容必须一致（迁移等价性）。
        let moved = read_snapshot(&store, "live:", snap.agent(), "node-b", snap.epoch())?;
        Ok((
            snap.root().to_string(),
            same.root().to_string(),
            snap.content_root()?,
            moved.content_root()?,
            written,
        ))
    })();
    match result {
        Ok((before, after, cb, ca, written)) if before == after && cb == ca => SelfCheck::pass(
            TRACK,
            name,
            format!("写入 {written} 块；同源 root 一致 {}；换节点 content_root 一致", short(&before)),
        ),
        Ok((before, after, cb, ca, _)) => SelfCheck::fail(
            TRACK,
            name,
            format!("往返不一致: root {before} vs {after}; content {cb} vs {ca}"),
        ),
        Err(e) => SelfCheck::fail(TRACK, name, format!("往返失败: {e}")),
    }
}

fn check_tamper_refused() -> SelfCheck {
    let name = "snapshot.tamper_refused";
    let agent = track_agent();
    let result = (|| -> CoreResult<bool> {
        let snap = sample_snapshot(&agent.did())?;
        let mut value = snap.to_value()?;
        // 替换块内容但保留旧 root：必须被拒。
        if let Some(blocks) = value.get_mut("blocks").and_then(|b| b.as_array_mut()) {
            if let Some(first) = blocks.first_mut() {
                first["value"] = json!({"tampered": true});
            }
        }
        Ok(StateSnapshot::from_value(&value).is_err())
    })();
    match result {
        Ok(true) => SelfCheck::pass(TRACK, name, "替换一个块的 value 后 from_value 返回 Err"),
        Ok(false) => SelfCheck::fail(TRACK, name, "篡改后的快照被接受"),
        Err(e) => SelfCheck::fail(TRACK, name, format!("篡改检测失败: {e}")),
    }
}

fn sample_snapshot(agent: &Did) -> CoreResult<StateSnapshot> {
    StateSnapshot::capture(agent, "node-a", 1, sample_state()?)
}

/// v1.3.2 演进后的样例状态：三区都有改动，用来证明 diff 不是只看一个区。
fn evolved_state() -> CoreResult<Vec<StateBlock>> {
    Ok(vec![
        // fs：改一块、删一块
        StateBlock::new(StateZone::Fs, "/work/notes.md", json!({"sha256": "11aa", "bytes": 77}))?,
        // memory：改一块、加一块
        StateBlock::new(StateZone::Memory, "last_task", json!("summarize.zh-en"))?,
        StateBlock::new(StateZone::Memory, "plan_version", json!(2))?,
        // context：改一块、加一块
        StateBlock::new(
            StateZone::Context,
            "goal",
            json!({"text": "已经在节点 B 上继续", "priority": 5}),
        )?,
        StateBlock::new(StateZone::Context, "done", json!(["capture", "transfer"]))?,
    ])
}

fn check_delta_three_zones() -> SelfCheck {
    let name = "delta.three_zone_set_del";
    let agent = track_agent();
    let result = (|| -> CoreResult<(Value, bool, String, String)> {
        let from = sample_snapshot(&agent.did())?;
        let to = StateSnapshot::capture(&agent.did(), "node-a", 1, evolved_state()?)?;
        let delta = StateDelta::between(&from, &to)?;
        let applied = delta.apply_to(&from)?;
        let same = applied.content_root()? == to.content_root()?;
        Ok((delta.per_zone(), same, to.content_root()?, applied.content_root()?))
    })();
    match result {
        Ok((per_zone, true, expect, _got)) => {
            let has_fs = per_zone["fs"]["set"].as_u64().unwrap_or(0)
                + per_zone["fs"]["del"].as_u64().unwrap_or(0)
                > 0;
            let has_mem = per_zone["memory"]["set"].as_u64().unwrap_or(0)
                + per_zone["memory"]["del"].as_u64().unwrap_or(0)
                > 0;
            let has_ctx = per_zone["context"]["set"].as_u64().unwrap_or(0)
                + per_zone["context"]["del"].as_u64().unwrap_or(0)
                > 0;
            if has_fs && has_mem && has_ctx {
                SelfCheck::pass(
                    TRACK,
                    name,
                    format!("三区均有 set/del；应用后 content_root={}", short(&expect)),
                )
            } else {
                SelfCheck::fail(TRACK, name, format!("有区没有差异: {per_zone}"))
            }
        }
        Ok((_, false, expect, got)) => SelfCheck::fail(
            TRACK,
            name,
            format!("应用差异后内容不一致: {expect} != {got}"),
        ),
        Err(e) => SelfCheck::fail(TRACK, name, format!("diff 失败: {e}")),
    }
}

fn check_transfer_resumable() -> SelfCheck {
    let name = "transfer.resumable";
    let agent = track_agent();
    let result = (|| -> CoreResult<(usize, usize, usize, String, String)> {
        let from = sample_snapshot(&agent.did())?;
        let to = StateSnapshot::capture(&agent.did(), "node-a", 1, evolved_state()?)?;
        let delta = StateDelta::between(&from, &to)?;
        let chunks = delta.chunked(2)?;
        let na = NodeId::new("node-a")?;
        let nb = NodeId::new("node-b")?;
        let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);
        // 第一批 2 块送达；第二批在「断线」中全丢；然后从断点续跑补齐。
        send_chunks(&mut net, &na, &nb, &delta, 2, 0, 2)?;
        let (session, accepted_a, _) = pull_into_session(&mut net, &nb, None)?;
        let resumed_from = session.resume_from();
        send_chunks(&mut net, &na, &nb, &delta, 2, 2, usize::MAX)?;
        let dropped = net.drop_pending(&nb)?;
        send_chunks(&mut net, &na, &nb, &delta, 2, resumed_from, usize::MAX)?;
        let (session, accepted_b, duplicates) =
            pull_into_session(&mut net, &nb, Some(session))?;
        let rebuilt = session.assemble(chunks.len(), agent.did())?;
        let applied = rebuilt.apply_to(&from)?;
        Ok((
            dropped,
            accepted_a + accepted_b,
            duplicates as usize,
            to.content_root()?,
            applied.content_root()?,
        ))
    })();
    match result {
        Ok((dropped, accepted, duplicates, expect, got)) if expect == got => SelfCheck::pass(
            TRACK,
            name,
            format!(
                "丢 {dropped} 帧、续收 {accepted} 块、重复 {duplicates} 块后内容根一致 {}",
                short(&expect)
            ),
        ),
        Ok((dropped, accepted, _, expect, got)) => SelfCheck::fail(
            TRACK,
            name,
            format!("续跑后不一致（丢 {dropped}，续收 {accepted}）: {expect} != {got}"),
        ),
        Err(e) => SelfCheck::fail(TRACK, name, format!("传输失败: {e}")),
    }
}

fn check_stale_base_refused() -> SelfCheck {
    let name = "delta.stale_base_refused";
    let agent = track_agent();
    let result = (|| -> CoreResult<(bool, bool)> {
        let from = sample_snapshot(&agent.did())?;
        let to = StateSnapshot::capture(&agent.did(), "node-a", 1, evolved_state()?)?;
        let delta = StateDelta::between(&from, &to)?;
        // 用「目标」当 base：起点不对，必须拒绝（StaleEpoch 语义 → CoreError::InvalidSignature）。
        let wrong_base = delta.apply_to(&to).is_err();
        // 用正确的 base：必须成功。
        let right_base = delta.apply_to(&from).is_ok();
        Ok((wrong_base, right_base))
    })();
    match result {
        Ok((true, true)) => SelfCheck::pass(
            TRACK,
            name,
            "错误 base 返回 Err；正确 base 应用成功",
        ),
        Ok((wrong, right)) => SelfCheck::fail(
            TRACK,
            name,
            format!("stale={wrong} correct={right}（期望 true/true）"),
        ),
        Err(e) => SelfCheck::fail(TRACK, name, format!("diff 失败: {e}")),
    }
}

fn short(s: &str) -> String {
    s.chars().take(16).collect()
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    let agent = track_agent();
    let snap = sample_snapshot(&agent.did())?;
    let mut store = MemoryStore::new();
    write_snapshot(&mut store, "live:", &snap)?;
    let restored = read_snapshot(
        &store,
        "live:",
        snap.agent(),
        snap.source_node(),
        snap.epoch(),
    )?;
    let moved = read_snapshot(&store, "live:", snap.agent(), "node-b", snap.epoch())?;
    let zone_roots: Vec<Value> = snap
        .zone_roots()?
        .into_iter()
        .map(|(z, r)| json!({"zone": z.as_str(), "root": r, "blocks": snap.block_count(z)}))
        .collect();
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "agent": snap.agent().as_str(),
        "root": snap.root(),
        "restored_root": restored.root(),
        "content_root": snap.content_root()?,
        "moved_content_root": moved.content_root()?,
        "blocks": snap.blocks().len(),
        "zones": zone_roots,
        "store_keys": store.len(),
        "checks": self_check().len(),
    }))
}

/// 端到端自有流程（v1.3.2）：注册 Agent → 冻结三区快照 → 落库读回 →
/// 状态演进 → 块级增量 diff → 双节点传输（中途断线）→ 从断点续跑 → 目标节点重建 → 篡改拒绝。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
pub fn scenario(kernel: &mut au4a_kernel::Kernel) -> CoreResult<Value> {
    let keys = track_agent();
    let did = keys.did();
    if kernel.card(&did).is_none() {
        kernel.register(&keys, "state-carrier", &["state.snapshot"], au4a_core::Credits(20))?;
    }
    let epoch = kernel.tick();

    // 1) 源节点 A 上的状态，冻结并落库。
    let before = StateSnapshot::capture(&did, "node-a", epoch, sample_state()?)?;
    let mut store = MemoryStore::new();
    let written = write_snapshot(&mut store, "live:", &before)?;
    // 同源读回（文档 root 必须一致）与跨节点读回（状态内容必须一致）。
    let restored = read_snapshot(&store, "live:", &did, "node-a", epoch)?;
    let moved = read_snapshot(&store, "live:", &did, "node-b", epoch)?;

    // 2) Agent 继续工作，状态演进（此前 B 上已有 before 的全量）。
    let after = StateSnapshot::capture(&did, "node-a", epoch, evolved_state()?)?;
    let delta = StateDelta::between(&before, &after)?;
    let chunks = delta.chunked(2)?;

    // 3) 跨节点传输：第一批送达 → 第二批「断线」全丢 → 从断点续跑补齐。
    let na = NodeId::new("node-a")?;
    let nb = NodeId::new("node-b")?;
    let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);
    let first_batch = 2usize;
    send_chunks(&mut net, &na, &nb, &delta, 2, 0, first_batch)?;
    let (session, accepted, duplicates) = pull_into_session(&mut net, &nb, None)?;
    let resumed_from = session.resume_from();
    send_chunks(&mut net, &na, &nb, &delta, 2, first_batch, usize::MAX)?;
    let dropped = net.drop_pending(&nb)?;
    let resume = send_chunks(&mut net, &na, &nb, &delta, 2, resumed_from, usize::MAX)?;
    let (session, accepted_resume, duplicates_resume) =
        pull_into_session(&mut net, &nb, Some(session))?;
    let rebuilt = session.assemble(chunks.len(), did.clone())?;
    let target = rebuilt.apply_moved(&before, "node-b", epoch)?;
    let network = net.stats();

    // 4) 篡改路径：必须有拒绝证据，否则「内容寻址」只是口号。
    let mut tampered = before.to_value()?;
    if let Some(first) = tampered
        .get_mut("blocks")
        .and_then(|b| b.as_array_mut())
        .and_then(|b| b.first_mut())
    {
        first["value"] = json!({"tampered": true});
    }
    let tamper_rejected = StateSnapshot::from_value(&tampered).is_err();
    // 被换过的 base 也必须被拒（竞争/过期语义 → StaleEpoch）。
    let stale_refused = delta.apply_to(&after).is_err();

    kernel.emit(
        &format!("{TRACK}.snapshot"),
        format!(
            "agent={} blocks={} root={}",
            au4a_core::short_id(did.as_str()),
            before.blocks().len(),
            short(before.root())
        ),
    );
    kernel.emit(
        &format!("{TRACK}.transfer"),
        format!(
            "delta_ops={} chunks={} dropped={} resumed_from={} frames={} bytes={}",
            delta.op_count(),
            chunks.len(),
            dropped,
            resumed_from,
            resume.frames,
            resume.bytes
        ),
    );
    kernel.emit(
        &format!("{TRACK}.restore"),
        format!(
            "source={} target={} content_identical={}",
            short(&after.content_root()?),
            short(&target.content_root()?),
            after.same_content(&target)
        ),
    );

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "agent": did.as_str(),
        "epoch": epoch,
        "before_root": before.root(),
        "after_root": target.root(),
        "before_content_root": before.content_root()?,
        "source_content_root": after.content_root()?,
        "target_content_root": target.content_root()?,
        "identical": before.same_state(&restored),
        "migration_identical": after.same_content(&target),
        "moved_content_root": moved.content_root()?,
        "blocks": {
            "fs": before.block_count(StateZone::Fs),
            "memory": before.block_count(StateZone::Memory),
            "context": before.block_count(StateZone::Context),
        },
        "delta": {
            "ops": delta.op_count(),
            "per_zone": delta.per_zone(),
            "id": delta.id()?,
        },
        "transfer": {
            "chunks": chunks.len(),
            "dropped_frames": dropped,
            "resumed_from": resumed_from,
            "resume_frames": resume.frames,
            "resume_bytes": resume.bytes,
            "accepted": accepted + accepted_resume,
            "duplicates": duplicates + duplicates_resume,
            "network_frames_sent": network.frames_sent,
            "network_bytes": network.bytes,
            "evidence_grade": "cpu-proto",
            "note": "本地双节点内存通道；无真实网络/文件 I/O",
        },
        "written": written,
        "store_keys": store.len(),
        "tamper_rejected": tamper_rejected,
        "stale_base_refused": stale_refused,
        "events": 3,
    }))
}

/// 内部自检辅助：快照整体自洽（root + 每块摘要）。库代码不 panic。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_constants_are_stable() {
        assert_eq!(TRACK, "1.3");
        assert_eq!(RANGE, "v1.3.1 → v1.3.10");
        assert_eq!(CRATE, "au4a_state");
    }

    #[test]
    fn self_check_all_passed() {
        let checks = self_check();
        assert!(au4a_core::all_passed(&checks));
        assert_eq!(checks.len(), 7);
    }

    #[test]
    fn results_json_is_consistent() {
        let value = results_json().unwrap();
        assert_eq!(value["root"], value["restored_root"]);
        assert_eq!(value["content_root"], value["moved_content_root"]);
        assert_ne!(value["root"], value["moved_content_root"]);
        assert_eq!(value["blocks"], 6);
    }

    #[test]
    fn scenario_is_deterministic_and_identical() {
        let mut k1 = au4a_kernel::Kernel::new(au4a_kernel::KernelConfig::default());
        let mut k2 = au4a_kernel::Kernel::new(au4a_kernel::KernelConfig::default());
        let a = scenario(&mut k1).unwrap();
        let b = scenario(&mut k2).unwrap();
        assert_eq!(a, b);
        assert_eq!(a["identical"], json!(true));
        assert_eq!(a["migration_identical"], json!(true));
        assert_eq!(a["tamper_rejected"], json!(true));
        assert_eq!(a["stale_base_refused"], json!(true));
        // 迁移后「内容一致」，但出处（node-b）不同 → 文档 root 必须不同。
        assert_eq!(a["source_content_root"], a["target_content_root"]);
        assert_ne!(a["before_root"], a["after_root"]);
        assert_eq!(a["transfer"]["resumed_from"], json!(2));
        assert_eq!(a["transfer"]["dropped_frames"], json!(2));
        k1.ledger().check_conservation().unwrap();
    }

    #[test]
    fn scenario_is_idempotent_on_a_reused_kernel() {
        let mut k = au4a_kernel::Kernel::new(au4a_kernel::KernelConfig::default());
        let a = scenario(&mut k).unwrap();
        let b = scenario(&mut k).unwrap();
        // 第二次调用逻辑时钟前进 → 出处（epoch）不同，但状态内容必须相同。
        assert_eq!(a["before_content_root"], b["before_content_root"]);
        assert_ne!(a["before_root"], b["before_root"]);
        assert_eq!(k.agent_count(), 1);
    }
}
