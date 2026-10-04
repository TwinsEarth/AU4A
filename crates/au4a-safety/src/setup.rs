//! 确定性身份装配（演示 / 测试 / 节点自检共用）。
//!
//! 种子带轨道前缀 `15 05 a5`：`scenario` 会被节点在**共享内核**上调用，
//! 若与其它轨道同样使用 `[i; 32]` 风格的种子就会撞 DID 并触发 `DuplicateAgent`。
//! 身份必须是确定性的（可重放），但不能是可碰撞的。

use au4a_core::{AgentKeys, CoreError, CoreResult, Credits};
use au4a_kernel::Kernel;

/// 轨道前缀。
const PREFIX: [u8; 3] = [0x15, 0x05, 0xA5];

/// 角色：举报方。
pub const ROLE_REPORTER: u8 = 1;
/// 角色：被举报方。
pub const ROLE_SUBJECT: u8 = 2;
/// 角色：仲裁者。
pub const ROLE_ARBITER: u8 = 3;
/// 角色：安全服务自身。
pub const ROLE_SERVICE: u8 = 4;

/// 角色 → 32 字节确定性种子。
pub fn seed(role: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[0] = PREFIX[0];
    s[1] = PREFIX[1];
    s[2] = PREFIX[2];
    s[3] = role;
    s[31] = 0x51;
    s
}

/// 角色 → 密钥。
pub fn keys(role: u8) -> AgentKeys {
    AgentKeys::from_seed(&seed(role))
}

/// 注册一个 Agent；已注册则复用（幂等）。
///
/// `scenario` 可能在同一个内核上被多次调用（端到端演示 + 多次部署验证），
/// 因此它必须是幂等的：重复注册不是失败，而是「身份已经就位」。
pub fn ensure_agent(
    kernel: &mut Kernel,
    keys: &AgentKeys,
    display: &str,
    skills: &[&str],
    stake: Credits,
) -> CoreResult<()> {
    match kernel.register(keys, display, skills, stake) {
        Ok(_) => Ok(()),
        Err(CoreError::DuplicateAgent) => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_deterministic_and_distinct_per_role() {
        assert_eq!(seed(ROLE_REPORTER), seed(ROLE_REPORTER));
        assert_ne!(seed(ROLE_REPORTER), seed(ROLE_SUBJECT));
        assert_eq!(keys(ROLE_REPORTER).did(), keys(ROLE_REPORTER).did());
        assert_ne!(keys(ROLE_ARBITER).did(), keys(ROLE_SERVICE).did());
    }

    #[test]
    fn track_seeds_do_not_collide_with_the_plain_trivial_seeds() {
        // 其它轨道常用 [i;32]；轨道前缀保证两类种子不会映射到同一个 DID。
        for role in [ROLE_REPORTER, ROLE_SUBJECT, ROLE_ARBITER, ROLE_SERVICE] {
            for trivial in 0u8..16 {
                assert_ne!(
                    keys(role).did(),
                    AgentKeys::from_seed(&[trivial; 32]).did(),
                    "role={role} trivial={trivial}"
                );
            }
        }
    }

    #[test]
    fn ensure_agent_is_idempotent() {
        let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
        let reporter = keys(ROLE_REPORTER);
        ensure_agent(&mut kernel, &reporter, "reporter", &["audit"], Credits(20)).unwrap();
        let balance_before = kernel.ledger().balance(&reporter.did());
        ensure_agent(&mut kernel, &reporter, "reporter", &["audit"], Credits(20)).unwrap();
        assert_eq!(kernel.agent_count(), 1);
        assert_eq!(kernel.ledger().balance(&reporter.did()), balance_before);
    }
}
