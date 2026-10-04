// v1.6.0 — 个体学习测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ExperienceStore, LearningLoop } from '../src/learning/experience.js';

test('经验库：入库/按类型统计/公开视图脱敏', () => {
  const store = new ExperienceStore();
  store.add({ taskId: 't1', taskType: 'translate', context: { secret: '内部' }, outcome: 'Success', reward: 40 });
  store.add({ taskId: 't2', taskType: 'translate', outcome: 'Failure', reward: 0 });
  store.add({ taskId: 't3', taskType: 'code', outcome: 'Success', reward: 60 });

  assert.equal(store.items.length, 3);
  assert.equal(store.stats('translate').count, 2);
  assert.equal(store.stats('translate').successRate, 0.5);
  assert.equal(store.stats('code').avgReward, 60);

  const pub = store.toPublic();
  assert.ok(!JSON.stringify(pub).includes('内部'), '公开视图必须脱敏 context');
});

test('学习循环：成功多的类型获得价格上调与选择偏好', () => {
  const loop = new LearningLoop();
  // translate 连续成功，code 连续失败
  for (let i = 0; i < 5; i++) {
    loop.learn({ taskId: `a${i}`, taskType: 'translate', outcome: 'Success', reward: 50 });
  }
  for (let i = 0; i < 5; i++) {
    loop.learn({ taskId: `b${i}`, taskType: 'code', outcome: 'Failure', reward: -10 });
  }

  const translatePrice = loop.quotedPrice('translate', 100);
  const codePrice = loop.quotedPrice('code', 100);
  assert.ok(translatePrice > 100, '成功类型应提价');
  assert.ok(codePrice < 100, '失败类型应降价');
  assert.ok(loop.chooseScore('translate') > loop.chooseScore('code'), '选择偏好应偏向成功类型');

  const ev = loop.evaluate();
  assert.equal(ev.totalExperiences, 10);
  assert.ok(ev.behavior.translate > 0 && ev.behavior.code < 0);
});
