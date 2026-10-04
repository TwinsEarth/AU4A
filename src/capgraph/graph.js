// v1.1.0 — 能力图（CapabilityGraph）
// Agent 不只声明“我会翻译”，还声明“收到英文 200ms 内返回中文，负载 30%，单价 5 积分”。
// 支持：声明/广播（内存总线）/查询（skill·格式·延迟·负载·价格）/能力路径规划。
export class Capability {
  constructor({ skill, latencyP50 = 0, throughput = 1, pricePerUnit = 1, reliability = 1.0, supportedFormats = [], constraints = [] }) {
    this.skill = skill;
    this.latencyP50 = latencyP50;          // 毫秒
    this.throughput = throughput;          // 每秒任务数
    this.pricePerUnit = pricePerUnit;      // 每单位积分
    this.reliability = reliability;        // 0.0-1.0
    this.supportedFormats = supportedFormats;
    this.constraints = constraints;
    this.currentLoad = 0.0;                // 0.0-1.0
    this.version = 1;
  }
}

export class CapabilityGraph {
  constructor(bus = null) {
    this.graph = new Map();   // did → Capability[]
    this._index = new Map();  // v1.1.7 skill → Set<did>（查询倒排索引）
    this.bus = bus;           // 可选内存总线：{ publish(topic, payload), subscribe(topic, fn) }
    this._topics = new Map(); // topic → Set<fn>（无 bus 时本地订阅）
  }

  /** Agent 声明/更新能力（变更即版本自增） */
  declare(did, caps) {
    const arr = caps.map((c) => (c instanceof Capability ? c : new Capability(c)));
    this.graph.set(did, arr);
    // 重建该 Agent 的索引条目（先摘除旧 skill 再登记新 skill）
    for (const [skill, set] of this._index) set.delete(did);
    for (const c of arr) {
      if (!this._index.has(c.skill)) this._index.set(c.skill, new Set());
      this._index.get(c.skill).add(did);
    }
    this._publish('/capgraph/1.0.0', { type: 'UPDATE', did, caps: arr.map((c) => c.version) });
    return arr;
  }

  /** 广播：向总线发布能力图快照 */
  _publish(topic, payload) {
    if (this.bus && typeof this.bus.publish === 'function') this.bus.publish(topic, payload);
    const subs = this._topics.get(topic);
    if (subs) for (const fn of subs) fn(payload);
  }

  subscribe(topic, fn) {
    if (!this._topics.has(topic)) this._topics.set(topic, new Set());
    this._topics.get(topic).add(fn);
    return () => this._topics.get(topic).delete(fn);
  }

  /** 查询：按 skill + 可选过滤条件（v1.1.7：走 skill 倒排索引，不扫描全图） */
  query(skill, { format = null, maxLatency = Infinity, maxLoad = 1.0, maxPrice = Infinity } = {}) {
    const out = [];
    const dids = this._index.get(skill) || new Set();
    for (const did of dids) {
      for (const c of this.graph.get(did) || []) {
        if (c.skill !== skill) continue;
        if (format && c.supportedFormats.length && !c.supportedFormats.includes(format)) continue;
        if (c.latencyP50 > maxLatency) continue;
        if (c.currentLoad > maxLoad) continue;
        if (c.pricePerUnit > maxPrice) continue;
        out.push({ did, ...c });
      }
    }
    return out.sort((a, b) => a.pricePerUnit - b.pricePerUnit || a.latencyP50 - b.latencyP50);
  }

  /**
   * 能力路径规划：为需求链 [skill1, skill2, ...] 找出可用流水线。
   * 约束：相邻能力必须格式兼容（上一能力输出 ⊆ 下一能力输入格式），负载 < maxLoad。
   * 返回 { path: [{did, capability}], totalPrice, compatible }
   */
  route(requiredSkills, { maxLoad = 1.0, maxLatency = Infinity } = {}) {
    const path = [];
    let prevOutput = null;
    for (const skill of requiredSkills) {
      let candidates = this.query(skill, { maxLoad, maxLatency });
      if (prevOutput) {
        candidates = candidates.filter(
          (c) => !c.supportedFormats.length || c.supportedFormats.includes(prevOutput),
        );
      }
      if (!candidates.length) return { path: [], totalPrice: 0, compatible: false, missingAt: skill };
      const pick = candidates[0];
      path.push({ did: pick.did, capability: { skill: pick.skill, pricePerUnit: pick.pricePerUnit, latencyP50: pick.latencyP50 } });
      prevOutput = pick.supportedFormats[0] || prevOutput;
    }
    return { path, totalPrice: path.reduce((s, p) => s + p.capability.pricePerUnit, 0), compatible: true };
  }

  /** 负载更新（由执行侧回调，供查询/规划参考） */
  setLoad(did, skill, load) {
    const caps = this.graph.get(did);
    if (!caps) throw new Error('Agent 未声明能力');
    const c = caps.find((x) => x.skill === skill);
    if (!c) throw new Error('能力不存在');
    c.currentLoad = Math.max(0, Math.min(1, load));
    c.version += 1;
    this._publish('/capgraph/1.0.0', { type: 'LOAD', did, skill, load });
    return c;
  }
}
