//! 安全服务的配置与信任锚。
//!
//! AU4A 里没有「运营方」这个角色：安全服务本身也是一个 Agent（有自己的 DID 与私钥），
//! 裁决权属于配置里列出的**仲裁者 DID 集合**，而不是某个进程或某个人的账户。
//! 因此配置里没有一个「人工审批开关」——只有 `arbiters` 这份可验证的公钥名单。

use std::collections::BTreeSet;

use au4a_core::{CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 安全服务配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyConfig {
    /// 安全服务自身的 DID（它必须同时持有一把私钥才能签发回执）。
    pub service: Did,
    /// 被信任的仲裁者集合。裁决数据的签名只有落在集合内才被接受。
    pub arbiters: BTreeSet<Did>,
}

impl SafetyConfig {
    pub fn new(service: Did, arbiters: impl IntoIterator<Item = Did>) -> Self {
        Self {
            service,
            arbiters: arbiters.into_iter().collect(),
        }
    }

    pub fn single_arbiter(service: Did, arbiter: Did) -> Self {
        Self::new(service, [arbiter])
    }

    pub fn is_arbiter(&self, did: &Did) -> bool {
        self.arbiters.contains(did)
    }

    /// 仲裁数据的信任闸门：不在名单内一律拒绝。
    pub fn require_arbiter(&self, did: &Did) -> CoreResult<()> {
        if self.is_arbiter(did) {
            Ok(())
        } else {
            Err(CoreError::InvalidSignature)
        }
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    #[test]
    fn only_listed_arbiters_pass_the_trust_gate() {
        let config = SafetyConfig::single_arbiter(did(1), did(2));
        assert!(config.require_arbiter(&did(2)).is_ok());
        assert_eq!(config.require_arbiter(&did(3)), Err(CoreError::InvalidSignature));
        assert!(!config.is_arbiter(&did(3)));
    }

    #[test]
    fn config_roundtrips_through_json() {
        let config = SafetyConfig::new(did(4), [did(5), did(6)]);
        let json = config.to_json().unwrap();
        assert_eq!(SafetyConfig::from_json(&json).unwrap(), config);
        // 名单按 DID 字节序排列，保证同一份信任锚得到同一串规范字节。
        let again = SafetyConfig::new(did(4), [did(6), did(5)]);
        assert_eq!(config.to_json().unwrap(), again.to_json().unwrap());
    }
}
