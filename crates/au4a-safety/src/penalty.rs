//! v1.5.4 处罚：由**仲裁者签名的数据契约**触发，且罚没受锁定余额约束。
//!
//! 这里是「无罪不罚」的另一半：账本唯一会被改动的入口就是 [`PenaltyOrder`] 的验证通过。
//! 契约是纯数据（可序列化 JSON），签名由配置里的仲裁者 DID 校验，因此
//! **安全服务不需要认识 `au4a-council` 这个 crate**，也不需要认识任何「运营方账户」。
//!
//! 两个语义细节：
//!
//! * `Warning` / `Quarantine` 不罚没，但同样生成**处罚记录**（处罚不等于扣钱）。
//! * `StakeSlash` 的请求额可以超过锁定余额：实际执行额按锁定余额封顶，
//!   记录里同时保留 `requested` 与 `applied`，因此「请求」与「实际」永远不会被混淆。

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Credits, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::SafetyConfig;

/// 处罚种类。封闭集合。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanctionKind {
    /// 警告：记入记录，不动账本。
    Warning,
    /// 罚没锁定质押。
    StakeSlash,
    /// 隔离：记入记录，由委员会（v1.7）决定是否执行。
    Quarantine,
}

impl SanctionKind {
    pub const ALL: [SanctionKind; 3] = [
        SanctionKind::Warning,
        SanctionKind::StakeSlash,
        SanctionKind::Quarantine,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SanctionKind::Warning => "warning",
            SanctionKind::StakeSlash => "stake_slash",
            SanctionKind::Quarantine => "quarantine",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        SanctionKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// 是否会改动账本。只有罚没会。
    pub fn moves_ledger(self) -> bool {
        matches!(self, SanctionKind::StakeSlash)
    }
}

/// 处罚契约（数据契约）：仲裁者的签名声明。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PenaltyOrder {
    /// 内容寻址 id（对未签名字段）。
    pub id: String,
    pub case: String,
    pub subject: Did,
    pub sanction: SanctionKind,
    /// 请求的罚没额（微积分）。非罚没类处罚必须为 0。
    pub amount: Credits,
    /// 出具契约的仲裁者。
    pub arbiter: Did,
    pub issued_at: u64,
    pub sig: String,
}

impl PenaltyOrder {
    pub fn new(
        case: impl Into<String>,
        subject: Did,
        sanction: SanctionKind,
        amount: Credits,
        arbiter: Did,
        issued_at: u64,
    ) -> Self {
        Self {
            id: String::new(),
            case: case.into(),
            subject,
            sanction,
            amount,
            arbiter,
            issued_at,
            sig: String::new(),
        }
    }

    fn unsigned_payload(&self) -> Value {
        json!({
            "case": self.case,
            "subject": self.subject,
            "sanction": self.sanction,
            "amount": self.amount,
            "arbiter": self.arbiter,
            "issued_at": self.issued_at,
        })
    }

    pub fn signing_payload(&self) -> Value {
        json!({
            "id": self.id,
            "case": self.case,
            "subject": self.subject,
            "sanction": self.sanction,
            "amount": self.amount,
            "arbiter": self.arbiter,
            "issued_at": self.issued_at,
        })
    }

    pub fn compute_id(&self) -> CoreResult<String> {
        canonical_hash(&self.unsigned_payload())
    }

    /// 契约自身的合法性：种类与金额必须一致。这是**语义**检查，不是签名检查。
    pub fn validate(&self) -> CoreResult<()> {
        if self.sanction.moves_ledger() {
            if self.amount == Credits::ZERO {
                // 零额罚没是调用错误（与基元层 `slash` 的语义一致）。
                return Err(CoreError::ZeroAmount);
            }
        } else if self.amount != Credits::ZERO {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }

    /// 由仲裁者签名并封口。
    pub fn sign(mut self, keys: &AgentKeys) -> CoreResult<Self> {
        if self.arbiter != keys.did() {
            return Err(CoreError::InvalidSignature);
        }
        self.validate()?;
        self.id = self.compute_id()?;
        self.sig = keys.sign_json(&self.signing_payload())?;
        Ok(self)
    }

    /// 信任闸门：仲裁者必须在配置名单内 + `id` 自洽 + 签名成立 + 语义合法。
    pub fn verify_against(&self, config: &SafetyConfig) -> CoreResult<()> {
        config.require_arbiter(&self.arbiter)?;
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        if self.compute_id()? != self.id {
            return Err(CoreError::InvalidSignature);
        }
        self.validate()?;
        let bytes = au4a_core::canonicalize(&self.signing_payload())?;
        self.arbiter.verify(bytes.as_bytes(), &self.sig)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 处罚记录：契约被接受后留下的**事实**。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PenaltyRecord {
    /// == 契约 id。
    pub id: String,
    pub case: String,
    pub subject: Did,
    pub sanction: SanctionKind,
    /// 契约请求的额度。
    pub requested: Credits,
    /// 实际执行的罚没额（受锁定余额封顶）。
    pub applied: Credits,
    pub arbiter: Did,
    /// 记录写入时的逻辑时钟读数。
    pub at: u64,
    /// 是否已被后续裁决回滚（v1.5.7 的 `rejected` 会归还罚没）。
    pub reversed: bool,
}

impl PenaltyRecord {
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
    use crate::setup;
    use au4a_core::AgentKeys;

    fn arbiter() -> AgentKeys {
        setup::keys(setup::ROLE_ARBITER)
    }

    fn config() -> SafetyConfig {
        SafetyConfig::single_arbiter(setup::keys(setup::ROLE_SERVICE).did(), arbiter().did())
    }

    fn order(amount: Credits) -> PenaltyOrder {
        PenaltyOrder::new(
            "case-1",
            AgentKeys::from_seed(&[0x21; 32]).did(),
            SanctionKind::StakeSlash,
            amount,
            arbiter().did(),
            11,
        )
    }

    #[test]
    fn an_arbiter_signed_order_passes_the_trust_gate() {
        let order = order(Credits(5)).sign(&arbiter()).unwrap();
        order.verify_against(&config()).unwrap();
        assert_eq!(order.id, order.compute_id().unwrap());
        assert_eq!(order.amount, Credits(5));
    }

    #[test]
    fn a_non_arbiter_cannot_seal_and_a_forged_one_fails() {
        let outsider = AgentKeys::from_seed(&[0x22; 32]);
        assert_eq!(
            order(Credits(5)).sign(&outsider),
            Err(CoreError::InvalidSignature)
        );

        let mut signed = order(Credits(5)).sign(&arbiter()).unwrap();
        signed.amount = Credits(500);
        assert_eq!(
            signed.verify_against(&config()),
            Err(CoreError::InvalidSignature)
        );

        // 未在名单内的签名者：即使签名自洽也不被信任。
        let stranger = AgentKeys::from_seed(&[0x23; 32]);
        let mut foreign = PenaltyOrder::new(
            "case-1",
            AgentKeys::from_seed(&[0x21; 32]).did(),
            SanctionKind::StakeSlash,
            Credits(5),
            stranger.did(),
            11,
        );
        foreign.id = foreign.compute_id().unwrap();
        foreign.sig = stranger.sign_json(&foreign.signing_payload()).unwrap();
        assert_eq!(
            foreign.verify_against(&config()),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn sanction_and_amount_must_agree() {
        // 非罚没类处罚不能带金额。
        let warning = PenaltyOrder::new(
            "case-1",
            AgentKeys::from_seed(&[0x21; 32]).did(),
            SanctionKind::Warning,
            Credits(3),
            arbiter().did(),
            11,
        );
        assert_eq!(warning.sign(&arbiter()), Err(CoreError::InvalidKind));
        // 零额罚没是调用错误。
        assert_eq!(
            order(Credits::ZERO).sign(&arbiter()),
            Err(CoreError::ZeroAmount)
        );
        assert!(!SanctionKind::Warning.moves_ledger());
        assert!(SanctionKind::StakeSlash.moves_ledger());
    }

    #[test]
    fn orders_and_records_roundtrip_through_json() {
        let signed = order(Credits(7)).sign(&arbiter()).unwrap();
        let json = signed.to_json().unwrap();
        assert_eq!(PenaltyOrder::from_json(&json).unwrap(), signed);

        let record = PenaltyRecord {
            id: signed.id.clone(),
            case: signed.case.clone(),
            subject: signed.subject.clone(),
            sanction: signed.sanction,
            requested: signed.amount,
            applied: Credits(7),
            arbiter: signed.arbiter.clone(),
            at: 12,
            reversed: false,
        };
        let json = record.to_json().unwrap();
        assert_eq!(PenaltyRecord::from_json(&json).unwrap(), record);
    }

    #[test]
    fn kinds_roundtrip_and_are_unique() {
        assert_eq!(SanctionKind::ALL.len(), 3);
        for kind in SanctionKind::ALL {
            assert_eq!(SanctionKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SanctionKind::parse("banish"), None);
    }
}
