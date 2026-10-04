// v1.0.1 — 自治内核测试：Agent 自主注册 + 人类观察层只读 + 守恒/审计
import { test } from 'node:test';
import assert from 'node:assert/strict';
import au from '@twinsearth/agent-universe';
import { AgentRegistry } from '../src/autonomy/registry.js';
import { HumanObserver } from '../src/observer/dashboard.js';
import { Identity } from '../src/core/identity.js';
import { Wallet } from '../src/core/wallet.js';

test('Agent 自主注册：自生成身份/自质押/自注册', () => {
  const market = new au.AgentMarket();
  const registry = new AgentRegistry(market);
  const a = registry.register({ name: 'Worker-A', skills: ['translate'], stake: 100, initial: 200 });
  assert.ok(a.ident.did.startsWith('did:'), 'DID 应为自主生成');
  assert.equal(a.status, 'active');
  assert.equal(market.balance(a.ident.did), 100, '质押后余额 = 初始 200 − 质押 100');
  assert.equal(registry.count(), 1);
});

test('相同能力可注册多个 Agent（DID 互不冲突）', () => {
  const market = new au.AgentMarket();
  const registry = new AgentRegistry(market);
  const a1 = registry.register({ name: 'W1', stake: 100, initial: 100 });
  const a2 = registry.register({ name: 'W2', stake: 100, initial: 100 });
  assert.notEqual(a1.ident.did, a2.ident.did);
});

test('质押不足被拒绝', () => {
  const market = new au.AgentMarket();
  const registry = new AgentRegistry(market);
  assert.throws(() => registry.register({ name: 'X', stake: 50, initial: 100 }), /质押不足/);
});

test('人类观察层只读：进度/结果/收益 + 守恒与独立审计', () => {
  const market = new au.AgentMarket();
  const registry = new AgentRegistry(market);
  const wallet = new Wallet(market);
  registry.register({ name: 'A', skills: ['translate'], stake: 100, initial: 200 });
  registry.register({ name: 'B', skills: ['summarize'], stake: 100, initial: 150 });
  const obs = new HumanObserver(market, registry);
  const s = obs.snapshot();

  assert.equal(s.progress.agents.length, 2, '进度：展示活跃 Agent');
  assert.ok(s.progress.agents.every((a) => a.balance >= 0));
  assert.equal(s.earnings.totalDeposits, 350, '收益：总充值可观测');
  assert.equal(s.integrity.conservation.conserved, true, '守恒必须成立');
  assert.equal(s.integrity.audit.passed, true, '独立审计必须通过');

  // 观察层必须只读：没有任何变更余额/状态的方法
  const writeKeys = Object.keys(obs).filter((k) => /deposit|register|settle|slash|publish|match|complete/.test(k));
  assert.equal(writeKeys.length, 0, '观察层不得暴露写方法');
  assert.equal(obs.summary().activeAgents, 2);
  assert.equal(wallet.conservation().conserved, true);
});

test('Identity 签名能力（供后续协商/投票使用）', () => {
  const ident = Identity.generate('Signer');
  const sig = Identity.sign(ident, 'AU4A-msg');
  assert.ok(typeof sig === 'string' && sig.length > 0, '应产出十六进制签名');
  assert.ok(ident.keypair.verify('AU4A-msg', sig), '验签通过');
});
