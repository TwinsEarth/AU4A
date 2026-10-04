// v1.2.0 — 协商协议（Negotiation Protocol）
// 状态机：IDLE → NEGOTIATING(REQUEST/COUNTER) → ACCEPTED → CONTRACT_SIGNED
//         → EXECUTING → SETTLED
//                     ↘ CONTRACT_BREACH → 仲裁
// 每个状态转换需双方确认；合约哈希 sha256 锚定（可上链/可审计）。
import { createHash } from 'node:crypto';

export const NegotiationState = {
  IDLE: 'IDLE',
  NEGOTIATING: 'NEGOTIATING',
  ACCEPTED: 'ACCEPTED',
  CONTRACT_SIGNED: 'CONTRACT_SIGNED',
  EXECUTING: 'EXECUTING',
  SETTLED: 'SETTLED',
  BREACH: 'BREACH',
};

export class Negotiation {
  constructor({ initiator, responder, goal, maxRounds = 5 } = {}) {
    if (!initiator || !responder || !goal) throw new Error('协商需 initiator/responder/goal');
    this.id = `neg-${initiator.slice(-8)}-${responder.slice(-8)}-${Date.now().toString(36)}`;
    this.initiator = initiator;
    this.responder = responder;
    this.goal = goal;
    this.state = NegotiationState.IDLE;
    this.rounds = [];
    this.maxRounds = maxRounds;
    this.accepted = null;
    this.contract = null;
    this.breachReason = null;
    this.settledAt = null;
  }

  /** 发起方出价（首轮） */
  propose({ price, terms = {} }) {
    this._ensureState([NegotiationState.IDLE]);
    this.state = NegotiationState.NEGOTIATING;
    this.rounds.push({ kind: 'PROPOSE', by: this.initiator, price, terms, at: Date.now() });
    return this.rounds.length;
  }

  /** 回应方还价（多轮） */
  counter({ price, terms = {} }) {
    this._ensureState([NegotiationState.NEGOTIATING]);
    if (this.rounds.length >= this.maxRounds * 2) {
      throw new Error('超过最大协商轮数');
    }
    this.rounds.push({ kind: 'COUNTER', by: this.responder, price, terms, at: Date.now() });
    return this.rounds.length;
  }

  /** 任一方接受当前出价 */
  accept(by) {
    this._ensureState([NegotiationState.NEGOTIATING]);
    const last = this.rounds[this.rounds.length - 1];
    if (last.by === by) throw new Error('不能接受自己提出的出价');
    this.accepted = last;
    this.state = NegotiationState.ACCEPTED;
    return last;
  }

  reject(by, reason = '') {
    this._ensureState([NegotiationState.NEGOTIATING]);
    this.state = NegotiationState.IDLE;
    this.rounds.push({ kind: 'REJECT', by, reason, at: Date.now() });
    return false;
  }

  /** 双方签订合约（生成合约哈希，锚定上链用） */
  sign(by) {
    this._ensureState([NegotiationState.ACCEPTED]);
    if (!this.accepted) throw new Error('无已接受条款');
    const payload = [
      this.id, this.initiator, this.responder, this.goal,
      this.accepted.price, JSON.stringify(this.accepted.terms),
    ].join('|');
    this.contract = {
      id: `ct-${this.id}`,
      initiator: this.initiator,
      responder: this.responder,
      goal: this.goal,
      price: this.accepted.price,
      terms: this.accepted.terms,
      signedBy: [by],
      hash: createHash('sha256').update(payload).digest('hex'),
      signedAt: Date.now(),
    };
    this.state = NegotiationState.CONTRACT_SIGNED;
    return this.contract;
  }

  execute() {
    this._ensureState([NegotiationState.CONTRACT_SIGNED]);
    this.state = NegotiationState.EXECUTING;
    return true;
  }

  settle(by) {
    this._ensureState([NegotiationState.EXECUTING]);
    this.settledBy = by;
    this.settledAt = Date.now();
    this.state = NegotiationState.SETTLED;
    return { id: this.id, state: this.state, settledAt: this.settledAt };
  }

  /** 违约 → 进入仲裁（记录违约方与原因，交给仲裁委员会 v1.7） */
  breach(by, reason) {
    this._ensureState([NegotiationState.EXECUTING, NegotiationState.CONTRACT_SIGNED]);
    this.breachBy = by;
    this.breachReason = reason;
    this.state = NegotiationState.BREACH;
    return { id: this.id, breachBy: by, reason, arbitrationRequired: true };
  }

  toJSON() {
    return JSON.parse(JSON.stringify(this));
  }

  static fromJSON(data) {
    const n = new Negotiation({ initiator: data.initiator, responder: data.responder, goal: data.goal, maxRounds: data.maxRounds });
    Object.assign(n, data);
    return n;
  }

  _ensureState(allowed) {
    if (!allowed.includes(this.state)) {
      throw new Error(`非法状态转换：${this.state} 不允许此操作`);
    }
  }
}
