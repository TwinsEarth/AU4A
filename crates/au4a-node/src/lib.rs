//! AU4A 节点库 —— v1.x 骨架版。
//!
//! 事实说明：平台层（注册编排 / PMB 路由 / 只读观察面板 / CLI）是在 v1.9.9 才逐版本快照的，
//! 因此 v1.0.10–v1.6.10 这些版本当时只有这个骨架。本文件即该版本的真实状态，
//! 不依赖任何轨道 crate 的后续 API，保证该版本的 workspace 能独立编译与测试。

use au4a_core::SelfCheck;

/// 骨架期自检：平台层尚未实现，只有一条「骨架存在」的检查。
pub fn self_check() -> Vec<SelfCheck> {
    vec![SelfCheck::pass(
        "1.0",
        "node.skeleton",
        "平台层骨架（该版本的真实状态：编排/面板/CLI 尚未实现）",
    )]
}

/// 该版本可用的轨道清单（骨架期只有宿主内核一条）。
pub fn track_table() -> Vec<String> {
    vec!["1.0 au4a-kernel".to_string()]
}
