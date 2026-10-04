// v1.7.9 — 治理审计测试（观察层只读视图 + 审计）
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Council, CouncilType } from '../src/council/committee.js';

const members = [
  { did: 'a', reputation: 95 }, { did: 'b', reputation: 90 }, { did: 'c', reputation: 85 },
  { did: 'd', reputation: 80 }, { did: 'e', reputation: 75 },
];

test('治理审计：提案/否决/紧急指令全部进入只读日志', () => {
  const c = new Council({ type: CouncilType.SECURITY, members, quorum: 3 });
  c.propose({ by: 'a', title: '升级隔离策略', payload: { level: 2 } });
  c.vote('prop-security-1', 'a', true);
  c.vote('prop-security-1', 'b', true);
  c.vote('prop-security-1', 'c', true);
  c.veto('prop-security-1', 'human-1', '极端情况下保留否决权');
  c.emergencyDirective({ by: 'a', content: '立即封禁 did:evil' });
  c.confirmEmergency('emg-1');

  const log = c.auditLog();
  assert.equal(log.proposals.length, 1);
  assert.equal(log.proposals[0].status, 'vetoed');
  assert.equal(log.proposals[0].yes, 3);
  assert.equal(log.vetoes.length, 1);
  assert.equal(log.vetoes[0].reason, '极端情况下保留否决权');
  assert.equal(log.emergencyDirectives.length, 1);
  assert.equal(log.emergencyDirectives[0].confirmed, true);
});

test('治理审计：审计日志不暴露内部 Map 引用（纯数据快照）', () => {
  const c = new Council({ type: CouncilType.TASK, members, quorum: 3 });
  c.propose({ by: 'a', title: 't1' });
  const log = c.auditLog();
  log.proposals[0].title = '篡改';
  log.vetoes.push({ fake: true });
  // 篡改审计快照不得影响内部状态
  assert.equal(c.proposals.get('prop-task-1').title, 't1');
  assert.equal(c.vetoes.length, 0);
});
