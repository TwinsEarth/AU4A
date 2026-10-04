// v1.0.7 — 质押/解质押自主测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import au from '@twinsearth/agent-universe';
import { AgentRegistry } from '../src/autonomy/registry.js';

const market = () => new au.AgentMarket();

test('解质押：锁定→主账户，守恒与独立审计仍通过', () => {
  const m = market();
  const reg = new AgentRegistry(m);
  const { ident } = reg.register({ name: 'A', stake: 100, initial: 200 });
  const did = ident.did;
  assert.equal(reg.stakeOf(did), 100);
  assert.equal(m.balance(did), 100);

  const r = reg.unstake(did, 40);
  assert.equal(r.remainingStake, 60);
  assert.equal(m.balance(did), 140, '主账户应增加 40');
  assert.equal(reg.stakeOf(did), 60);
  assert.equal(m.conservationCheck(m.totalDeposits).conserved, true, '解质押不破坏守恒');
  assert.equal(m.independentAudit().passed, true, 'Unstaked 记录可被审计回放');
});

test('解质押边界：超质押额拒绝；非法金额拒绝', () => {
  const m = market();
  const reg = new AgentRegistry(m);
  const { ident } = reg.register({ name: 'B', stake: 100, initial: 100 });
  assert.throws(() => reg.unstake(ident.did, 101), /超过已质押额/);
  assert.throws(() => reg.unstake(ident.did, -5), /正整数/);
  assert.throws(() => reg.unstake('did:none', 10), /未注册/);
});

test('解质押致剩余 < MIN_STAKE → 状态转 suspended', () => {
  const m = market();
  const reg = new AgentRegistry(m);
  const { ident } = reg.register({ name: 'C', stake: 100, initial: 100 });
  const r = reg.unstake(ident.did, 100);
  assert.equal(r.remainingStake, 0);
  assert.equal(r.status, 'suspended');
  assert.equal(reg.get(ident.did).status, 'suspended');
});
