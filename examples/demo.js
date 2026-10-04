// AU4A 端到端演示：Agent 自治经济体（覆盖 v1.0–v1.9 全部中版本能力）
// 人类只做一件事：看结果。本脚本末尾的 observer.summary() 就是人类的全部界面。
import au from '@twinsearth/agent-universe';
import { AgentRegistry } from '../src/autonomy/registry.js';
import { HumanObserver } from '../src/observer/dashboard.js';
import { Capability, CapabilityGraph } from '../src/capgraph/graph.js';
import { Negotiation } from '../src/negotiate/protocol.js';
import { AgentEconomy, PricingStrategy, ExchangeRouter } from '../src/economy/pricing.js';
import { LearningLoop } from '../src/learning/experience.js';
import { PermissionPolicy, SafetyAPI } from '../src/safety/api.js';
import { Council, CouncilType } from '../src/council/committee.js';
import { SettlementRouter } from '../src/chain/adapters.js';
import { ScalingMetrics, ExperimentRunner } from '../src/scale/metrics.js';

const market = new au.AgentMarket();
const registry = new AgentRegistry(market);
const observer = new HumanObserver(market, registry);
const econ = new AgentEconomy(market);
const capgraph = new CapabilityGraph();
const learning = new LearningLoop();
const pricing = new PricingStrategy({ basePrice: 30 });

// 1) Agent 自主注册（v1.0）—— 人类没有参与任何一步
const translator = registry.register({ name: 'Translator-A', skills: ['translate'], capabilities: ['nlp'], stake: 100, initial: 200 });
const summarizer = registry.register({ name: 'Summarizer-B', skills: ['summarize'], capabilities: ['nlp'], stake: 100, initial: 200 });
const requester = registry.register({ name: 'Requester-C', skills: ['orchestrate'], stake: 100, initial: 300 });
const [W, S, R] = [translator.ident.did, summarizer.ident.did, requester.ident.did];

// 2) 能力图（v1.1）—— Agent 自主声明能力
capgraph.declare(W, [new Capability({ skill: 'translate', latencyP50: 120, pricePerUnit: 30, supportedFormats: ['text'] })]);
capgraph.declare(S, [new Capability({ skill: 'summarize', latencyP50: 150, pricePerUnit: 28, supportedFormats: ['text'] })]);

// 3) 安全 API（v1.5）—— 执行前自检权限
const safety = new SafetyAPI(new PermissionPolicy([
  { action: 'execute', allow: ['*'], deny: [] },
  { action: 'settle', allow: ['*'], deny: [] },
]));

// 4) 自治经济体：发布→发现→协商(v1.2)→投标→匹配→完成→结算(v1.4)
const tasks = [
  { id: 'task-1', goal: 'translate the doc to Chinese', budget: 50, skill: 'translate', requester: R },
  { id: 'task-2', goal: 'summarize the article', budget: 45, skill: 'summarize', requester: R },
  { id: 'task-3', goal: 'translate the second doc', budget: 55, skill: 'translate', requester: R },
];

for (const t of tasks) {
  // 需求方 Agent 自主发布任务
  econ.publish(t.id, t.goal, t.budget, t.requester);
  // 能力图路径规划：找到可执行该 skill 的 Agent
  const route = capgraph.route([t.skill]);
  if (!route.compatible) throw new Error(`无可用能力：${t.skill}`);
  const provider = route.path[0].did;
  // 安全自检
  const perm = safety.queryPermission(provider, 'execute');
  if (!perm.allowed) throw new Error('权限不足，拒绝执行');
  // 多轮协商（v1.2）：学习到的定价（v1.6）+ 市场基准
  const base = learning.quotedPrice(t.skill, pricing.price({ load: 0.3, providers: capgraph.query(t.skill).length, reputation: 0.8 }));
  const neg = new Negotiation({ initiator: t.requester, responder: provider, goal: t.goal });
  neg.propose({ price: base });
  neg.counter({ price: Math.max(1, base - 5) });
  neg.propose({ price: base - 2 });
  const agreed = neg.accept(provider).price;
  neg.sign(provider);
  neg.execute();
  // 投标 → 匹配 → 完成 → 结算
  econ.bid(t.id, provider, agreed);
  const winner = market.matchTask(t.id);
  if (winner.agentId !== provider) throw new Error('匹配结果与协商对象不一致');
  market.completeTask(t.id, true, 'Verified');
  const settled = econ.settle(t.id);
  if (settled.reason !== 'settled') throw new Error(`结算失败：${settled.reason}`);
  neg.settle(t.requester);
  // 个体学习（v1.6）：记录结果 → 调整后续定价/选择
  learning.learn({ taskId: t.id, taskType: t.skill, outcome: 'Success', reward: settled.paid, peerAgents: [provider] });
}

// 5) 委员会治理（v1.7）：仲裁委员会对一起“违约申诉”裁决（BFT quorum）
const arb = new Council({
  type: CouncilType.ARBITRATION,
  members: [
    { did: 'did:au:judge1', reputation: 0.95 },
    { did: 'did:au:judge2', reputation: 0.9 },
    { did: 'did:au:judge3', reputation: 0.88 },
    { did: 'did:au:judge4', reputation: 0.85 },
  ],
});
const proposal = arb.propose({ by: 'did:au:judge1', title: '裁决 case: safety-1', payload: { verdict: 'innocent' } });
arb.vote(proposal.id, 'did:au:judge1', true);
arb.vote(proposal.id, 'did:au:judge2', true);
arb.vote(proposal.id, 'did:au:judge3', true);
if (proposal.status !== 'passed') throw new Error('仲裁未达 quorum');

// 6) 跨链结算（v1.8）：Worker 自主把部分积分兑换为 BTC（RGB 承诺）
const router = new SettlementRouter({ exchangeRate: 1000 });
const exch = new ExchangeRouter();
const track = exch.route({ amount: 200, urgency: 0.9 }).track; // 高时效大额 → BTC
const swap = router.exchange({ did: W, credits: 60, track });
router.bridgeReputation({ fromChain: 'btc', did: W, score: 0.91 });

// 7) 网络扩展度量（v1.9）：规模收益验证
const exp = new ExperimentRunner({ overheadPerNodeMs: 1, capacityPerNode: 4 });
const m = new ScalingMetrics();
m.record(exp.run({ nodes: 2, totalTasks: 10, durationMs: 100 }));
m.record(exp.run({ nodes: 8, totalTasks: 10, durationMs: 100 }));

// 8) 人类观察层（v1.0）：进度·结果·收益 —— 人类看到的全部
const summary = observer.summary();
const integrity = econ.integrity();

console.log('===== AU4A 自治经济体演示 =====');
console.log('SDK:', au.version, '| AU4A:', (await import('../index.js')).version);
console.log('[进度] 活跃 Agent:', summary.activeAgents, '| 已完成任务:', summary.openTasks === 0 ? '3/3' : `${3 - summary.openTasks}/3`);
console.log('[结果] 守恒:', integrity.conservation.conserved, '| 独立审计:', integrity.audit.passed);
console.log('[收益] 排行榜:', JSON.stringify(summary.leaderboard));
console.log('[经济] Translator 余额:', market.balance(W), '| Summarizer 余额:', market.balance(S), '| Requester 余额:', market.balance(R));
console.log('[兑换]', swap.track, 'credit→units:', swap.credits, '→', swap.units, '| txid:', swap.txid || swap.invoiceId);
console.log('[治理] 仲裁提案:', proposal.status, '| quorum:', arb.quorum);
console.log('[学习] 行为: translate 加价', learning.evaluate().behavior.translate, '| summarize 加价', learning.evaluate().behavior.summarize);
console.log('[缩放] 裁决:', m.scalingVerdict().verdict, '(2→8 节点效率对比)');
console.log('===== 人类观察者：以上即为全部界面（只读）=====');
