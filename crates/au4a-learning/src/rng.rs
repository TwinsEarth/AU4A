//! 确定性伪随机与无状态散列。
//!
//! 学习必须可重放：同一个种子必须得到逐字节相同的结果。所以本轨道**不引入任何随机库**，
//! 只用 SplitMix64 这一个 64 位整数生成器（公开算法、无浮点、无平台差异），
//! 并提供无状态散列 `hash64` 供「按内容取样」使用。
//!
//! 为什么不用系统随机：个体学习的效果评估是「学习组 vs 对照组」的对照实验，
//! 两组必须面对**同一个合成市场**；任何不可复现的随机源都会让提升数据无法被第三方重算。

/// SplitMix64：状态是一个 u64，输出是一个 u64。周期 2^64，无浮点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// 下一个 64 位输出（算法与 SplitMix64 参考实现一致，全部为整数运算）。
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `[0, bound)` 上的均匀整数。`bound == 0` 时返回 0（空域，不是错误）。
    ///
    /// 用拒绝采样去掉取模偏置：偏置虽然小，但会让「对照实验」在长跑里出现系统性差异，
    /// 而系统性差异正是本轨道最需要排除的东西。
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound <= 1 {
            return 0;
        }
        let zone = u64::MAX - (u64::MAX % bound);
        loop {
            let r = self.next_u64();
            if r < zone {
                return r % bound;
            }
        }
    }

    /// `[0, 10000)` 的万分比取值。
    pub fn bp(&mut self) -> i64 {
        self.below(10_000) as i64
    }

    /// `[lo, hi]` 闭区间取值（`lo > hi` 时返回 `lo`）。
    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo) as u64 + 1;
        lo + self.below(span) as i64
    }
}

/// 无状态散列：同样的 `(seed, parts)` 永远得到同样的 u64。
///
/// 用途：为「第 tick 个任务的难度 / 报价是否被接受」这类**由内容决定**的量取值。
/// 与 `SplitMix64` 的区别是它不消耗序列——因此结果与调用顺序无关，
/// 学习组和对照组即使内部流程长度不同，也能面对同一个市场。
pub fn hash64(seed: u64, parts: &[u64]) -> u64 {
    let mut acc = seed ^ 0xA076_1D64_78BD_642F;
    for p in parts {
        acc = acc.wrapping_add(*p).wrapping_add(0x9E37_79B9_7F4A_7C15);
        acc = mix(acc);
    }
    mix(acc ^ (parts.len() as u64))
}

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `hash64` 的 `[0, bound)` 投影（`bound == 0` 时返回 0）。
pub fn hash_below(seed: u64, parts: &[u64], bound: u64) -> u64 {
    if bound == 0 {
        return 0;
    }
    hash64(seed, parts) % bound
}

/// `hash64` 的万分比投影。
pub fn hash_bp(seed: u64, parts: &[u64]) -> i64 {
    hash_below(seed, parts, 10_000) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut r = SplitMix64::new(7);
        let mut seen = [false; 5];
        for _ in 0..500 {
            let v = r.below(5);
            assert!(v < 5);
            seen[v as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(r.below(0), 0);
        assert_eq!(r.below(1), 0);
    }

    #[test]
    fn hash_is_stateless_and_order_sensitive() {
        assert_eq!(hash64(9, &[1, 2, 3]), hash64(9, &[1, 2, 3]));
        assert_ne!(hash64(9, &[1, 2, 3]), hash64(9, &[3, 2, 1]));
        assert_ne!(hash64(9, &[1, 2, 3]), hash64(10, &[1, 2, 3]));
        assert!(hash_bp(3, &[4, 5]) < 10_000);
        assert_eq!(hash_below(3, &[4], 0), 0);
    }

    #[test]
    fn range_is_inclusive() {
        let mut r = SplitMix64::new(11);
        for _ in 0..200 {
            let v = r.range_i64(-5, 5);
            assert!((-5..=5).contains(&v));
        }
        assert_eq!(r.range_i64(3, 3), 3);
        assert_eq!(r.range_i64(4, 2), 4);
    }
}
