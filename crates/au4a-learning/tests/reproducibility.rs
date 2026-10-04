//! v1.6.7 逐字节复现：同一输入两次独立运行必须产生**完全相同的规范 JSON**。
//!
//! 覆盖轨道里所有对外产物：经验库、反馈报告、策略调整、模型摘要、市场跑批、公开视图、场景摘要。
//! 复现性是本轨道的核心卖点之一（「同种子两次运行结果逐字节一致」），所以每类产物都要被直接断言，
//! 而不是只断言某一个顶层摘要。

use au4a_core::{canonicalize, AgentKeys, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome};
use au4a_learning::feedback::FeedbackAnalyser;
use au4a_learning::model::LearningModel;
use au4a_learning::policy::{adjust, PolicyBounds, PolicyParams, PolicyTargets, Signals};
use au4a_learning::privacy::{publish, seal, PrivacyPolicy};
use au4a_learning::sim::{ab_test, MarketConfig};

fn peers() -> Vec<Did> {
    (0..3u8)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect()
}

fn build_store() -> ExperienceStore {
    let peers = peers();
    let mut store = ExperienceStore::new(24).unwrap();
    for i in 0..12u64 {
        let task_type = ["translate.en-zh", "summarize.zh", "classify.zh"][(i % 3) as usize];
        let outcome = match i % 4 {
            0 | 1 => Outcome::Success,
            2 => Outcome::Partial,
            _ => Outcome::Failure,
        };
        let context = format!("复现测试上下文 #{i}");
        store
            .record(
                Experience::new(
                    &format!("task-{i:03}"),
                    task_type,
                    &context,
                    "deliver",
                    outcome,
                    Credits(((i % 5) * 10) as i64),
                    i,
                    &peers[..1 + (i % 3) as usize],
                )
                .unwrap(),
            )
            .unwrap();
    }
    store
}

/// 把一次完整的学习闭环压成一个可比较的 JSON 值。
fn pipeline_snapshot() -> serde_json::Value {
    let store = build_store();
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let signals = Signals::cold_start(
        store.len(),
        report.overall.quality_bp,
        report.overall.mean_reward,
    );
    let adjustment = adjust(
        &PolicyParams::baseline(),
        &report,
        &au4a_learning::violation::ViolationLog::new(),
        &signals,
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();
    let mut model = LearningModel::new(PolicyBounds::default());
    let mut record = None;
    for _ in 0..3 {
        record = Some(model.apply(&adjustment).unwrap());
    }
    let policy = PrivacyPolicy::default();
    let view = publish(&store, &policy).unwrap();
    let sealed = seal(&store, &[0x16u8; 32]).unwrap();

    serde_json::json!({
        "store_digest": store.digest().unwrap(),
        "store_canonical": store.canonical_json().unwrap(),
        "feedback_digest": report.digest().unwrap(),
        "adjustment": adjustment.to_value().unwrap(),
        "model_digest": model.digest().unwrap(),
        "update_record": record,
        "view_digest": view.digest().unwrap(),
        "view": view.to_value().unwrap(),
        "sealed_key_tag": sealed.key_tag,
        "sealed_ciphertext": sealed.ciphertext_hex,
    })
}

#[test]
fn every_product_is_byte_identical_across_two_independent_pipelines() {
    let a = pipeline_snapshot();
    let b = pipeline_snapshot();
    assert_eq!(
        canonicalize(&a).unwrap(),
        canonicalize(&b).unwrap(),
        "两次独立流水线的规范 JSON 必须逐字节相同"
    );
    // 也断言中间产物各自的摘要相同（更细的失败定位）
    assert_eq!(a["store_digest"], b["store_digest"]);
    assert_eq!(a["feedback_digest"], b["feedback_digest"]);
    assert_eq!(a["model_digest"], b["model_digest"]);
    assert_eq!(a["view_digest"], b["view_digest"]);
    assert_eq!(a["sealed_ciphertext"], b["sealed_ciphertext"]);
}

#[test]
fn changing_any_input_changes_the_digest() {
    let base = build_store();
    let mut mutated = build_store();
    // 改一条经验的结局 → 经验库摘要必须变化（内容寻址必须敏感）
    let mut entries = mutated.entries().to_vec();
    let first = entries.remove(0);
    let context = first.context.clone();
    let changed = Experience::new(
        &first.task_id,
        &first.task_type,
        &context,
        "different-action",
        Outcome::Failure,
        first.reward,
        first.timestamp,
        &first.peer_agents,
    )
    .unwrap();
    mutated.record(changed).unwrap();
    assert_ne!(base.digest().unwrap(), mutated.digest().unwrap());
    assert_ne!(
        base.canonical_json().unwrap(),
        mutated.canonical_json().unwrap()
    );

    // 反馈报告、公开视图、加密切片也都随内容变化
    let policy = PrivacyPolicy::default();
    assert_ne!(
        FeedbackAnalyser::analyse(&base).unwrap().digest().unwrap(),
        FeedbackAnalyser::analyse(&mutated)
            .unwrap()
            .digest()
            .unwrap()
    );
    assert_ne!(
        publish(&base, &policy).unwrap().digest().unwrap(),
        publish(&mutated, &policy).unwrap().digest().unwrap()
    );
    assert_ne!(
        seal(&base, &[1u8; 32]).unwrap().ciphertext_hex,
        seal(&mutated, &[1u8; 32]).unwrap().ciphertext_hex
    );
}

#[test]
fn market_and_scenario_are_reproducible() {
    let config = MarketConfig::default();
    let first = ab_test(&config).unwrap();
    let second = ab_test(&config).unwrap();
    let a = canonicalize(&first.1.to_value().unwrap()).unwrap();
    let b = canonicalize(&second.1.to_value().unwrap()).unwrap();
    assert_eq!(a, b);

    // 场景摘要（含内核账本、结算、隐私投影）两次运行逐字节一致
    let mut k1 = Kernel::new(KernelConfig::default());
    let mut k2 = Kernel::new(KernelConfig::default());
    let s1 = au4a_learning::scenario(&mut k1).unwrap();
    let s2 = au4a_learning::scenario(&mut k2).unwrap();
    assert_eq!(canonicalize(&s1).unwrap(), canonicalize(&s2).unwrap());
    // 内核账本状态也必须一致（结算真的动账，且动得一样）
    assert_eq!(
        serde_json::to_string(&k1.ledger().view()).unwrap(),
        serde_json::to_string(&k2.ledger().view()).unwrap()
    );
    assert_eq!(k1.now(), k2.now(), "逻辑时钟读数是内容的一部分");
}

#[test]
fn digest_is_a_function_of_content_not_of_insertion_order_of_math() {
    // 同一批经验用不同**构造顺序**写入 → 摘要不同（追加序是证据序），
    // 但内容键集合相同；这防止把「摘要相同」误当成「顺序无关」。
    let peers = peers();
    let make = |order: &[u64]| {
        let mut store = ExperienceStore::new(16).unwrap();
        for i in order {
            let context = format!("顺序测试 #{i}");
            store
                .record(
                    Experience::new(
                        &format!("t-{i}"),
                        "x",
                        &context,
                        "d",
                        Outcome::Success,
                        Credits(*i as i64),
                        *i,
                        &peers[..1],
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        store
    };
    let a = make(&[1, 2, 3]);
    let b = make(&[3, 2, 1]);
    assert_eq!(a.len(), b.len());
    assert_ne!(a.digest().unwrap(), b.digest().unwrap());
    // 同一顺序 → 同一摘要
    assert_eq!(make(&[1, 2, 3]).digest().unwrap(), a.digest().unwrap());
}
