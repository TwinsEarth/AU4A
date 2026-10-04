// v1.6.0 — 个体学习（Individual Learning）
// 经验收集 → 反馈分析 → 行为调整 → 效果评估 → 经验更新。
// 学习信号：任务完成质量、结算金额、信誉变化、违规记录。
export class Experience {
  constructor({ taskId, taskType, context = {}, action = {}, outcome, reward = 0, timestamp = Date.now(), peerAgents = [] }) {
    this.taskId = taskId;
    this.taskType = taskType;
    this.context = context;
    this.action = action;
    this.outcome = outcome;   // 'Success' | 'Failure' | 'Partial'
    this.reward = reward;
    this.timestamp = timestamp;
    this.peerAgents = peerAgents;
  }
}

export class ExperienceStore {
  constructor() {
    this.items = [];
    this.byType = new Map();
  }

  add(exp) {
    const e = exp instanceof Experience ? exp : new Experience(exp);
    this.items.push(e);
    if (!this.byType.has(e.taskType)) this.byType.set(e.taskType, []);
    this.byType.get(e.taskType).push(e);
    return e;
  }

  recent(n = 10) {
    return this.items.slice(-n);
  }

  ofType(taskType) {
    return this.byType.get(taskType) || [];
  }

  stats(taskType) {
    const list = this.ofType(taskType);
    if (!list.length) return { count: 0, successRate: 0, avgReward: 0 };
    const succ = list.filter((e) => e.outcome === 'Success').length;
    const avg = list.reduce((s, e) => s + e.reward, 0) / list.length;
    return { count: list.length, successRate: succ / list.length, avgReward: avg };
  }

  /** 隐私：公开视图剥离 context/action 明细 */
  toPublic() {
    return this.items.map((e) => ({
      taskType: e.taskType,
      outcome: e.outcome,
      reward: e.reward,
      timestamp: e.timestamp,
    }));
  }
}

/** 学习循环：基于经验调整行为（定价偏好 + 任务选择偏好） */
export class LearningLoop {
  constructor(store = new ExperienceStore()) {
    this.store = store;
    this.behavior = { priceBump: new Map(), selectivity: new Map() }; // taskType → 系数
  }

  /** 一次经验入库 → 更新行为 */
  learn(exp) {
    const e = this.store.add(exp);
    const stats = this.store.stats(e.taskType);
    const prev = this.store.items.length;
    // 行为调整（随样本量收敛）
    const bump = Math.min(0.5, Math.max(-0.3, (stats.successRate - 0.5) * 0.8));
    this.behavior.priceBump.set(e.taskType, Math.round(bump * 100) / 100);
    this.behavior.selectivity.set(e.taskType, Math.max(0, Math.min(1, stats.successRate)));
    return { learned: true, samples: prev, stats, priceBump: this.behavior.priceBump.get(e.taskType) };
  }

  /** Agent 报价时参考：价格 = base × (1 + priceBump) */
  quotedPrice(taskType, basePrice) {
    const bump = this.behavior.priceBump.get(taskType) || 0;
    return Math.max(1, Math.round(basePrice * (1 + bump)));
  }

  /** 任务选择评分：越高越优先接 */
  chooseScore(taskType) {
    const sel = this.behavior.selectivity.get(taskType) ?? 0.5;
    const stats = this.store.stats(taskType);
    return Math.round(sel * 100) / 100 + (stats.count ? Math.min(0.2, stats.count * 0.02) : 0);
  }

  evaluate() {
    return {
      behavior: Object.fromEntries(this.behavior.priceBump),
      selectivity: Object.fromEntries(this.behavior.selectivity),
      totalExperiences: this.store.items.length,
    };
  }
}
