// v1.5.0 — 安全 API 测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PermissionPolicy, SafetyAPI, SafetyStatus } from '../src/safety/api.js';

const policy = new PermissionPolicy([
  { action: 'read', allow: ['*'], deny: [] },
  { action: 'publish', allow: ['*'], deny: ['did:au:rogue'] },
  { action: 'settle', allow: ['did:au:treasurer'], deny: [] },
]);

test('权限边界查询：允许/拒绝/未定义', () => {
  const api = new SafetyAPI(policy);
  assert.equal(api.queryPermission('did:x', 'read').allowed, true);
  assert.equal(api.queryPermission('did:au:rogue', 'publish').allowed, false);
  assert.equal(api.queryPermission('did:y', 'settle').allowed, false, '未列入 allow 的 Agent 无权 settle');
  assert.equal(api.queryPermission('did:x', 'unknown-action').allowed, false);
  assert.ok(api.queryPermission('did:x', 'read').boundary.length >= 1, '边界查询应返回其可见动作');
});

test('举报 → 申诉 → 仲裁 → 处罚记录', () => {
  const api = new SafetyAPI(policy);
  const c = api.report('did:au:watcher', 'did:au:rogue', '越权发布');
  assert.equal(c.status, SafetyStatus.REPORTED);

  api.appeal('did:au:rogue', c.id, '证据：任务 ID 授权记录');
  assert.equal(c.status, SafetyStatus.APPEALED);
  assert.throws(() => api.appeal('did:other', c.id, 'x'), /只有被举报方可申诉/);

  api.arbitrate(c.id, 'guilty', { slashed: 50, reason: '越权发布属实' });
  const rec = api.penaltyRecord('did:au:rogue');
  assert.equal(rec.length, 1);
  assert.equal(rec[0].slashed, 50);
  assert.equal(api.penaltyRecord('did:au:clean').length, 0);
  assert.equal(api.cases.get(c.id).decision, 'guilty');
});

test('无罪裁决不产生处罚记录', () => {
  const api = new SafetyAPI(policy);
  const c = api.report('a', 'b', '误报');
  api.arbitrate(c.id, 'innocent');
  assert.equal(api.penaltyRecord('b').length, 0);
});
