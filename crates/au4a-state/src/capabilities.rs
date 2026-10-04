//! 能力清单（v1.3.9）：**文档里每条声明都必须指向实现与自检**。
//!
//! 参考项目最常见的失败模式不是「没做」，而是「文档声称的能力在代码里不可达」。
//! 这里把「声称」变成代码里的一个常量表，再用一次交叉检查把两边钉死：
//!
//! * 每条能力有：id、版本、证据等级、实现入口（api）、覆盖它的测试名、对应的自检名。
//! * [`coverage_check`] 断言：**每个声明的自检名都真实存在且通过**、
//!   并且**每个自检项都被某条能力认领**（没有孤儿自检，也没有空头声明）。
//!
//! 于是「文档撒谎」不再是一种可能性：要么交叉检查通过，要么测试红。

use au4a_core::{CoreResult, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 一条能力声明。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// 稳定的能力 id（点分小写）。
    pub id: &'static str,
    /// 一句话说明。
    pub title: &'static str,
    /// 从哪个小版本起可用。
    pub since: &'static str,
    /// 证据等级：`verified` / `cpu-proto` / `unverified`。
    pub grade: &'static str,
    /// 实现入口（crate 内的路径）。
    pub api: &'static str,
    /// 覆盖它的测试函数名。
    pub test: &'static str,
    /// 对应的自检项名（必须出现在 [`crate::self_check`] 里）。
    pub check: &'static str,
    /// 补充说明（原型在哪里、边界在哪里）。
    pub note: &'static str,
}

/// 轨道 1.3 的全部能力声明。新增能力必须同时补上测试与自检，否则覆盖检查会红。
pub const CAPABILITIES: [Capability; 21] = [
    Capability {
        id: "state.snapshot.three_zones",
        title: "文件系统/内存/上下文三区状态可被冻结为快照",
        since: "v1.3.1",
        grade: "verified",
        api: "snapshot::StateSnapshot::capture",
        test: "all_three_zones_survive_capture",
        check: "snapshot.three_zones",
        note: "空区也有确定的 zone_root，因此「三区完整」可被断言",
    },
    Capability {
        id: "state.snapshot.content_addressed",
        title: "快照内容寻址：插入序不影响 root，root 覆盖 node/epoch 与全部块摘要",
        since: "v1.3.1",
        grade: "verified",
        api: "snapshot::StateSnapshot::capture / content_root",
        test: "a_snapshot_is_content_addressed_and_insertion_order_free",
        check: "snapshot.content_addressed",
        note: "root=文档标识（含出处），content_root=状态标识（不含出处），迁移验收用后者",
    },
    Capability {
        id: "state.snapshot.tamper_refused",
        title: "被替换/篡改的快照必须被拒",
        since: "v1.3.1",
        grade: "verified",
        api: "snapshot::StateSnapshot::from_value / verify",
        test: "a_replaced_snapshot_is_refused",
        check: "snapshot.tamper_refused",
        note: "块摘要与 root 双向校验，改一个字节即失败",
    },
    Capability {
        id: "store.state_store_trait",
        title: "分布式状态存储契约（put/get/list/remove）与本地内存实现",
        since: "v1.3.1",
        grade: "verified",
        api: "store::StateStore / MemoryStore",
        test: "store_roundtrip_is_byte_identical",
        check: "store.roundtrip",
        note: "主网换实现即可；本版只提供本地内存实现（无 I/O）",
    },
    Capability {
        id: "delta.three_zone_set_del",
        title: "块级增量 diff：三区 set/del，且能重建目标内容根",
        since: "v1.3.2",
        grade: "verified",
        api: "diff::StateDelta::between / apply_to",
        test: "diff_covers_all_three_zones_with_set_and_del",
        check: "delta.three_zone_set_del",
        note: "块级而非字节级：应用是幂等的，从而可续跑",
    },
    Capability {
        id: "delta.stale_base_refused",
        title: "base 不是这笔差异的起点时拒绝应用",
        since: "v1.3.2",
        grade: "verified",
        api: "diff::StateDelta::apply_to",
        test: "applying_to_the_wrong_base_is_refused",
        check: "delta.stale_base_refused",
        note: "CoreError::InvalidSignature 承载「过期/竞争」语义，内核侧记 StaleEpoch",
    },
    Capability {
        id: "transfer.resumable",
        title: "跨节点块级传输，断线后从断点续跑",
        since: "v1.3.2",
        grade: "cpu-proto",
        api: "transfer::LocalNetwork / TransferSession / send_chunks",
        test: "a_dropped_transfer_resumes_from_the_break_point",
        check: "transfer.resumable",
        note: "**原型**：本地双节点内存通道，无真实网络/丢包/TLS；分帧与失败注入是真实的",
    },
    Capability {
        id: "signature.tamper_refused",
        title: "快照 Ed25519 签名：篡改与替换都必须被拒",
        since: "v1.3.3",
        grade: "verified",
        api: "signed::SignedSnapshot::sign / verify / verify_policy",
        test: "a_replaced_snapshot_with_a_valid_signature_of_its_own_is_refused_by_policy",
        check: "signature.tamper_refused",
        note: "签名有效 ≠ 是我要的那份：策略层再校验 content_root 与 epoch",
    },
    Capability {
        id: "signature.identity_bound",
        title: "身份绑定：不能替别人签，不能用别人的状态冒充",
        since: "v1.3.3",
        grade: "verified",
        api: "signed::SignedSnapshot::sign / SnapshotPolicy",
        test: "nobody_can_sign_for_someone_else",
        check: "signature.identity_bound",
        note: "签名的主体就是 Agent 自己（不是节点、不是运营方）",
    },
    Capability {
        id: "migration.two_phase_commit",
        title: "两阶段提交迁移：prepare → commit → confirm",
        since: "v1.3.4",
        grade: "verified",
        api: "recovery::Migration / migrate / NodeStore",
        test: "a_happy_path_runs_prepare_commit_confirm_in_order",
        check: "migration.two_phase_commit",
        note: "影子代 + 单点 head 切换；prepare 之后 live 仍是 base",
    },
    Capability {
        id: "migration.rollback_no_partial_state",
        title: "任意阶段失败都回滚，目标节点不留部分状态",
        since: "v1.3.4",
        grade: "verified",
        api: "recovery::Migration::rollback / migrate",
        test: "a_fault_at_every_point_rolls_back_without_partial_state",
        check: "migration.rollback_clean",
        note: "5 个注入点；回滚后逐块等于 base，孤儿代为 0",
    },
    Capability {
        id: "consistency.clean",
        title: "一致性检查：三区摘要相等即为干净",
        since: "v1.3.5",
        grade: "verified",
        api: "integrity::compare / audit_node",
        test: "a_clean_node_audit_reports_no_orphans_or_intent",
        check: "consistency.clean",
        note: "提交后与源一致；回滚后与 base 一致",
    },
    Capability {
        id: "consistency.locates_damage",
        title: "损坏可定位到区 + 键 + 类型（missing/extra/modified）",
        since: "v1.3.5",
        grade: "verified",
        api: "integrity::compare_store / FindingCode",
        test: "store_level_damage_is_found_without_going_through_read_snapshot",
        check: "consistency.locates_damage",
        note: "直接读存储原始字节，不经过 read_snapshot（读不出来本身也是损坏）",
    },
    Capability {
        id: "udos.object_roundtrip",
        title: "UDOS 数据契约：状态对象可被独立复算与还原",
        since: "v1.3.6",
        grade: "verified",
        api: "udos::snapshot_to_object / object_to_snapshot / digest_of",
        test: "a_snapshot_exports_as_a_self_verifying_udos_object",
        check: "udos.object_roundtrip",
        note: "摘要格式 sha256:<hex>、证据等级字符串与 UDOS 一致；不引入依赖、不联网",
    },
    Capability {
        id: "udos.contract_rejects_tampering",
        title: "UDOS 对象被改后自证失败",
        since: "v1.3.6",
        grade: "verified",
        api: "udos::UdosObject::validate",
        test: "tampering_is_caught_by_the_object_id_alone",
        check: "udos.contract_rejects_tampering",
        note: "object_id 覆盖 payload；provenance 冒充在导入时被 InvalidDid 拦下",
    },
    Capability {
        id: "perf.digest_cache_reuse",
        title: "块摘要复用：未变块不再重新哈希（工作量可复算）",
        since: "v1.3.7",
        grade: "verified",
        api: "perf::DigestCache / WorkCounter",
        test: "a_second_capture_of_unchanged_state_does_no_hashing_at_all",
        check: "perf.digest_cache_reuse",
        note: "缓存键含规范 JSON 字节，命中即同一输入；不读墙钟，数字可重放",
    },
    Capability {
        id: "perf.resume_saves_work",
        title: "续跑只补缺口，省下的操作数可被核算",
        since: "v1.3.7",
        grade: "verified",
        api: "perf::measure_transfer / resume_savings",
        test: "resuming_costs_strictly_less_than_restarting",
        check: "perf.resume_saves_work",
        note: "以操作数而非毫秒计量——本轨道禁止计时",
    },
    Capability {
        id: "chain.full_migration",
        title: "全链路一次调用：快照→签名→传输→2PC→一致性→UDOS",
        since: "v1.3.8",
        grade: "verified",
        api: "chain::run_chain",
        test: "the_full_chain_holds_end_to_end",
        check: "chain.full_migration",
        note: "链路报告逐字节可重放；灾难演练与端到端演示共用",
    },
    Capability {
        id: "docs.capabilities_declared",
        title: "能力清单本身可枚举、可交叉检查（文档不撒谎）",
        since: "v1.3.9",
        grade: "verified",
        api: "capabilities::CAPABILITIES / coverage_check",
        test: "coverage_check_passes_and_has_no_orphans",
        check: "docs.capabilities_declared",
        note: "每条声明都指向实现入口、测试名与自检名；孤儿自检也会被发现",
    },
    Capability {
        id: "disaster.drill",
        title: "灾难恢复演练：备份 → 源丢失 → 2PC 重建 → 增量续跑",
        since: "v1.3.10",
        grade: "verified",
        api: "disaster::run_drill / Backup",
        test: "a_full_drill_survives_the_loss_of_the_source",
        check: "disaster.drill",
        note: "重建第一次注入故障 → 回滚 → 重试提交；续跑只补缺口（操作数可核算）",
    },
    Capability {
        id: "disaster.corrupted_backup_refused",
        title: "损坏的备份（或被冒名的备份）在恢复时必须被拒",
        since: "v1.3.10",
        grade: "verified",
        api: "disaster::Backup::{verify,restore_into}",
        test: "a_corrupted_backup_is_refused_at_restore_time",
        check: "disaster.corrupted_backup_refused",
        note: "备份不是文件副本，而是 Agent 签名的内容寻址对象：改一字节即拒",
    },
];

/// 全部能力声明（借用切片）。
pub fn capabilities() -> &'static [Capability] {
    &CAPABILITIES
}

/// 覆盖检查结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    pub capabilities: usize,
    pub checks: usize,
    /// 声明了但自检里不存在的名字。
    pub unknown_checks: Vec<String>,
    /// 自检里有、但没有任何能力认领的名字（孤儿自检）。
    pub unclaimed_checks: Vec<String>,
    /// 声明了但自检没有通过的检查。
    pub failing_checks: Vec<String>,
    /// 等级不在三档之内的声明。
    pub bad_grades: Vec<String>,
}

impl CoverageReport {
    pub fn is_clean(&self) -> bool {
        self.unknown_checks.is_empty()
            && self.unclaimed_checks.is_empty()
            && self.failing_checks.is_empty()
            && self.bad_grades.is_empty()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 交叉检查：能力声明 ↔ 自检项，两个方向都要对得上。
pub fn coverage_check() -> CoreResult<CoverageReport> {
    let checks: Vec<SelfCheck> = crate::self_check();
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    let mut unknown_checks = Vec::new();
    let mut failing_checks = Vec::new();
    let mut bad_grades = Vec::new();
    let mut claimed: Vec<&str> = Vec::new();
    for capability in CAPABILITIES.iter() {
        if !names.contains(&capability.check) {
            unknown_checks.push(capability.check.to_string());
        } else if let Some(check) = checks.iter().find(|c| c.name == capability.check) {
            if !check.passed {
                failing_checks.push(capability.check.to_string());
            }
        }
        if au4a_core::EvidenceGrade::parse(capability.grade).is_none() {
            bad_grades.push(capability.id.to_string());
        }
        claimed.push(capability.check);
    }
    let unclaimed_checks: Vec<String> = names
        .iter()
        .filter(|name| !claimed.contains(name))
        .map(|name| (*name).to_string())
        .collect();
    Ok(CoverageReport {
        capabilities: CAPABILITIES.len(),
        checks: checks.len(),
        unknown_checks,
        unclaimed_checks,
        failing_checks,
        bad_grades,
    })
}

/// 机器可读的能力清单（文档、观察层、发布说明共用一份）。
pub fn manifest() -> Value {
    let items: Vec<Value> = CAPABILITIES
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "title": c.title,
                "since": c.since,
                "grade": c.grade,
                "api": c.api,
                "test": c.test,
                "check": c.check,
                "note": c.note,
            })
        })
        .collect();
    let by_grade = |grade: &str| CAPABILITIES.iter().filter(|c| c.grade == grade).count();
    json!({
        "track": crate::TRACK,
        "title": crate::TITLE,
        "range": crate::RANGE,
        "count": CAPABILITIES.len(),
        "verified": by_grade("verified"),
        "cpu_proto": by_grade("cpu-proto"),
        "unverified": by_grade("unverified"),
        "items": items,
    })
}

/// 只检查声明表自身的形状（不调用 `self_check`，避免递归）。
pub(crate) fn declared_shape_ok() -> bool {
    if CAPABILITIES.is_empty() {
        return false;
    }
    let mut ids: Vec<&str> = CAPABILITIES.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    let unique = ids.windows(2).all(|w| w[0] != w[1]);
    unique
        && CAPABILITIES.iter().all(|c| {
            !c.id.is_empty()
                && !c.title.is_empty()
                && !c.api.is_empty()
                && !c.test.is_empty()
                && !c.check.is_empty()
                && au4a_core::EvidenceGrade::parse(c.grade).is_some()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declaration_points_at_a_real_check() {
        let report = coverage_check().unwrap();
        assert!(report.unknown_checks.is_empty(), "{report:?}");
        assert!(report.failing_checks.is_empty(), "{report:?}");
        assert!(report.bad_grades.is_empty(), "{report:?}");
    }

    #[test]
    fn there_are_no_orphan_checks() {
        let report = coverage_check().unwrap();
        assert!(report.unclaimed_checks.is_empty(), "{report:?}");
        assert_eq!(report.capabilities, report.checks);
    }

    #[test]
    fn the_manifest_reports_the_grade_split_honestly() {
        let value = manifest();
        assert_eq!(value["count"], json!(CAPABILITIES.len()));
        let verified = value["verified"].as_u64().unwrap();
        let proto = value["cpu_proto"].as_u64().unwrap();
        assert_eq!(verified + proto, CAPABILITIES.len() as u64);
        // 至少有一条 cpu-proto（跨节点传输是原型），否则说明等级标注失真。
        assert!(proto >= 1);
        assert_eq!(value["unverified"], json!(0));
    }

    #[test]
    fn declared_shape_is_valid() {
        assert!(declared_shape_ok());
    }
}
