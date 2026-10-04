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

export const version = '1.0.1';
export { Identity, Wallet, AgentRegistry, HumanObserver };
export const sdk = au;
export const sdkVersion = au.version;
