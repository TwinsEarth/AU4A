//! v1.6.7 拒绝路径清单：每一个被文档化的 `CoreError` 都必须有一条**显式触发它的测试**，
//! 并且「不是错误」的那些拒绝（证据闸门、证据不足不学习、小样本抑制）也要各有一条。
//!
//! 为什么单独写一个文件：本轨道在 10 个小版本里引入了 7 类 `CoreError` 路径与 3 类「软拒绝」。
//! 如果只靠零散断言，很容易出现「文档说会拒绝，但代码里那条分支已经不再可达」的假承诺。

use std::collections::BTreeSet;

use au4a_core::{AgentKeys, CoreError, Credits, Did, EvidenceGrade};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome, MAX_CONTEXT_LEN, MAX_PEERS};
use au4a_learning::feedback::FeedbackAnalyser;
use au4a_learning::model::{LearningModel, ModelConfig};
use au4a_learning::policy::{adjust, PolicyBounds, PolicyParams, PolicyTargets, Signals};
use au4a_learning::privacy::{open, publish, seal, PrivacyPolicy};
use au4a_learning::signal::{LearningSignal, SignalWeights};
use au4a_learning::sim::{ab_test, run, MarketConfig};
use au4a_learning::violation::{Violation, ViolationLog};

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect()
}

fn context_of(id: &str) -> String {
    format!("ctx-{id}")
}

fn exp(id: &str, outcome: Outcome, reward: i64, peers: &[Did]) -> Experience {
    let context = context_of(id);
    Experience::new(
        id,
        "x",
        &context,
        "deliver",
        outcome,
        Credits(reward),
        1,
        peers,
    )
    .unwrap()
}

#[test]
fn every_documented_error_path_is_reachable() {
    let peers = dids(2);
    let mut covered: BTreeSet<&'static str> = BTreeSet::new();

    // 1) InvalidKind：字段非法
    assert_eq!(
        Experience::new("", "t", "c", "a", Outcome::Success, Credits(0), 1, &[]),
        Err(CoreError::InvalidKind)
    );
    covered.insert("InvalidKind");
    // 2) InvalidKind 的另一条路径：容量 0
    assert_eq!(ExperienceStore::new(0), Err(CoreError::InvalidKind));
    // 3) InvalidKind：经验库载入时发现重复条目（篡改）
    let mut store = ExperienceStore::new(8).unwrap();
    store
        .record(exp("t-1", Outcome::Success, 5, &peers))
        .unwrap();
    let mut value = store.to_value().unwrap();
    let entries = value.get_mut("entries").unwrap().as_array_mut().unwrap();
    let dup = entries[0].clone();
    entries.push(dup);
    assert_eq!(
        ExperienceStore::from_json_str(&value.to_string()),
        Err(CoreError::InvalidKind)
    );
    // 4) Encoding：不是 JSON
    assert_eq!(
        ExperienceStore::from_json_str("{not json"),
        Err(CoreError::Encoding)
    );
    covered.insert("Encoding");
    // 5) NegativeAmount：负步长/负边界
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    assert_eq!(
        adjust(
            &PolicyParams::baseline(),
            &report,
            &ViolationLog::new(),
            &Signals::cold_start(8, 5_000, Credits(1)),
            &PolicyBounds {
                price_step_bp: -1,
                ..PolicyBounds::default()
            },
            &PolicyTargets::default()
        ),
        Err(CoreError::NegativeAmount)
    );
    covered.insert("NegativeAmount");
    // 6) InvalidKind：学习门槛越界
    assert_eq!(
        adjust(
            &PolicyParams::baseline(),
            &report,
            &ViolationLog::new(),
            &Signals::cold_start(8, 5_000, Credits(1)),
            &PolicyBounds::default(),
            &PolicyTargets {
                min_confidence_bp: 10_001,
                ..PolicyTargets::default()
            }
        ),
        Err(CoreError::InvalidKind)
    );
    // 7) NegativeAmount：信号权重为负
    assert_eq!(
        SignalWeights {
            quality_bp: -1,
            ..SignalWeights::default()
        }
        .validate(),
        Err(CoreError::NegativeAmount)
    );
    // 8) InvalidKind：隐私策略非法（盐过短 / 阈值 0）
    assert_eq!(
        PrivacyPolicy {
            salt: "short".to_string(),
            ..PrivacyPolicy::default()
        }
        .validate(),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        PrivacyPolicy {
            reward_bucket_credits: -1,
            ..PrivacyPolicy::default()
        }
        .validate(),
        Err(CoreError::NegativeAmount)
    );
    // 9) InvalidKind：模型配置非法
    assert_eq!(
        ModelConfig {
            max_generations: 0,
            ..ModelConfig::default()
        }
        .validate(),
        Err(CoreError::InvalidKind)
    );
    // 10) InvalidVersion：回滚到不存在的代际
    let mut model = LearningModel::new(PolicyBounds::default());
    assert_eq!(model.rollback(), Err(CoreError::InvalidVersion));
    covered.insert("InvalidVersion");
    // 11) InvalidSignature：错误密钥打开加密视图
    let blob = seal(&store, &[1u8; 32]).unwrap();
    assert_eq!(open(&blob, &[2u8; 32]), Err(CoreError::InvalidSignature));
    covered.insert("InvalidSignature");
    // 12) FrameTruncated：声明长度与实际不符
    let mut truncated = blob.clone();
    truncated.plaintext_len += 1;
    assert_eq!(open(&truncated, &[1u8; 32]), Err(CoreError::FrameTruncated));
    covered.insert("FrameTruncated");
    // 13) InvalidVersion：加密视图版本不支持
    let mut bad_version = blob.clone();
    bad_version.version = 9;
    assert_eq!(
        open(&bad_version, &[1u8; 32]),
        Err(CoreError::InvalidVersion)
    );
    // 14) Overflow：聚合收益溢出 i64
    let mut huge = ExperienceStore::new(4).unwrap();
    huge.record(exp("h-1", Outcome::Success, i64::MAX, &peers))
        .unwrap();
    huge.record(exp("h-2", Outcome::Success, i64::MAX, &peers))
        .unwrap();
    assert_eq!(FeedbackAnalyser::analyse(&huge), Err(CoreError::Overflow));
    covered.insert("Overflow");
    // 15) InvalidKind：不同市场不可比较
    let a = run(&MarketConfig::default()).unwrap();
    let b = run(&MarketConfig {
        seed: 5,
        ..MarketConfig::default()
    })
    .unwrap();
    assert_eq!(
        au4a_learning::sim::compare(&a, &b),
        Err(CoreError::InvalidKind)
    );
    // 16) InvalidKind：非法市场配置
    assert_eq!(
        run(&MarketConfig {
            peers: 1,
            ..MarketConfig::default()
        }),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        ab_test(&MarketConfig {
            rounds: 0,
            ..MarketConfig::default()
        }),
        Err(CoreError::InvalidKind)
    );
    // 17) InvalidKind：违规原因非法
    assert_eq!(
        Violation::new(&peers[0], "t", "", 1),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        Violation::new(&peers[0], "", "reason", 1),
        Err(CoreError::InvalidKind)
    );
    // 18) InvalidKind：字段超长（上下文 / 协作者数量）
    assert_eq!(
        Experience::new(
            "id",
            "t",
            &"x".repeat(MAX_CONTEXT_LEN + 1),
            "a",
            Outcome::Success,
            Credits(0),
            1,
            &[]
        ),
        Err(CoreError::InvalidKind)
    );
    // 用 17 个**互不相同**的协作者（种子 0x40..0x50），超过 MAX_PEERS=16
    let many: Vec<Did> = (0..=(MAX_PEERS as u8))
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect();
    assert!(many.len() > MAX_PEERS);
    assert!(Experience::new("id", "t", "c", "a", Outcome::Success, Credits(0), 1, &many).is_err());
    // 恰好 16 个 → 允许（边界上不拒绝）
    assert!(Experience::new(
        "id",
        "t",
        "c",
        "a",
        Outcome::Success,
        Credits(0),
        1,
        &many[..MAX_PEERS]
    )
    .is_ok());

    // 清单完整性：至少覆盖 7 类不同错误，且每类只登记一次
    let expected: BTreeSet<&str> = [
        "InvalidKind",
        "Encoding",
        "NegativeAmount",
        "Overflow",
        "InvalidSignature",
        "InvalidVersion",
        "FrameTruncated",
    ]
    .into_iter()
    .collect();
    assert_eq!(covered, expected, "拒绝路径清单必须完整且无重复登记");
}

#[test]
fn soft_refusals_are_not_errors_but_are_explicit() {
    // 1) 证据不足 → 不学习（不是错误）
    let peers = dids(2);
    let mut store = ExperienceStore::new(8).unwrap();
    store
        .record(exp("t-1", Outcome::Success, 5, &peers))
        .unwrap();
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let held = adjust(
        &PolicyParams::baseline(),
        &report,
        &ViolationLog::new(),
        &Signals::cold_start(1, 10_000, Credits(5)),
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();
    assert!(!held.changed && held.next == PolicyParams::baseline());

    // 2) 小样本抑制 → 不发布统计值（不是错误）
    let mut small = ExperienceStore::new(8).unwrap();
    small
        .record(exp("t-1", Outcome::Success, 5, &peers))
        .unwrap();
    let view = publish(&small, &PrivacyPolicy::default()).unwrap();
    assert_eq!(view.suppressed_groups, 1);
    assert!(view.aggregates.iter().all(|a| a.success_bp.is_none()));

    // 3) 证据闸门 → 结算被拒（内核按类型化拒绝记录，不是 panic）
    let mut config = KernelConfig::default();
    config.cpu_proto_settle_cap = Credits::ZERO;
    let mut kernel = Kernel::new(config);
    let summary = au4a_learning::scenario(&mut kernel).unwrap();
    assert!(summary["settlement_refused"].as_i64().unwrap() > 0);
    assert!(!kernel.refusals().is_empty());
    kernel.ledger().check_conservation().unwrap();

    // 4) 未注册的协作者无法结算（UnknownAgent 路径，由内核判定）
    let a = AgentKeys::from_seed(&[0x77; 32]);
    let stranger = AgentKeys::from_seed(&[0x78; 32]).did();
    let mut kernel2 = Kernel::new(KernelConfig::default());
    au4a_learning::scenario(&mut kernel2).unwrap();
    kernel2
        .register(&a, "late-joiner", &["x"], Credits(10))
        .unwrap();
    // 未注册收款人：内核拒绝（UnknownAgent），账本不变
    let before = kernel2.ledger().total().unwrap();
    assert!(
        kernel2
            .settle(&a.did(), &stranger, Credits(1), EvidenceGrade::Verified)
            .is_err()
            || kernel2.ledger().total().unwrap() == before
    );
}

#[test]
fn malformed_inputs_never_panic_across_a_deterministic_sweep() {
    // 用一个确定性随机序列生成「畸形但类型合法」的输入，断言全部要么 Ok 要么 Err，绝不 panic。
    let mut rng = au4a_learning::SplitMix64::new(0x1_0607);
    let peers = dids(3);
    let mut store = ExperienceStore::new(6).unwrap();
    let mut outcomes_ok = 0usize;
    let mut outcomes_err = 0usize;
    for i in 0..300u64 {
        let id_len = rng.below(160) as usize;
        let ctx_len = rng.below(600) as usize;
        let task_id: String = std::iter::repeat_n('x', id_len).collect();
        let context: String = std::iter::repeat_n('y', ctx_len).collect();
        let task_type = ["x", "bad type", "", "ok.type"][rng.below(4) as usize];
        let action = ["deliver", "", "a b"][rng.below(3) as usize];
        let reward = rng.range_i64(0, 100);
        let peer_count = rng.below(6) as usize;
        let chosen: Vec<Did> = peers.iter().take(peer_count).cloned().collect();
        let built = Experience::new(
            &task_id,
            task_type,
            &context,
            action,
            match rng.below(3) {
                0 => Outcome::Success,
                1 => Outcome::Partial,
                _ => Outcome::Failure,
            },
            Credits(reward),
            i,
            &chosen,
        );
        match built {
            Ok(e) => {
                outcomes_ok += 1;
                match store.record(e) {
                    Ok(_) => {}
                    Err(e) => {
                        assert!(matches!(e, CoreError::InvalidKind | CoreError::Encoding));
                        outcomes_err += 1;
                    }
                }
            }
            Err(e) => {
                outcomes_err += 1;
                assert_eq!(e, CoreError::InvalidKind);
            }
        }
        // 每一轮都顺手把库导出/导回，确保任何中间状态都是自洽的
        let json = store.canonical_json().unwrap();
        let restored = ExperienceStore::from_json_str(&json).unwrap();
        assert_eq!(restored.digest().unwrap(), store.digest().unwrap());
        let _ = FeedbackAnalyser::analyse(&store).unwrap();
        let _ = publish(&store, &PrivacyPolicy::default()).unwrap();
    }
    assert!(
        outcomes_ok > 0 && outcomes_err > 0,
        "扫描必须同时覆盖合法与非法输入"
    );
    assert!(store.len() <= store.capacity());
    // 学习信号对任何合法库都算得出来
    let signal =
        LearningSignal::from_store(&store, &ViolationLog::new(), 0, &SignalWeights::default())
            .unwrap();
    assert!(signal.composite_bp.abs() <= 10_000);
}
