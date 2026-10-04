//! v1.6.6 隐私保护（Privacy）。
//!
//! 经验库是**本地**的：上下文原文、协作者 DID、任务标识都可能包含敏感信息。
//! 本模块把「本地」与「对外」两条路径彻底分开：
//!
//! | 路径 | 内容 | 出口 |
//! |---|---|---|
//! | 本地明文经验库 | 全字段（`context` / `peer_agents` / `task_id` / 精确金额） | 只在 Agent 自己的内存里 |
//! | 本地加密视图 [`SealedBlob`] | 规范 JSON 的密钥流混淆 + 完整性标签 | 可交给节点层落盘/传输（本轨道不做文件 I/O） |
//! | 对外公开视图 [`PublicView`] | 只有**按任务类型的聚合**：样本数、成功率、金额分桶、上下文长度桶、协作者**数量** | 可发布 |
//!
//! 三条硬规则（都有测试）：
//!
//! 1. 公开视图里**不出现** `context` 原文、`task_id`、任何 `did:` 字符串、任何精确金额。
//! 2. **小样本抑制**：聚合样本数低于 `min_bucket_sample` 时只发布「被抑制」这个事实，不发布统计值。
//! 3. 加密视图用**密钥流 XOR + 完整性标签**；密钥不对 → [`CoreError::InvalidSignature`]，不会解出半个经验库。
//!
//! # 证据等级（如实标注）
//!
//! 加密部分是**原型**：直接用 SHA-256 计数器模式做密钥流 + XOR + 明文摘要标签，
//! 没有经过审计的认证加密（AEAD）、没有密钥派生（KDF）、没有 nonce 管理。
//! 它足以支撑「本地视图不是明文」这个断言，但**不足以**声称生产级机密性 → `cpu-proto`。

use std::collections::BTreeMap;

use au4a_core::{canonicalize, content_hash, short_id, CoreError, CoreResult, Did, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::experience::{Experience, ExperienceStore, Outcome};

/// 公开聚合的样本下限：低于它只发布「被抑制」。
pub const DEFAULT_MIN_BUCKET_SAMPLE: usize = 3;
/// 盐的最小长度（太短的盐等于没有盐）。
pub const MIN_SALT_LEN: usize = 8;

/// 脱敏策略。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyPolicy {
    /// 本地盐：用于派生协作者标签与公开视图指纹。换盐 → 换标签（跨发布不可关联）。
    pub salt: String,
    /// 聚合样本下限（k-匿名阈值）。
    pub min_bucket_sample: usize,
    /// 结算金额分桶宽度（微积分）。0 表示**不发布**任何金额信息。
    pub reward_bucket_credits: i64,
    /// 上下文长度分桶宽度（字符数）。0 表示不发布长度信息。
    pub context_bucket_chars: usize,
}

impl Default for PrivacyPolicy {
    fn default() -> Self {
        Self {
            salt: "au4a-1.6-local-salt".to_string(),
            min_bucket_sample: DEFAULT_MIN_BUCKET_SAMPLE,
            reward_bucket_credits: 25,
            context_bucket_chars: 32,
        }
    }
}

impl PrivacyPolicy {
    pub fn validate(&self) -> CoreResult<()> {
        if self.salt.len() < MIN_SALT_LEN {
            return Err(CoreError::InvalidKind);
        }
        if self.min_bucket_sample == 0 {
            return Err(CoreError::InvalidKind);
        }
        if self.reward_bucket_credits < 0 {
            return Err(CoreError::NegativeAmount);
        }
        Ok(())
    }
}

/// 一个任务类型的公开聚合。**所有字段都是脱敏后的**。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicAggregate {
    pub task_type: String,
    pub sample: usize,
    /// 是否因样本不足被抑制。
    pub suppressed: bool,
    /// 成功率（万分比）；被抑制时为 `None`。
    pub success_bp: Option<i64>,
    /// 平均金额**桶**（不是精确值）；被抑制或不发布金额时为 `None`。
    pub mean_reward_bucket: Option<i64>,
    /// 上下文长度桶（字符数 / bucket_chars）；不是原文、也不是摘要。
    pub context_bucket: Option<i64>,
    /// 出现过的协作者**数量**（不是 DID）。
    pub distinct_peers: Option<usize>,
}

/// 对外公开视图。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicView {
    /// 该视图对应的策略指纹（盐 + 阈值），不含敏感内容。
    pub policy_tag: String,
    /// 参与聚合的经验总数。
    pub total: usize,
    /// 被抑制的聚合数量。
    pub suppressed_groups: usize,
    /// 按任务类型排序的聚合。
    pub aggregates: Vec<PublicAggregate>,
    pub by_outcome: BTreeMap<String, usize>,
    /// 协作者标签数量（证明「同一协作者在本地可对齐、对外只暴露计数」）。
    pub peer_tag_count: usize,
}

impl PublicView {
    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        au4a_core::canonical_hash(&self.to_value()?)
    }
}

/// 本地加密视图（原型：SHA-256 计数器模式密钥流 + XOR + 明文摘要标签）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedBlob {
    pub version: u8,
    /// 完整性标签：`key + 明文摘要` 的散列前缀。密钥不对 → 标签不匹配。
    pub key_tag: String,
    /// 明文字节数（解密后必须一致）。
    pub plaintext_len: usize,
    /// 密文（hex）。
    pub ciphertext_hex: String,
}

/// 协作者标签：加盐散列前缀。同一 Agent 用它对齐协作者，但标签里没有 DID 的任何字节。
pub fn peer_tag(policy: &PrivacyPolicy, peer: &Did) -> CoreResult<String> {
    policy.validate()?;
    Ok(short_id(&content_hash(
        format!("{}:{}", policy.salt, peer.as_str()).as_bytes(),
    )))
}

/// 生成对外公开视图：按任务类型聚合 + 小样本抑制 + 上下文/金额分桶。
pub fn publish(store: &ExperienceStore, policy: &PrivacyPolicy) -> CoreResult<PublicView> {
    policy.validate()?;
    let mut by_type: BTreeMap<String, Vec<&Experience>> = BTreeMap::new();
    let mut by_outcome: BTreeMap<String, usize> = BTreeMap::new();
    let mut peer_tags: BTreeMap<String, ()> = BTreeMap::new();
    for e in store.entries() {
        by_type.entry(e.task_type.clone()).or_default().push(e);
        *by_outcome
            .entry(e.outcome.as_str().to_string())
            .or_insert(0) += 1;
        for p in &e.peer_agents {
            peer_tags.insert(peer_tag(policy, p)?, ());
        }
    }

    let mut aggregates = Vec::new();
    let mut suppressed_groups = 0usize;
    for (task_type, entries) in &by_type {
        let sample = entries.len();
        if sample < policy.min_bucket_sample {
            suppressed_groups += 1;
            aggregates.push(PublicAggregate {
                task_type: task_type.clone(),
                sample,
                suppressed: true,
                success_bp: None,
                mean_reward_bucket: None,
                context_bucket: None,
                distinct_peers: None,
            });
            continue;
        }
        let successes = entries.iter().filter(|e| e.is_success()).count() as i64;
        let success_bp = successes * 10_000 / sample as i64;
        let reward_sum: i64 = entries.iter().map(|e| e.reward.get()).sum();
        let mean_reward = reward_sum / sample as i64;
        let context_sum: i64 = entries
            .iter()
            .map(|e| e.context.chars().count() as i64)
            .sum();
        let mean_context = context_sum / sample as i64;
        let mut peers = BTreeMap::new();
        for e in entries {
            for p in &e.peer_agents {
                peers.insert(peer_tag(policy, p)?, ());
            }
        }
        aggregates.push(PublicAggregate {
            task_type: task_type.clone(),
            sample,
            suppressed: false,
            success_bp: Some(success_bp),
            mean_reward_bucket: if policy.reward_bucket_credits > 0 {
                Some(round_to_bucket(mean_reward, policy.reward_bucket_credits))
            } else {
                None
            },
            context_bucket: if policy.context_bucket_chars > 0 {
                Some(mean_context / policy.context_bucket_chars as i64)
            } else {
                None
            },
            distinct_peers: Some(peers.len()),
        });
    }

    let policy_tag = short_id(&content_hash(
        format!(
            "{}:{}:{}:{}",
            policy.salt,
            policy.min_bucket_sample,
            policy.reward_bucket_credits,
            policy.context_bucket_chars
        )
        .as_bytes(),
    ));
    Ok(PublicView {
        policy_tag,
        total: store.len(),
        suppressed_groups,
        aggregates,
        by_outcome,
        peer_tag_count: peer_tags.len(),
    })
}

/// 加密：把经验库的规范 JSON 用密钥流混淆，并附完整性标签。
pub fn seal(store: &ExperienceStore, key: &[u8; 32]) -> CoreResult<SealedBlob> {
    let plaintext = store.canonical_json()?;
    let bytes = plaintext.as_bytes();
    let key_hex = hex_of(key);
    let stream = keystream(&key_hex, bytes.len());
    let cipher: Vec<u8> = bytes
        .iter()
        .zip(stream.iter())
        .map(|(b, k)| b ^ k)
        .collect();
    Ok(SealedBlob {
        version: 1,
        key_tag: integrity_tag(&key_hex, bytes),
        plaintext_len: bytes.len(),
        ciphertext_hex: hex_of(&cipher),
    })
}

/// 解密：标签不匹配 → [`CoreError::InvalidSignature`]；内容不合法 → [`CoreError::Encoding`] / [`CoreError::InvalidKind`]。
///
/// 注意检查顺序：**先比完整性标签，再做 UTF-8 解码**。这样「密钥不对」永远得到
/// `InvalidSignature`（而不是「碰巧解码失败」的 `Encoding`），调用方不需要靠错误类型猜原因。
pub fn open(blob: &SealedBlob, key: &[u8; 32]) -> CoreResult<ExperienceStore> {
    if blob.version != 1 {
        return Err(CoreError::InvalidVersion);
    }
    let cipher = unhex(&blob.ciphertext_hex).ok_or(CoreError::Encoding)?;
    if cipher.len() != blob.plaintext_len {
        return Err(CoreError::FrameTruncated);
    }
    let key_hex = hex_of(key);
    let stream = keystream(&key_hex, cipher.len());
    let plain: Vec<u8> = cipher
        .iter()
        .zip(stream.iter())
        .map(|(c, k)| c ^ k)
        .collect();
    if integrity_tag(&key_hex, &plain) != blob.key_tag {
        return Err(CoreError::InvalidSignature);
    }
    let plaintext = String::from_utf8(plain).map_err(|_| CoreError::Encoding)?;
    ExperienceStore::from_json_str(&plaintext)
}

/// 公开视图可编码为规范 JSON（无浮点）。
pub fn public_json(store: &ExperienceStore, policy: &PrivacyPolicy) -> CoreResult<String> {
    canonicalize(&publish(store, policy)?.to_value()?)
}

fn round_to_bucket(value: i64, width: i64) -> i64 {
    if width <= 0 {
        return 0;
    }
    // 向下取整到桶；负值不会出现（金额非负），但仍然写得对称。
    let q = value.div_euclid(width);
    q * width
}

/// 完整性标签：`key + SHA-256(明文字节)` 的散列前缀。密钥或内容任何一个变了，标签就不匹配。
fn integrity_tag(key_hex: &str, plaintext: &[u8]) -> String {
    short_id(&content_hash(
        format!("{key_hex}:{}", content_hash(plaintext)).as_bytes(),
    ))
}

/// SHA-256 计数器模式密钥流：每块 64 个 ASCII hex 字符，不够就再来一块。
fn keystream(key_hex: &str, len: usize) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(len + 64);
    let mut counter: u64 = 0;
    while out.len() < len {
        let block = content_hash(format!("{key_hex}:{counter}").as_bytes());
        out.extend_from_slice(block.as_bytes());
        counter = counter.saturating_add(1);
    }
    out.truncate(len);
    out
}

fn hex_of(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

/// 真实断言：公开视图脱敏、小样本被抑制、加密往返、错密钥被拒。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    let peers: Vec<Did> = (0..2u8)
        .map(|s| au4a_core::AgentKeys::from_seed(&[0x40 + s; 32]).did())
        .collect();
    let policy = PrivacyPolicy::default();
    let mut store = match ExperienceStore::new(32) {
        Ok(s) => s,
        Err(_) => {
            return vec![crate::check(
                "privacy.public_redacted",
                false,
                "经验库构造失败",
            )]
        }
    };
    // 3 条热门类型 + 1 条冷门类型（后者应被抑制），上下文带可搜索的秘密串
    for i in 0..4u64 {
        let task_type = if i < 3 { "hot.type" } else { "cold.type" };
        let context = format!("SECRET-CONTEXT-{i}-不可外泄");
        if let Ok(e) = Experience::new(
            &format!("task-{i}"),
            task_type,
            &context,
            "deliver",
            if i == 3 {
                Outcome::Failure
            } else {
                Outcome::Success
            },
            au4a_core::Credits(37),
            i,
            &peers[..1],
        ) {
            let _ = store.record(e);
        }
    }
    let view = publish(&store, &policy);
    let (public_ok, detail) = match &view {
        Ok(v) => {
            let text = v
                .to_value()
                .map(|value| value.to_string())
                .unwrap_or_default();
            let no_secret = !text.contains("SECRET-CONTEXT") && !text.contains("不可外泄");
            let no_did = !text.contains("did:au4a:");
            let no_task_id = !text.contains("task-");
            // 金额只以桶出现：37 微积分 → 25 微积分的桶（宽度 25）
            let bucketed = v
                .aggregates
                .iter()
                .filter(|a| !a.suppressed)
                .all(|a| a.mean_reward_bucket == Some(25));
            let suppressed = v.suppressed_groups == 1
                && v.aggregates
                    .iter()
                    .any(|a| a.suppressed && a.success_bp.is_none());
            (
                no_secret && no_did && no_task_id && bucketed && suppressed,
                format!(
                    "无上下文原文={no_secret} 无 DID={no_did} 无 task_id={no_task_id} \
                     金额只出现分桶(37→25)={bucketed} 抑制 1 组={suppressed}"
                ),
            )
        }
        Err(e) => (false, format!("发布失败: {e:?}")),
    };
    checks.push(crate::check("privacy.public_redacted", public_ok, detail));

    // 加密往返 + 错密钥 + 篡改
    let key = [7u8; 32];
    let wrong = [8u8; 32];
    let sealed = seal(&store, &key);
    let roundtrip = match &sealed {
        Ok(blob) => {
            let obfuscated = store
                .canonical_json()
                .map(|plain| hex_of(plain.as_bytes()) != blob.ciphertext_hex)
                .unwrap_or(false);
            let restored_ok = open(blob, &key)
                .map(|restored| {
                    restored.len() == store.len() && restored.digest().ok() == store.digest().ok()
                })
                .unwrap_or(false);
            obfuscated && restored_ok
        }
        Err(_) => false,
    };
    let wrong_key_refused = match &sealed {
        Ok(blob) => open(blob, &wrong) == Err(CoreError::InvalidSignature),
        Err(_) => false,
    };
    let tamper_refused = match &sealed {
        Ok(blob) => {
            let mut tampered = blob.clone();
            // 翻转最后一个 hex 字符 → 明文变化 → 完整性标签不匹配
            let last = tampered.ciphertext_hex.pop().unwrap_or('0');
            let flipped = if last == '0' { '1' } else { '0' };
            tampered.ciphertext_hex.push(flipped);
            open(&tampered, &key).is_err()
        }
        Err(_) => false,
    };
    checks.push(crate::check(
        "privacy.sealed_roundtrip",
        roundtrip && wrong_key_refused && tamper_refused,
        format!(
            "密文≠明文且往返一致={roundtrip}；错密钥 → InvalidSignature={wrong_key_refused}；篡改被拒={tamper_refused}"
        ),
    ));

    // 盐的作用：换盐 → 换标签
    let other = PrivacyPolicy {
        salt: "another-salt-1.6".to_string(),
        ..policy.clone()
    };
    let salt_matters = match (peer_tag(&policy, &peers[0]), peer_tag(&other, &peers[0])) {
        (Ok(a), Ok(b)) => a != b && !a.contains("did:au4a:"),
        _ => false,
    };
    let policy_rejected = PrivacyPolicy {
        salt: "short".to_string(),
        ..policy.clone()
    }
    .validate()
    .is_err()
        && PrivacyPolicy {
            min_bucket_sample: 0,
            ..policy.clone()
        }
        .validate()
        .is_err();
    checks.push(crate::check(
        "privacy.salt_and_policy",
        salt_matters && policy_rejected,
        format!("换盐换标签={salt_matters}；非法策略被拒={policy_rejected}"),
    ));
    checks
}
