// v1.6.9 — 学习效果评估测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { runEvaluation, mulberry32 } from '../src/learning/eval.js';

test('效果评估：学习组成功率显著高于对照组，且可复现', () => {
  const r1 = runEvaluation({ rounds: 400, seed: 42 });
  assert.equal(r1.verdict, 'learning-effective', '学习应带来可测量提升');
  assert.ok(r1.delta.successRate > 0.02, `提升 ${r1.delta.successRate} 应 > 2 个百分点`);
  assert.ok(r1.delta.avgReward > 0, '收益也应提升');
  // 可复现：同种子结果一致
  const r2 = runEvaluation({ rounds: 400, seed: 42 });
  assert.deepEqual(r1, r2, '固定种子必须完全可复现');
});

test('效果评估：对照组成功率收敛在基础值附近', () => {
  const r = runEvaluation({ rounds: 2000, seed: 7 });
  assert.ok(Math.abs(r.baseline.successRate - 0.55) < 0.05, `基线 ${r.baseline.successRate} 应≈0.55`);
});

test('mulberry32：固定种子输出确定', () => {
  const a = mulberry32(1);
  const b = mulberry32(1);
  const seq1 = [a(), a(), a()];
  const seq2 = [b(), b(), b()];
  assert.deepEqual(seq1, seq2);
  assert.ok(seq1.every((x) => x >= 0 && x < 1));
});
