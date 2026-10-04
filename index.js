// AU4A — Agent Universe For Agent（agent-universeForAngent）
// 根本转变：一切为智能体服务；人类用户兼任观察者，只展示进度/结果/收益。
// 底层逻辑重构，参考（不修改）：
//   - https://github.com/TwinsEarth/agent-universe
//   - https://github.com/TwinsEarth/NewAgentUniverseByDeepSeek
import au from '@twinsearth/agent-universe';
import { Identity } from './src/core/identity.js';
import { Wallet } from './src/core/wallet.js';
import { AgentRegistry } from './src/autonomy/registry.js';
import { HumanObserver } from './src/observer/dashboard.js';

export const version = '1.6.0';
export { Identity, Wallet, AgentRegistry, HumanObserver };
export const sdk = au;
export const sdkVersion = au.version;

export { Capability, CapabilityGraph } from './src/capgraph/graph.js';

export { Negotiation, NegotiationState } from './src/negotiate/protocol.js';

export { NodeState, Migrator, takeSnapshot, verifySnapshot } from './src/state/portable.js';

export { PricingStrategy, AgentEconomy, ExchangeRouter } from './src/economy/pricing.js';

export { PermissionPolicy, SafetyAPI, SafetyStatus } from './src/safety/api.js';

export { Experience, ExperienceStore, LearningLoop } from './src/learning/experience.js';
