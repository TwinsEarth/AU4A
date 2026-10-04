//! v1.6.6 隐私保护集成测试：公开视图脱敏、小样本抑制、本地加密视图、错密钥被拒。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome};
use au4a_learning::privacy::{
    open, peer_tag, public_json, publish, seal, PrivacyPolicy, PublicView, MIN_SALT_LEN,
};

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect()
}

/// 5 条热门类型（含秘密上下文）+ 1 条冷门类型 + 1 条带敏感任务标识的经验。
fn secret_store(peers: &[Did]) -> ExperienceStore {
    assert!(!peers.is_empty(), "至少要有一个协作者");
    let cold_peers = if peers.len() > 1 {
        &peers[1..2]
    } else {
        &peers[..1]
    };
    let mut store = ExperienceStore::new(32).unwrap();
    for i in 0..5u64 {
        let context = format!("SECRET-CONTEXT-{i}-客户原始需求不可外泄");
        store
            .record(
                Experience::new(
                    &format!("secret-task-{i}"),
                    "translate.en-zh",
                    &context,
                    "deliver",
                    if i == 4 {
                        Outcome::Failure
                    } else {
                        Outcome::Success
                    },
                    Credits(37),
                    10 + i,
                    &peers[..1],
                )
                .unwrap(),
            )
            .unwrap();
    }
    store
        .record(
            Experience::new(
                "secret-task-9",
                "cold.start.type",
                "另一个秘密上下文 SECRET-CONTEXT-9",
                "deliver",
                Outcome::Partial,
                Credits(11),
                99,
                cold_peers,
            )
            .unwrap(),
        )
        .unwrap();
    store
}

#[test]
fn public_view_leaks_nothing_sensitive() {
    let peers = dids(2);
    let store = secret_store(&peers);
    let policy = PrivacyPolicy::default();
    let view = publish(&store, &policy).unwrap();
    let text = view.to_value().unwrap().to_string();

    assert!(
        !text.contains("SECRET-CONTEXT"),
        "公开视图泄露上下文：{text}"
    );
    assert!(!text.contains("不可外泄"));
    assert!(!text.contains("did:au4a:"), "公开视图泄露 DID：{text}");
    assert!(
        !text.contains("secret-task"),
        "公开视图泄露 task_id：{text}"
    );
    // 金额只以桶出现：37 → 25（宽度 25）；冷门组被抑制
    let hot = view
        .aggregates
        .iter()
        .find(|a| a.task_type == "translate.en-zh")
        .unwrap();
    assert!(!hot.suppressed);
    assert_eq!(hot.sample, 5);
    assert_eq!(hot.success_bp, Some(8_000));
    assert_eq!(hot.mean_reward_bucket, Some(25));
    assert_eq!(hot.distinct_peers, Some(1));
    assert!(hot.context_bucket.is_some());
    assert_eq!(view.total, 6);
    assert_eq!(view.peer_tag_count, 2);
    // 规范 JSON 可编码（无浮点）
    au4a_core::canonicalize(&view.to_value().unwrap()).unwrap();
    let _ = public_json(&store, &policy).unwrap();
}

#[test]
fn small_samples_are_suppressed_not_published() {
    let peers = dids(1);
    let store = secret_store(&peers);
    // 阈值 6 > 热门组 5 条 → 两组都被抑制
    let policy = PrivacyPolicy {
        min_bucket_sample: 6,
        ..PrivacyPolicy::default()
    };
    let view = publish(&store, &policy).unwrap();
    assert_eq!(view.suppressed_groups, 2, "两组都不到 6 条");
    assert!(view.aggregates.iter().all(|a| a.suppressed));
    for a in &view.aggregates {
        assert!(a.success_bp.is_none());
        assert!(a.mean_reward_bucket.is_none());
        assert!(a.context_bucket.is_none());
        assert!(a.distinct_peers.is_none());
        assert!(a.sample > 0, "被抑制的组仍然报告样本数这个事实");
    }
    // 阈值正好等于热门组样本数 → 恰好边界上不抑制（>= 语义），冷门组仍被抑制
    let boundary = publish(
        &store,
        &PrivacyPolicy {
            min_bucket_sample: 5,
            ..PrivacyPolicy::default()
        },
    )
    .unwrap();
    assert_eq!(boundary.suppressed_groups, 1);
    assert!(boundary
        .aggregates
        .iter()
        .any(|a| a.task_type == "translate.en-zh" && !a.suppressed));
    // 阈值放宽到 1 → 不再抑制
    let loose = publish(
        &store,
        &PrivacyPolicy {
            min_bucket_sample: 1,
            ..PrivacyPolicy::default()
        },
    )
    .unwrap();
    assert_eq!(loose.suppressed_groups, 0);
    assert!(loose.aggregates.iter().all(|a| !a.suppressed));
}

#[test]
fn sealed_view_roundtrips_and_refuses_wrong_keys() {
    let peers = dids(2);
    let store = secret_store(&peers);
    let key = [7u8; 32];
    let blob = seal(&store, &key).unwrap();

    // 密文里没有明文（也不是明文的 hex）
    let plaintext_hex = {
        let mut s = String::new();
        for b in store.canonical_json().unwrap().as_bytes() {
            s.push_str(&format!("{b:02x}"));
        }
        s
    };
    assert_ne!(blob.ciphertext_hex, plaintext_hex);
    assert!(!blob.ciphertext_hex.contains("53454352")); // "SECR" 的 ASCII hex

    // 正确密钥往返：条目数与内容摘要一致
    let restored = open(&blob, &key).unwrap();
    assert_eq!(restored.len(), store.len());
    assert_eq!(restored.digest().unwrap(), store.digest().unwrap());
    assert_eq!(
        restored.canonical_json().unwrap(),
        store.canonical_json().unwrap()
    );

    // 错误密钥 → InvalidSignature（不会解出半个库）
    assert_eq!(open(&blob, &[8u8; 32]), Err(CoreError::InvalidSignature));
    // 不同密钥 → 不同密文与标签
    let other = seal(&store, &[9u8; 32]).unwrap();
    assert_ne!(other.ciphertext_hex, blob.ciphertext_hex);
    assert_ne!(other.key_tag, blob.key_tag);
    // 同一密钥 + 同一内容 → 逐字节一致（可复现）
    assert_eq!(seal(&store, &key).unwrap(), blob);
}

#[test]
fn tampering_and_truncation_are_detected() {
    let peers = dids(1);
    let store = secret_store(&peers);
    let key = [3u8; 32];
    let blob = seal(&store, &key).unwrap();

    // 翻转一位密文 → 明文变化 → 标签不匹配
    let mut tampered = blob.clone();
    let last = tampered.ciphertext_hex.pop().unwrap();
    tampered
        .ciphertext_hex
        .push(if last == '0' { '1' } else { '0' });
    assert_eq!(open(&tampered, &key), Err(CoreError::InvalidSignature));

    // 长度不一致 → FrameTruncated
    let mut truncated = blob.clone();
    truncated.plaintext_len += 1;
    assert_eq!(open(&truncated, &key), Err(CoreError::FrameTruncated));

    // 非法 hex → Encoding
    let mut bad_hex = blob.clone();
    bad_hex.ciphertext_hex = "zz".to_string();
    assert!(open(&bad_hex, &key).is_err());

    // 版本不对 → InvalidVersion
    let mut bad_version = blob.clone();
    bad_version.version = 2;
    assert_eq!(open(&bad_version, &key), Err(CoreError::InvalidVersion));
}

#[test]
fn peer_tags_are_salted_and_never_contain_the_did() {
    let peers = dids(2);
    let a = PrivacyPolicy::default();
    let b = PrivacyPolicy {
        salt: "another-salt-value".to_string(),
        ..PrivacyPolicy::default()
    };
    let ta = peer_tag(&a, &peers[0]).unwrap();
    let tb = peer_tag(&b, &peers[0]).unwrap();
    assert_ne!(ta, tb, "换盐必须换标签（跨发布不可关联）");
    assert_eq!(ta.len(), 16);
    assert!(!ta.contains("did:au4a:"));
    assert!(!ta.contains(&peers[0].as_str()[9..20]));
    // 同盐同协作者 → 稳定标签（本地可对齐）
    assert_eq!(ta, peer_tag(&a, &peers[0]).unwrap());
    assert_ne!(ta, peer_tag(&a, &peers[1]).unwrap());
    // 公开视图里没有任何标签明文，只有计数
    let store = secret_store(&peers);
    let view = publish(&store, &a).unwrap();
    let text = view.to_value().unwrap().to_string();
    assert!(!text.contains(&ta), "公开视图不应发布协作者标签");
    assert_eq!(view.peer_tag_count, 2);
}

#[test]
fn policy_is_validated() {
    let peers = dids(1);
    let store = secret_store(&peers);
    assert_eq!(
        PrivacyPolicy {
            salt: "x".repeat(MIN_SALT_LEN - 1),
            ..PrivacyPolicy::default()
        }
        .validate(),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        PrivacyPolicy {
            min_bucket_sample: 0,
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
    // 长度桶宽度是 usize，不存在负值；但 0 表示「不发布长度信息」，必须被接受
    assert!(PrivacyPolicy {
        context_bucket_chars: 0,
        ..PrivacyPolicy::default()
    }
    .validate()
    .is_ok());
    assert!(publish(
        &store,
        &PrivacyPolicy {
            salt: "short".into(),
            ..PrivacyPolicy::default()
        }
    )
    .is_err());

    // 桶宽度 0 = 完全不发布该类信息
    let bare = publish(
        &store,
        &PrivacyPolicy {
            reward_bucket_credits: 0,
            context_bucket_chars: 0,
            min_bucket_sample: 1,
            ..PrivacyPolicy::default()
        },
    )
    .unwrap();
    assert!(bare
        .aggregates
        .iter()
        .all(|a| a.mean_reward_bucket.is_none() && a.context_bucket.is_none()));
}

#[test]
fn policy_tag_changes_with_the_policy_salt() {
    let peers = dids(1);
    let store = secret_store(&peers);
    let a = publish(&store, &PrivacyPolicy::default()).unwrap();
    let b = publish(
        &store,
        &PrivacyPolicy {
            salt: "yet-another-salt".to_string(),
            ..PrivacyPolicy::default()
        },
    )
    .unwrap();
    assert_ne!(a.policy_tag, b.policy_tag);
    assert_eq!(a.policy_tag.len(), 16);
    // 同一策略两次发布逐字节一致
    assert_eq!(a, publish(&store, &PrivacyPolicy::default()).unwrap());
    assert_eq!(
        a.digest().unwrap(),
        publish(&store, &PrivacyPolicy::default())
            .unwrap()
            .digest()
            .unwrap()
    );
    let _: PublicView = a;
}

#[test]
fn scenario_publishes_a_redacted_view_and_a_sealed_blob() {
    // 端到端：场景里的经验库必须能发布出脱敏视图，且视图里没有 DID
    let mut kernel = au4a_kernel::Kernel::new(au4a_kernel::KernelConfig::default());
    let summary = au4a_learning::scenario(&mut kernel).unwrap();
    let privacy = &summary["privacy"];
    assert!(privacy.is_object(), "场景必须输出隐私投影：{summary}");
    let text = privacy.to_string();
    assert!(!text.contains("did:au4a:"));
    assert!(!text.contains("场景上下文"), "公开视图泄露了上下文原文");
    assert!(privacy["view"]["total"].as_u64().unwrap() > 0);
    assert!(privacy["sealed_bytes"].as_u64().unwrap() > 0);
    assert!(au4a_learning::self_check()
        .iter()
        .any(|c| c.name == "privacy.public_redacted"));
}
