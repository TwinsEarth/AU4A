//! 节点身份（v2.6.2）：把「面板报告的来源」接到一个真实密钥上。
//!
//! ## 三种来源全支持，优先级从显式到隐式
//!
//! | 顺序 | 来源 | 用途 |
//! |---|---|---|
//! | 1 | `--node-key <64hex>` | 显式传入（脚本/测试） |
//! | 2 | `--node-key-file <path>` | 文件（便于权限收敛到 0600 / ACL） |
//! | 3 | 环境变量 `AU4A_NODE_KEY` | **默认**（容器友好） |
//!
//! 三者都没有 → **匿名只读模式**（`Ok(None)`）：面板照常工作，只是不带来源签名（与 v2.6.0 之前一致）。
//!
//! ## 安全约束
//!
//! * 私钥**只**存在于 [`NodeIdentity`] 内部；`Debug` 只打印 DID，绝不打印种子；
//! * HTTP 响应里只出现 **DID + 签名**，永远不出现私钥；
//! * 但请注意：`--node-key` 会出现在进程命令行里（同机其它进程可见），
//!   生产环境推荐 `--node-key-file` 或环境变量。

use au4a_core::{AgentKeys, CoreError, CoreResult, Did};

/// 环境变量名（默认来源）。
pub const NODE_KEY_ENV: &str = "AU4A_NODE_KEY";

/// 节点身份：一个 32 字节种子派生的 Ed25519 密钥。
pub struct NodeIdentity {
    keys: AgentKeys,
}

impl NodeIdentity {
    /// 从 64 位小写 hex 种子构造。
    pub fn from_seed_hex(seed_hex: &str) -> CoreResult<Self> {
        let trimmed = seed_hex.trim();
        if trimmed.len() != 64 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
            // 注意：这里**不区分**大小写，但要求长度精确为 64，
            // 避免"截断的种子被当成合法密钥"这种静默降级。
            return Err(CoreError::InvalidSignature);
        }
        // 手写解码：不为此新增外部依赖（项目坚持"只 6 个外部依赖"）。
        // 长度与字符集已在上一步校验，因此这里只需处理进制转换。
        let bytes = trimmed.as_bytes();
        let mut arr = [0u8; 32];
        for i in 0..32 {
            let hi = (bytes[i * 2] as char)
                .to_digit(16)
                .ok_or(CoreError::InvalidSignature)? as u8;
            let lo = (bytes[i * 2 + 1] as char)
                .to_digit(16)
                .ok_or(CoreError::InvalidSignature)? as u8;
            arr[i] = (hi << 4) | lo;
        }
        Ok(Self {
            keys: AgentKeys::from_seed(&arr),
        })
    }

    /// 从文件读取（文件内容可以是 64 位 hex，或 32 字节裸二进制）。
    pub fn from_file(path: &str) -> CoreResult<Self> {
        let raw = std::fs::read(path).map_err(|_| CoreError::Encoding)?;
        // 只有**形如 64 位 hex** 的内容才按文本处理。
        // 反例（本文件测试抓到的）：32 字节裸数据 `[0x07; 32]` 恰好是合法 UTF-8（控制字符），
        // 若先按文本走就会把它当成"非法 hex 种子"而报错——那是把二进制密钥文件误判成损坏文本。
        if let Ok(text) = std::str::from_utf8(&raw) {
            let trimmed = text.trim();
            if trimmed.len() == 64 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Self::from_seed_hex(trimmed);
            }
        }
        let arr: [u8; 32] = raw
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::InvalidSignature)?;
        Ok(Self {
            keys: AgentKeys::from_seed(&arr),
        })
    }

    /// 纯函数版（便于测试）：把"环境变量的值"作为参数传进来。
    pub fn resolve_with(
        cli_key: Option<&str>,
        cli_file: Option<&str>,
        env_value: Option<&str>,
    ) -> CoreResult<Option<Self>> {
        if let Some(seed) = cli_key {
            return Self::from_seed_hex(seed).map(Some);
        }
        if let Some(path) = cli_file {
            return Self::from_file(path).map(Some);
        }
        if let Some(seed) = env_value {
            let trimmed = seed.trim();
            if trimmed.is_empty() {
                // 显式设成空串 = 明确要求匿名（不是错误）
                return Ok(None);
            }
            return Self::from_seed_hex(trimmed).map(Some);
        }
        Ok(None)
    }

    /// 读取环境变量后转调 [`NodeIdentity::resolve_with`]。
    pub fn resolve(cli_key: Option<&str>, cli_file: Option<&str>) -> CoreResult<Option<Self>> {
        let env_value = std::env::var(NODE_KEY_ENV).ok();
        Self::resolve_with(cli_key, cli_file, env_value.as_deref())
    }

    pub fn did(&self) -> Did {
        self.keys.did()
    }

    pub fn keys(&self) -> &AgentKeys {
        &self.keys
    }
}

/// **私钥不出结构体**：`Debug` 只打印 DID。
impl std::fmt::Debug for NodeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeIdentity({})", self.did())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED_A: &str = "0101010101010101010101010101010101010101010101010101010101010101";
    const SEED_B: &str = "0202020202020202020202020202020202020202020202020202020202020202";

    #[test]
    fn hex_seed_parses_and_is_deterministic() {
        let a = NodeIdentity::from_seed_hex(SEED_A).unwrap();
        let b = NodeIdentity::from_seed_hex(SEED_A).unwrap();
        let c = NodeIdentity::from_seed_hex(SEED_B).unwrap();
        assert_eq!(a.did(), b.did());
        assert_ne!(a.did(), c.did());
        assert!(a.did().as_str().starts_with("did:au4a:"));
        // 前后空白容忍（文件里常见换行）
        assert_eq!(
            NodeIdentity::from_seed_hex(&format!("  {SEED_A}\n"))
                .unwrap()
                .did(),
            a.did()
        );
    }

    #[test]
    fn malformed_seeds_are_refused_not_silently_truncated() {
        for bad in [
            "",
            "00",
            &SEED_A[..62],          // 少 2 位
            &format!("{SEED_A}00"), // 多 2 位
            "zz0101010101010101010101010101010101010101010101010101010101010101",
        ] {
            assert!(
                NodeIdentity::from_seed_hex(bad).is_err(),
                "非法种子必须被拒: {bad:?}"
            );
        }
    }

    #[test]
    fn precedence_is_cli_key_then_file_then_env() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("au4a-node-key-{}.hex", std::process::id()));
        std::fs::write(&path, SEED_B).unwrap();
        let path_str = path.to_string_lossy().to_string();

        // 1) 显式 hex 优先于文件与环境
        let by_cli = NodeIdentity::resolve_with(Some(SEED_A), Some(&path_str), Some(SEED_B))
            .unwrap()
            .unwrap();
        assert_eq!(
            by_cli.did(),
            NodeIdentity::from_seed_hex(SEED_A).unwrap().did()
        );

        // 2) 无显式 hex 时文件优先于环境
        let by_file = NodeIdentity::resolve_with(None, Some(&path_str), Some(SEED_A))
            .unwrap()
            .unwrap();
        assert_eq!(
            by_file.did(),
            NodeIdentity::from_seed_hex(SEED_B).unwrap().did()
        );

        // 3) 只有环境变量
        let by_env = NodeIdentity::resolve_with(None, None, Some(SEED_A))
            .unwrap()
            .unwrap();
        assert_eq!(
            by_env.did(),
            NodeIdentity::from_seed_hex(SEED_A).unwrap().did()
        );

        // 4) 都没有 → 匿名只读模式
        assert!(NodeIdentity::resolve_with(None, None, None)
            .unwrap()
            .is_none());
        // 空环境变量 = 明确匿名（不是错误）
        assert!(NodeIdentity::resolve_with(None, None, Some("  "))
            .unwrap()
            .is_none());

        // 5) 非法来源要报错，而不是悄悄回退成匿名（静默降级会让"以为签了其实没签"）
        assert!(NodeIdentity::resolve_with(Some("nope"), None, None).is_err());
        assert!(NodeIdentity::resolve_with(None, Some("/definitely/missing/key"), None).is_err());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn debug_never_leaks_the_seed_and_file_can_be_binary() {
        let id = NodeIdentity::from_seed_hex(SEED_A).unwrap();
        let shown = format!("{id:?}");
        assert!(shown.contains(id.did().as_str()));
        assert!(!shown.contains(SEED_A), "Debug 不得打印种子");

        // 32 字节裸二进制文件同样可读
        let dir = std::env::temp_dir();
        let bin = dir.join(format!("au4a-node-key-bin-{}.key", std::process::id()));
        std::fs::write(&bin, [7u8; 32]).unwrap();
        let from_bin = NodeIdentity::from_file(&bin.to_string_lossy()).unwrap();
        assert_eq!(
            from_bin.did(),
            NodeIdentity::from_seed_hex(&"07".repeat(32)).unwrap().did()
        );
        std::fs::remove_file(&bin).ok();
    }
}
