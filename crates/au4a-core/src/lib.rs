//! AU4A 冻结基元（frozen primitives）。
//!
//! 这一层在 v1.0.1 冻结：10 条轨道都依赖它，任何签名变更都必须由 Lead 批准并落到新的中版本。
//! 冻结的目的是让「Agent 自主」这件事有不可协商的地基：
//!
//! * 身份自证：DID 就是公钥，不需要人类账户，也不需要解析器。
//! * 规范 JSON：字节级可复现的签名与哈希输入，整数优先，禁止浮点。
//! * 整数账本：守恒不变式可在任意时刻被断言，不出现浮点误差。
//! * 证据分级：`unverified` 永远不可结算。
//! * 类型化拒绝：区分「竞争导致的拒绝」与「恶意导致的拒绝」。
//! * PMB 信封：4 字节大端长度前缀 + 规范 JSON，1 MiB 上限，超限在分配前拒绝。
//!
//! 基元层不读时钟、不碰文件、不开网络：所有时间来自 [`prims::LogicalClock`]，
//! 因此每一个语义都可以被确定性地重放和测试。

pub mod canon;
pub mod did;
pub mod error;
pub mod evidence;
pub mod ledger;
pub mod msg;
pub mod prims;
pub mod refusal;

pub use canon::{canonical_hash, canonicalize};
pub use did::{AgentKeys, Did};
pub use error::{CoreError, CoreResult};
pub use evidence::EvidenceGrade;
pub use ledger::{Account, Credits, Ledger, LedgerView};
pub use msg::{decode_frame, encode_frame, kinds, Envelope, MsgKind, MAX_FRAME};
pub use prims::{all_passed, content_hash, short_id, LogicalClock, SelfCheck};
pub use refusal::{Escalation, Refusal, RefusalCode};

/// 版本系列：v1.0.1 → v1.9.9。
pub const SERIES: &str = "v1.0.1 → v1.9.9";
/// 中版本数量。
pub const MEDIUM_VERSIONS: usize = 10;
/// 小版本数量。
pub const SMALL_VERSIONS: usize = 99;
