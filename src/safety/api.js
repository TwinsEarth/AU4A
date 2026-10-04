// v1.5.0 — 安全 API（Safety API）
// Agent 侧安全接口：查询自己的权限边界、举报违规、提交申诉证据、查询处罚记录。
// 申诉接入仲裁（v1.7 委员会治理中由仲裁委员会裁决）。
export const SafetyStatus = {
  REPORTED: 'REPORTED',
  APPEALED: 'APPEALED',
  ARBITRATED: 'ARBITRATED',
  DISMISSED: 'DISMISSED',
};

export class PermissionPolicy {
  /** rules: [{ action, allow: [did|'*'], deny: [did] }] */
  constructor(rules = []) {
    this.rules = rules;
  }

  can(did, action) {
    const rule = this.rules.find((r) => r.action === action);
    if (!rule) return false;
    if ((rule.deny || []).includes(did)) return false;
    if ((rule.allow || []).includes('*') || (rule.allow || []).includes(did)) return true;
    return false;
  }

  /** Agent 查询自身权限边界（只读） */
  boundary(did) {
    return this.rules
      .filter((r) => r.allow.includes('*') || r.allow.includes(did))
      .map((r) => ({ action: r.action, allowed: !r.deny.includes(did) }));
  }
}

export class SafetyAPI {
  constructor(policy) {
    this.policy = policy;
    this.cases = new Map();
    this.penalties = new Map(); // did → [{ caseId, reason, slashed }]
    this._seq = 0;
  }

  /** 权限查询：Agent 执行动作前自检 */
  queryPermission(did, action) {
    const allowed = this.policy.can(did, action);
    return { did, action, allowed, boundary: this.policy.boundary(did) };
  }

  /** 举报违规：任何 Agent 可举报其他 Agent */
  report(reporter, target, reason, evidence = '') {
    const id = `safety-${++this._seq}`;
    const c = { id, reporter, target, reason, evidence, status: SafetyStatus.REPORTED, decision: null, at: Date.now() };
    this.cases.set(id, c);
    return c;
  }

  /** 申诉：被处罚 Agent 提交证据，进入仲裁 */
  appeal(did, caseId, evidence) {
    const c = this.cases.get(caseId);
    if (!c) throw new Error('案件不存在');
    if (c.target !== did) throw new Error('只有被举报方可申诉');
    c.evidence = evidence;
    c.status = SafetyStatus.APPEALED;
    return c;
  }

  /** 仲裁裁决（v1.7 委员会调用；此处提供裁决与罚没记录入口） */
  arbitrate(caseId, decision, { slashed = 0, reason = '' } = {}) {
    const c = this.cases.get(caseId);
    if (!c) throw new Error('案件不存在');
    c.status = SafetyStatus.ARBITRATED;
    c.decision = decision; // 'guilty' | 'innocent'
    if (decision === 'guilty') {
      const rec = { caseId, reason: reason || c.reason, slashed, at: Date.now() };
      const list = this.penalties.get(c.target) || [];
      list.push(rec);
      this.penalties.set(c.target, list);
    }
    return c;
  }

  /** 处罚记录查询（Agent 可查自身；观察层可查全局） */
  penaltyRecord(did) {
    return this.penalties.get(did) || [];
  }
}
