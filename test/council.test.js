// v1.7.0 — 委员会治理测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Council, CouncilType } from '../src/council/committee.js';

test('选举：按信誉降序选出委员', () => {
  const members = Council.elect(
    [
      { did: 'a', reputation: 0.3 },
      { did: 'b', reputation: 0.9 },
      { did: 'c', reputation: 0.6 },
      { did: 'd', reputation: 0.8 },
    ],
    3,
  );
  assert.deepEqual(members.map((m) => m.did), ['b', 'd', 'c']);
});

test('提案表决：BFT-lite quorum（n=7 → f=2 → quorum=5）', () => {
  const dids = Array.from({ length: 7 }, (_, i) => `did:m${i}`);
  const council = new Council({ type: CouncilType.TASK, members: dids.map((d) => ({ did: d, reputation: 0.7 })) });
  assert.equal(council.quorum, 5);

  const p = council.propose({ by: dids[0], title: '路由策略调整', payload: { policy: 'latency-first' } });
  for (let i = 0; i < 4; i++) {
    assert.equal(council.vote(p.id, dids[i], true).status, 'voting');
  }
  const final = council.vote(p.id, dids[4], true);
  assert.equal(final.status, 'passed');
  assert.ok(p.executedAt > 0, '通过即执行');
});

test('人类否决权：阻止执行并公开理由', () => {
  const members = [{ did: 'm1', reputation: 0.8 }, { did: 'm2', reputation: 0.8 }];
  const council = new Council({ type: CouncilType.EVOLUTION, members });
  const p = council.propose({ by: 'm1', title: '协议升级' });
  const v = council.veto(p.id, 'human-observer', '升级窗口与主网维护冲突');
  assert.equal(v.status, 'vetoed');
  assert.equal(p.vetoed, true);
  assert.ok(council.vetoes.some((x) => x.reason.includes('维护冲突')), '否决理由公开记录');
});

test('安全委员会紧急通道：即时下发、事后确认；非安全委员会禁用', () => {
  const sec = new Council({ type: CouncilType.SECURITY, members: [{ did: 's1', reputation: 0.9 }] });
  const d = sec.emergencyDirective({ by: 's1', content: '暂停可疑 Agent 的交易' });
  assert.equal(d.confirmed, false);
  sec.confirmEmergency(d.id);
  assert.equal(d.confirmed, true);

  const task = new Council({ type: CouncilType.TASK, members: [{ did: 't1', reputation: 0.5 }] });
  assert.throws(() => task.emergencyDirective({ by: 't1', content: 'x' }), /仅安全委员会/);
});
