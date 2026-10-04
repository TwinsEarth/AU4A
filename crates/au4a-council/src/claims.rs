//! 能力清单与证据分级（v1.7.8）。
//!
//! 项目信誉建立在一条规则上：**文档里写的每条能力，代码里必须可达且被测试覆盖**。
//! 本模块把这句话变成数据结构：每条能力都必须声明
//!
//! * 公开接口（`interface`），
//! * **覆盖它的测试名字**（`test`），
//! * 证据等级（[`EvidenceGrade`]）与必要说明（`note`）。
//!
//! 然后由 `tests/claims.rs` 做机器校验：清单里的每个测试名必须能在本 crate 的
//! `tests/*.rs` 或 `src/**/*.rs` 里找到 `fn <名字>(`，`verified` 的条目不得缺测试名，
//! `cpu-proto` 的条目必须写明「哪里是原型、为什么」。测试名不存在 → 测试变红。
//!
//! 换句话说：**吹牛会挂测试**。

use au4a_core::EvidenceGrade;
use serde_json::{json, Value};

/// 一条能力声明。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    /// 稳定 id（`能力.子项`）。
    pub id: &'static str,
    /// 能力描述。
    pub capability: &'static str,
    /// 公开接口（类型或函数名）。
    pub interface: &'static str,
    /// 覆盖它的测试函数名（必须在 crate 源码里真实存在）。
    pub test: &'static str,
    /// 证据等级。
    pub grade: EvidenceGrade,
    /// 说明（`cpu-proto` 必须写明原型边界）。
    pub note: &'static str,
}

impl Claim {
    /// 只读 JSON 投影。
    pub fn as_json(&self) -> Value {
        json!({
            "id": self.id,
            "capability": self.capability,
            "interface": self.interface,
            "test": self.test,
            "grade": self.grade.as_str(),
            "note": self.note,
        })
    }
}

/// 轨道 1.7 的能力清单（单一事实来源：文档与自检都引用它）。
pub const CLAIMS: [Claim; 19] = [
    Claim {
        id: "election.weighted",
        capability: "委员会席位由信誉×在线时长加权选举产生，高信誉长期在线者当选",
        interface: "Council::elect / election::run",
        test: "high_reputation_long_online_agents_win_the_seats",
        grade: EvidenceGrade::Verified,
        note: "本机实测；整数权重，无浮点",
    },
    Claim {
        id: "election.deterministic",
        capability: "同一花名册与同一批选票（任意到达顺序）给出同一选举内容地址",
        interface: "election::run / ElectionOutcome::id",
        test: "election_is_reproducible_across_runs_and_ballot_order",
        grade: EvidenceGrade::Verified,
        note: "两轮选举与乱序选票均断言 id 相同",
    },
    Claim {
        id: "election.sybil",
        capability: "大量信誉为零的空壳 DID 无法当选，也无法改变当选名单",
        interface: "ElectionConfig::min_voter_weight / min_reputation_bp",
        test: "sybil_flood_cannot_win_or_shift_the_outcome",
        grade: EvidenceGrade::Verified,
        note: "60 个空壳 DID 投票后当选名单与基线一致，其选票全部留痕",
    },
    Claim {
        id: "quorum.bft_lite",
        capability: "法定人数 quorum = n - f（f = ⌊(n-1)/3⌋），n = 3f+1 时等于 2f+1",
        interface: "Committee::quorum / Committee::fault_bound / Tally",
        test: "quorum_is_n_minus_f_and_exactly_decides",
        grade: EvidenceGrade::Verified,
        note: "n=4→3、n=7→5、n=10→7 均有断言",
    },
    Claim {
        id: "vote.duplicate_refused",
        capability: "同一委员在同一轮重复投票被拒，且不影响本轮票数",
        interface: "Council::cast_vote",
        test: "a_duplicate_vote_in_the_same_round_is_refused_and_the_round_survives",
        grade: EvidenceGrade::Verified,
        note: "按 conflict 分类（非单次恶意）",
    },
    Claim {
        id: "vote.ambiguous_voids_round",
        capability: "模棱两可（同轮改投/双签）作废整轮，必须重开一轮",
        interface: "RoundOutcome::VoidAmbiguous",
        test: "an_ambiguous_double_vote_voids_the_whole_round",
        grade: EvidenceGrade::Verified,
        note: "作废轮不再收票、动议回到 open、重开一轮可通过、作废留痕",
    },
    Claim {
        id: "proposal.agent_only",
        capability: "动议只能由在任委员 Agent 提出，且内容必须由其私钥签名",
        interface: "AgentIdentity / ProposalDraft / Council::propose",
        test: "a_non_member_agent_cannot_propose",
        grade: EvidenceGrade::Verified,
        note: "非委员 → InvalidSignature + unauthorized；篡改内容 → 签名失效",
    },
    Claim {
        id: "proposal.human_read_only",
        capability: "人类观察者没有 propose / edit / cast_vote / execute 任何方法",
        interface: "HumanObserver（compile_fail 文档测试）",
        test: "the_human_observer_is_read_only",
        grade: EvidenceGrade::Verified,
        note: "4 段 compile_fail 文档测试 + 1 段正例文档测试共同把守",
    },
    Claim {
        id: "veto.blocks_only",
        capability: "人类否决只能把动议推进到 blocked：不可执行、不可复活、不可重开轮次",
        interface: "Veto / HumanObserver::veto / Council::apply_veto",
        test: "a_blocked_motion_can_never_be_executed_or_revived",
        grade: EvidenceGrade::Verified,
        note: "Veto 无 propose/edit 方法（2 段 compile_fail），载荷无可执行字段",
    },
    Claim {
        id: "veto.reason_public",
        capability: "否决必须携带非空公开理由，理由进入治理事件与只读投影",
        interface: "Veto::reason / HumanView::vetoes",
        test: "a_veto_must_carry_a_public_reason",
        grade: EvidenceGrade::Verified,
        note: "空/空白理由在铸造阶段即被拒绝",
    },
    Claim {
        id: "execution.state_change",
        capability: "通过的决议由执行引擎落成真实状态变更（策略/信誉/账本），且幂等",
        interface: "Council::execute / ExecutionReceipt",
        test: "a_passed_motion_becomes_a_real_state_change",
        grade: EvidenceGrade::Verified,
        note: "第二次执行被拒；失败不留部分状态",
    },
    Claim {
        id: "execution.conservation",
        capability: "执行引起的账本变更保持守恒：转账不动总量、罚没量入 slashed",
        interface: "ExecutionReceipt::ledger_effect_consistent / Ledger::check_conservation",
        test: "a_transfer_execution_moves_credits_and_keeps_conservation",
        grade: EvidenceGrade::Verified,
        note: "每张收据记录账本前后快照，可复算",
    },
    Claim {
        id: "ongov.governor_token",
        capability: "治理状态映射为 GovernorToken 语义对象（pending/active/succeeded/executed/defeated/canceled）",
        interface: "ongov::GovernorToken::project",
        test: "the_mapping_covers_the_whole_lifecycle",
        grade: EvidenceGrade::CpuProto,
        note: "只有语义映射：没有真实链、没有真实 tx_ref；真实链上执行属 v1.8 au4a-chain",
    },
    Claim {
        id: "invariants.replay",
        capability: "确定性重放 + 每步 17 条不变式：治理历史可复现且始终自洽",
        interface: "invariants::replay / check_all / state_digest",
        test: "replay_exercises_every_failure_mode_and_holds_invariants",
        grade: EvidenceGrade::Verified,
        note: "同种子逐字节可复现；四种结局均被真实触发",
    },
    Claim {
        id: "docs.claims_machine_checked",
        capability: "本清单的每个测试名都能在源码里找到，文档与代码不允许脱节",
        interface: "claims::CLAIMS + tests/claims.rs",
        test: "every_claim_points_at_a_real_test",
        grade: EvidenceGrade::Verified,
        note: "tests/claims.rs 会扫描 tests/ 与 src/ 源码核对测试名",
    },
    Claim {
        id: "emergency.security_channel",
        capability: "安全委员会紧急通道可即时下发策略，事后必须由全体在任委员签名确认（否决/超期则回滚）",
        interface: "Council::issue_emergency / confirm_emergency / EmergencyDirective",
        test: "the_scenario_runs_the_full_flow_including_emergency",
        grade: EvidenceGrade::Verified,
        note: "只有安全委员会可下发（其他委员会 → unauthorized）；确认窗口用逻辑刻度",
    },
    Claim {
        id: "example.runnable",
        capability: "可运行示例 governance_demo：跑完整治理流程并打印自检、清单与重放结果",
        interface: "examples/governance_demo.rs",
        test: "the_example_source_is_a_real_runnable_program",
        grade: EvidenceGrade::Verified,
        note: "不读文件、不开网络、不读墙钟；由 cargo test 编译保证可构建",
    },
    Claim {
        id: "audit.hash_chain",
        capability: "治理事件串成哈希链：改一条/删一条/换顺序都会断链并指出第一处位置",
        interface: "audit::AuditLog::rebuild / verify / root",
        test: "tampering_with_any_entry_breaks_the_chain_at_that_position",
        grade: EvidenceGrade::Verified,
        note: "链由逻辑刻度与内容哈希构成，可被任何节点独立复算",
    },
    Claim {
        id: "audit.readonly_export",
        capability: "只读导出提案/否决/紧急指令/执行收据/策略/不变式的审计快照（导出不改变治理状态）",
        interface: "audit::AuditExport / Council::audit_export",
        test: "the_export_is_read_only_and_carries_the_whole_governance_record",
        grade: EvidenceGrade::Verified,
        note: "导出前后 state_digest 相同，证明没有写路径",
    },
];

/// 清单自洽：id 唯一、测试名非空、等级与说明匹配。
pub fn claims_are_well_formed() -> bool {
    let mut ids: Vec<&str> = CLAIMS.iter().map(|c| c.id).collect();
    let before = ids.len();
    ids.sort();
    ids.dedup();
    let ids_unique = ids.len() == before;
    let populated = CLAIMS
        .iter()
        .all(|c| !c.id.is_empty() && !c.capability.is_empty() && !c.interface.is_empty() && !c.test.is_empty());
    // verified 必须有测试名；cpu-proto 必须写明边界；unverified 不允许出现在清单里。
    let graded = CLAIMS.iter().all(|c| match c.grade {
        EvidenceGrade::Verified => !c.test.is_empty(),
        EvidenceGrade::CpuProto => !c.test.is_empty() && !c.note.is_empty(),
        EvidenceGrade::Unverified => false,
    });
    ids_unique && populated && graded
}

/// 清单 JSON（自检与观察层共用）。
pub fn claims_json() -> Value {
    json!({
        "total": CLAIMS.len(),
        "verified": CLAIMS.iter().filter(|c| c.grade == EvidenceGrade::Verified).count(),
        "cpu_proto": CLAIMS.iter().filter(|c| c.grade == EvidenceGrade::CpuProto).count(),
        "unverified": CLAIMS.iter().filter(|c| c.grade == EvidenceGrade::Unverified).count(),
        "claims": CLAIMS.iter().map(|c| c.as_json()).collect::<Vec<_>>(),
    })
}

/// 一行摘要（人类可读）。
pub fn summary() -> String {
    let verified = CLAIMS.iter().filter(|c| c.grade == EvidenceGrade::Verified).count();
    let proto = CLAIMS.iter().filter(|c| c.grade == EvidenceGrade::CpuProto).count();
    format!("{} 条能力：{} verified / {} cpu-proto / 0 unverified", CLAIMS.len(), verified, proto)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_claim_table_is_well_formed() {
        assert!(claims_are_well_formed());
        assert_eq!(CLAIMS.len(), 19);
        // 不允许出现 unverified：做不到的能力不写进清单。
        assert!(CLAIMS.iter().all(|c| c.grade != EvidenceGrade::Unverified));
        let summary = summary();
        assert!(summary.contains("verified"));
        assert!(summary.contains("cpu-proto"));
    }

    #[test]
    fn cpu_proto_claims_explain_their_boundary() {
        for claim in CLAIMS.iter().filter(|c| c.grade == EvidenceGrade::CpuProto) {
            assert!(
                claim.note.contains("v1.8") || claim.note.contains("没有真实链"),
                "{} 必须写明原型边界：{}",
                claim.id,
                claim.note
            );
        }
    }

    #[test]
    fn claims_json_is_machine_readable() {
        let value = claims_json();
        assert_eq!(value["total"], 19);
        assert_eq!(value["unverified"], 0);
        assert_eq!(
            value["verified"].as_u64().unwrap_or(0) + value["cpu_proto"].as_u64().unwrap_or(0),
            19
        );
        let first = &value["claims"][0];
        assert!(first["id"].is_string());
        assert!(EvidenceGrade::parse(first["grade"].as_str().unwrap_or("")).is_some());
    }
}
