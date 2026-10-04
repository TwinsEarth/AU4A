//! v3.x 插件迁移适配器（v1.0.8）：把旧式插件接口**如实映射**到 PMB 能力路由。
//!
//! 参考项目的插件接口长这样（v3.x 风格）：
//!
//! ```json
//! { "name": "notes", "entry": "plugins/notes.wasm",
//!   "hooks": ["on_start", "on_offer", "on_settle"],
//!   "permissions": ["write:progress", "settle:verified", "net:http"],
//!   "requires_operator_approval": true, "owner": "acme" }
//! ```
//!
//! AU4A 拒绝其中两个东西，适配器必须**显式**处理而不是假装兼容：
//!
//! 1. `requires_operator_approval: true`（以及 `owner:*` 这类运营方授权）→ 适配失败：
//!    人类/运营方不是权限主体，AU4A 里没有这种授权路径（`policy_denied`，属竞争码而非恶意码）。
//! 2. 不受限的宿主权限（`net:*` / `fs:*` / `exec:*`）→ 内核里没有对应能力，
//!    如实记进 `refused_permissions`（`unsupported`），**不静默放行**。
//!
//! 适配产物 [`MigrationPlan`] 是内容寻址的，包含钩子路由、能力授予与拒绝清单；
//! 证据等级恒为 [`EvidenceGrade::CpuProto`]：本模块做的是纯 CPU 语义映射，
//! **不执行任何插件字节码、不读文件、不联网**。

use au4a_core::{
    canonical_hash, AgentKeys, CoreError, CoreResult, Envelope, EvidenceGrade, RefusalCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::permission::{explain, Capability};
use crate::pmb::{self, kinds_ext, MessageClass};
use crate::Kernel;

/// v3.x 插件清单（旧格式）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct V3PluginManifest {
    pub name: String,
    pub entry: String,
    pub hooks: Vec<String>,
    pub permissions: Vec<String>,
    /// 旧格式里最常见的「等人类/运营方批准」开关。
    pub requires_operator_approval: bool,
    /// 旧格式里的归属者字段（元数据；只有声明 `owner:*` 权限时才构成违规）。
    pub owner: Option<String>,
}

impl V3PluginManifest {
    /// 从 JSON 解析。缺字段按空值处理（旧插件经常省略），但名字与入口必须非空。
    pub fn parse(value: &Value) -> CoreResult<Self> {
        let strings = |key: &str| -> Vec<String> {
            value[key]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };
        Ok(Self {
            name: value["name"].as_str().unwrap_or("").to_string(),
            entry: value["entry"].as_str().unwrap_or("").to_string(),
            hooks: strings("hooks"),
            permissions: strings("permissions"),
            requires_operator_approval: value["requires_operator_approval"]
                .as_bool()
                .unwrap_or(false),
            owner: value["owner"].as_str().map(|s| s.to_string()),
        })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "entry": self.entry,
            "hooks": self.hooks,
            "permissions": self.permissions,
            "requires_operator_approval": self.requires_operator_approval,
            "owner": self.owner,
        })
    }

    /// 是否依赖人类/运营方审批。
    pub fn has_operator_dependency(&self) -> bool {
        self.requires_operator_approval
            || self
                .permissions
                .iter()
                .any(|p| p.starts_with("owner:") || p.starts_with("operator:"))
    }
}

/// 迁移拒绝（具名）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason", content = "detail")]
pub enum MigrationRefusal {
    /// 清单没有名字。
    EmptyName,
    /// 清单没有入口。
    MissingEntry,
    /// 钩子无法映射到任何 PMB 消息类型。
    UnsupportedHook(String),
    /// 权限无法映射到任何内核能力。
    UnsupportedPermission(String),
    /// 依赖运营方/人类审批。
    OperatorApprovalRequired,
    /// 同一钩子声明了两次。
    DuplicateHook(String),
    /// 钩子数量超过适配上限。
    TooManyHooks(usize),
    /// 计划无法做内容寻址（规范 JSON 拒绝了载荷）。
    PlanNotEncodable,
}

impl MigrationRefusal {
    pub const ALL_SAMPLES: [MigrationRefusal; 8] = [
        MigrationRefusal::EmptyName,
        MigrationRefusal::MissingEntry,
        MigrationRefusal::UnsupportedHook(String::new()),
        MigrationRefusal::UnsupportedPermission(String::new()),
        MigrationRefusal::OperatorApprovalRequired,
        MigrationRefusal::DuplicateHook(String::new()),
        MigrationRefusal::TooManyHooks(0),
        MigrationRefusal::PlanNotEncodable,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            MigrationRefusal::EmptyName => "empty_name",
            MigrationRefusal::MissingEntry => "missing_entry",
            MigrationRefusal::UnsupportedHook(_) => "unsupported_hook",
            MigrationRefusal::UnsupportedPermission(_) => "unsupported_permission",
            MigrationRefusal::OperatorApprovalRequired => "operator_approval_required",
            MigrationRefusal::DuplicateHook(_) => "duplicate_hook",
            MigrationRefusal::TooManyHooks(_) => "too_many_hooks",
            MigrationRefusal::PlanNotEncodable => "plan_not_encodable",
        }
    }

    /// 映射回类型化拒绝码，保持「恶意 2 码 vs 竞争 8 码」。
    pub fn to_refusal_code(&self) -> RefusalCode {
        match self {
            // 清单本身不合法（畸形）：与 malformed 帧同类。
            MigrationRefusal::EmptyName
            | MigrationRefusal::MissingEntry
            | MigrationRefusal::PlanNotEncodable => RefusalCode::Malformed,
            MigrationRefusal::UnsupportedHook(_) | MigrationRefusal::UnsupportedPermission(_) => {
                RefusalCode::Unsupported
            }
            // 依赖人类审批是**策略**问题，不是恶意：单次只记警告。
            MigrationRefusal::OperatorApprovalRequired => RefusalCode::PolicyDenied,
            MigrationRefusal::DuplicateHook(_) => RefusalCode::Conflict,
            MigrationRefusal::TooManyHooks(_) => RefusalCode::ResourceExhausted,
        }
    }

    pub fn detail(&self) -> String {
        match self {
            MigrationRefusal::UnsupportedHook(hook) => format!("钩子 {hook} 在 PMB 里没有对应消息类型"),
            MigrationRefusal::UnsupportedPermission(perm) => {
                format!("权限 {perm} 在内核能力表里没有对应项")
            }
            MigrationRefusal::DuplicateHook(hook) => format!("钩子 {hook} 被声明了两次"),
            MigrationRefusal::TooManyHooks(count) => format!("钩子数量 {count} 超过适配上限"),
            MigrationRefusal::OperatorApprovalRequired => {
                "清单依赖运营方/人类审批：AU4A 没有这种权限主体".to_string()
            }
            MigrationRefusal::EmptyName => "插件没有名字".to_string(),
            MigrationRefusal::MissingEntry => "插件没有入口".to_string(),
            MigrationRefusal::PlanNotEncodable => {
                "迁移计划无法做内容寻址（规范 JSON 拒绝了载荷）".to_string()
            }
        }
    }
}

/// 适配上限（人类可以设上限，但不能决定谁能迁移）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationLimits {
    pub max_hooks: usize,
}

impl Default for MigrationLimits {
    fn default() -> Self {
        Self { max_hooks: 16 }
    }
}

/// 一条钩子路由：旧钩子 → PMB 消息类型 + 内核能力。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookRoute {
    pub hook: String,
    pub message_kind: String,
    pub class: MessageClass,
    pub capability: Capability,
}

/// 一条权限授予（映射成功）或拒绝（映射失败，记在 `refused_permissions`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    pub declared: String,
    pub capability: Option<Capability>,
    pub note: String,
}

/// 迁移计划：内容寻址、可审计、可执行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub id: String,
    pub plugin: String,
    pub entry: String,
    pub routes: Vec<HookRoute>,
    pub grants: Vec<CapabilityGrant>,
    pub refused_permissions: Vec<MigrationRefusal>,
    /// 恒为 `cpu-proto`：这是纯 CPU 语义映射，不执行插件代码。
    pub evidence: EvidenceGrade,
}

impl MigrationPlan {
    fn payload(&self) -> Value {
        json!({
            "plugin": self.plugin,
            "entry": self.entry,
            "routes": self.routes,
            "grants": self.grants,
            "refused_permissions": self.refused_permissions,
            "evidence": self.evidence.as_str(),
        })
    }

    pub fn compute_id(&self) -> CoreResult<String> {
        canonical_hash(&self.payload())
    }

    pub fn is_intact(&self) -> bool {
        self.compute_id().map(|id| id == self.id).unwrap_or(false)
    }

    /// 是否有权限被拒绝（计划仍可用，但必须让人看见缺口）。
    pub fn is_partial(&self) -> bool {
        !self.refused_permissions.is_empty()
    }

    /// 计划里涉及的全部内核能力（去重，保持首次出现顺序）。
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut out: Vec<Capability> = Vec::new();
        for route in &self.routes {
            if !out.contains(&route.capability) {
                out.push(route.capability);
            }
        }
        for grant in &self.grants {
            if let Some(capability) = grant.capability {
                if !out.contains(&capability) {
                    out.push(capability);
                }
            }
        }
        out
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "plugin": self.plugin,
            "entry": self.entry,
            "evidence": self.evidence.as_str(),
            "routes": self.routes,
            "grants": self.grants,
            "refused": self.refused_permissions.iter().map(|r| json!({
                "reason": r.as_str(),
                "detail": r.detail(),
                "code": r.to_refusal_code().as_str(),
            })).collect::<Vec<Value>>(),
            "partial": self.is_partial(),
            "intact": self.is_intact(),
        })
    }

    pub fn fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self).map_err(|_| CoreError::Encoding)?;
        canonical_hash(&value)
    }
}

/// 旧钩子 → PMB 路由表。**这就是「v3.x 插件接口 → PMB 能力路由」的映射本体**。
pub fn hook_route(hook: &str) -> Option<HookRoute> {
    let (message_kind, capability) = match hook {
        "on_start" | "on_announce" => (kinds_ext::AGENT_CARD, Capability::PublishCard),
        "on_tick" => (kinds_ext::AUTONOMY_HEARTBEAT, Capability::PublishCard),
        "on_progress" => (kinds_ext::PROGRESS_EVENT, Capability::PublishCard),
        "on_offer" => (kinds_ext::NEGOTIATE_OFFER, Capability::Offer),
        "on_decline" => (kinds_ext::NEGOTIATE_DECLINE, Capability::Offer),
        "on_settle" => (kinds_ext::SETTLE_REQUEST, Capability::SettleVerified),
        "on_council_motion" => (kinds_ext::COUNCIL_MOTION, Capability::ProposeMotion),
        "on_council_vote" => (kinds_ext::COUNCIL_VOTE, Capability::VoteInCouncil),
        _ => return None,
    };
    Some(HookRoute {
        hook: hook.to_string(),
        message_kind: message_kind.to_string(),
        class: pmb::classify_kind(message_kind),
        capability,
    })
}

/// 旧权限 → 内核能力。返回 `None` 表示内核里没有这项能力（必须如实记录为拒绝）。
pub fn permission_capability(permission: &str) -> Option<Capability> {
    match permission {
        "write:progress" | "announce:card" => Some(Capability::PublishCard),
        "message:direct" => Some(Capability::DeliverDirect),
        "settle:verified" => Some(Capability::SettleVerified),
        "settle:cpu-proto" => Some(Capability::SettleCpuProto),
        "negotiate:offer" => Some(Capability::Offer),
        "governance:vote" => Some(Capability::VoteInCouncil),
        "governance:propose" => Some(Capability::ProposeMotion),
        "skills:declare" => Some(Capability::ListSkill),
        "stake:withdraw" => Some(Capability::WithdrawStake),
        _ => None,
    }
}

/// 适配：旧清单 → [`MigrationPlan`]。
///
/// 失败是**具名**的（`Err(MigrationRefusal)`），不静默降级；不可映射的权限记进计划里而不是丢弃。
pub fn adapt(
    manifest: &V3PluginManifest,
    limits: &MigrationLimits,
) -> Result<MigrationPlan, MigrationRefusal> {
    if manifest.name.trim().is_empty() {
        return Err(MigrationRefusal::EmptyName);
    }
    if manifest.entry.trim().is_empty() {
        return Err(MigrationRefusal::MissingEntry);
    }
    if manifest.has_operator_dependency() {
        return Err(MigrationRefusal::OperatorApprovalRequired);
    }
    if manifest.hooks.len() > limits.max_hooks {
        return Err(MigrationRefusal::TooManyHooks(manifest.hooks.len()));
    }

    let mut routes: Vec<HookRoute> = Vec::new();
    for hook in &manifest.hooks {
        if routes.iter().any(|r| &r.hook == hook) {
            return Err(MigrationRefusal::DuplicateHook(hook.clone()));
        }
        match hook_route(hook) {
            Some(route) => routes.push(route),
            None => return Err(MigrationRefusal::UnsupportedHook(hook.clone())),
        }
    }

    let mut grants: Vec<CapabilityGrant> = Vec::new();
    let mut refused: Vec<MigrationRefusal> = Vec::new();
    for permission in &manifest.permissions {
        match permission_capability(permission) {
            Some(capability) => grants.push(CapabilityGrant {
                declared: permission.clone(),
                capability: Some(capability),
                note: format!("映射到内核能力 {}", capability.as_str()),
            }),
            None => {
                let refusal = MigrationRefusal::UnsupportedPermission(permission.clone());
                grants.push(CapabilityGrant {
                    declared: permission.clone(),
                    capability: None,
                    note: format!("无对应能力：{}", refusal.detail()),
                });
                refused.push(refusal);
            }
        }
    }

    let mut plan = MigrationPlan {
        id: String::new(),
        plugin: manifest.name.clone(),
        entry: manifest.entry.clone(),
        routes,
        grants,
        refused_permissions: refused,
        // 诚实标注：适配器只做语义映射，绝不执行插件代码。
        evidence: EvidenceGrade::CpuProto,
    };
    plan.id = plan
        .compute_id()
        .map_err(|_| MigrationRefusal::PlanNotEncodable)?;
    Ok(plan)
}

/// 计划执行结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationApplication {
    pub plugin: String,
    pub sent: usize,
    /// 被权限模型拒绝的能力 → 类型化码（fail-closed：拒绝的能力不发送任何消息）。
    pub denied: Vec<(String, RefusalCode)>,
    pub message_ids: Vec<String>,
}

/// 在真实内核上执行计划：先过权限模型，再发真实签名信封。
///
/// **fail-closed**：能力被拒 → 该路由不发消息，并如实记进 `denied`。
pub fn apply_plan(
    kernel: &mut Kernel,
    keys: &AgentKeys,
    plan: &MigrationPlan,
) -> CoreResult<MigrationApplication> {
    if !plan.is_intact() {
        return Err(CoreError::InvalidSignature);
    }
    let did = keys.did();
    let report = explain(kernel, &did)?;
    let mut sent = 0usize;
    let mut denied: Vec<(String, RefusalCode)> = Vec::new();
    let mut message_ids: Vec<String> = Vec::new();

    let mut blocked: Vec<Capability> = Vec::new();
    for capability in plan.capabilities() {
        if let Some(denial) = report.denial_for(capability) {
            blocked.push(capability);
            denied.push((capability.as_str().to_string(), denial.code));
        }
    }

    for route in &plan.routes {
        if blocked.contains(&route.capability) {
            continue;
        }
        let to = if route.class.broadcastable() {
            None
        } else {
            // 点对点的旧钩子没有收件人信息，只发给计划所有者自己以外的最小集合：
            // 由调用方在内核里选；这里发给第一个在册的其它 Agent，找不到就不发。
            kernel
                .agents()
                .map(|card| card.did.clone())
                .find(|other| other != &did)
        };
        if !route.class.broadcastable() && to.is_none() {
            denied.push((route.hook.clone(), RefusalCode::StaleEpoch));
            continue;
        }
        let env = Envelope::new(
            did.clone(),
            to,
            &route.message_kind,
            kernel.now() + 1,
            None,
            json!({
                "migrated_plugin": plan.plugin,
                "plan": plan.id,
                "hook": route.hook,
                "evidence": plan.evidence.as_str(),
            }),
        )?
        .seal(keys)?;
        kernel.send(&env)?;
        sent += 1;
        message_ids.push(env.id.clone());
    }

    kernel.emit(
        "migration.applied",
        format!(
            "plugin={} sent={} denied={} evidence={}",
            plan.plugin,
            sent,
            denied.len(),
            plan.evidence.as_str()
        ),
    );

    Ok(MigrationApplication {
        plugin: plan.plugin.clone(),
        sent,
        denied,
        message_ids,
    })
}

/// 编译期证据：适配器只吃清单与上限，不接触内核、不读文件、不开网络。
const _: fn(&V3PluginManifest, &MigrationLimits) -> Result<MigrationPlan, MigrationRefusal> = adapt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KernelConfig;
    use au4a_core::Credits;

    fn manifest(name: &str, hooks: &[&str], permissions: &[&str]) -> V3PluginManifest {
        V3PluginManifest {
            name: name.to_string(),
            entry: format!("plugins/{name}.wasm"),
            hooks: hooks.iter().map(|s| (*s).to_string()).collect(),
            permissions: permissions.iter().map(|s| (*s).to_string()).collect(),
            requires_operator_approval: false,
            owner: None,
        }
    }

    fn keys(tag: u8) -> AgentKeys {
        AgentKeys::from_seed(&[tag; 32])
    }

    #[test]
    fn a_well_formed_manifest_maps_every_hook_to_a_pmb_route() {
        let plan = adapt(
            &manifest(
                "notes",
                &["on_start", "on_offer", "on_settle", "on_council_vote"],
                &["write:progress", "settle:cpu-proto"],
            ),
            &MigrationLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.routes.len(), 4);
        assert_eq!(plan.routes[0].message_kind, kinds_ext::AGENT_CARD);
        assert_eq!(plan.routes[0].class, MessageClass::Announce);
        assert!(plan.routes[0].class.broadcastable());
        assert_eq!(plan.routes[1].message_kind, kinds_ext::NEGOTIATE_OFFER);
        assert!(!plan.routes[1].class.broadcastable());
        assert_eq!(plan.routes[2].capability, Capability::SettleVerified);
        assert_eq!(plan.routes[3].capability, Capability::VoteInCouncil);
        assert_eq!(plan.grants.len(), 2);
        assert!(plan.grants.iter().all(|g| g.capability.is_some()));
        assert!(!plan.is_partial());
        assert!(plan.is_intact());
        assert_eq!(plan.evidence, EvidenceGrade::CpuProto, "适配器只做 CPU 语义");
        assert_eq!(plan.to_json()["evidence"], "cpu-proto");
    }

    #[test]
    fn operator_dependency_is_refused_not_silently_dropped() {
        let mut bad = manifest("legacy", &["on_start"], &[]);
        bad.requires_operator_approval = true;
        let err = adapt(&bad, &MigrationLimits::default()).unwrap_err();
        assert_eq!(err, MigrationRefusal::OperatorApprovalRequired);
        assert_eq!(err.to_refusal_code(), RefusalCode::PolicyDenied);
        assert!(!err.to_refusal_code().is_misconduct(), "人类审批依赖不是恶意");
        assert!(err.detail().contains("运营方"));

        // owner:* / operator:* 权限同样被视为运营方授权。
        for perm in ["owner:transfer", "operator:approve"] {
            let bad = manifest("scoped", &["on_start"], &[perm]);
            assert!(bad.has_operator_dependency());
            assert_eq!(
                adapt(&bad, &MigrationLimits::default()).unwrap_err(),
                MigrationRefusal::OperatorApprovalRequired
            );
        }
    }

    #[test]
    fn unmappable_permissions_are_recorded_with_a_typed_reason() {
        let plan = adapt(
            &manifest(
                "netty",
                &["on_progress"],
                &["write:progress", "net:http", "fs:read", "exec:shell"],
            ),
            &MigrationLimits::default(),
        )
        .unwrap();
        assert!(plan.is_partial());
        assert_eq!(plan.refused_permissions.len(), 3);
        for refusal in &plan.refused_permissions {
            assert_eq!(refusal.to_refusal_code(), RefusalCode::Unsupported);
            assert!(!refusal.to_refusal_code().is_misconduct());
        }
        assert_eq!(plan.grants.len(), 4, "被拒的权限也留在授予表里（置空 + 说明）");
        assert_eq!(
            plan.grants.iter().filter(|g| g.capability.is_none()).count(),
            3
        );
        assert!(plan.to_json()["partial"].as_bool().unwrap_or(false));
        assert_eq!(plan.to_json()["refused"].as_array().map(|a| a.len()), Some(3));
    }

    #[test]
    fn malformed_manifests_are_malformed_and_unknown_hooks_are_unsupported() {
        let mut no_name = manifest("", &["on_start"], &[]);
        no_name.name = String::new();
        assert_eq!(
            adapt(&no_name, &MigrationLimits::default()).unwrap_err(),
            MigrationRefusal::EmptyName
        );
        assert!(MigrationRefusal::EmptyName.to_refusal_code().is_misconduct());

        let no_entry = V3PluginManifest {
            entry: String::new(),
            ..manifest("x", &["on_start"], &[])
        };
        assert_eq!(
            adapt(&no_entry, &MigrationLimits::default()).unwrap_err(),
            MigrationRefusal::MissingEntry
        );
        assert!(MigrationRefusal::MissingEntry.to_refusal_code().is_misconduct());

        let err = adapt(
            &manifest("odd", &["on_start", "on_teleport"], &[]),
            &MigrationLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err, MigrationRefusal::UnsupportedHook("on_teleport".to_string()));
        assert!(!err.to_refusal_code().is_misconduct());

        let err = adapt(
            &manifest("dup", &["on_start", "on_start"], &[]),
            &MigrationLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err, MigrationRefusal::DuplicateHook("on_start".to_string()));
        assert_eq!(err.to_refusal_code(), RefusalCode::Conflict);

        let err = adapt(
            &manifest("many", &["on_start", "on_tick"], &[]),
            &MigrationLimits { max_hooks: 1 },
        )
        .unwrap_err();
        assert_eq!(err, MigrationRefusal::TooManyHooks(2));
        assert_eq!(err.to_refusal_code(), RefusalCode::ResourceExhausted);
    }

    #[test]
    fn the_plan_is_content_addressed_and_reproducible() {
        let m = manifest("notes", &["on_start", "on_offer"], &["write:progress"]);
        let a = adapt(&m, &MigrationLimits::default()).unwrap();
        let b = adapt(&m, &MigrationLimits::default()).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
        // 改一个字节（钩子顺序）→ 换 id。
        let mut reordered = m.clone();
        reordered.hooks.reverse();
        let c = adapt(&reordered, &MigrationLimits::default()).unwrap();
        assert_ne!(a.id, c.id);
        // 篡改内容 → is_intact 为假。
        let mut tampered = a.clone();
        tampered.entry = "plugins/evil.wasm".to_string();
        assert!(!tampered.is_intact());
    }

    #[test]
    fn parsing_tolerates_missing_legacy_fields_but_keeps_the_essentials() {
        let value = json!({
            "name": "notes",
            "entry": "plugins/notes.wasm",
            "hooks": ["on_start", "on_settle"],
            "permissions": ["settle:verified"]
        });
        let parsed = V3PluginManifest::parse(&value).unwrap();
        assert_eq!(parsed.name, "notes");
        assert_eq!(parsed.hooks.len(), 2);
        assert!(!parsed.requires_operator_approval);
        assert!(parsed.owner.is_none());
        assert_eq!(parsed.to_json()["name"], "notes");
    }

    #[test]
    fn applying_a_plan_sends_real_signed_envelopes_and_fails_closed() {
        let mut kernel = Kernel::new(KernelConfig::default());
        let agent = keys(210);
        let peer = keys(211);
        kernel.register(&agent, "plugin-host", &["x"], Credits(20)).unwrap();
        kernel.register(&peer, "peer", &["y"], Credits(20)).unwrap();

        let plan = adapt(
            &manifest("notes", &["on_start", "on_offer", "on_settle"], &["write:progress"]),
            &MigrationLimits::default(),
        )
        .unwrap();
        let application = apply_plan(&mut kernel, &agent, &plan).unwrap();
        assert_eq!(application.sent, 3);
        assert!(application.denied.is_empty());
        assert_eq!(kernel.queued_envelopes().len(), 3);
        for env in kernel.queued_envelopes() {
            assert!(env.verify().is_ok(), "迁移产生的信封必须真签名");
            assert_eq!(env.from, agent.did());
        }
        assert!(kernel
            .progress_events()
            .iter()
            .any(|e| e.kind == "migration.applied"));
        assert!(kernel.audit().is_clean());
    }

    #[test]
    fn a_plan_touching_denied_capabilities_sends_nothing_for_them() {
        let mut kernel = Kernel::new(KernelConfig::default());
        // 质押恰好等于准入下限 → withdraw_stake 被权限模型拒绝。
        let agent = keys(212);
        kernel.register(&agent, "tight", &["x"], Credits(10)).unwrap();
        let plan = adapt(
            &manifest("tight", &["on_start"], &["stake:withdraw"]),
            &MigrationLimits::default(),
        )
        .unwrap();
        assert!(plan.capabilities().contains(&Capability::WithdrawStake));
        let application = apply_plan(&mut kernel, &agent, &plan).unwrap();
        assert!(application.denied.iter().any(|(cap, code)| {
            cap == "withdraw_stake" && *code == RefusalCode::PolicyDenied
        }));
        // fail-closed：被拒能力对应的路由不发送任何消息；on_start 仍能发（PublishCard 允许）。
        assert_eq!(application.sent, 1);
        assert_eq!(kernel.queued_envelopes().len(), 1);
    }
}
