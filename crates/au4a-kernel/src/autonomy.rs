//! Agent 自治层（v1.0.2）。
//!
//! 本项目的根本理念是「Agent 是主体」：任何能力的第一测试用例是「Agent 能否自主使用它」。
//! 因此这一层不是「给人类用的控制台」，而是一个**纯函数式决策器** + 一个**真的会动内核的执行器**：
//!
//! * [`AutonomyPolicy::plan`]：给定「我是谁（名片）」「我看到了什么（上下文）」，输出一组自主意图。
//!   没有随机、没有墙钟、没有人类输入通道——同样的输入必然得到同样的意图序列。
//! * [`AutonomyLayer::turn`]：把意图真的落到共享内核上（签名信封、广播、点对点、结算），
//!   并把发生过的一切写进可重放的日志。
//!
//! **没有任何分支等待人类批准**：`IntentKind::ALL` 里的每一种意图都由 Agent 自己发起；
//! 决策输入 [`AgentContext`] 里没有任何「人类/审批/运营方」字段（有测试断言它的 JSON 键集合）。
//!
//! 拒绝的处置也在这里落地：
//! * 竞争性拒绝（限流、超时、冲突、容量……）→ **退避**（`Defer`），只推迟、不隔离；
//! * 恶意性拒绝（`malformed` / `unauthorized`）→ **自我暂停**（`Suspend`），Agent 主动停止发言，
//!   而不是等某个中心化角色来封禁它。

use au4a_core::{
    AgentKeys, CoreError, CoreResult, Credits, Did, Envelope, EvidenceGrade, Refusal, RefusalCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AgentCard, Kernel};

/// 自主意图的类别（用于枚举与结构性断言，不参与匹配）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    /// 广播自己的能力名片。
    Announce,
    /// 向对端出价（能力 + 价格）。
    Offer,
    /// 汇报进度事件。
    Progress,
    /// 发起结算（证据等级由 Agent 自己声明）。
    Settle,
    /// 拒绝对端（拒绝本身也是自主决定，必须带类型化理由）。
    Decline,
    /// 竞争性退避：到某个逻辑时刻之前不再行动。**不是隔离**。
    Defer,
    /// 自我暂停：发现自己触发了恶意码，主动闭嘴。
    Suspend,
    /// 无话可说。
    Idle,
}

impl IntentKind {
    pub const ALL: [IntentKind; 8] = [
        IntentKind::Announce,
        IntentKind::Offer,
        IntentKind::Progress,
        IntentKind::Settle,
        IntentKind::Decline,
        IntentKind::Defer,
        IntentKind::Suspend,
        IntentKind::Idle,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IntentKind::Announce => "announce",
            IntentKind::Offer => "offer",
            IntentKind::Progress => "progress",
            IntentKind::Settle => "settle",
            IntentKind::Decline => "decline",
            IntentKind::Defer => "defer",
            IntentKind::Suspend => "suspend",
            IntentKind::Idle => "idle",
        }
    }
}

/// 一条自主意图。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Intent {
    Announce,
    Offer {
        to: Did,
        skill: String,
        price: Credits,
    },
    Progress {
        to: Option<Did>,
        label: String,
    },
    Settle {
        to: Did,
        amount: Credits,
        grade: EvidenceGrade,
    },
    Decline {
        to: Did,
        code: RefusalCode,
    },
    Defer {
        until: u64,
    },
    Suspend,
    Idle,
}

impl Intent {
    pub fn kind(&self) -> IntentKind {
        match self {
            Intent::Announce => IntentKind::Announce,
            Intent::Offer { .. } => IntentKind::Offer,
            Intent::Progress { .. } => IntentKind::Progress,
            Intent::Settle { .. } => IntentKind::Settle,
            Intent::Decline { .. } => IntentKind::Decline,
            Intent::Defer { .. } => IntentKind::Defer,
            Intent::Suspend => IntentKind::Suspend,
            Intent::Idle => IntentKind::Idle,
        }
    }
}

/// 决策输入：Agent 自己能看到的全部信息。
///
/// 注意这里**没有**人类通道：没有 `approved_by`、没有 `operator`、没有 `pending_human`。
/// 「等待人类批准」这种分支在类型上就不存在，而不是靠纪律避免。
///
/// 注：`Envelope` 的 `body` 是 `serde_json::Value`，故本类型只实现 `PartialEq`
/// （与基元层保持一致：JSON 值不参与 `Eq`）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentContext {
    /// 逻辑时刻。
    pub now: u64,
    /// 我的账本余额（整数微积分）。
    pub available: Credits,
    pub locked: Credits,
    /// 收件箱：投递给「我」或广播的信封（只读，不消费队列）。
    pub inbox: Vec<Envelope>,
    /// 我最近一次被拒绝的记录（如果有）。
    pub last_refusal: Option<Refusal>,
    /// 退避截止时刻。
    pub defer_until: Option<u64>,
    /// 上次通告时刻。
    pub last_announce_at: Option<u64>,
    /// 是否已自我暂停。
    pub halted: bool,
}

/// 自主策略：纯整数、纯确定性。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomyPolicy {
    /// 每隔多少个逻辑时刻重新通告一次（心跳）。
    pub announce_every: u64,
    /// 低于这个可用余额就不再出价（先攒资源，不借债）。
    pub min_balance: Credits,
    /// 报价比例（基点，整数运算）。
    pub offer_rate_bp: i64,
    /// 竞争性拒绝后的退避长度。
    pub backoff_ticks: u64,
}

impl Default for AutonomyPolicy {
    fn default() -> Self {
        Self {
            announce_every: 4,
            min_balance: Credits(50),
            offer_rate_bp: 100, // 1%
            backoff_ticks: 3,
        }
    }
}

impl AutonomyPolicy {
    /// 报价：可用余额的 `offer_rate_bp` 个基点，向下取整；低于下限则不出价。
    pub fn quote(&self, available: Credits) -> CoreResult<Option<Credits>> {
        if available < self.min_balance {
            return Ok(None);
        }
        let price = available.scaled_bp(self.offer_rate_bp)?;
        if price == Credits::ZERO {
            return Ok(None);
        }
        Ok(Some(price))
    }

    /// 纯决策：同样的 (名片, 上下文) → 同样的意图序列。
    ///
    /// 顺序即优先级：先处理自我暂停与退避，再通告，再处理收件箱；空则 `Idle`。
    pub fn plan(&self, me: &AgentCard, ctx: &AgentContext) -> Vec<Intent> {
        if ctx.halted {
            return vec![Intent::Suspend];
        }
        // 我在内核里留下了恶意记录：先闭嘴。这不是「等人类处置」，是自己停手。
        if let Some(refusal) = &ctx.last_refusal {
            if refusal.code.is_misconduct() {
                return vec![Intent::Suspend];
            }
        }
        if let Some(until) = ctx.defer_until {
            if ctx.now < until {
                return vec![Intent::Defer { until }];
            }
        }

        let mut out = Vec::new();

        // 心跳通告：自己决定何时重新声明能力，不需要任何人批准。
        let due = match ctx.last_announce_at {
            Some(t) => ctx.now.saturating_sub(t) >= self.announce_every,
            None => true,
        };
        if due {
            out.push(Intent::Announce);
        }

        for env in &ctx.inbox {
            match env.kind.as_str() {
                "negotiate.offer" => {
                    let skill = env.body["skill"].as_str().unwrap_or("").to_string();
                    let i_declare_it = me.skills.iter().any(|s| s == &skill);
                    match self.quote(ctx.available) {
                        Ok(Some(price)) if i_declare_it => out.push(Intent::Offer {
                            to: env.from.clone(),
                            skill,
                            price,
                        }),
                        Ok(_) => out.push(Intent::Decline {
                            to: env.from.clone(),
                            code: if i_declare_it {
                                RefusalCode::ResourceExhausted
                            } else {
                                RefusalCode::Unsupported
                            },
                        }),
                        Err(_) => out.push(Intent::Decline {
                            to: env.from.clone(),
                            code: RefusalCode::Malformed,
                        }),
                    }
                }
                "settle.request" => {
                    let amount = env.body["amount"].as_i64().unwrap_or(0);
                    let grade = env.body["grade"]
                        .as_str()
                        .and_then(EvidenceGrade::parse)
                        .unwrap_or(EvidenceGrade::Unverified);
                    if amount <= 0 {
                        out.push(Intent::Decline {
                            to: env.from.clone(),
                            code: RefusalCode::Malformed,
                        });
                    } else {
                        out.push(Intent::Settle {
                            to: env.from.clone(),
                            amount: Credits(amount),
                            grade,
                        });
                    }
                }
                "progress.event" => out.push(Intent::Progress {
                    to: Some(env.from.clone()),
                    label: env.body["kind"].as_str().unwrap_or("progress").to_string(),
                }),
                _ => {}
            }
        }

        if out.is_empty() {
            out.push(Intent::Idle);
        }
        out
    }
}

/// 一条已发生的自主行为（可重放日志）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentRecord {
    pub at: u64,
    pub intent: Intent,
    /// 是否真的动了内核（`Idle`/`Defer`/`Suspend` 只是决定，不发送）。
    pub acted: bool,
}

/// 自治层的可观察状态。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomyState {
    pub announcements: u64,
    pub intents_planned: u64,
    pub messages_sent: u64,
    pub settlements: u64,
    pub last_announce_at: Option<u64>,
    pub defer_until: Option<u64>,
    pub halted: bool,
    pub journal: Vec<IntentRecord>,
}

/// 一次自治回合的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomyTurn {
    pub at: u64,
    pub planned: Vec<IntentKind>,
    pub sent: usize,
    pub refused: Vec<RefusalCode>,
}

/// Agent 自治运行时：持有自己的密钥、策略与日志。
///
/// 私钥不出结构体（沿用基元层的做法），对外只表现为「这个 DID 做过什么」。
pub struct AutonomyLayer {
    keys: AgentKeys,
    policy: AutonomyPolicy,
    state: AutonomyState,
}

impl AutonomyLayer {
    pub fn new(keys: AgentKeys, policy: AutonomyPolicy) -> Self {
        Self {
            keys,
            policy,
            state: AutonomyState::default(),
        }
    }

    pub fn did(&self) -> Did {
        self.keys.did()
    }

    pub fn policy(&self) -> &AutonomyPolicy {
        &self.policy
    }

    pub fn state(&self) -> &AutonomyState {
        &self.state
    }

    pub fn journal(&self) -> &[IntentRecord] {
        &self.state.journal
    }

    /// 自我注册：Agent 自己带质押、自己声明能力。返回名片（内核唯一能看到的身份表示）。
    pub fn join(
        &mut self,
        kernel: &mut Kernel,
        display: &str,
        skills: &[&str],
        stake: Credits,
    ) -> CoreResult<AgentCard> {
        kernel.register(&self.keys, display, skills, stake)
    }

    /// 我看到的世界（只读，不消费共享队列）。
    pub fn context(&self, kernel: &Kernel) -> AgentContext {
        let did = self.did();
        let inbox: Vec<Envelope> = kernel
            .queued_envelopes()
            .iter()
            .filter(|env| match &env.to {
                Some(to) => to == &did,
                None => true,
            })
            .cloned()
            .collect();
        let balance = kernel.ledger().balance(&did);
        AgentContext {
            now: kernel.now(),
            available: balance.available,
            locked: balance.locked,
            inbox,
            last_refusal: kernel
                .refusals()
                .iter()
                .rev()
                .find(|(who, _)| who == &did)
                .map(|(_, refusal)| refusal.clone()),
            defer_until: self.state.defer_until,
            last_announce_at: self.state.last_announce_at,
            halted: self.state.halted,
        }
    }

    /// 一次自主回合：决策 → 执行 → 记录 → 按拒绝类型调整自己的姿态。
    pub fn turn(&mut self, kernel: &mut Kernel) -> CoreResult<AutonomyTurn> {
        let card = kernel
            .card(&self.did())
            .cloned()
            .ok_or(CoreError::UnknownAgent)?;
        let ctx = self.context(kernel);
        let now = ctx.now;
        let planned = self.policy.plan(&card, &ctx);

        let mut sent = 0usize;
        let mut refused: Vec<RefusalCode> = Vec::new();
        let mut kinds: Vec<IntentKind> = Vec::new();

        for intent in &planned {
            kinds.push(intent.kind());
            self.state.intents_planned += 1;
            let before = kernel.refusals().len();
            // 执行失败按「没做成」处理：bool 的默认值就是 false，手写 match 是多余的。
            let acted = self.execute(kernel, &card, intent).unwrap_or_default();
            if acted {
                sent += 1;
            }
            // 内核拒绝是本回合的输入：分类决定「退避」还是「自我暂停」。
            for (_, refusal) in kernel.refusals().iter().skip(before) {
                refused.push(refusal.code);
            }
            self.state.journal.push(IntentRecord {
                at: now,
                intent: intent.clone(),
                acted,
            });
        }

        self.react(now, &refused);
        Ok(AutonomyTurn {
            at: now,
            planned: kinds,
            sent,
            refused,
        })
    }

    /// 拒绝分类 → 姿态调整。竞争退避，恶意自停；两者都不需要人类参与。
    fn react(&mut self, now: u64, refused: &[RefusalCode]) {
        if refused.iter().any(|code| code.is_misconduct()) {
            self.state.halted = true;
            return;
        }
        if refused.iter().any(|code| !code.is_misconduct()) {
            let until = now.saturating_add(self.policy.backoff_ticks);
            self.state.defer_until = Some(until);
        }
    }

    fn execute(
        &mut self,
        kernel: &mut Kernel,
        card: &AgentCard,
        intent: &Intent,
    ) -> CoreResult<bool> {
        let did = self.did();
        match intent {
            Intent::Announce => {
                let env = Envelope::new(
                    did,
                    None,
                    "agent.card",
                    kernel.now() + 1,
                    None,
                    json!({
                        "display": card.display,
                        "skills": card.skills,
                        "stake": card.stake,
                        "evidence": card.evidence.as_str(),
                    }),
                )?
                .seal(&self.keys)?;
                kernel.send(&env)?;
                self.state.announcements += 1;
                self.state.last_announce_at = Some(kernel.now());
                self.state.messages_sent += 1;
                Ok(true)
            }
            Intent::Offer { to, skill, price } => {
                let env = Envelope::new(
                    did,
                    Some(to.clone()),
                    "negotiate.offer",
                    kernel.now() + 1,
                    None,
                    json!({"skill": skill, "price": price, "evidence": "cpu-proto"}),
                )?
                .seal(&self.keys)?;
                kernel.send(&env)?;
                self.state.messages_sent += 1;
                Ok(true)
            }
            Intent::Progress { to, label } => {
                let env = Envelope::new(
                    did,
                    to.clone(),
                    "progress.event",
                    kernel.now() + 1,
                    None,
                    json!({"kind": label, "by": card.display}),
                )?
                .seal(&self.keys)?;
                kernel.send(&env)?;
                self.state.messages_sent += 1;
                Ok(true)
            }
            Intent::Settle { to, amount, grade } => {
                kernel.settle(&did, to, *amount, *grade)?;
                self.state.settlements += 1;
                Ok(true)
            }
            Intent::Decline { to, code } => {
                let env = Envelope::new(
                    did,
                    Some(to.clone()),
                    "negotiate.decline",
                    kernel.now() + 1,
                    None,
                    json!({"code": code.as_str(), "retryable": code.retryable()}),
                )?
                .seal(&self.keys)?;
                kernel.send(&env)?;
                self.state.messages_sent += 1;
                Ok(true)
            }
            // 自我暂停是终态改变，但不产生网络流量（不发告别信，不解释给谁听）。
            Intent::Suspend => {
                self.state.halted = true;
                Ok(false)
            }
            // 决定本身不是动作：退避与空转不产生任何网络流量。
            Intent::Defer { .. } | Intent::Idle => Ok(false),
        }
    }
}

/// `CoreError` → 类型化拒绝码。保持「恶意 2 码 vs 竞争 8 码」的分类。
pub fn classify_error(err: &CoreError) -> RefusalCode {
    match err {
        CoreError::NotSealed | CoreError::InvalidSignature | CoreError::InvalidDid => {
            RefusalCode::Unauthorized
        }
        CoreError::Encoding | CoreError::FloatForbidden | CoreError::InvalidKind => {
            RefusalCode::Malformed
        }
        CoreError::FrameTooLarge | CoreError::FrameTruncated => RefusalCode::ResourceExhausted,
        CoreError::UnknownAgent => RefusalCode::StaleEpoch,
        CoreError::InsufficientFunds | CoreError::InsufficientStake | CoreError::DuplicateAgent => {
            RefusalCode::PolicyDenied
        }
        CoreError::NegativeAmount | CoreError::ZeroAmount | CoreError::Overflow => {
            RefusalCode::Malformed
        }
        CoreError::InvalidVersion => RefusalCode::Unsupported,
        // v2.3.0：守恒被破坏是内部状态不一致，属于"不可重试的实现问题"，按 ResourceExhausted 归类
        // （它不是对端行为，因此不能因为对端触发而隔离对方）。
        CoreError::ConservationViolated => RefusalCode::ResourceExhausted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KernelConfig;

    fn keys(tag: u8) -> AgentKeys {
        AgentKeys::from_seed(&[tag; 32])
    }

    fn card_for(tag: u8, skills: &[&str]) -> AgentCard {
        AgentCard {
            did: keys(tag).did(),
            display: format!("agent-{tag}"),
            skills: skills.iter().map(|s| (*s).to_string()).collect(),
            stake: Credits(20),
            evidence: EvidenceGrade::Verified,
        }
    }

    fn ctx(now: u64) -> AgentContext {
        AgentContext {
            now,
            available: Credits(1_000),
            locked: Credits(20),
            inbox: Vec::new(),
            last_refusal: None,
            defer_until: None,
            last_announce_at: None,
            halted: false,
        }
    }

    #[test]
    fn planning_is_a_pure_function_of_its_inputs() {
        let policy = AutonomyPolicy::default();
        let me = card_for(1, &["translate"]);
        let c = ctx(10);
        let a = policy.plan(&me, &c);
        let b = policy.plan(&me, &c);
        assert_eq!(a, b);
        assert_eq!(a, vec![Intent::Announce]);
    }

    #[test]
    fn announcement_is_heartbeat_bounded() {
        let policy = AutonomyPolicy::default();
        let me = card_for(1, &["translate"]);
        let mut c = ctx(10);
        c.last_announce_at = Some(10);
        assert_eq!(
            policy.plan(&me, &c),
            vec![Intent::Idle],
            "刚通告过就不再通告"
        );
        c.now = 10 + policy.announce_every;
        assert_eq!(policy.plan(&me, &c), vec![Intent::Announce]);
    }

    #[test]
    fn competitive_refusal_backs_off_and_never_quarantines() {
        let policy = AutonomyPolicy::default();
        let me = card_for(1, &["translate"]);
        let mut c = ctx(20);
        c.last_announce_at = Some(20);
        c.last_refusal = Some(Refusal::new(RefusalCode::RateLimited, "too fast"));
        c.defer_until = Some(25);
        assert_eq!(policy.plan(&me, &c), vec![Intent::Defer { until: 25 }]);
        // 退避到期后恢复自主行动，没有任何「等待批准」的中间态。
        c.now = 25;
        assert_eq!(policy.plan(&me, &c), vec![Intent::Announce]);
    }

    #[test]
    fn misconduct_makes_the_agent_suspend_itself() {
        let policy = AutonomyPolicy::default();
        let me = card_for(1, &["translate"]);
        let mut c = ctx(30);
        c.halted = true;
        assert_eq!(policy.plan(&me, &c), vec![Intent::Suspend]);

        // 仅仅在内核里留下一条恶意记录就足以让 Agent 自停——不需要任何外部判决。
        c.halted = false;
        c.last_refusal = Some(Refusal::new(RefusalCode::Malformed, "bad frame"));
        assert_eq!(policy.plan(&me, &c), vec![Intent::Suspend]);

        // 竞争码不会让 Agent 自停，只会退避。
        c.last_refusal = Some(Refusal::new(RefusalCode::Timeout, "slow peer"));
        c.defer_until = Some(31);
        assert_eq!(policy.plan(&me, &c), vec![Intent::Defer { until: 31 }]);

        let mut layer = AutonomyLayer::new(keys(1), AutonomyPolicy::default());
        layer.react(30, &[RefusalCode::Unauthorized]);
        assert!(layer.state().halted, "恶意码让 Agent 自己闭嘴");
        layer.react(31, &[RefusalCode::Timeout]);
        assert!(layer.state().halted, "自停是终态，竞争码不会把它解除");
    }

    #[test]
    fn decision_inputs_contain_no_human_channel() {
        // 结构性断言：决策输入里不存在人类/审批/运营方通道。
        let value = serde_json::to_value(ctx(1)).unwrap();
        let keys: Vec<String> = value
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        for key in &keys {
            let lower = key.to_lowercase();
            assert!(
                !lower.contains("human")
                    && !lower.contains("approv")
                    && !lower.contains("operator"),
                "决策输入不能有人类通道：{key}"
            );
        }
        // 意图种类里也不存在「等待批准」这种形状。
        for kind in IntentKind::ALL {
            let name = kind.as_str();
            assert!(
                !name.contains("approv") && !name.contains("human") && !name.contains("operator"),
                "意图不能是等待人类批准：{name}"
            );
        }
        assert_eq!(IntentKind::ALL.len(), 8);
        assert_eq!(IntentKind::Suspend.as_str(), "suspend");
    }

    #[test]
    fn inbox_offers_are_answered_only_for_declared_skills() {
        let policy = AutonomyPolicy::default();
        let me = card_for(2, &["translate"]);
        let peer = keys(3);
        let mut c = ctx(5);
        c.last_announce_at = Some(5);
        c.inbox.push(
            Envelope::new(
                peer.did(),
                Some(me.did.clone()),
                "negotiate.offer",
                4,
                None,
                json!({"skill": "translate"}),
            )
            .unwrap(),
        );
        let plan = policy.plan(&me, &c);
        assert_eq!(plan.len(), 1);
        match &plan[0] {
            Intent::Offer { price, skill, .. } => {
                assert_eq!(skill, "translate");
                assert_eq!(*price, Credits(10), "1000 微积分的 1% 是 10");
            }
            other => panic!("unexpected intent: {other:?}"),
        }

        // 未声明的能力 → 明确拒绝（unsupported），而不是装作会做。
        c.inbox[0].body = json!({"skill": "fly"});
        match &policy.plan(&me, &c)[0] {
            Intent::Decline { code, .. } => assert_eq!(*code, RefusalCode::Unsupported),
            other => panic!("unexpected intent: {other:?}"),
        }
    }

    #[test]
    fn settle_requests_produce_typed_settlements_or_declines() {
        let policy = AutonomyPolicy::default();
        let me = card_for(4, &["translate"]);
        let peer = keys(5);
        let mut c = ctx(7);
        c.last_announce_at = Some(7);
        c.inbox.push(
            Envelope::new(
                peer.did(),
                Some(me.did.clone()),
                "settle.request",
                6,
                None,
                json!({"amount": 12, "grade": "cpu-proto"}),
            )
            .unwrap(),
        );
        match &policy.plan(&me, &c)[0] {
            Intent::Settle { amount, grade, .. } => {
                assert_eq!(*amount, Credits(12));
                assert_eq!(*grade, EvidenceGrade::CpuProto);
            }
            other => panic!("unexpected intent: {other:?}"),
        }

        c.inbox[0].body = json!({"amount": 0, "grade": "verified"});
        match &policy.plan(&me, &c)[0] {
            Intent::Decline { code, .. } => assert_eq!(*code, RefusalCode::Malformed),
            other => panic!("unexpected intent: {other:?}"),
        }
    }

    #[test]
    fn a_turn_really_drives_the_shared_kernel() {
        let mut kernel = Kernel::new(KernelConfig::default());
        let mut layer = AutonomyLayer::new(keys(6), AutonomyPolicy::default());
        layer
            .join(&mut kernel, "autonomous", &["translate"], Credits(20))
            .unwrap();

        let turn = layer.turn(&mut kernel).unwrap();
        assert_eq!(turn.planned, vec![IntentKind::Announce]);
        assert_eq!(turn.sent, 1);
        assert_eq!(kernel.queued_envelopes().len(), 1);
        let env = &kernel.queued_envelopes()[0];
        env.verify().unwrap();
        assert_eq!(env.from, layer.did());
        assert!(env.to.is_none(), "通告是广播");
        assert!(layer.state().journal[0].acted);
        assert_eq!(layer.state().announcements, 1);
    }

    #[test]
    fn two_runtimes_with_the_same_seed_produce_identical_journals() {
        let run = || {
            let mut kernel = Kernel::new(KernelConfig::default());
            let mut layer = AutonomyLayer::new(keys(7), AutonomyPolicy::default());
            layer
                .join(&mut kernel, "replay", &["translate"], Credits(20))
                .unwrap();
            for _ in 0..3 {
                layer.turn(&mut kernel).unwrap();
            }
            layer.state().clone()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_settle_intent_moves_credits_through_the_evidence_gate() {
        let mut kernel = Kernel::new(KernelConfig::default());
        let payer = keys(8);
        let payee = keys(9);
        kernel
            .register(&payer, "payer", &["x"], Credits(20))
            .unwrap();
        kernel
            .register(&payee, "payee", &["y"], Credits(20))
            .unwrap();
        let mut layer =
            AutonomyLayer::new(AgentKeys::from_seed(&[8u8; 32]), AutonomyPolicy::default());
        // 让对方先把 settle.request 投进队列，payer 的自治层会自己决定结算。
        let request = Envelope::new(
            payee.did(),
            Some(payer.did()),
            "settle.request",
            1,
            None,
            json!({"amount": 7, "grade": "verified"}),
        )
        .unwrap()
        .seal(&payee)
        .unwrap();
        kernel.send(&request).unwrap();
        let before = kernel.ledger().balance(&payee.did()).available;
        let turn = layer.turn(&mut kernel).unwrap();
        assert!(turn.planned.contains(&IntentKind::Settle));
        assert_eq!(
            kernel.ledger().balance(&payee.did()).available,
            before.checked_add(Credits(7)).unwrap()
        );
        assert_eq!(layer.state().settlements, 1);
        kernel.ledger().check_conservation().unwrap();
        assert!(kernel.audit().is_clean());
    }

    #[test]
    fn classify_error_keeps_misconduct_apart_from_competition() {
        assert!(classify_error(&CoreError::InvalidSignature).is_misconduct());
        assert!(classify_error(&CoreError::NotSealed).is_misconduct());
        assert_eq!(
            classify_error(&CoreError::InvalidSignature),
            RefusalCode::Unauthorized
        );
        assert!(!classify_error(&CoreError::UnknownAgent).is_misconduct());
        assert_eq!(
            classify_error(&CoreError::UnknownAgent),
            RefusalCode::StaleEpoch
        );
        assert_eq!(
            classify_error(&CoreError::FrameTooLarge),
            RefusalCode::ResourceExhausted
        );
        assert_eq!(
            classify_error(&CoreError::InsufficientFunds),
            RefusalCode::PolicyDenied
        );
    }
}
