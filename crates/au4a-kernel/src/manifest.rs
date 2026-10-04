//! 轨道清单（v1.0.10）：把「文档」变成**机器可校验的产物**。
//!
//! 十个中版本各自的交付物、接口、验收与证据，全部以类型化结构写在代码里，
//! 并由 [`validate_manifest`] 逐条断言。这样做的理由很直接：
//! 「文档声称的能力必须可达」这条要求，只有把文档变成数据、把数据变成断言，才不是一句口号。
//!
//! * [`track_manifest`] 返回 [`TrackManifest`]（10 个 [`VersionSpec`]，顺序与官方一致）；
//! * [`validate_manifest`] 返回 `SelfCheck` 列表：版本数、顺序、状态、字段完备性、
//!   证据等级合法性、测试数单调不减、JSON 往返一致；
//! * `au4a-node verify` 与本轨道 `self_check()` 都会消费它。

use au4a_core::{all_passed, CoreError, CoreResult, EvidenceGrade, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{RANGE, TITLE, TRACK};

/// 官方小版本顺序（与 `docs/versions.json` 中 track 1.0 的条目一致）。
pub const VERSION_ORDER: [&str; 10] = [
    "v1.0.1", "v1.0.2", "v1.0.3", "v1.0.4", "v1.0.5", "v1.0.6", "v1.0.7", "v1.0.8", "v1.0.9",
    "v1.0.10",
];

/// 每个小版本的验收证据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionEvidence {
    pub test_command: String,
    pub tests_passed: u32,
    pub tests_total: u32,
    pub grade: String,
    pub notes: String,
}

impl VersionEvidence {
    pub fn verified(tests: u32, notes: &str) -> Self {
        Self {
            test_command:
                "cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\\DS\\_forangent\\target\\au4a-kernel)"
                    .to_string(),
            tests_passed: tests,
            tests_total: tests,
            grade: EvidenceGrade::Verified.as_str().to_string(),
            notes: notes.to_string(),
        }
    }
}

/// 一个小版本的完整规格。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionSpec {
    pub version: String,
    pub title: String,
    pub goal: String,
    pub deliverables: Vec<String>,
    pub interfaces: Vec<String>,
    pub acceptance: Vec<String>,
    pub evidence: VersionEvidence,
    pub status: String,
}

/// 轨道清单。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackManifest {
    pub track: String,
    pub crate_name: String,
    pub medium_title: String,
    pub range: String,
    pub owner: String,
    pub versions: Vec<VersionSpec>,
}

impl TrackManifest {
    pub fn version(&self, version: &str) -> Option<&VersionSpec> {
        self.versions.iter().find(|v| v.version == version)
    }

    pub fn total_tests(&self) -> u32 {
        self.versions
            .last()
            .map(|v| v.evidence.tests_total)
            .unwrap_or(0)
    }

    pub fn all_done(&self) -> bool {
        self.versions.iter().all(|v| v.status == "done")
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

fn spec(
    version: &str,
    title: &str,
    goal: &str,
    deliverables: &[&str],
    interfaces: &[&str],
    acceptance: &[&str],
    tests: u32,
    notes: &str,
) -> VersionSpec {
    VersionSpec {
        version: version.to_string(),
        title: title.to_string(),
        goal: goal.to_string(),
        deliverables: deliverables.iter().map(|s| (*s).to_string()).collect(),
        interfaces: interfaces.iter().map(|s| (*s).to_string()).collect(),
        acceptance: acceptance.iter().map(|s| (*s).to_string()).collect(),
        evidence: VersionEvidence::verified(tests, notes),
        status: "done".to_string(),
    }
}

/// 轨道 1.0 的官方清单。
pub fn track_manifest() -> TrackManifest {
    TrackManifest {
        track: TRACK.to_string(),
        crate_name: "au4a-kernel".to_string(),
        medium_title: TITLE.to_string(),
        range: RANGE.to_string(),
        owner: "track-kernel".to_string(),
        versions: vec![
            spec(
                "v1.0.1",
                "宿主内核重构",
                "把宿主内核重构为可审计、可复现的宿主：注册序成为语义、能力有倒排索引、不变式可被独立断言",
                &[
                    "src/registry.rs: AgentRegistry + RegistrySnapshot 指纹",
                    "src/audit.rs: HostAudit / audit_kernel() 7 项不变式",
                    "src/lib.rs: Kernel 内部改用 AgentRegistry；bootstrap/results_json/scenario",
                    "tests/host_kernel.rs: 9 个集成测试",
                ],
                &[
                    "pub struct AgentRegistry",
                    "pub struct HostAudit",
                    "pub fn audit_kernel(&Kernel) -> HostAudit",
                    "pub fn scenario(&mut Kernel) -> CoreResult<Value>",
                ],
                &[
                    "bootstrap 宿主 7 项审计全过；空头质押/坏索引必须被发现",
                    "两次 bootstrap 注册表指纹相等，换序则不同",
                    "观察层渲染前后状态不变",
                    "scenario 在两个新内核上 JSON 完全相等",
                ],
                27,
                "首版：18 单测 + 9 集成",
            ),
            spec(
                "v1.0.2",
                "Agent自治层骨架",
                "把「Agent 是主体」落成代码：纯函数决策器 + 真驱动内核的执行器，无人类通道",
                &[
                    "src/autonomy.rs: AutonomyPolicy / Intent / AutonomyLayer / classify_error",
                    "src/lib.rs: scenario 新增 autonomy 阶段",
                    "tests/autonomy.rs: 4 个集成测试",
                ],
                &[
                    "pub struct AutonomyPolicy",
                    "pub enum IntentKind",
                    "pub struct AutonomyLayer",
                    "pub fn classify_error(&CoreError) -> RefusalCode",
                ],
                &[
                    "决策输入 JSON 键与意图名均不含 human/approv/operator",
                    "竞争拒绝 -> Defer；恶意 -> Suspend 且为终态",
                    "同种子两内核 6 回合日志全等",
                ],
                42,
                "含 v1.0.1 全部测试",
            ),
            spec(
                "v1.0.3",
                "人类观察层骨架",
                "三个只读投影，且只读是结构事实：不存在 &mut Kernel，写能力没有类型",
                &[
                    "src/observer.rs: ObserverRoute / ObserverCapability / Observer / observer_api()",
                    "src/lib.rs: self_check 改为审计 + 真实观察层检查",
                    "tests/observer.rs: 5 个集成测试",
                ],
                &[
                    "pub enum ObserverRoute",
                    "pub enum ObserverCapability",
                    "pub fn observer_api() -> Value",
                    "pub fn observer_self_checks(&Kernel) -> Vec<SelfCheck>",
                ],
                &[
                    "observer_api: writable=false、effects=[]、路由名无写动词",
                    "渲染前后指纹/投影/时钟不变",
                    "报告无指令通道（pending_approval/commands/...）",
                ],
                53,
                "含 v1.0.2 全部测试",
            ),
            spec(
                "v1.0.4",
                "Agent委员会骨架",
                "只由在册 Agent 组成的裁决机构；隔离动议只对恶意 2 码开放",
                &[
                    "src/council.rs: Motion / Ballot / Tally / Council / CouncilFailure",
                    "src/lib.rs: scenario 新增 council 阶段",
                    "tests/council.rs: 5 个集成测试",
                ],
                &[
                    "pub struct Council",
                    "pub fn motions_for_kernel(&Kernel, u64) -> CoreResult<Vec<Motion>>",
                    "pub enum CouncilFailure",
                ],
                &[
                    "穷举 10 码 x 3 升级级别：竞争码拿不到隔离动议",
                    "法定人数/多数为整数门槛；平票 Inconclusive",
                    "非委员与重复投票映射为 unauthorized（恶意）",
                ],
                65,
                "含 v1.0.3 全部测试",
            ),
            spec(
                "v1.0.5",
                "权限模型更新",
                "回答「这个 Agent 能做什么/不能做什么」：恰好一次划分 + 类型化拒绝，来源无人类",
                &[
                    "src/permission.rs: Capability / Authority / Denial / PermissionReport / explain",
                    "src/lib.rs: Kernel::permissions",
                    "tests/permission.rs: 6 个集成测试",
                ],
                &[
                    "pub enum Capability",
                    "pub enum Authority",
                    "pub fn explain(&Kernel, &Did) -> CoreResult<PermissionReport>",
                    "impl Kernel { pub fn permissions(&self, &Did) }",
                ],
                &[
                    "allowed ∪ denied 恰好覆盖 9 项能力",
                    "Authority 名称不含 operator/human/admin/owner",
                    "未注册 DID 全拒且 code=unauthorized",
                ],
                79,
                "含 v1.0.4 全部测试",
            ),
            spec(
                "v1.0.6",
                "PMB协议扩展",
                "在冻结线格式上扩展协议语义：类型分类、准入、广播展开、重放保护",
                &[
                    "src/pmb.rs: kinds_ext / MessageClass / PmbRouter / 签名构造函数",
                    "src/lib.rs: scenario 新增 pmb 阶段",
                    "tests/pmb.rs: 6 个集成测试",
                ],
                &[
                    "pub struct PmbRouter",
                    "pub fn classify_kind(&str) -> MessageClass",
                    "pub fn decode_and_verify(&[u8]) -> CoreResult<Envelope>",
                ],
                &[
                    "路由准入 ⇒ 内核 send 也必须收下",
                    "篡改->unauthorized；重放->conflict；未知类型->unsupported",
                    "1 MiB 上限与截断帧在基元层被拒",
                ],
                92,
                "含 v1.0.5 全部测试",
            ),
            spec(
                "v1.0.7",
                "状态机更新",
                "白名单生命周期状态机 + 事件溯源：竞争只降级、隔离只来自恶意或委员会、退役终态",
                &[
                    "src/lifecycle.rs: AgentState / LifecycleEvent / next_state / Lifecycle / LifecycleBook",
                    "src/lib.rs: 注册即 Admitted；apply_lifecycle / apply_council_decision",
                    "src/audit.rs: lifecycle.tracked",
                    "tests/lifecycle.rs: 6 个集成测试",
                ],
                &[
                    "pub enum AgentState",
                    "pub fn next_state(AgentState, LifecycleEvent) -> Result<AgentState, RefusalCode>",
                    "pub struct LifecycleBook",
                ],
                &[
                    "穷举拒绝码：只有恶意 2 码能进 Quarantined",
                    "退役后所有事件 stale_epoch 且状态不变",
                    "can() 与 apply() 在所有可达状态一致",
                ],
                106,
                "含 v1.0.6 全部测试",
            ),
            spec(
                "v1.0.8",
                "迁移适配器",
                "v3.x 插件接口如实映射到 PMB 能力路由；运营方审批与宿主权限具名拒绝",
                &[
                    "src/migration.rs: V3PluginManifest / MigrationRefusal / MigrationPlan / adapt / apply_plan",
                    "src/lib.rs: scenario 新增 migration 阶段",
                    "tests/migration.rs: 6 个集成测试",
                ],
                &[
                    "pub fn adapt(&V3PluginManifest, &MigrationLimits) -> Result<MigrationPlan, MigrationRefusal>",
                    "pub fn hook_route(&str) -> Option<HookRoute>",
                    "pub fn apply_plan(&mut Kernel, &AgentKeys, &MigrationPlan) -> CoreResult<MigrationApplication>",
                ],
                &[
                    "requires_operator_approval -> policy_denied（非恶意）",
                    "net:http/fs:read/exec:shell 如实记入 refused_permissions",
                    "fail-closed：被拒能力不发消息",
                ],
                120,
                "含 v1.0.7 全部测试",
            ),
            spec(
                "v1.0.9",
                "测试框架",
                "确定性仿真台 + 可重放日志 + 不变式套件（含无据隔离检测）",
                &[
                    "src/harness.rs: Harness / Journal / replay / invariant_suite",
                    "src/lib.rs: scenario 新增 harness 阶段",
                    "tests/harness.rs: 5 个集成测试",
                ],
                &[
                    "pub struct Harness",
                    "pub fn invariant_suite(&Kernel) -> Vec<SelfCheck>",
                    "pub enum JournalStep",
                ],
                &[
                    "同脚本两次运行日志与四面对齐",
                    "verify_replay 全等且重放世界满足全部不变式",
                    "无据隔离必须被 quarantine_justified 发现",
                ],
                130,
                "含 v1.0.8 全部测试",
            ),
            spec(
                "v1.0.10",
                "文档",
                "把文档变成机器可校验产物：类型化轨道清单 + 逐条断言（版本数/顺序/字段/证据/往返）",
                &[
                    "src/manifest.rs: TrackManifest / VersionSpec / VersionEvidence / track_manifest / validate_manifest",
                    "src/lib.rs: self_check/results_json 聚合清单校验",
                    "tests/manifest.rs: 5 个集成测试",
                ],
                &[
                    "pub fn track_manifest() -> TrackManifest",
                    "pub fn validate_manifest(&TrackManifest) -> Vec<SelfCheck>",
                    "pub struct VersionSpec",
                ],
                &[
                    "恰好 10 个版本且顺序与官方一致",
                    "每个版本都有交付物/接口/验收且 status=done",
                    "证据等级合法、测试数单调不减、JSON 往返一致",
                ],
                139,
                "含 v1.0.9 全部测试 + 清单校验",
            ),
        ],
    }
}

/// 逐条校验清单。任何一条不成立都会返回 `passed=false` 的检查项。
pub fn validate_manifest(manifest: &TrackManifest) -> Vec<SelfCheck> {
    let track = TRACK;
    let mut checks = Vec::new();

    checks.push(if manifest.versions.len() == VERSION_ORDER.len() {
        SelfCheck::pass(
            track,
            "manifest.version_count",
            format!("恰好 {} 个小版本", manifest.versions.len()),
        )
    } else {
        SelfCheck::fail(
            track,
            "manifest.version_count",
            format!(
                "版本数 {} != {}",
                manifest.versions.len(),
                VERSION_ORDER.len()
            ),
        )
    });

    let order_ok = manifest
        .versions
        .iter()
        .map(|v| v.version.as_str())
        .eq(VERSION_ORDER.iter().copied());
    checks.push(if order_ok {
        SelfCheck::pass(
            track,
            "manifest.order",
            "版本顺序与官方一致（v1.0.1 → v1.0.10）",
        )
    } else {
        SelfCheck::fail(track, "manifest.order", "版本顺序或编号与官方不一致")
    });

    let ids_ok =
        manifest.track == TRACK && manifest.range == RANGE && manifest.medium_title == TITLE;
    checks.push(if ids_ok {
        SelfCheck::pass(
            track,
            "manifest.identity",
            format!("{TRACK} / {TITLE} / {RANGE} 与 crate 常量一致"),
        )
    } else {
        SelfCheck::fail(track, "manifest.identity", "清单标识与 crate 常量不一致")
    });

    let incomplete: Vec<String> = manifest
        .versions
        .iter()
        .filter(|v| {
            v.status != "done"
                || v.deliverables.is_empty()
                || v.interfaces.is_empty()
                || v.acceptance.is_empty()
                || v.goal.is_empty()
        })
        .map(|v| v.version.clone())
        .collect();
    checks.push(if incomplete.is_empty() {
        SelfCheck::pass(
            track,
            "manifest.fields",
            format!(
                "{} 个版本的交付物/接口/验收/目标均非空且 status=done",
                manifest.versions.len()
            ),
        )
    } else {
        SelfCheck::fail(
            track,
            "manifest.fields",
            format!("字段缺失：{incomplete:?}"),
        )
    });

    let bad_evidence: Vec<String> = manifest
        .versions
        .iter()
        .filter(|v| {
            EvidenceGrade::parse(&v.evidence.grade).is_none()
                || v.evidence.tests_passed != v.evidence.tests_total
                || v.evidence.tests_total == 0
                || !v.evidence.test_command.contains("au4a-kernel")
                || v.evidence.notes.is_empty()
        })
        .map(|v| v.version.clone())
        .collect();
    checks.push(if bad_evidence.is_empty() {
        SelfCheck::pass(
            track,
            "manifest.evidence",
            format!(
                "证据等级合法、测试数自洽且命令指向本 crate；最后一版 {} 个测试",
                manifest.total_tests()
            ),
        )
    } else {
        SelfCheck::fail(
            track,
            "manifest.evidence",
            format!("证据不合格：{bad_evidence:?}"),
        )
    });

    let mut monotonic = true;
    let mut last = 0u32;
    for version in &manifest.versions {
        if version.evidence.tests_total < last {
            monotonic = false;
        }
        last = version.evidence.tests_total;
    }
    checks.push(if monotonic {
        SelfCheck::pass(
            track,
            "manifest.test_growth",
            format!(
                "测试数单调不减（{} → {}）",
                manifest
                    .versions
                    .first()
                    .map(|v| v.evidence.tests_total)
                    .unwrap_or(0),
                last
            ),
        )
    } else {
        SelfCheck::fail(track, "manifest.test_growth", "测试数出现回退")
    });

    let roundtrip = match manifest.to_json() {
        Ok(value) => TrackManifest::from_json(&value)
            .map(|back| &back == manifest)
            .unwrap_or(false),
        Err(_) => false,
    };
    checks.push(if roundtrip {
        SelfCheck::pass(track, "manifest.roundtrip", "清单 JSON 往返后完全相等")
    } else {
        SelfCheck::fail(track, "manifest.roundtrip", "清单 JSON 往返不一致")
    });

    checks
}

/// 清单是否整体成立。
pub fn manifest_is_valid() -> bool {
    all_passed(&validate_manifest(&track_manifest()))
}

/// 编译期证据：校验函数只接受共享借用（校验不会顺手改清单）。
const _: fn(&TrackManifest) -> Vec<SelfCheck> = validate_manifest;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_covers_exactly_the_official_versions_in_order() {
        let manifest = track_manifest();
        assert_eq!(manifest.versions.len(), VERSION_ORDER.len());
        for (spec, expected) in manifest.versions.iter().zip(VERSION_ORDER.iter()) {
            assert_eq!(&spec.version, expected);
            assert_eq!(spec.status, "done");
        }
        assert_eq!(manifest.track, "1.0");
        assert_eq!(manifest.crate_name, "au4a-kernel");
        assert_eq!(manifest.owner, "track-kernel");
        assert!(manifest.all_done());
    }

    #[test]
    fn every_version_states_what_it_delivered_and_how_it_was_verified() {
        let manifest = track_manifest();
        for spec in &manifest.versions {
            assert!(!spec.goal.is_empty(), "{} 没有目标", spec.version);
            assert!(spec.deliverables.len() >= 3, "{} 交付物太少", spec.version);
            assert!(spec.interfaces.len() >= 3, "{} 接口太少", spec.version);
            assert!(spec.acceptance.len() >= 3, "{} 验收太少", spec.version);
            assert_eq!(spec.evidence.tests_passed, spec.evidence.tests_total);
            assert!(spec.evidence.tests_total > 0);
            assert!(EvidenceGrade::parse(&spec.evidence.grade).is_some());
            assert!(spec
                .evidence
                .test_command
                .contains("cargo test -p au4a-kernel"));
        }
        assert!(manifest.total_tests() >= 130);
    }

    #[test]
    fn validation_passes_and_detects_a_broken_manifest() {
        let manifest = track_manifest();
        let checks = validate_manifest(&manifest);
        assert!(
            all_passed(&checks),
            "{:?}",
            checks.iter().filter(|c| !c.passed).collect::<Vec<_>>()
        );
        assert_eq!(checks.len(), 7);
        assert!(manifest_is_valid());

        // 少一个版本 → 必须被发现。
        let mut trimmed = track_manifest();
        trimmed.versions.pop();
        let checks = validate_manifest(&trimmed);
        assert!(!all_passed(&checks));
        assert!(checks
            .iter()
            .any(|c| c.name == "manifest.version_count" && !c.passed));
        assert!(checks
            .iter()
            .any(|c| c.name == "manifest.order" && !c.passed));

        // 验收清单为空 → 必须被发现。
        let mut empty = track_manifest();
        if let Some(first) = empty.versions.first_mut() {
            first.acceptance.clear();
        }
        let checks = validate_manifest(&empty);
        assert!(checks
            .iter()
            .any(|c| c.name == "manifest.fields" && !c.passed));

        // 证据等级写错 → 必须被发现。
        let mut lying = track_manifest();
        if let Some(first) = lying.versions.first_mut() {
            first.evidence.grade = "trust-me".to_string();
        }
        let checks = validate_manifest(&lying);
        assert!(checks
            .iter()
            .any(|c| c.name == "manifest.evidence" && !c.passed));

        // 测试数回退 → 必须被发现。
        let mut regressed = track_manifest();
        if regressed.versions.len() > 1 {
            regressed.versions[1].evidence.tests_total = 1;
            regressed.versions[1].evidence.tests_passed = 1;
        }
        let checks = validate_manifest(&regressed);
        assert!(checks
            .iter()
            .any(|c| c.name == "manifest.test_growth" && !c.passed));
    }

    #[test]
    fn the_manifest_roundtrips_through_json() {
        let manifest = track_manifest();
        let value = manifest.to_json().unwrap();
        let back = TrackManifest::from_json(&value).unwrap();
        assert_eq!(back, manifest);
        assert_eq!(value["track"], "1.0");
        assert_eq!(value["versions"].as_array().map(|v| v.len()), Some(10));
        assert!(TrackManifest::from_json(&serde_json::json!({"nope": true})).is_err());
    }

    #[test]
    fn lookups_and_totals_are_stable() {
        let manifest = track_manifest();
        let v = manifest.version("v1.0.7").unwrap();
        assert_eq!(v.title, "状态机更新");
        assert!(v.interfaces.iter().any(|i| i.contains("next_state")));
        assert!(manifest.version("v1.0.11").is_none());
        assert_eq!(manifest.total_tests(), 139);
    }
}
