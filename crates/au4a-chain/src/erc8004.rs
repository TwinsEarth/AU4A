//! v1.8.3 ERC-8004：身份 / 声誉 / 验证注册表语义（**ETH 侧确定性测试网原型**）。
//!
//! ERC-8004 描述的是「无信任 Agent 的注册表」：身份注册表（Agent 的链上身份）、
//! 声誉注册表（客户打分）、验证注册表（验证者的背书）。真实链上有合约地址、ERC-721 式身份、
//! 事件日志与 gas；这里把**语义**抽出来做成纯 CPU 状态机：
//!
//! * `erc8004.register`：把 DID 绑定成一个链上身份（`agent_id = H(did ‖ nonce)`），**不可转让**；
//! * `erc8004.give_feedback`：客户给 0..=10000bp 的分数（整数，无浮点）；同一客户只能留一条
//!   有效反馈（撤销后可重发）；**自评不计入**；
//! * `erc8004.revoke_feedback`：撤销自己的反馈；
//! * `erc8004.validate`：验证者对某个身份给出 0..=10000bp 的背书；
//! * 读取：`summary()` 给出 `count / distinct_clients / average_bp / validations`（全整数）。
//!
//! 具名拒绝（**按名字**拒绝 + [`RefusalCode`]）见 [`ERC8004_REFUSALS`]：身份转让、自评、
//! 无界铸造、设置运营方、读取远端注册表。
//!
//! 证据等级恒为 `cpu-proto`：没有真实合约、没有事件日志、没有 gas、没有 EVM。

use std::collections::BTreeMap;

use au4a_core::{CoreError, CoreResult, Credits, Did, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::testnet::{ChainRefusal, ChainTx, Receipt, RefusalSpec, Testnet, ONCHAIN_GRADE};

/// 本适配器支持的操作。
pub const ERC8004_SUPPORTED: [&str; 4] = [
    "erc8004.register",
    "erc8004.give_feedback",
    "erc8004.revoke_feedback",
    "erc8004.validate",
];

/// 具名拒绝清单。
pub const ERC8004_REFUSALS: [RefusalSpec; 5] = [
    RefusalSpec {
        op: "erc8004.transfer_identity",
        code: RefusalCode::PolicyDenied,
        reason: "身份与 DID 一一绑定，转让身份等于伪造主体（策略永久拒绝）",
    },
    RefusalSpec {
        op: "erc8004.self_feedback",
        code: RefusalCode::PolicyDenied,
        reason: "自评不计入信誉：自己给自己打分没有信息量",
    },
    RefusalSpec {
        op: "erc8004.mint_unbounded",
        code: RefusalCode::PolicyDenied,
        reason: "无界铸造会破坏身份唯一性",
    },
    RefusalSpec {
        op: "erc8004.set_operator",
        code: RefusalCode::Unsupported,
        reason: "AU4A 没有「运营方」角色：Agent 就是主体，不存在代操作账户",
    },
    RefusalSpec {
        op: "erc8004.load_remote_registry",
        code: RefusalCode::Unsupported,
        reason: "本轨道不联网：不能从远端注册表读取任何数据",
    },
];

/// 链上身份记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub agent_id: String,
    pub did: String,
    pub registered_at: u64,
}

/// 一条反馈。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feedback {
    pub agent_id: String,
    pub client: String,
    pub score_bp: i64,
    pub tag: String,
    pub at: u64,
    pub revoked: bool,
}

/// 一条验证背书。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validation {
    pub agent_id: String,
    pub validator: String,
    pub result_bp: i64,
    pub at: u64,
}

/// 信誉摘要（全部整数基点）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReputationSummary {
    pub agent_id: String,
    pub count: usize,
    pub distinct_clients: usize,
    pub score_sum_bp: i64,
    /// 平均值（向下取整，整数运算）。
    pub average_bp: i64,
    pub validations: usize,
    /// 验证背书的平均值（向下取整）。
    pub validation_average_bp: i64,
    pub tags: Vec<String>,
}

impl ReputationSummary {
    pub fn to_json(&self) -> Value {
        json!({
            "agent_id": self.agent_id,
            "count": self.count,
            "distinct_clients": self.distinct_clients,
            "score_sum_bp": self.score_sum_bp,
            "average_bp": self.average_bp,
            "validations": self.validations,
            "validation_average_bp": self.validation_average_bp,
            "tags": self.tags,
            "grade": ONCHAIN_GRADE.as_str(),
        })
    }
}

/// 适配器执行结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Erc8004Outcome {
    pub op: String,
    pub detail: String,
    pub receipt: Option<Receipt>,
    pub grade: String,
}

/// ERC-8004 语义适配器（ETH 侧）。
#[derive(Clone, Debug, Default)]
pub struct Erc8004Adapter {
    identities: BTreeMap<String, Identity>,
    feedback: Vec<Feedback>,
    validations: Vec<Validation>,
}

impl Erc8004Adapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn identity_count(&self) -> usize {
        self.identities.len()
    }

    pub fn identity_of(&self, agent_id: &str) -> Option<&Identity> {
        self.identities.get(agent_id)
    }

    pub fn identity_of_did(&self, did: &Did) -> Option<&Identity> {
        self.identities
            .values()
            .find(|identity| identity.did == did.as_str())
    }

    fn agent_id_for(did: &Did, nonce: u64) -> CoreResult<String> {
        au4a_core::canonical_hash(&json!({ "did": did, "nonce": nonce, "std": "erc8004" }))
    }

    /// 执行：已知但拒绝的操作按其具名码拒绝；未知操作按名字拒绝。
    pub fn execute(
        &mut self,
        net: &mut Testnet,
        tx: &ChainTx,
    ) -> Result<Erc8004Outcome, ChainRefusal> {
        match tx.op.as_str() {
            "erc8004.register" => self.register(net, tx),
            "erc8004.give_feedback" => self.give_feedback(net, tx),
            "erc8004.revoke_feedback" => self.revoke_feedback(net, tx),
            "erc8004.validate" => self.validate(net, tx),
            _ => {
                let refusal = match ERC8004_REFUSALS.iter().find(|spec| spec.op == tx.op) {
                    Some(spec) => ChainRefusal::new(&tx.op, spec.code, spec.reason),
                    None => ChainRefusal::unsupported(&tx.op, &ERC8004_REFUSALS),
                };
                net.record_refusal(refusal.clone());
                Err(refusal)
            }
        }
    }

    fn register(
        &mut self,
        net: &mut Testnet,
        tx: &ChainTx,
    ) -> Result<Erc8004Outcome, ChainRefusal> {
        if self.identity_of_did(&tx.submitter).is_some() {
            let refusal = ChainRefusal::new(
                "erc8004.register",
                RefusalCode::Conflict,
                "该 DID 已经注册过身份（身份唯一且不可转让）",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let agent_id = Self::agent_id_for(&tx.submitter, tx.nonce).map_err(|_| {
            ChainRefusal::new(
                "erc8004.register",
                RefusalCode::Malformed,
                "agent_id 计算失败",
            )
        })?;
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "erc8004");
        let height = receipt.height;
        self.identities.insert(
            agent_id.clone(),
            Identity {
                agent_id: agent_id.clone(),
                did: tx.submitter.as_str().to_string(),
                registered_at: height,
            },
        );
        Ok(Erc8004Outcome {
            op: "erc8004.register".to_string(),
            detail: format!(
                "身份 {} 绑定到 {}（高度 {height}）",
                au4a_core::short_id(&agent_id),
                au4a_core::short_id(tx.submitter.as_str())
            ),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    fn give_feedback(
        &mut self,
        net: &mut Testnet,
        tx: &ChainTx,
    ) -> Result<Erc8004Outcome, ChainRefusal> {
        let malformed = |detail: &str| {
            ChainRefusal::new("erc8004.give_feedback", RefusalCode::Malformed, detail)
        };
        let agent_id = tx
            .payload
            .get("agent_id")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("缺少 agent_id"))?
            .to_string();
        let score_bp = tx
            .payload
            .get("score_bp")
            .and_then(Value::as_i64)
            .ok_or_else(|| malformed("缺少 score_bp"))?;
        if !(0..=10_000).contains(&score_bp) {
            let refusal = malformed("分数必须在 0..=10000 基点之间");
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let tag = tx
            .payload
            .get("tag")
            .and_then(Value::as_str)
            .unwrap_or("generic")
            .to_string();
        let identity = self.identities.get(&agent_id).cloned().ok_or_else(|| {
            let refusal = ChainRefusal::new(
                "erc8004.give_feedback",
                RefusalCode::Conflict,
                format!("身份 {} 未注册", au4a_core::short_id(&agent_id)),
            );
            net.record_refusal(refusal.clone());
            refusal
        })?;
        // 自评不计入：按具名拒绝码拒绝。
        if identity.did == tx.submitter.as_str() {
            let refusal = ChainRefusal::new(
                "erc8004.self_feedback",
                RefusalCode::PolicyDenied,
                ERC8004_REFUSALS[1].reason,
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        // 只有注册过身份的客户才能反馈（否则任何人都能刷分）。
        if self.identity_of_did(&tx.submitter).is_none() {
            let refusal = ChainRefusal::new(
                "erc8004.give_feedback",
                RefusalCode::Unauthorized,
                "反馈方必须先注册身份（未注册客户不得刷分）",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        // 同一客户对同一身份只能有一条有效反馈。
        if self
            .feedback
            .iter()
            .any(|f| f.agent_id == agent_id && f.client == tx.submitter.as_str() && !f.revoked)
        {
            let refusal = ChainRefusal::new(
                "erc8004.give_feedback",
                RefusalCode::Conflict,
                "同一客户只能留下一条有效反馈（可先撤销）",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "erc8004");
        let height = receipt.height;
        self.feedback.push(Feedback {
            agent_id,
            client: tx.submitter.as_str().to_string(),
            score_bp,
            tag,
            at: height,
            revoked: false,
        });
        Ok(Erc8004Outcome {
            op: "erc8004.give_feedback".to_string(),
            detail: format!("反馈已记录：{score_bp}bp（高度 {height}）"),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    fn revoke_feedback(
        &mut self,
        net: &mut Testnet,
        tx: &ChainTx,
    ) -> Result<Erc8004Outcome, ChainRefusal> {
        let agent_id = tx
            .payload
            .get("agent_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainRefusal::new(
                    "erc8004.revoke_feedback",
                    RefusalCode::Malformed,
                    "缺少 agent_id",
                )
            })?
            .to_string();
        let found = self
            .feedback
            .iter_mut()
            .find(|f| f.agent_id == agent_id && f.client == tx.submitter.as_str() && !f.revoked);
        let Some(entry) = found else {
            let refusal = ChainRefusal::new(
                "erc8004.revoke_feedback",
                RefusalCode::Conflict,
                "没有可撤销的有效反馈",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        };
        entry.revoked = true;
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "erc8004");
        Ok(Erc8004Outcome {
            op: "erc8004.revoke_feedback".to_string(),
            detail: "反馈已撤销（不再计入信誉）".to_string(),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    fn validate(
        &mut self,
        net: &mut Testnet,
        tx: &ChainTx,
    ) -> Result<Erc8004Outcome, ChainRefusal> {
        let malformed =
            |detail: &str| ChainRefusal::new("erc8004.validate", RefusalCode::Malformed, detail);
        let agent_id = tx
            .payload
            .get("agent_id")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("缺少 agent_id"))?
            .to_string();
        let result_bp = tx
            .payload
            .get("result_bp")
            .and_then(Value::as_i64)
            .ok_or_else(|| malformed("缺少 result_bp"))?;
        if !(0..=10_000).contains(&result_bp) {
            let refusal = malformed("背书必须在 0..=10000 基点之间");
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        if !self.identities.contains_key(&agent_id) {
            let refusal = ChainRefusal::new(
                "erc8004.validate",
                RefusalCode::Conflict,
                "被验证身份不存在",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        if self.identity_of_did(&tx.submitter).is_none() {
            let refusal = ChainRefusal::new(
                "erc8004.validate",
                RefusalCode::Unauthorized,
                "验证者必须先注册身份",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        if self
            .validations
            .iter()
            .any(|v| v.agent_id == agent_id && v.validator == tx.submitter.as_str())
        {
            let refusal = ChainRefusal::new(
                "erc8004.validate",
                RefusalCode::Conflict,
                "同一验证者对同一身份只能背书一次",
            );
            net.record_refusal(refusal.clone());
            return Err(refusal);
        }
        let mut receipt = net.accept(tx)?;
        Testnet::stamp(&mut receipt, "erc8004");
        let height = receipt.height;
        self.validations.push(Validation {
            agent_id: agent_id.clone(),
            validator: tx.submitter.as_str().to_string(),
            result_bp,
            at: height,
        });
        Ok(Erc8004Outcome {
            op: "erc8004.validate".to_string(),
            detail: format!("验证背书 {result_bp}bp 已记录（高度 {height}）"),
            receipt: Some(receipt),
            grade: ONCHAIN_GRADE.as_str().to_string(),
        })
    }

    /// 只读：信誉摘要（整数基点，撤销与自评都不计入）。
    pub fn summary(&self, agent_id: &str) -> CoreResult<ReputationSummary> {
        if !self.identities.contains_key(agent_id) {
            return Err(CoreError::UnknownAgent);
        }
        let mut active: Vec<&Feedback> = self
            .feedback
            .iter()
            .filter(|f| f.agent_id == agent_id && !f.revoked)
            .collect();
        active.sort_by(|a, b| a.client.cmp(&b.client));
        let count = active.len();
        let mut sum = 0i64;
        let mut clients: Vec<String> = Vec::new();
        let mut tags: Vec<String> = Vec::new();
        for entry in &active {
            sum = sum.checked_add(entry.score_bp).ok_or(CoreError::Overflow)?;
            if !clients.iter().any(|c| c == &entry.client) {
                clients.push(entry.client.clone());
            }
            if !tags.iter().any(|t| t == &entry.tag) {
                tags.push(entry.tag.clone());
            }
        }
        let average_bp = if count == 0 { 0 } else { sum / count as i64 };
        let validations: Vec<&Validation> = self
            .validations
            .iter()
            .filter(|v| v.agent_id == agent_id)
            .collect();
        let mut vsum = 0i64;
        for v in &validations {
            vsum = vsum.checked_add(v.result_bp).ok_or(CoreError::Overflow)?;
        }
        let validation_average_bp = if validations.is_empty() {
            0
        } else {
            vsum / validations.len() as i64
        };
        Ok(ReputationSummary {
            agent_id: agent_id.to_string(),
            count,
            distinct_clients: clients.len(),
            score_sum_bp: sum,
            average_bp,
            validations: validations.len(),
            validation_average_bp,
            tags,
        })
    }
}

/// 一个身份在声誉桥接里能贡献的最大权重（基点）：由 [`ReputationSummary`] 压缩而来。
pub fn reputation_weight_bp(summary: &ReputationSummary) -> i64 {
    if summary.count == 0 {
        return 0;
    }
    // 权重 = 平均分 × 客户数占比（最多 5 个独立客户算满分），全部整数运算。
    let client_factor = if summary.distinct_clients >= 5 {
        10_000
    } else {
        summary.distinct_clients as i64 * 2_000
    };
    summary.average_bp.saturating_mul(client_factor) / 10_000
}

/// 信誉摘要的金额化投影（只读，用于观察层）。
pub fn summary_credits(summary: &ReputationSummary) -> CoreResult<Credits> {
    Credits::new(reputation_weight_bp(summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testnet::ChainId;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn tx(net: &Testnet, op: &str, who: u8, nonce: u64, payload: Value) -> ChainTx {
        ChainTx::new(net.chain(), op, &did(who), nonce, payload).unwrap()
    }

    /// 注册三个身份：0 = 被评价者，1/2/3 = 客户，4 = 验证者。
    fn world() -> (Testnet, Erc8004Adapter, Vec<String>) {
        let mut net = Testnet::new(ChainId::EthLocal, 1);
        let mut reg = Erc8004Adapter::new();
        let mut ids = Vec::new();
        for (who, nonce) in [(0u8, 1u64), (1, 1), (2, 1), (3, 1), (4, 1)] {
            let t = tx(&net, "erc8004.register", who, nonce, json!({}));
            reg.execute(&mut net, &t).unwrap();
            ids.push(reg.identity_of_did(&did(who)).unwrap().agent_id.clone());
        }
        (net, reg, ids)
    }

    #[test]
    fn identities_bind_to_dids_and_cannot_be_registered_twice() {
        let (mut net, mut reg, ids) = world();
        assert_eq!(reg.identity_count(), 5);
        assert_eq!(reg.identity_of(&ids[0]).unwrap().did, did(0).as_str());
        let again = tx(&net, "erc8004.register", 0, 2, json!({}));
        let err = reg.execute(&mut net, &again).unwrap_err();
        assert_eq!(err.code, RefusalCode::Conflict);
        assert_eq!(reg.identity_count(), 5);
    }

    #[test]
    fn feedback_is_averaged_with_integer_math() {
        let (mut net, mut reg, ids) = world();
        for (who, score) in [(1u8, 9_000i64), (2, 8_000), (3, 7_000)] {
            let t = tx(
                &net,
                "erc8004.give_feedback",
                who,
                2,
                json!({ "agent_id": ids[0], "score_bp": score, "tag": "translation" }),
            );
            reg.execute(&mut net, &t).unwrap();
        }
        let summary = reg.summary(&ids[0]).unwrap();
        assert_eq!(summary.count, 3);
        assert_eq!(summary.distinct_clients, 3);
        assert_eq!(summary.score_sum_bp, 24_000);
        assert_eq!(summary.average_bp, 8_000); // 24000 / 3，整数
        assert_eq!(summary.tags, vec!["translation".to_string()]);
        // 权重：平均 8000 × 客户因子 (3×2000=6000) / 10000 = 4800bp。
        assert_eq!(reputation_weight_bp(&summary), 4_800);
        assert_eq!(summary_credits(&summary).unwrap(), Credits(4_800));
        assert!(au4a_core::canonicalize(&summary.to_json()).is_ok());
    }

    #[test]
    fn self_feedback_and_unregistered_clients_are_refused() {
        let (mut net, mut reg, ids) = world();
        let selfish = tx(
            &net,
            "erc8004.give_feedback",
            0,
            2,
            json!({ "agent_id": ids[0], "score_bp": 10_000 }),
        );
        let err = reg.execute(&mut net, &selfish).unwrap_err();
        assert_eq!(err.code, RefusalCode::PolicyDenied);
        assert_eq!(err.op, "erc8004.self_feedback");

        // 未注册的客户（种子 9）不能反馈。
        let stranger = ChainTx::new(
            ChainId::EthLocal,
            "erc8004.give_feedback",
            &did(9),
            1,
            json!({ "agent_id": ids[0], "score_bp": 10_000 }),
        )
        .unwrap();
        let err = reg.execute(&mut net, &stranger).unwrap_err();
        assert_eq!(err.code, RefusalCode::Unauthorized);
        assert_eq!(reg.summary(&ids[0]).unwrap().count, 0);
    }

    #[test]
    fn one_active_feedback_per_client_and_revocation_removes_it() {
        let (mut net, mut reg, ids) = world();
        let first = tx(
            &net,
            "erc8004.give_feedback",
            1,
            2,
            json!({ "agent_id": ids[0], "score_bp": 5_000, "tag": "generic" }),
        );
        reg.execute(&mut net, &first).unwrap();
        // 同一客户再来一条 → Conflict。
        let second = tx(
            &net,
            "erc8004.give_feedback",
            1,
            3,
            json!({ "agent_id": ids[0], "score_bp": 9_000, "tag": "generic" }),
        );
        assert_eq!(
            reg.execute(&mut net, &second).unwrap_err().code,
            RefusalCode::Conflict
        );
        assert_eq!(reg.summary(&ids[0]).unwrap().average_bp, 5_000);
        // 撤销后不再计入。
        let revoke = tx(
            &net,
            "erc8004.revoke_feedback",
            1,
            4,
            json!({ "agent_id": ids[0] }),
        );
        reg.execute(&mut net, &revoke).unwrap();
        let summary = reg.summary(&ids[0]).unwrap();
        assert_eq!(summary.count, 0);
        assert_eq!(summary.average_bp, 0);
        // 撤销后可以重新反馈。
        let again = tx(
            &net,
            "erc8004.give_feedback",
            1,
            5,
            json!({ "agent_id": ids[0], "score_bp": 9_500, "tag": "retry" }),
        );
        reg.execute(&mut net, &again).unwrap();
        assert_eq!(reg.summary(&ids[0]).unwrap().average_bp, 9_500);
        // 撤销不存在的反馈 → Conflict。
        let revoke_again = tx(
            &net,
            "erc8004.revoke_feedback",
            2,
            2,
            json!({ "agent_id": ids[0] }),
        );
        assert_eq!(
            reg.execute(&mut net, &revoke_again).unwrap_err().code,
            RefusalCode::Conflict
        );
    }

    #[test]
    fn out_of_range_scores_and_unknown_agents_are_refused() {
        let (mut net, mut reg, ids) = world();
        for score in [10_001i64, -1] {
            let t = tx(
                &net,
                "erc8004.give_feedback",
                1,
                2,
                json!({ "agent_id": ids[0], "score_bp": score }),
            );
            assert_eq!(
                reg.execute(&mut net, &t).unwrap_err().code,
                RefusalCode::Malformed
            );
        }
        let unknown = tx(
            &net,
            "erc8004.give_feedback",
            1,
            2,
            json!({ "agent_id": "deadbeef", "score_bp": 5_000 }),
        );
        assert_eq!(
            reg.execute(&mut net, &unknown).unwrap_err().code,
            RefusalCode::Conflict
        );
        assert_eq!(reg.summary("deadbeef"), Err(CoreError::UnknownAgent));
    }

    #[test]
    fn validations_require_a_registered_validator_and_are_counted_once() {
        let (mut net, mut reg, ids) = world();
        let ok = tx(
            &net,
            "erc8004.validate",
            4,
            2,
            json!({ "agent_id": ids[0], "result_bp": 9_200 }),
        );
        reg.execute(&mut net, &ok).unwrap();
        // 同一验证者再背书 → Conflict。
        let twice = tx(
            &net,
            "erc8004.validate",
            4,
            3,
            json!({ "agent_id": ids[0], "result_bp": 9_900 }),
        );
        assert_eq!(
            reg.execute(&mut net, &twice).unwrap_err().code,
            RefusalCode::Conflict
        );
        // 未注册验证者 → Unauthorized。
        let stranger = ChainTx::new(
            ChainId::EthLocal,
            "erc8004.validate",
            &did(8),
            1,
            json!({ "agent_id": ids[0], "result_bp": 1 }),
        )
        .unwrap();
        assert_eq!(
            reg.execute(&mut net, &stranger).unwrap_err().code,
            RefusalCode::Unauthorized
        );
        let summary = reg.summary(&ids[0]).unwrap();
        assert_eq!(summary.validations, 1);
        assert_eq!(summary.validation_average_bp, 9_200);
    }

    #[test]
    fn named_refusals_cover_the_erc8004_unsupported_set() {
        let (mut net, mut reg, _ids) = world();
        for (op, code) in [
            ("erc8004.transfer_identity", RefusalCode::PolicyDenied),
            ("erc8004.mint_unbounded", RefusalCode::PolicyDenied),
            ("erc8004.set_operator", RefusalCode::Unsupported),
            ("erc8004.load_remote_registry", RefusalCode::Unsupported),
            ("erc8004.unknown_call", RefusalCode::Unsupported),
        ] {
            let t = tx(&net, op, 1, 9, json!({}));
            let err = reg.execute(&mut net, &t).unwrap_err();
            assert_eq!(err.op, op, "拒绝必须带名字");
            assert_eq!(err.code, code);
        }
        assert_eq!(net.refusals().len(), 5);
        assert_eq!(ERC8004_REFUSALS.len(), 5);
        assert_eq!(ERC8004_SUPPORTED.len(), 4);
    }

    #[test]
    fn the_summary_is_reproducible_and_float_free() {
        let (mut net, mut reg, ids) = world();
        for (who, score) in [(1u8, 3_333i64), (2, 3_333), (3, 3_334)] {
            let t = tx(
                &net,
                "erc8004.give_feedback",
                who,
                2,
                json!({ "agent_id": ids[0], "score_bp": score }),
            );
            reg.execute(&mut net, &t).unwrap();
        }
        let a = reg.summary(&ids[0]).unwrap();
        let b = reg.summary(&ids[0]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.average_bp, 3_333); // 10000 / 3 向下取整
        let value = a.to_json();
        au4a_core::canonicalize(&value).unwrap();
        assert_eq!(value["grade"], json!("cpu-proto"));
    }
}
