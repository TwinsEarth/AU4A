// v1.9.x — 网络扩展度量（Network Scaling Metrics）
// 多智能体缩放定律度量：节点数、交互复杂度、吞吐、效率、编排开销占比。
// 验证命题：结构决定增长曲线形状，规模决定站在曲线哪一段；
// 在错误的结构上加规模，收益为负（效率越过顶点后随节点数下降）。
export class ScalingMetrics {
  constructor() {
    this.samples = [];
  }

  /**
   * 记录一次实验样本。
   * @param {object} s { nodes, completed, totalTasks, overheadMs, durationMs }
   */
  record({ nodes, completed, totalTasks, overheadMs, durationMs }) {
    const throughput = durationMs > 0 ? completed / (durationMs / 1000) : 0;
    const efficiency = totalTasks > 0 ? completed / totalTasks : 0;
    const overheadRatio = durationMs > 0 ? overheadMs / durationMs : 0;
    const costEfficiency = totalTasks > 0 ? completed / Math.max(1, overheadMs) : 0;
    const sample = {
      nodes,
      completed,
      efficiency: Math.round(efficiency * 1000) / 1000,
      throughput: Math.round(throughput * 100) / 100,
      overheadRatio: Math.round(overheadRatio * 1000) / 1000,
      costEfficiency: Math.round(costEfficiency * 1000) / 1000,
      at: Date.now(),
    };
    this.samples.push(sample);
    return sample;
  }

  /**
   * 缩放裁决：完成率（efficiency）的峰值即“有用规模的顶点”；
   * 顶点之后完成率下降、或完成率持平但编排开销占比显著恶化 → 加规模收益为负（saturated）。
   * @returns {{verdict: 'scaling'|'saturated'|'insufficient', peakNodes, peakEfficiency}}
   */
  scalingVerdict() {
    const sorted = [...this.samples].sort((a, b) => a.nodes - b.nodes);
    if (sorted.length < 2) return { verdict: 'insufficient', peakNodes: null, peakEfficiency: null };
    let peak = sorted[0];
    let declining = false;
    for (let i = 1; i < sorted.length; i++) {
      if (sorted[i].efficiency > peak.efficiency) peak = sorted[i];
      else if (sorted[i].efficiency < peak.efficiency) declining = true;
    }
    const last = sorted[sorted.length - 1];
    const plateauOverhead = last.efficiency === peak.efficiency && last.overheadRatio > peak.overheadRatio;
    const verdict = last.nodes > peak.nodes && (declining || plateauOverhead) ? 'saturated' : 'scaling';
    return { verdict, peakNodes: peak.nodes, peakEfficiency: peak.efficiency };
  }

  summarize() {
    const verdict = this.scalingVerdict();
    return {
      samples: this.samples.length,
      nodes: this.samples.map((s) => s.nodes),
      verdict,
    };
  }
}

/** 实验运行器：模拟 N 个节点 × R 轮，统计完成量、编排开销与耗时 */
export class ExperimentRunner {
  constructor({ overheadPerNodeMs = 1, capacityPerNode = 4 } = {}) {
    this.overheadPerNodeMs = overheadPerNodeMs;
    this.capacityPerNode = capacityPerNode; // 每节点并行能力（超过则互相争抢 → 效率下降）
    this.metrics = new ScalingMetrics();
  }

  /** 运行一轮：总任务 totalTasks，节点数 nodes；返回样本 */
  run({ nodes, totalTasks, durationMs = 100 }) {
    const overheadMs = nodes * this.overheadPerNodeMs * totalTasks;
    const capacity = nodes * this.capacityPerNode;
    const completed = Math.min(totalTasks, Math.max(0, Math.floor(capacity * (totalTasks / Math.max(totalTasks, 1)))));
    return this.metrics.record({ nodes, completed, totalTasks, overheadMs, durationMs });
  }
}
