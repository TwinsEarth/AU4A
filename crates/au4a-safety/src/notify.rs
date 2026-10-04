//! v1.5.5 通知机制：按案件订阅状态变更，可退订，只投递给订阅者。
//!
//! 三条规则，全部为了「通知是可预期的东西，不是广播噪音」：
//!
//! 1. **订阅即回放当前状态快照**：Agent 订阅时立刻拿到「现在是什么状态」，
//!    否则它必须再去查一次；但**不投递订阅前的历史**（那会把别人早先的申诉
//!    变成订阅者的信息优势）。
//! 2. **状态集合是显式枚举**：只投递订阅者点名的状态，不投递「全部」。
//! 3. **退订只由本人发起**：订阅是订阅者的私有关系，第三方不能替他撤销。
//!
//! 通知本身是**派生投影**（由「案件状态 + 订阅关系」确定性推出），不是新的状态源；
//! 订阅与退订才是状态变更，因此它们写进哈希链，而通知不写。

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::case::CaseStatus;

/// 一个订阅：某 Agent 关心某案件的哪些状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub subscriber: Did,
    pub case: String,
    /// 关心的状态集合：非空、去重、按枚举序升序（规范化，保证同一意图同一 id）。
    pub statuses: Vec<CaseStatus>,
    pub at: u64,
    pub sig: String,
}

impl Subscription {
    pub fn new(
        case: impl Into<String>,
        subscriber: Did,
        mut statuses: Vec<CaseStatus>,
        at: u64,
    ) -> Self {
        statuses.sort_unstable();
        statuses.dedup();
        Self {
            id: String::new(),
            subscriber,
            case: case.into(),
            statuses,
            at,
            sig: String::new(),
        }
    }

    fn unsigned_payload(&self) -> Value {
        json!({
            "subscriber": self.subscriber,
            "case": self.case,
            "statuses": self.statuses,
            "at": self.at,
        })
    }

    pub fn signing_payload(&self) -> Value {
        json!({
            "id": self.id,
            "subscriber": self.subscriber,
            "case": self.case,
            "statuses": self.statuses,
            "at": self.at,
        })
    }

    pub fn compute_id(&self) -> CoreResult<String> {
        canonical_hash(&self.unsigned_payload())
    }

    pub fn sign(mut self, keys: &AgentKeys) -> CoreResult<Self> {
        if self.subscriber != keys.did() {
            return Err(CoreError::InvalidSignature);
        }
        if self.statuses.is_empty() {
            // 空集合不是「全都订阅」，而是没有说清要订阅什么。
            return Err(CoreError::InvalidKind);
        }
        self.id = self.compute_id()?;
        self.sig = keys.sign_json(&self.signing_payload())?;
        Ok(self)
    }

    pub fn verify(&self) -> CoreResult<()> {
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        if self.statuses.is_empty() {
            return Err(CoreError::InvalidKind);
        }
        if self.compute_id()? != self.id {
            return Err(CoreError::InvalidSignature);
        }
        let bytes = au4a_core::canonicalize(&self.signing_payload())?;
        self.subscriber.verify(bytes.as_bytes(), &self.sig)
    }

    pub fn watches(&self, status: CaseStatus) -> bool {
        self.statuses.contains(&status)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 一条已投递的通知。`seq` 指向链上造成该状态的事件序号，因此通知可被独立复核。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub subscription: String,
    pub to: Did,
    pub case: String,
    pub status: CaseStatus,
    /// 触发该状态的事件的链上序号。
    pub seq: u64,
    pub at: u64,
}

impl Notification {
    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subscription(seed: u8, statuses: Vec<CaseStatus>) -> (AgentKeys, Subscription) {
        let keys = AgentKeys::from_seed(&[seed; 32]);
        let sub = Subscription::new("case-1", keys.did(), statuses, 5)
            .sign(&keys)
            .unwrap();
        (keys, sub)
    }

    #[test]
    fn statuses_are_normalized_and_the_id_is_stable() {
        let (_, a) = subscription(
            41,
            vec![
                CaseStatus::Arbitrated,
                CaseStatus::Reported,
                CaseStatus::Reported,
            ],
        );
        let (_, b) = subscription(41, vec![CaseStatus::Reported, CaseStatus::Arbitrated]);
        assert_eq!(
            a.statuses,
            vec![CaseStatus::Reported, CaseStatus::Arbitrated]
        );
        assert_eq!(a.id, b.id, "同一意图必须得到同一订阅 id");
        assert_eq!(a.sig, b.sig);
    }

    #[test]
    fn an_empty_status_set_cannot_be_sealed() {
        let keys = AgentKeys::from_seed(&[42u8; 32]);
        let empty = Subscription::new("case-1", keys.did(), Vec::new(), 5);
        assert_eq!(empty.clone().sign(&keys), Err(CoreError::InvalidKind));
        assert_eq!(empty.verify(), Err(CoreError::NotSealed));
    }

    #[test]
    fn a_third_party_cannot_subscribe_or_seal_on_behalf_of_someone_else() {
        let (subscriber, sub) = subscription(43, vec![CaseStatus::Reported]);
        let stranger = AgentKeys::from_seed(&[44u8; 32]);
        let forged = Subscription::new(
            sub.case.clone(),
            subscriber.did(),
            vec![CaseStatus::Reported],
            5,
        )
        .sign(&stranger);
        assert_eq!(forged, Err(CoreError::InvalidSignature));
    }

    #[test]
    fn tampering_with_case_or_statuses_is_detected() {
        let (_, mut sub) = subscription(45, vec![CaseStatus::Reported]);
        sub.case = "case-2".to_string();
        assert_eq!(sub.verify(), Err(CoreError::InvalidSignature));

        let (_, mut sub) = subscription(46, vec![CaseStatus::Reported]);
        sub.statuses = vec![CaseStatus::Arbitrated];
        assert_eq!(sub.verify(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn watches_only_the_named_statuses() {
        let (_, sub) = subscription(47, vec![CaseStatus::Appealed, CaseStatus::Arbitrated]);
        assert!(sub.watches(CaseStatus::Appealed));
        assert!(sub.watches(CaseStatus::Arbitrated));
        assert!(!sub.watches(CaseStatus::Reported));
        assert!(!sub.watches(CaseStatus::Penalized));
    }

    #[test]
    fn subscriptions_roundtrip_through_json() {
        let (_, sub) = subscription(48, vec![CaseStatus::Penalized]);
        let json = sub.to_json().unwrap();
        assert_eq!(Subscription::from_json(&json).unwrap(), sub);
    }
}
