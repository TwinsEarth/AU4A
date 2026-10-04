// v1.4.0 — 经济自主（Economic Autonomy）
// Agent 依据 信誉/负载/稀缺度 自主定价，自主投标、自主结算；
// 积分兑换 BTC/ETH 由 Agent 决策（金额/时效路由，v1.8 接入真适配器）。
import au from '@twinsearth/agent-universe';

const { MIN_STAKE } = au;

export class PricingStrategy {
  /**
   * @param {object} opts basePrice 基础价；maxLoadFactor 负载加价上限；scarcityWindow 同能力提供者数阈值
   */
  constructor({ basePrice = 1, maxLoadFactor = 2.0 } = {}) {
    this.basePrice = basePrice;
    this.maxLoadFactor = maxLoadFactor;
  }

  /**
   * 自主定价：price = base × (1 + load) × (1 + scarcity) × (1 + reputationBonus)
   * @param {object} ctx { load(0-1), providers(同能力提供者数), reputation(0-1) }
   */
  price(ctx) {
    const load = Math.max(0, Math.min(1, ctx.load ?? 0));
    const scarcity = Math.max(0, 1 - (ctx.providers ?? 1));
    const rep = Math.max(0, Math.min(1, ctx.reputation ?? 0.5));
    const p = this.basePrice * (1 + load) * (1 + scarcity * 0.5) * (1 + rep * 0.5);
    return Math.max(1, Math.round(p));
  }
}

export class AgentEconomy {
  constructor(market) {
    this.market = market;
  }

  /** Agent 自主充值 */
  fund(account, amount) {
    this.market.deposit(account, amount);
    return this.market.balance(account);
  }

  /** Agent 自主质押/注册（由自治层调用） */
  stakeRegister(card, stake) {
    if (stake < MIN_STAKE) throw new Error(`质押不足，最低 ${MIN_STAKE}`);
    return this.market.registerAgent(card, stake);
  }

  /** Agent 自主发布任务（requester 也是 Agent） */
  publish(taskId, goal, budget, requester) {
    return this.market.publishTask(taskId, goal, budget, requester);
  }

  /** Agent 自主投标 */
  bid(taskId, agentId, price) {
    return this.market.submitBid(taskId, agentId, price);
  }

  /** Agent 自主结算（验收通过才支付） */
  settle(taskId) {
    return this.market.settle(taskId);
  }

  /** 守恒 + 独立审计（系统底线，任何自治不得破坏） */
  integrity() {
    return {
      conservation: this.market.conservationCheck(this.market.totalDeposits),
      audit: this.market.independentAudit(),
    };
  }
}

/** 兑换路由（v1.4 接口 + v1.8 真适配器） */
export class ExchangeRouter {
  /**
   * Agent 自主决策兑换轨道。
   * @returns {{track: 'btc'|'eth'|'hold', reason: string}}
   */
  route({ amount, urgency = 0.5, ethGasPremium = 1.0 } = {}) {
    if (amount < 100) return { track: 'hold', reason: '小额聚合后再兑换' };
    if (urgency >= 0.8) return { track: 'btc', reason: '高时效：BTC RGB 结算确定性优先' };
    if (ethGasPremium > 1.5) return { track: 'btc', reason: 'ETH gas 偏高，转 BTC' };
    return { track: 'eth', reason: '常规金额走 ERC-8004/x402' };
  }
}
