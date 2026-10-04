//! 宿主内核审计（v1.0.1）：把「内核自己声称的不变式」变成可断言的对象。
//!
//! 自治内核不能靠人来检查。每一次注册、投递、结算之后，网络里任何一个节点都应该能
//! 独立跑一遍同一组检查，并得到同一个结论。本模块就是那组检查：
//!
//! * 账本守恒：`Σ可用 + Σ锁定 + 罚没 == 已发行`（基元层不变式，这里把它纳入宿主视图）；
//! * 质押覆盖：名片上声明的质押必须真的锁在账本里（声明的 ≠ 已锁定的 = 空头承诺）；
//! * 准入下限：没有 Agent 低于 `min_stake`；
//! * 注册表自洽：能力倒排索引是名片集合的忠实派生，且无重复身份；
//! * 队列成员：待投递信封的收发双方都在册（防止伪造来源长期滞留）；
//! * 拒绝分类：每条拒绝的码都必须是十个已知码之一，并统计恶意 2 码 vs 竞争 8 码；
//! * 逻辑时钟单调：进度事件的时间戳不倒退，且不超前于当前读数。
//!
//! 审计结果是值（可序列化、可比较），既给 `au4a-node verify` 聚合，
//! 也给人类观察层的「结果」面板展示——人类只能看，不能改。

use au4a_core::{CoreResult, RefusalCode, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Kernel;

/// 一条审计结论。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFinding {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

impl AuditFinding {
    pub fn pass(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            ok: true,
            detail: detail.into(),
        }
    }

    pub fn fail(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            ok: false,
            detail: detail.into(),
        }
    }
}

/// 一次宿主审计的全部结论。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostAudit {
    pub findings: Vec<AuditFinding>,
}

impl HostAudit {
    pub fn is_clean(&self) -> bool {
        self.findings.iter().all(|f| f.ok)
    }

    pub fn failures(&self) -> Vec<&AuditFinding> {
        self.findings.iter().filter(|f| !f.ok).collect()
    }

    /// 转成自检项（节点聚合时使用）。
    pub fn to_self_checks(&self, track: &str) -> Vec<SelfCheck> {
        self.findings
            .iter()
            .map(|f| {
                if f.ok {
                    SelfCheck::pass(track, &f.name, f.detail.clone())
                } else {
                    SelfCheck::fail(track, &f.name, f.detail.clone())
                }
            })
            .collect()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "clean": self.is_clean(),
            "checks": self.findings.len(),
            "failed": self.failures().len(),
            "findings": self.findings,
        })
    }
}

/// 对宿主内核跑一遍全部不变式检查。
pub fn audit_kernel(kernel: &Kernel) -> HostAudit {
    let mut findings = Vec::new();

    // 1. 账本守恒。
    findings.push(match kernel.ledger().check_conservation() {
        Ok(()) => AuditFinding::pass(
            "ledger.conservation",
            format!(
                "Σ可用+Σ锁定+罚没 == 发行（发行 {}，罚没 {}）",
                kernel.ledger().minted(),
                kernel.ledger().slashed()
            ),
        ),
        Err(e) => AuditFinding::fail("ledger.conservation", format!("守恒被打破：{e}")),
    });

    // 2. 准入下限。
    let under_staked: Vec<&str> = kernel
        .agents()
        .filter(|c| c.stake < kernel.config().min_stake)
        .map(|c| c.display.as_str())
        .collect();
    findings.push(if under_staked.is_empty() {
        AuditFinding::pass("admission.min_stake", "无 Agent 低于质押下限")
    } else {
        AuditFinding::fail(
            "admission.min_stake",
            format!("{} 个 Agent 低于下限：{:?}", under_staked.len(), under_staked),
        )
    });

    // 3. 质押覆盖：名片声明必须等于账本锁定（允许锁定更多，不允许更少）。
    let uncovered: Vec<String> = kernel
        .agents()
        .filter(|c| kernel.ledger().balance(&c.did).locked < c.stake)
        .map(|c| {
            format!(
                "{} 声明 {} 实锁 {}",
                c.display,
                c.stake,
                kernel.ledger().balance(&c.did).locked
            )
        })
        .collect();
    findings.push(if uncovered.is_empty() {
        AuditFinding::pass("stake.covered", "每个 Agent 的声明质押都真的锁在账本里")
    } else {
        AuditFinding::fail("stake.covered", format!("空头质押：{uncovered:?}"))
    });

    // 4. 注册表自洽：索引忠实 + 无重复身份 + 无重复技能声明。
    let dups = kernel.registry_defects();
    findings.push(if dups.is_empty() {
        AuditFinding::pass("registry.consistent", "注册序唯一、能力索引忠实于名片集合")
    } else {
        AuditFinding::fail("registry.consistent", format!("注册表瑕疵：{dups:?}"))
    });

    // 5. 队列成员：在册才能收发。
    let outsiders: Vec<String> = kernel
        .queued_envelopes()
        .iter()
        .filter(|env| {
            !kernel.is_registered(&env.from)
                || env
                    .to
                    .as_ref()
                    .map(|did| !kernel.is_registered(did))
                    .unwrap_or(false)
        })
        .map(|env| env.id.clone())
        .collect();
    findings.push(if outsiders.is_empty() {
        AuditFinding::pass("queue.membership", "待投递信封的收发双方都在册")
    } else {
        AuditFinding::fail("queue.membership", format!("非成员信封：{outsiders:?}"))
    });

    // 6. 拒绝分类：码必须合法，并给出恶意/竞争的分布（审计本身不改变处置）。
    let mut unknown = 0usize;
    let mut misconduct = 0usize;
    let mut competitive = 0usize;
    for (_, refusal) in kernel.refusals() {
        if RefusalCode::ALL.contains(&refusal.code) {
            if refusal.code.is_misconduct() {
                misconduct += 1;
            } else {
                competitive += 1;
            }
        } else {
            unknown += 1;
        }
    }
    findings.push(if unknown == 0 {
        AuditFinding::pass(
            "refusal.classification",
            format!("{} 条拒绝：恶意码 {misconduct} / 竞争码 {competitive}", kernel.refusals().len()),
        )
    } else {
        AuditFinding::fail(
            "refusal.classification",
            format!("{unknown} 条拒绝使用了未知码"),
        )
    });

    // 7. 逻辑时钟单调：进度事件不倒退、不超前。
    let now = kernel.now();
    let mut last = 0u64;
    let mut backwards = 0usize;
    for event in kernel.progress_events() {
        if event.at < last || event.at > now {
            backwards += 1;
        }
        last = event.at;
    }
    findings.push(if backwards == 0 {
        AuditFinding::pass("clock.monotonic", format!("{} 条事件时间戳单调且不超前", kernel.progress_events().len()))
    } else {
        AuditFinding::fail("clock.monotonic", format!("{backwards} 条事件时间戳异常"))
    });

    HostAudit { findings }
}

/// 审计结论的稳定摘要（供 scenario 返回 JSON 用）。
pub fn audit_digest(audit: &HostAudit) -> CoreResult<Value> {
    Ok(json!({
        "clean": audit.is_clean(),
        "checks": audit.findings.len(),
        "failed": audit.failures().iter().map(|f| f.name.clone()).collect::<Vec<String>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{registry::AgentRegistry, AgentCard, KernelConfig};
    use au4a_core::{AgentKeys, Credits, EvidenceGrade};

    fn kernel_with_two_agents() -> Kernel {
        let mut k = Kernel::new(KernelConfig::default());
        for (seed, name) in [(1u8, "a"), (2u8, "b")] {
            let keys = AgentKeys::from_seed(&[seed; 32]);
            k.register(&keys, name, &["x"], Credits(20)).unwrap();
        }
        k
    }

    #[test]
    fn a_healthy_kernel_passes_every_finding() {
        let k = kernel_with_two_agents();
        let audit = audit_kernel(&k);
        assert_eq!(audit.failures().len(), 0, "{:?}", audit.failures());
        assert!(audit.is_clean());
        assert!(audit.findings.len() >= 7);
        assert!(audit.to_json()["clean"].as_bool().unwrap_or(false));
    }

    #[test]
    fn empty_stake_claim_is_caught_even_when_registration_was_bypassed() {
        // 直接构造一张「声明了 100 但账本只锁了 20」的名片，审计必须发现空头质押。
        let keys = AgentKeys::from_seed(&[9u8; 32]);
        let card = AgentCard {
            did: keys.did(),
            display: "liar".to_string(),
            skills: vec!["x".to_string()],
            stake: Credits(100),
            evidence: EvidenceGrade::Verified,
        };
        let mut k = Kernel::new(KernelConfig::default());
        k.register(&keys, "liar", &["x"], Credits(20)).unwrap();
        k.replace_card_for_audit(card);
        let audit = audit_kernel(&k);
        assert!(!audit.is_clean());
        assert!(audit
            .failures()
            .iter()
            .any(|f| f.name == "stake.covered"));
    }

    #[test]
    fn an_inconsistent_skill_index_is_caught() {
        // 制造「索引与名片集合不一致」：审计必须发现，而不是相信索引。
        let mut k = kernel_with_two_agents();
        k.break_skill_index_for_audit();
        let audit = audit_kernel(&k);
        assert!(!audit.is_clean());
        assert!(audit
            .failures()
            .iter()
            .any(|f| f.name == "registry.consistent"));
        assert!(!audit.to_self_checks("1.0").iter().all(|c| c.passed));
    }

    #[test]
    fn duplicate_skill_declarations_are_a_registry_defect() {
        let keys = AgentKeys::from_seed(&[4u8; 32]);
        let mut k = Kernel::new(KernelConfig::default());
        k.register(&keys, "dup", &["x", "x"], Credits(20)).unwrap();
        assert!(!k.registry_defects().is_empty());
        let audit = audit_kernel(&k);
        assert!(audit
            .failures()
            .iter()
            .any(|f| f.name == "registry.consistent"));
    }

    #[test]
    fn audit_json_is_stable_for_the_same_state() {
        let a = audit_kernel(&kernel_with_two_agents());
        let b = audit_kernel(&kernel_with_two_agents());
        assert_eq!(a, b);
        assert_eq!(a.to_json(), b.to_json());
        assert_eq!(
            audit_digest(&a).unwrap(),
            audit_digest(&b).unwrap()
        );
    }

    #[test]
    fn registry_of_a_fresh_kernel_is_faithful_by_construction() {
        let reg = AgentRegistry::new();
        assert!(reg.is_empty());
        assert!(reg.index_is_faithful());
    }
}
