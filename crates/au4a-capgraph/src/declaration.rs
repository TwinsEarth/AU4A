//! v1.1.2 —— 声明 API。
//!
//! 能力进入图里只有一条合法路径：**Agent 用自己的私钥签一份完整声明**。
//! 没有注册中心、没有管理员代填、没有「运营商信任库」——参考项目 `Approval::Operator`
//! 那种人工审批在这里不存在（见 `REF2-ARCHITECTURE.md` 的 H1/H3 反向清单）。
//!
//! 三个刻意的决定：
//!
//! 1. **声明是完整状态，不是增量**。增量声明会让「图当前是什么」依赖于消息是否丢过，
//!    而 Agent 之间不该为了对齐状态先假设信道可靠。完整状态 + 版本号（v1.1.7）可以自愈。
//! 2. **签名覆盖规范 JSON 字节**（`au4a-core::canonicalize`），因此签名与哈希同源：
//!    同一个声明在任何节点上得到同一串被签字节。
//! 3. **自己签自己**：`Declaration::sign` 只在 `declaration.agent == keys.did()` 时成功，
//!    否则返回 `InvalidSignature`。替别人声明在构造阶段就被拒，不依赖接收方小心。

use au4a_core::{canonical_hash, canonicalize, CoreError, CoreResult, Did, AgentKeys};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::{Capability, SkillId};

/// 一份能力声明：某个 Agent 在某个逻辑时刻声明的**全部**能力。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    /// 声明者。签名必须由它出具。
    pub agent: Did,
    /// 声明者自己的序号（v1.1.7 起由能力图自动递增）。
    pub epoch: u64,
    /// 逻辑时刻（不读墙钟）。
    pub issued_at: u64,
    /// 完整能力集合，按技能名升序（保证规范字节唯一）。
    pub capabilities: Vec<Capability>,
}

impl Declaration {
    /// 构造并规范化：校验每条能力、拒绝重复技能、按技能名排序。
    pub fn new(
        agent: Did,
        epoch: u64,
        issued_at: u64,
        mut capabilities: Vec<Capability>,
    ) -> CoreResult<Self> {
        capabilities.sort_by(|a, b| a.skill.cmp(&b.skill));
        let declaration = Self {
            agent,
            epoch,
            issued_at,
            capabilities,
        };
        declaration.check_coherence()?;
        Ok(declaration)
    }

    /// 内部一致性：能力本身合法 + 技能不重复（一份声明里同一技能只能有一个报价，
    /// 否则「这个 Agent 对翻译收多少钱」就有两个答案）。
    fn check_coherence(&self) -> CoreResult<()> {
        for cap in &self.capabilities {
            cap.validate()?;
        }
        let mut seen: Option<&SkillId> = None;
        for cap in &self.capabilities {
            if let Some(prev) = seen {
                if prev == &cap.skill {
                    // 冻结错误集里没有「重复条目」变体：声明形态错误统一走 Encoding。
                    return Err(CoreError::Encoding);
                }
            }
            seen = Some(&cap.skill);
        }
        Ok(())
    }

    /// 上限检查（图配置决定每账户能声明多少条能力，防公告轰炸）。
    pub fn validate_against(&self, max_skills: usize) -> CoreResult<()> {
        self.check_coherence()?;
        if self.capabilities.len() > max_skills {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }

    /// 用私钥签名。**只能签自己的**：`agent != keys.did()` 直接 `InvalidSignature`。
    pub fn sign(self, keys: &AgentKeys) -> CoreResult<SignedDeclaration> {
        if self.agent != keys.did() {
            return Err(CoreError::InvalidSignature);
        }
        let value = self.to_value()?;
        let sig = keys.sign_json(&value)?;
        Ok(SignedDeclaration {
            declaration: self,
            sig,
        })
    }

    pub fn skills(&self) -> Vec<&SkillId> {
        self.capabilities.iter().map(|c| &c.skill).collect()
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_value(value: &Value) -> CoreResult<Self> {
        let declaration: Declaration =
            serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)?;
        declaration.check_coherence()?;
        Ok(declaration)
    }

    /// 内容指纹（不含签名的规范字节哈希）。
    pub fn fingerprint(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 被签名的字节（规范 JSON）。签名与内容寻址共用同一串字节。
    pub fn signing_bytes(&self) -> CoreResult<String> {
        canonicalize(&self.to_value()?)
    }
}

/// 已签名声明。签名覆盖 `declaration` 的规范 JSON 字节。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDeclaration {
    pub declaration: Declaration,
    /// hex 编码的 Ed25519 签名。
    pub sig: String,
}

impl SignedDeclaration {
    /// 验签。任何对声明的改动都会让 `compute bytes` 变化 → `InvalidSignature`。
    pub fn verify(&self) -> CoreResult<()> {
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        let bytes = self.declaration.signing_bytes()?;
        self.declaration.agent.verify(bytes.as_bytes(), &self.sig)
    }

    pub fn agent(&self) -> &Did {
        &self.declaration.agent
    }

    pub fn epoch(&self) -> u64 {
        self.declaration.epoch
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.declaration.capabilities
    }

    /// 声明内容指纹（不含签名）：用于「同版本同内容」的判定。
    pub fn content_fingerprint(&self) -> CoreResult<String> {
        self.declaration.fingerprint()
    }

    /// 含签名的整体指纹：内容相同但签名不同（重放/替换签名）也能被发现。
    pub fn fingerprint(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 从 JSON 恢复。**不在这一步验签**：验签是 [`SignedDeclaration::verify`] 的职责，
    /// 这样「篡改的声明」会以 `InvalidSignature` 而不是 `Encoding` 被拒，拒绝码才是准的。
    pub fn from_value(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::Credits;

    fn keys(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn cap(name: &str) -> Capability {
        Capability::new(
            SkillId::new(name).expect("valid skill"),
            Credits(3),
        )
    }

    #[test]
    fn a_self_signed_declaration_verifies() {
        let a = keys(1);
        let d = Declaration::new(a.did(), 1, 10, vec![cap("translate.en-zh"), cap("sentiment.analyze")])
            .expect("coherent");
        let signed = d.sign(&a).expect("self-signing works");
        signed.verify().expect("verifies");
        assert_eq!(
            signed.capabilities().iter().map(|c| c.skill.as_str()).collect::<Vec<_>>(),
            vec!["sentiment.analyze", "translate.en-zh"],
            "声明按技能名排序"
        );
    }

    #[test]
    fn nobody_can_declare_for_someone_else() {
        let a = keys(1);
        let b = keys(2);
        let d = Declaration::new(a.did(), 1, 10, vec![cap("x")]).expect("coherent");
        assert_eq!(d.sign(&b), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn tampering_with_a_signed_declaration_is_detected() {
        let a = keys(3);
        let signed = Declaration::new(a.did(), 1, 10, vec![cap("x")])
            .expect("coherent")
            .sign(&a)
            .expect("signed");
        let mut value = signed.to_value().expect("serialisable");
        value["declaration"]["capabilities"][0]["price_per_unit"] = serde_json::json!(0);
        let tampered = SignedDeclaration::from_value(&value).expect("parses");
        assert_eq!(tampered.verify(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn duplicate_skills_in_one_declaration_are_refused() {
        let a = keys(4);
        let dup = Declaration::new(a.did(), 1, 10, vec![cap("x"), cap("x")]);
        assert_eq!(dup, Err(CoreError::Encoding));
    }

    #[test]
    fn incoherent_capabilities_are_refused_at_declaration_time() {
        let a = keys(5);
        let bad = cap("x").with_latency(900, 100);
        assert_eq!(Declaration::new(a.did(), 1, 10, vec![bad]), Err(CoreError::Encoding));
    }

    #[test]
    fn oversized_declarations_are_refused() {
        let a = keys(6);
        let caps: Vec<Capability> = (0..5).map(|i| cap(&format!("s{i}"))).collect();
        let d = Declaration::new(a.did(), 1, 10, caps).expect("coherent");
        assert!(d.validate_against(5).is_ok());
        assert_eq!(d.validate_against(4), Err(CoreError::InvalidKind));
    }

    #[test]
    fn json_roundtrip_keeps_the_signature_valid() {
        let a = keys(7);
        let signed = Declaration::new(a.did(), 9, 99, vec![cap("x")])
            .expect("coherent")
            .sign(&a)
            .expect("signed");
        let value = signed.to_value().expect("serialisable");
        let restored = SignedDeclaration::from_value(&value).expect("parses");
        assert_eq!(restored, signed);
        restored.verify().expect("round trip keeps signature valid");
        assert_eq!(
            restored.content_fingerprint().expect("hash"),
            signed.content_fingerprint().expect("hash")
        );
    }
}
