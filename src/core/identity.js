// v1.0.1 — Agent 自主身份（Identity）
// 一切为智能体服务：Agent 自行生成 Ed25519 密钥对、DID 与 AgentCard，
// 不再依赖人类通过 CLI/API 代注册。人类只读观察。
import au from '@twinsearth/agent-universe';

const { Keypair, AgentCard } = au;

export class Identity {
  /** 自主生成身份：返回 { keypair, did, name } */
  static generate(name = 'unnamed-agent') {
    const keypair = Keypair.generate();
    return { keypair, did: keypair.did, name };
  }

  /** 自主构造 AgentCard（did/name 必填，skills/capabilities 用于能力声明） */
  static card({ did, name, skills = [], capabilities = [] }) {
    const card = AgentCard.new({ did, name });
    if (skills.length) card.skills = skills;
    if (capabilities.length) card.capabilities = capabilities;
    return card;
  }

  /** 签名（供协商/投票/证据使用） */
  static sign(identity, message) {
    return identity.keypair.sign(message);
  }
}
