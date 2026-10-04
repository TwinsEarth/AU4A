// v1.9.x — 网络扩展度量测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ScalingMetrics, ExperimentRunner } from '../src/scale/metrics.js';

test('度量记录：吞吐/效率/编排开销占比', () => {
  const m = new ScalingMetrics();
  const s = m.record({ nodes: 4, completed: 8, totalTasks: 10, overheadMs: 30, durationMs: 100 });
  assert.equal(s.throughput, 80, '8 任务/100ms = 80 任务/秒');
  assert.equal(s.efficiency, 0.8);
  assert.equal(s.overheadRatio, 0.3);
});

test('缩放裁决：容量顶点后效率下降 → saturated（规模收益为负）', () => {
  const m = new ScalingMetrics();
  m.record({ nodes: 1, completed: 4, totalTasks: 10, overheadMs: 10, durationMs: 100 });
  m.record({ nodes: 2, completed: 8, totalTasks: 10, overheadMs: 40, durationMs: 100 });
  m.record({ nodes: 4, completed: 10, totalTasks: 10, overheadMs: 160, durationMs: 100 });
  m.record({ nodes: 8, completed: 10, totalTasks: 10, overheadMs: 640, durationMs: 100 });
  const v = m.scalingVerdict();
  assert.equal(v.verdict, 'saturated', '8 节点与 4 节点完成量相同但开销×4，效率下降');
  assert.equal(v.peakNodes, 4);
});

test('实验运行器：节点数超过容量后完成量触顶、开销增长', () => {
  const r = new ExperimentRunner({ overheadPerNodeMs: 1, capacityPerNode: 4 });
  const s1 = r.run({ nodes: 2, totalTasks: 10, durationMs: 100 });
  const s2 = r.run({ nodes: 8, totalTasks: 10, durationMs: 100 });
  assert.equal(s1.completed, 8);
  assert.equal(s2.completed, 10);
  assert.ok(s2.overheadRatio > s1.overheadRatio, '节点翻倍，编排开销占比上升');
});

test('样本不足时裁决 insufficient', () => {
  const m = new ScalingMetrics();
  m.record({ nodes: 1, completed: 1, totalTasks: 1, overheadMs: 0, durationMs: 10 });
  assert.equal(m.scalingVerdict().verdict, 'insufficient');
});
