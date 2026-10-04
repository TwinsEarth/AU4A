// v1.7.0 — 委员会治理（Committee Governance）
// 五大委员会：资源/任务/仲裁/进化/安全。选举（高信誉）→ 提案 → 表决（BFT-lite
// quorum = 2f+1）→ 执行。人类观察层保留否决权（公开理由）；安全委员会保留
// 紧急广播通道（即时下发、事后确认）。
export const CouncilType = {
  RESOURCE: 'resource',
  TASK: 'task',
  ARBITRATION: 'arbitration',
  EVOLUTION: 'evolution',
  SECURITY: 'security',
};

export class Council {
  constructor({ type = CouncilType.TASK, members = [], quorum = null } = {}) {
    this.type = type;
    this.members = members; // [{ did, reputation }]
    this.proposals = new Map();
    this.emergencyDirectives = [];
    this.vetoes = [];
    this._propSeq = 0;
    this._emgSeq = 0;
    this.quorum = quorum || this._quorumFor(members.length);
  }

  _quorumFor(n) {
    if (n === 0) return 1;
    const f = Math.floor((n - 1) / 3);
    return 2 * f + 1;
  }

  /** 选举：按信誉降序选前 k 名进入委员会 */
  static elect(candidates, k) {
    return [...candidates].sort((a, b) => b.reputation - a.reputation).slice(0, k);
  }

  propose({ by, title, payload = {} }) {
    if (!this.members.some((m) => m.did === by)) throw new Error('仅委员可提案');
    const id = `prop-${this.type}-${++this._propSeq}`;
    const p = { id, by, title, payload, votes: new Map(), status: 'voting', vetoed: false };
    this.proposals.set(id, p);
    return p;
  }

  /** 表决：支持/反对；达成 quorum（仅计支持票）即通过并执行 */
  vote(proposalId, did, support) {
    const p = this.proposals.get(proposalId);
    if (!p || p.status !== 'voting') throw new Error('提案不存在或不在表决中');
    if (!this.members.some((m) => m.did === did)) throw new Error('仅委员可表决');
    if (p.votes.has(did)) throw new Error('委员不可重复表决');
    p.votes.set(did, Boolean(support));
    const yes = [...p.votes.values()].filter(Boolean).length;
    if (yes >= this.quorum) {
      p.status = 'passed';
      p.executedAt = Date.now();
    } else if (p.votes.size >= this.members.length) {
      p.status = 'rejected';
    }
    return { proposalId, status: p.status, yes, quorum: this.quorum };
  }

  /** 人类否决权：仅极端情况，理由公开记录 */
  veto(proposalId, human, reason) {
    const p = this.proposals.get(proposalId);
    if (!p) throw new Error('提案不存在');
    p.status = 'vetoed';
    p.vetoed = true;
    this.vetoes.push({ proposalId, human, reason, at: Date.now() });
    return { status: p.status, publicReason: reason };
  }

  /** 安全委员会紧急广播：即时下发策略，事后由治理确认 */
  emergencyDirective({ by, content }) {
    if (this.type !== CouncilType.SECURITY) throw new Error('仅安全委员会可下发紧急指令');
    const d = { id: `emg-${++this._emgSeq}`, by, content, at: Date.now(), confirmed: false };
    this.emergencyDirectives.push(d);
    return d;
  }

  confirmEmergency(directiveId) {
    const d = this.emergencyDirectives.find((x) => x.id === directiveId);
    if (!d) throw new Error('指令不存在');
    d.confirmed = true;
    return d;
  }

  /** v1.7.9 治理审计：只读导出全部治理事件（观察层展示 + 审计） */
  auditLog() {
    return {
      type: this.type,
      proposals: [...this.proposals.values()].map((p) => ({
        id: p.id, by: p.by, title: p.title, status: p.status,
        vetoed: p.vetoed, yes: [...p.votes.values()].filter(Boolean).length,
        quorum: this.quorum, executedAt: p.executedAt || null,
      })),
      vetoes: this.vetoes.map((v) => ({ ...v })),
      emergencyDirectives: this.emergencyDirectives.map((d) => ({ id: d.id, by: d.by, content: d.content, at: d.at, confirmed: d.confirmed })),
    };
  }
}
