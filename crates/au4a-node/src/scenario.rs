//! 端到端编排：Agent 自举（自主注册 + 自主声明能力），然后 10 条轨道依次自主运行。
//!
//! 这里**没有人类输入的落点**：Agent 由确定性种子生成身份，自己决定声明什么能力、质押多少；
//! 轨道 `scenario()` 之间的协作也全部通过内核的 PMB 与账本，不经过任何人工审批环节。

use au4a_core::{AgentKeys, CoreResult, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use serde_json::{json, Value};

use crate::{observe_all, run_tracks, TrackOutcome};

/// 默认 Agent 数量（部署验证脚本用 `--agents 8` 与此一致）。
pub const DEFAULT_AGENTS: usize = 8;

/// 可声明能力池（Agent 自选，人类不参与分配）。
pub const SKILLS: [&str; 8] = [
    "translate.en-zh",
    "sentiment.analyse",
    "summarise.zh",
    "code.review",
    "data.clean",
    "image.tag",
    "audio.transcribe",
    "plan.route",
];

/// 确定性身份：同样的序号永远得到同一个 DID，便于重放与对账。
pub fn agent_keys(index: usize) -> AgentKeys {
    let mut seed = [0u8; 32];
    seed[0] = (index as u8).wrapping_add(1);
    seed[30] = 0xA4;
    seed[31] = 0x4A;
    AgentKeys::from_seed(&seed)
}

/// Agent 自举：每个 Agent 自己生成身份、自带质押、自报能力。
pub fn bootstrap(kernel: &mut Kernel, agents: usize) -> CoreResult<Vec<Did>> {
    let mut dids = Vec::with_capacity(agents);
    for i in 0..agents {
        let keys = agent_keys(i);
        let skill = SKILLS[i % SKILLS.len()];
        let stake = Credits(20 + (i as i64) * 5);
        let card = kernel.register(&keys, format!("agent-{i:02}"), &[skill], stake)?;
        dids.push(card.did);
    }
    Ok(dids)
}

/// 跑完整链路：自举 → 10 条轨道 → 返回内核、各轨道结果与 Agent 身份。
pub fn run_full<F>(
    agents: usize,
    mut on_step: F,
) -> CoreResult<(Kernel, Vec<TrackOutcome>, Vec<Did>)>
where
    F: FnMut(&Kernel),
{
    let mut kernel = Kernel::new(KernelConfig::default());
    let dids = bootstrap(&mut kernel, agents)?;
    kernel.emit(
        "scenario.bootstrap",
        format!("{} agents registered autonomously", dids.len()),
    );
    on_step(&kernel);
    let outcomes = run_tracks(&mut kernel, |k| on_step(k));
    kernel.emit(
        "scenario.done",
        format!("{} tracks executed", outcomes.len()),
    );
    on_step(&kernel);
    Ok((kernel, outcomes, dids))
}

/// 端到端摘要（`run --json` 的输出）。
pub fn summary_json(kernel: &Kernel, outcomes: &[TrackOutcome], dids: &[Did]) -> CoreResult<Value> {
    let view = kernel.observe();
    let ok = outcomes.iter().filter(|o| o.ok).count();
    Ok(json!({
        "version": crate::version(),
        "agents": dids.len(),
        "tracks_total": outcomes.len(),
        "tracks_ok": ok,
        "tracks": outcomes.iter().map(|o| json!({
            "track": o.track,
            "title": o.title,
            "ok": o.ok,
            "detail": o.detail,
        })).collect::<Vec<_>>(),
        "messages_delivered": view.messages_delivered,
        "refusal_count": view.refusal_count,
        "ledger_total": view.ledger.total,
        "ledger_minted": view.ledger.minted,
        "ledger_slashed": view.ledger.slashed,
        "deterministic_seeds": true,
        "observe": observe_all(kernel),
    }))
}

/// 只读投影的规范 JSON（观察面板初值）。
pub fn view_json(kernel: &Kernel) -> String {
    au4a_core::canonicalize(&observe_all(kernel)).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_bootstrap_themselves_with_distinct_dids() {
        let mut kernel = Kernel::new(KernelConfig::default());
        let dids = bootstrap(&mut kernel, 8).unwrap();
        assert_eq!(dids.len(), 8);
        let mut sorted = dids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 8, "DIDs must be distinct");
        kernel.ledger().check_conservation().unwrap();
    }

    #[test]
    fn same_seed_same_did() {
        assert_eq!(agent_keys(3).did(), agent_keys(3).did());
        assert_ne!(agent_keys(3).did(), agent_keys(4).did());
    }

    #[test]
    fn full_run_is_reproducible_and_conserves_credits() {
        let (k1, o1, _) = run_full(8, |_| {}).unwrap();
        let (k2, o2, _) = run_full(8, |_| {}).unwrap();
        let s1 = au4a_core::canonicalize(&summary_json(&k1, &o1, &[]).unwrap()).unwrap();
        let s2 = au4a_core::canonicalize(&summary_json(&k2, &o2, &[]).unwrap()).unwrap();
        // observe 里含进度事件，两次运行的轨道自检细节可能不同；比较结构性字段
        assert_eq!(o1.len(), o2.len());
        assert_eq!(
            o1.iter().filter(|o| o.ok).count(),
            o2.iter().filter(|o| o.ok).count()
        );
        assert!(s1.contains("\"tracks_total\":10"));
        assert!(s2.contains("\"tracks_total\":10"));
        k1.ledger().check_conservation().unwrap();
        k2.ledger().check_conservation().unwrap();
    }

    #[test]
    fn summary_reports_all_ten_tracks() {
        let (kernel, outcomes, dids) = run_full(4, |_| {}).unwrap();
        let summary = summary_json(&kernel, &outcomes, &dids).unwrap();
        assert_eq!(summary["tracks_total"], 10);
        assert_eq!(summary["agents"], 4, "自举的 Agent 数就是传进来的 4 个");

        // 观察投影必须覆盖全部自举 Agent。注意：10 条轨道的 `scenario()` 会在这个共享内核上
        // 追加它们自己的 Agent，所以这里的行数**必须 >= 4**，而不是恰好等于 4。
        let rows = summary["observe"]["progress"]["agents"]
            .as_array()
            .unwrap_or_else(|| panic!("progress.agents 必须是数组：{summary}"));
        assert!(
            rows.len() >= dids.len(),
            "观察投影只看到 {} 个 Agent，少于自举的 {} 个",
            rows.len(),
            dids.len()
        );
        for did in &dids {
            assert!(
                rows.iter().any(|row| row["did"] == did.as_str()),
                "观察投影缺少自举 Agent {did}"
            );
        }
        assert_eq!(
            summary["observe"]["progress"]["messages_delivered"], summary["messages_delivered"],
            "投影与摘要必须来自同一个内核状态"
        );
    }
}
