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
}
