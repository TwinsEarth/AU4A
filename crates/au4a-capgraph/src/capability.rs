//! v1.1.1 —— 能力图的数据结构。
//!
//! 一条「能力」不是一句自我介绍，而是一组**可比较、可约束、可定价**的整数度量。
//! 为什么全部用整数：能力图会被签名、被内容寻址、被拿去和别人比较并决定钱怎么走，
//! 一旦出现浮点，规范 JSON 会直接拒签（`CoreError::FloatForbidden`），
//! 而且「两个 Agent 对同一份能力的哈希」这件事就不再成立。见 `au4a-core::canon` 的规则 3。
//!
//! 度量口径（全部整数，单位写进字段名）：
//!
//! * `latency_p50_ms` / `latency_p99_ms`：毫秒，p99 必须 ≥ p50。
//! * `throughput_per_min`：每分钟可处理单元数。
//! * `current_load_bp`：当前负载，万分比（0..=10_000），10_000 表示满载。
//! * `price_per_unit`：微积分/单元（[`Credits`]，不可为负）。
//! * `reliability_bp`：历史可靠度，万分比（0..=10_000）。
//! * `supported_formats` / `produced_formats`：接受的输入格式 / 产出的输出格式。
//!   路径规划里的格式兼容性就是「上一步产出 ∩ 下一步接受 ≠ ∅」。
//! * `constraints`：硬约束（载荷上限、最小期限、并发、区域）。约束是**拒绝**的理由，
//!   不是建议：不满足的候选在规划阶段就被剔除，而不是等到执行时才发现。
//!
//! 这一层刻意不放进「Agent 是谁」——`Capability` 是纯值类型，谁声明它由上层决定。

use std::collections::BTreeSet;

use au4a_core::{canonical_hash, CoreError, CoreResult, Credits};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

/// 万分比的分母。所有比率字段都是「相对它的整数」。
pub const BP_SCALE: u16 = 10_000;
/// 技能名最大字节数。
pub const MAX_SKILL_LEN: usize = 64;
/// 格式名最大字节数。
pub const MAX_FORMAT_LEN: usize = 64;

/// 技能标识（如 `translate.en-zh`、`sentiment.analyze`）。
///
/// 反序列化也会校验：不合法技能名**不能**通过 JSON 混进图里，
/// 否则「能力图的节点集合」就不再是封闭可枚举的。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SkillId(String);

impl SkillId {
    pub fn new(s: &str) -> CoreResult<Self> {
        if valid_name(s, MAX_SKILL_LEN, true) {
            Ok(Self(s.to_string()))
        } else {
            Err(CoreError::InvalidKind)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkillId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SkillId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        SkillId::new(&raw).map_err(serde::de::Error::custom)
    }
}

/// 内容格式标识（如 `text/plain`、`application/json`、`audio/wav`）。
///
/// 与技能名同构，额外允许 `/` 和 `+`，并强制小写：
/// 「同一份格式」必须在字节层只有一种写法，否则格式兼容性判断会假阴性。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct FormatId(String);

impl FormatId {
    pub fn new(s: &str) -> CoreResult<Self> {
        if valid_format(s) {
            Ok(Self(s.to_string()))
        } else {
            Err(CoreError::InvalidKind)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for FormatId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for FormatId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        FormatId::new(&raw).map_err(serde::de::Error::custom)
    }
}

/// `text/plain` 的常量构造：它是编译期已知合法的字面量，
/// 走 `FormatId::new` 只会在库里多出一个不可能失败的错误分支。
pub(crate) fn text_plain() -> FormatId {
    FormatId(String::from("text/plain"))
}

fn valid_name(s: &str, max: usize, allow_dot_dash_underscore: bool) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > max {
        return false;
    }
    let head = bytes[0];
    if !(head.is_ascii_lowercase() || head.is_ascii_digit()) {
        return false;
    }
    let extra = |b: u8| {
        if allow_dot_dash_underscore {
            b == b'.' || b == b'-' || b == b'_'
        } else {
            false
        }
    };
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || extra(*b))
}

fn valid_format(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_FORMAT_LEN {
        return false;
    }
    let head = bytes[0];
    if !(head.is_ascii_lowercase() || head.is_ascii_digit()) {
        return false;
    }
    bytes.iter().all(|b| {
        b.is_ascii_lowercase()
            || b.is_ascii_digit()
            || matches!(*b, b'.' | b'-' | b'_' | b'/' | b'+')
    })
}

/// 硬约束。这些字段是**可拒绝的条件**，不是建议。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraints {
    /// 单次载荷上限（字节）。
    pub max_input_bytes: u64,
    /// 能接受的最小期限（毫秒）：期限比它更紧的任务直接不接。
    pub min_deadline_ms: u32,
    /// 并发上限（同时处理的单元数）。
    pub max_concurrency: u32,
    /// 允许的区域白名单；空集表示不限区域。
    pub regions: BTreeSet<String>,
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            max_input_bytes: 1 << 20,
            min_deadline_ms: 1,
            max_concurrency: 1,
            regions: BTreeSet::new(),
        }
    }
}

impl Constraints {
    pub fn validate(&self) -> CoreResult<()> {
        if self.max_input_bytes == 0 || self.min_deadline_ms == 0 || self.max_concurrency == 0 {
            return Err(CoreError::Encoding);
        }
        Ok(())
    }

    /// 区域是否被接受：白名单为空表示「不限」。
    pub fn accepts_region(&self, region: Option<&str>) -> bool {
        if self.regions.is_empty() {
            return true;
        }
        match region {
            Some(r) => self.regions.contains(r),
            None => false,
        }
    }

    /// 载荷是否在硬上限内。
    pub fn accepts_payload(&self, bytes: u64) -> bool {
        bytes <= self.max_input_bytes
    }

    /// 期限是否够宽。
    pub fn accepts_deadline(&self, deadline_ms: u32) -> bool {
        deadline_ms >= self.min_deadline_ms
    }
}

/// 一条能力声明。这是能力图的边权与顶点属性。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub skill: SkillId,
    pub latency_p50_ms: u32,
    pub latency_p99_ms: u32,
    pub throughput_per_min: u32,
    /// 当前负载，万分比。
    pub current_load_bp: u16,
    pub price_per_unit: Credits,
    /// 历史可靠度，万分比。
    pub reliability_bp: u16,
    /// 接受的输入格式集合。
    pub supported_formats: BTreeSet<FormatId>,
    /// 产出的输出格式集合（路径规划的兼容性靠它接续）。
    pub produced_formats: BTreeSet<FormatId>,
    pub constraints: Constraints,
}

impl Capability {
    /// 构造一条「合理默认」的能力，再由 `with_*` 覆盖要测的字段。
    ///
    /// 默认值不是魔法：p50=100ms / p99=250ms / 60 单元每分钟 / 负载 0 /
    /// 可靠度 9000bp（90%）/ `text/plain` 进 `text/plain` 出。
    pub fn new(skill: SkillId, price_per_unit: Credits) -> Self {
        let text = text_plain();
        let mut formats = BTreeSet::new();
        formats.insert(text);
        Self {
            skill,
            latency_p50_ms: 100,
            latency_p99_ms: 250,
            throughput_per_min: 60,
            current_load_bp: 0,
            price_per_unit,
            reliability_bp: 9_000,
            supported_formats: formats.clone(),
            produced_formats: formats,
            constraints: Constraints::default(),
        }
    }

    pub fn with_latency(mut self, p50_ms: u32, p99_ms: u32) -> Self {
        self.latency_p50_ms = p50_ms;
        self.latency_p99_ms = p99_ms;
        self
    }

    pub fn with_throughput(mut self, per_min: u32) -> Self {
        self.throughput_per_min = per_min;
        self
    }

    pub fn with_load_bp(mut self, load_bp: u16) -> Self {
        self.current_load_bp = load_bp;
        self
    }

    pub fn with_reliability_bp(mut self, reliability_bp: u16) -> Self {
        self.reliability_bp = reliability_bp;
        self
    }

    pub fn with_price(mut self, price_per_unit: Credits) -> Self {
        self.price_per_unit = price_per_unit;
        self
    }

    pub fn with_constraints(mut self, constraints: Constraints) -> Self {
        self.constraints = constraints;
        self
    }

    /// 设置输入/产出格式。名字非法（大写、空格、空串）就是 `InvalidKind`。
    pub fn with_formats(mut self, inputs: &[&str], outputs: &[&str]) -> CoreResult<Self> {
        let mut supported = BTreeSet::new();
        for f in inputs {
            supported.insert(FormatId::new(f)?);
        }
        let mut produced = BTreeSet::new();
        for f in outputs {
            produced.insert(FormatId::new(f)?);
        }
        self.supported_formats = supported;
        self.produced_formats = produced;
        Ok(self)
    }

    /// 校验：任何一条不成立，这条能力就不允许进入能力图。
    pub fn validate(&self) -> CoreResult<()> {
        if self.latency_p50_ms == 0 || self.latency_p99_ms == 0 {
            return Err(CoreError::Encoding);
        }
        if self.latency_p99_ms < self.latency_p50_ms {
            return Err(CoreError::Encoding);
        }
        if self.throughput_per_min == 0 {
            return Err(CoreError::Encoding);
        }
        if self.current_load_bp > BP_SCALE || self.reliability_bp > BP_SCALE {
            return Err(CoreError::Encoding);
        }
        if self.supported_formats.is_empty() || self.produced_formats.is_empty() {
            return Err(CoreError::Encoding);
        }
        self.constraints.validate()
    }

    /// 剩余容量，万分比。
    pub fn available_bp(&self) -> u16 {
        BP_SCALE - self.current_load_bp
    }

    /// 负载加权后的期望延迟：空载时是 p50，满载时接近 p99。
    ///
    /// 这是整数线性插值，没有浮点，也不会因为「性能优化」而改变结果。
    pub fn effective_latency_ms(&self) -> u64 {
        let spread = u64::from(self.latency_p99_ms - self.latency_p50_ms);
        u64::from(self.latency_p50_ms)
            + spread * u64::from(self.current_load_bp) / u64::from(BP_SCALE)
    }

    /// 负载加权后的可靠度（万分比）：越忙越不可靠。
    pub fn effective_reliability_bp(&self) -> u16 {
        (u32::from(self.reliability_bp) * u32::from(self.available_bp()) / u32::from(BP_SCALE))
            as u16
    }

    /// 是否接受某个输入格式。
    pub fn accepts_format(&self, format: &FormatId) -> bool {
        self.supported_formats.contains(format)
    }

    /// 是否产出某个输出格式。
    pub fn produces_format(&self, format: &FormatId) -> bool {
        self.produced_formats.contains(format)
    }

    /// 两个能力之间是否可以接续：本能力产出 ∩ 下一能力接受 ≠ ∅。
    /// 返回第一个（字典序最小，保证确定性）可接续的格式。
    pub fn handoff_format(&self, next: &Capability) -> Option<FormatId> {
        self.produced_formats
            .intersection(&next.supported_formats)
            .next()
            .cloned()
    }

    /// 规范 JSON 表示（键序固定、整数、集合升序）。
    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 从 JSON 恢复，并**校验**。反序列化不校验的字段（如 p99 < p50）在这里被挡住。
    pub fn from_value(value: &Value) -> CoreResult<Self> {
        let cap: Capability =
            serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)?;
        cap.validate()?;
        Ok(cap)
    }

    /// 内容寻址指纹：同一条能力在任何节点上得到同一个哈希。
    pub fn fingerprint(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 单行摘要，供只读投影使用（不是证据，只是可读性）。
    pub fn summary(&self) -> Value {
        json!({
            "skill": self.skill.as_str(),
            "latency_p50_ms": self.latency_p50_ms,
            "latency_p99_ms": self.latency_p99_ms,
            "throughput_per_min": self.throughput_per_min,
            "current_load_bp": self.current_load_bp,
            "price_per_unit": self.price_per_unit.get(),
            "reliability_bp": self.reliability_bp,
            "supported_formats": self.supported_formats.iter().map(FormatId::as_str).collect::<Vec<_>>(),
            "produced_formats": self.produced_formats.iter().map(FormatId::as_str).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(s: &str) -> SkillId {
        SkillId::new(s).expect("test skill name is valid")
    }

    #[test]
    fn skill_names_are_validated_on_both_paths() {
        assert!(SkillId::new("translate.en-zh").is_ok());
        assert!(SkillId::new("sentiment_2").is_ok());
        for bad in ["", "Upper", "has space", "trailing:", &"a".repeat(65)] {
            assert_eq!(SkillId::new(bad), Err(CoreError::InvalidKind), "{bad}");
        }
        // JSON 路径同样校验：非法技能名不能混进图里。
        let raw = serde_json::json!({"skill": "Bad Name"});
        assert!(serde_json::from_value::<SkillId>(raw).is_err());
    }

    #[test]
    fn format_names_allow_slash_and_plus_but_stay_lowercase() {
        assert!(FormatId::new("application/json").is_ok());
        assert!(FormatId::new("audio/wav").is_ok());
        assert!(FormatId::new("text/plain+utf8").is_ok());
        for bad in ["", "Text/Plain", "a b", "/leading"] {
            assert_eq!(FormatId::new(bad), Err(CoreError::InvalidKind), "{bad}");
        }
    }

    #[test]
    fn validation_rejects_incoherent_metrics() {
        let base = Capability::new(skill("x"), Credits(3));
        assert!(base.validate().is_ok());
        assert_eq!(
            base.clone().with_latency(500, 100).validate(),
            Err(CoreError::Encoding),
            "p99 < p50 必须被拒"
        );
        assert_eq!(
            base.clone().with_throughput(0).validate(),
            Err(CoreError::Encoding)
        );
        assert_eq!(
            base.clone().with_load_bp(10_001).validate(),
            Err(CoreError::Encoding)
        );
        assert_eq!(
            base.clone().with_reliability_bp(10_001).validate(),
            Err(CoreError::Encoding)
        );
        let mut no_formats = base.clone();
        no_formats.supported_formats.clear();
        assert_eq!(no_formats.validate(), Err(CoreError::Encoding));
    }

    #[test]
    fn load_weighted_metrics_are_integer_math() {
        let idle = Capability::new(skill("x"), Credits(1))
            .with_latency(100, 300)
            .with_load_bp(0);
        assert_eq!(idle.effective_latency_ms(), 100);
        assert_eq!(idle.effective_reliability_bp(), 9_000);
        let busy = idle.clone().with_load_bp(5_000);
        assert_eq!(busy.effective_latency_ms(), 200);
        assert_eq!(busy.effective_reliability_bp(), 4_500);
        let full = idle.clone().with_load_bp(10_000);
        assert_eq!(full.effective_latency_ms(), 300);
        assert_eq!(full.effective_reliability_bp(), 0);
        assert_eq!(full.available_bp(), 0);
    }

    #[test]
    fn canonical_value_roundtrip_is_stable_and_float_free() {
        let cap = Capability::new(skill("translate.en-zh"), Credits(7))
            .with_formats(&["text/plain"], &["text/plain", "application/json"])
            .expect("valid formats");
        let v = cap.to_value().expect("serialisable");
        let back = Capability::from_value(&v).expect("valid capability");
        assert_eq!(back, cap);
        assert_eq!(
            back.fingerprint().expect("hashes"),
            cap.fingerprint().expect("hashes")
        );
        // 规范 JSON 想过浮点就会报 FloatForbidden，能成功本身就是「无浮点」的证据。
        let canonical = au4a_core::canonicalize(&v).expect("canonical");
        assert!(!canonical.is_empty());
        for field in [
            "latency_p50_ms",
            "price_per_unit",
            "reliability_bp",
            "current_load_bp",
        ] {
            assert!(v[field].as_i64().is_some(), "{field} 必须是整数");
        }
    }

    #[test]
    fn handoff_format_is_the_intersection() {
        let a = Capability::new(skill("a"), Credits(1))
            .with_formats(&["text/plain"], &["text/plain", "application/json"])
            .expect("valid");
        let b = Capability::new(skill("b"), Credits(1))
            .with_formats(&["application/json"], &["application/json"])
            .expect("valid");
        assert_eq!(
            a.handoff_format(&b).map(|f| f.as_str().to_string()),
            Some("application/json".to_string())
        );
        let c = Capability::new(skill("c"), Credits(1))
            .with_formats(&["audio/wav"], &["audio/wav"])
            .expect("valid");
        assert_eq!(a.handoff_format(&c), None);
    }

    #[test]
    fn constraints_are_hard_conditions() {
        let mut regions = BTreeSet::new();
        regions.insert("eu-west".to_string());
        let c = Constraints {
            max_input_bytes: 10,
            min_deadline_ms: 50,
            max_concurrency: 2,
            regions,
        };
        assert!(c.accepts_payload(10));
        assert!(!c.accepts_payload(11));
        assert!(c.accepts_deadline(50));
        assert!(!c.accepts_deadline(49));
        assert!(c.accepts_region(Some("eu-west")));
        assert!(!c.accepts_region(Some("us-east")));
        assert!(!c.accepts_region(None));
        assert!(Constraints::default().accepts_region(None));
        assert_eq!(
            Constraints {
                max_concurrency: 0,
                ..Constraints::default()
            }
            .validate(),
            Err(CoreError::Encoding)
        );
    }
}
