// v1.9.2 — 大规模集群模拟（真机可运行）
// 按节点规模矩阵（10/100/1k/10k）运行 ExperimentRunner，输出缩放度量与饱和判定。
// 运行：node tools/cluster-sim.js
import { ExperimentRunner } from '../src/scale/metrics.js';

const SIZES = [10, 100, 1000, 10000];
const runner = new ExperimentRunner({ overheadPerNodeMs: 1, capacityPerNode: 4 });
const TASKS = 50000;

console.log('=== Agent 集群缩放模拟（totalTasks=' + TASKS + '）===');
console.log('节点数 | 完成量 | 吞吐(任务/百ms) | 效率 | 编排开销占比');
for (const nodes of SIZES) {
  const s = runner.run({ nodes, totalTasks: TASKS, durationMs: 100 });
  console.log(
    String(nodes).padStart(7) + ' | ' +
    String(s.completed).padStart(6) + ' | ' +
    String(s.throughput).padStart(13) + ' | ' +
    (s.efficiency * 100).toFixed(1) + '% | ' +
    (s.overheadRatio * 100).toFixed(1) + '%',
  );
}
const verdict = runner.metrics.summarize();
console.log(`\n缩放判定：${verdict.verdict === 'saturated' ? 'SATURATED（已达饱和，加节点不再提升吞吐，编排开销恶化）' : 'SCALING（仍在扩展区）'}`);

// 打印各规模样本
console.log('\n样本明细：');
for (const s of runner.metrics.samples) {
  console.log(`  nodes=${s.nodes} completed=${s.completed} efficiency=${(s.efficiency * 100).toFixed(1)}% overheadRatio=${(s.overheadRatio * 100).toFixed(1)}%`);
}
