// v1.0.1 — Agent 自主注册（AgentRegistry）
// 流程：自生成身份 → 自充值（可选）→ 自质押 → 自注册 → 获得网络准入。
// 注册的发起者与受益者都是 Agent 自身；人类不参与、不审批。
import au from '@twinsearth/agent-universe';
import { Identity } from '../core/identity.js';

const { MIN_STAKE } = au;

export class AgentRegistry {
  constructor(market) {
    this.market = market;
    this.agents = new Map(); // did → { ident, card, status, registeredAt }
  }

  /**
   * Agent 自主注册。
   * @param {object} opts { name, skills, capabilities, stake(≥MIN_STAKE), initial(初始充值，默认0) }
   */
  register({ name, skills = [], capabilities = [], stake = MIN_STAKE, initial = 0 } = {}) {
    if (!Number.isInteger(stake) || stake < MIN_STAKE) {
      throw new Error(`质押不足，最低 ${MIN_STAKE}`);
    }
    const ident = Identity.generate(name);
    const card = Identity.card({ did: ident.did, name, skills, capabilities });
    if (initial > 0) this.market.deposit(ident.did, initial);
    const ok = this.market.registerAgent(card, stake);
    const agent = { ident, card, status: ok ? 'active' : 'rejected', registeredAt: Date.now() };
    this.agents.set(ident.did, agent);
    return agent;
  }

  get(did) {
    return this.agents.get(did);
  }

  list() {
    return [...this.agents.values()];
  }

  count() {
    return this.agents.size;
  }

  /** Agent 声明/更新能力（供后续 v1.1 能力图消费） */
  declareCapabilities(did, capabilities) {
    const entry = this.agents.get(did);
    if (!entry) throw new Error('Agent 未注册');
    entry.card.capabilities = capabilities;
    return entry.card;
  }

  /**
   * v1.0.7 — 质押/解质押自主。
   * 解质押把锁定账户（__stake__:<did>）资金转回主账户；
   * balances 总和不变 → 守恒恒真；记录 Unstaked（from/to）→ 独立审计可回放。
   * 解后剩余质押 < MIN_STAKE → 状态转 suspended。
   */
  unstake(did, amount) {
    const agent = this.agents.get(did);
    if (!agent) throw new Error('Agent 未注册');
    if (!Number.isInteger(amount) || amount <= 0) throw new Error('解质押金额必须为正整数');
    const lockedKey = `__stake__:${did}`;
    const locked = this.market.balances.get(lockedKey) || 0;
    if (amount > locked) throw new Error(`解质押超过已质押额（当前 ${locked}）`);
    // 锁定 → 主账户（balances 总和不变）
    this.market.balances.set(did, (this.market.balances.get(did) || 0) + amount);
    const rest = locked - amount;
    if (rest === 0) this.market.balances.delete(lockedKey);
    else this.market.balances.set(lockedKey, rest);
    agent.status = rest < MIN_STAKE ? 'suspended' : agent.status;
    agent.unstakedAt = Date.now();
    this.market.records.push({ reason: 'Unstaked', from: lockedKey, to: did, amount, at: Date.now() });
    return { did, unstaked: amount, remainingStake: rest, status: agent.status };
  }

  /** 查询 Agent 当前质押（只读） */
  stakeOf(did) {
    return this.market.balances.get(`__stake__:${did}`) || 0;
  }
}
