<!-- 本文件是 AU4A 立项阶段的只读架构调查记录，来源为参考仓库的本地克隆。 -->
<!-- 参考仓库：https://github.com/TwinsEarth/agent-universe -->
<!-- 调查方式：只读浏览（glob/grep/read）+ 只读构建检查；未修改任何参考仓库。 -->

# REF1 — Architecture Brief: TwinsEarth / agent-universe

**Subject:** `E:\DS\TwinsEarth\agent-universe` (vendored snapshot of https://github.com/TwinsEarth/agent-universe) and its host monorepo `E:\DS\TwinsEarth`.
**Purpose:** evidence base for designing a NEW agent-native system that borrows *concepts*, not code.
**Method:** read real code; anything only seen in prose is marked `[documented, unverified in code]`.
**Date of survey:** 2026-10-04. Survey was read-only.

---

## 1. Repo layout

### 1.1 `E:\DS\TwinsEarth\` (fusion monorepo, self-versioned v1.2.3)

| Path | What it is |
|---|---|
| `README.md` (211 L) | Trinity framing: UDOS = "soul", PixelToCivilization = "body", agent-universe = "social network". |
| `VERSION` | `1.2.3` (single line, machine-readable). |
| `CHANGELOG.md` (239 L) | Monorepo changelog; also carries per-subproject changelog sections. |
| `docs/` | `ARCHITECTURE.md`, `INTEGRATION.md`, `PCE-FORMAT.md`, `SUBPROJECTS.md`, `VISION.md`, `ROADMAP.md`. Claims are tagged `[已实现]/[部分]/[设计]` — unusually honest. |
| `spec/` | **Frozen cross-language contract (TEP-0/1/2/3)** + vectors + schemas + 4 language impls + conformance runner. The most rigorous artifact in the repo. |
| `interop/` | Python integration layer (`twinsearth_interop`) bridging the three subsystems; also contains `stubs.py` protocol stubs. |
| `demos/` | `e2e_pixel_to_market.py` (276 L) — offline deterministic end-to-end demo. |
| `cycle_agents/` | 4-layer nested cycle agent framework (climate→population→institution→event), 685 Python lines. |
| `udos-reasoning-engine/` | Python cognitive kernel (v7.5.0 in-repo). Out of scope for this brief. |
| `PixelToCivilization/` | C#/Tuanjie (Unity) 4X civilization sim (V7.0.2 in-repo). Not built in CI. |
| `agent-universe/` | The target project. |

> Note: `E:\DS\TwinsEarth\gsn-core\` **does not exist** (the task brief lists it); the only `gsn-core` is `agent-universe/gsn-core/`.

### 1.2 `agent-universe/` top level

| Path | Purpose | Substance |
|---|---|---|
| `gsn-core/` | Rust core lib + 2 binaries. `Cargo.toml` v**0.2.35**, edition 2021, crate-type `rlib`. | **~15.2k Rust lines / 92 files** (incl. tests+examples). The real system. |
| `js/` | Zero-dependency JS SDK (`index.js` + `lib/{keychain,models,dht,market}.js`) + `test/test.js`. | 824 lines. A **second, independent reimplementation** of the market. |
| `aip-sdk-py/` | Python "AIP" SDK (`aip/models.py`, `dht_backend.py`, `index/sharded.py`). | 243 lines, 6 tests. Data classes + in-memory dicts only. |
| `contracts/` | 4 Solidity files in `src/`. | 219 lines. **No build config, no tests, no deploy scripts, no ABIs.** |
| `desktop/` | Tauri 2 demo shell (`src-tauri/src/lib.rs` = 14 L exposing only `get_sdk_version`). Front end runs the JS market in-process with a hard-coded DID. | Demo, not a client. |
| `client/` | Tauri 2 "five-platform" shell; `src-tauri/src/lib.rs` exposes only `get_platform`/`get_sdk_version` (reports **2.3.5**). | Shell only; `platforms/*.md` are per-OS build notes. |
| `deploy/` | `deploy-mainnet.sh` (macOS/Mac-mini install), `monitor.sh`, `macos/com.gsn.node.plist` (launchd, ports 4001/4002), `pf.conf.append`, `pmset-setup.sh`. | Real ops scripts, macOS-only. |
| `releases/` | 27 per-version markdown notes (v1.0.0 → v2.3.4). | Prose. |
| `docs/` | `v2.3.4-deep-analysis.md` (423 L), `部署与验证报告-2026-09-23.md` (192 L real-machine deploy report). | Prose; the deploy report is a valuable honesty artifact but is **stale** (see §6). |
| `index.js` / `index.d.ts` / `package.json` | Root npm facade: `module.exports = require('./js')`. `package.json` name `@twinsearth/agent-universe`, version `2.3.4`, publishes to GitHub Packages, `files: [index.js, index.d.ts, js]`. | Thin re-export. |

---

## 2. Version / roadmap state

**Five different version numbers describe the same snapshot:**

| Artifact | Version |
|---|---|
| `E:\DS\TwinsEarth\VERSION` (monorepo line) | **1.2.3** |
| `agent-universe/package.json`, `README.md`, `js/index.js:10` | **2.3.4** ("Market" codename) |
| `agent-universe/gsn-core/Cargo.toml:3` | **0.2.35** |
| libp2p Identify protocol string (`net/peer.rs:122`, `net/dual_node.rs:54`) | `"/gsn/0.2.34"` (stale) |
| `client/src-tauri/src/lib.rs:24` | **2.3.5** |
| `aip-sdk-py/aip/__init__.py:7` | 2.3.4 |

**Numbering scheme:** monorepo uses its own linear semver (`v1.0.0 → v1.0.1 → v1.2.3`); each subproject keeps an *independent* semver line. `docs/SUBPROJECTS.md:23-34` states this explicitly and warns that subproject lines are ahead:
- vendored agent-universe = v2.3.4 / gsn-core 0.2.35
- **upstream agent-universe = v3.1.0 / gsn-core 0.3.x**; vendored subset ≈ **25% of upstream files** (`docs/SUBPROJECTS.md:44`)
- upstream PixelToCivilization = V9.2.3 vs vendored V7.0.2

**Latest in-tree release:** `releases/v2.3.4.md`, dated 2026-09-23, codename "Market".
**Codename lineage** (`README.md:215-225`): Genesis → Shard → Bridge → Mesh → P2P → Swarm → MCP+ACA → P2P Net → **Market**.

**Roadmap** (`docs/ROADMAP.md`): V1.1.x "interface-through" and V1.2.x "dual-node interconnect" are delivered; V2.0.0 testnet, V2.x sharding, V3.0 hybrid open network are unchecked. Milestone rule: *"each milestone must have re-runnable tests and metrics; unverified is not marked complete."*

---

## 3. Core architectural concepts — present or absent

`gsn-core/src/lib.rs:9-102` is the authoritative module + re-export list.

### 3.1 Agent identity / DID — **EXISTS (fragmented)**

- `identity/did.rs:9-15` — `Did::from_public_key` = `"did:aip:" + hex(SHA256(pubkey)[..8])`. 16 hex chars, no DID-document resolution, no verification methods, no resolvers.
- `identity/signer.rs` — **real Ed25519** via `ed25519-dalek` 2 (`Keypair::generate/from_seed/public_key/seed`, `Ed25519Signer::sign/verify/verify_with_pubkey`).
- `identity/keyring.rs:5-47` — `trait SecureKeyring { store/load/delete }`; **only implementation is `MemoryKeyring`** (a `Mutex<HashMap>`). Platform keychain is documented in `README.md:71`, not implemented.
- JS uses a **different namespace**: `js/lib/keychain.js:26` → `did:au:<32 hex>`. Python SDK has no DID derivation at all.
- **Three DID prefixes / derivations across three languages** — no single identity contract.

### 3.2 Agent cards / skills — **EXISTS, three overlapping types**

| Type | Location | Fields |
|---|---|---|
| `AgentCard` | `agent/card.rs:5-12` | did, name, capabilities, endpoints, reputation_bps(u16), version |
| `MarketAgentCard` | `marketplace/agent_card.rs:61-100` | 19 fields: agent_id, version, name, description, skills, modalities, models, endpoint, `Pricing`, `Sla`, owner, stake, reputation_score, total_calls, success_rate, `EvidenceGrade`, verified, created_at, updated_at |
| `AgentManifest` (ACA) | `aca/manifest.rs:51-73` | did, name, version, capabilities, endpoints, `HardwareProfile`, `verification_modes`, stake, reputation_ref, model_hash, mcp_tool_count, signature, timestamp |
| `SkillManifest` | `marketplace/agent_card.rs:104-123` | skill_id, name, description, input/output `SchemaField`s, tool_permissions, timeout_ms, estimated_cost, evidence_grade |
| `AgentNode` (swarm) | `swarm/collective.rs:12-20` | did + card + stake + reputation(u16) + online + tasks_completed + success_rate |

Public matching helpers: `AgentMarket::discover_by_skill`, `::search_agents` (`marketplace/mod.rs:175-197`); `GsnNode::discover_by_capability` (`net/libp2p_node.rs:33`, in-memory only); `Swarm::select_by_capability` (`swarm/collective.rs:132`).

### 3.3 Marketplace / matching — **EXISTS, in-process library only**

`marketplace/mod.rs:61-518` — `AgentMarket` with `HashMap` state. Lifecycle `register_agent → publish_task → submit_bid → match_task → submit_result → verify_result → settle_task`, plus `open_dispute` / `arbitrate`.

- **Admission:** stake ≥ `min_stake` (default 100.0) (`mod.rs:84,123`).
- **TaskSpec six-field validation** (`marketplace/task.rs:113-140`): goal, context, todo, budget>0, required_skills, requester non-empty. Note `done`, `trace`, `owner` are *declared* in the six-field doc comment but **not all validated**.
- **Task state machine** (`task.rs:11-63`): Draft, Open, Matched, Running, Verifying, Accepted, Settled, Rework, Disputed, Arbitration, Slashed, NoQuorum. `is_terminal()` only Settled|Slashed.
- **Match score** (`mod.rs:250-258`): `(reputation.overall() / price) × 1/(1 + latency_ms/1000)`.
- **Crucially not wired to anything:** grep shows `AgentMarket` is referenced only in `marketplace/*`, `lib.rs`, `tests/v234_test.rs`, `examples/market_demo.rs`. **`gsn-daemon.rs` has zero market endpoints** (routes are only `/`, `/health`, `/version`, `/agents`, `/tasks`, `/peers`, `POST /agents`, `OPTIONS` — `bin/gsn-daemon.rs:236-334`). The v2.3.4 headline feature is unreachable over the network.

### 3.4 Settlement / payments — **PARTIAL; RGB/Taproot/x402/ERC-8004 ABSENT**

- **No match anywhere** for `x402`, `ERC-8004`/`8004`, `Taproot`, or Bitcoin `RGB` in `agent-universe/**` or `docs/**`. (Repo-wide hits for "RGB" are all image RGB in UDOS docs.) These concepts are **not in this project**.
- Ledger is an in-memory `f64` HashMap: `marketplace/settlement.rs:45-56`. Invariant `balance_sum == total_budget − total_slashed` enforced in `conservation_check()` (`settlement.rs:161-183`), tolerance 0.001. Duplicate-payment short-circuit via `settled_tasks: HashSet` (`:90-100`). `SettlementReason` = Completed | AlreadyPaid | DuplicateWork | Rejected.
- `chain/mod.rs` (3 lines) exports only `pocv`. **`chain/pocv.rs:33-37` "verify_proof" is just SHA-256(input)==stored and SHA-256(output)==stored** — no execution trace, no challenge, no EVM light client (despite `docs/ARCHITECTURE.md:88` claiming one).
- **Solidity is decorative:** `GovernorToken.sol` (AGU ERC-20-ish, 1e9 supply to deployer, `delegateVotes` reads an uninitialized `votes` map), `AgentCardAnchor.sol` (CID hash anchor), `PoCVSettlement.sol` (escrow/verify/dispute/settle), `ReputationBridge.sol` (verifier-gated snapshots). CI only greps for `pragma`/`contract` (`.github/workflows/ci.yml:93-104`) — **never compiled, never tested, no hardhat/foundry config**.
- Cross-node settlement exists as *simulation* of two libp2p nodes: `net/dual_node.rs:24` uses `marketplace::ConservationReport`; `CrossNodeSettlement`/`ReputationSync`/`try_launder` (documented at `dual_node.rs:10-22`).

### 3.5 P2P networking — **SPLIT: one real stack, several convincing fakes**

**Real (libp2p 0.54, `gsn-core/Cargo.toml:39-50`: tokio, tcp, noise, yamux, kad, gossipsub, identify, dns, request-response, macros):**
- `net/peer.rs:63-252` — `P2pPeer` wrapping `Swarm<PeerBehaviour{kademlia, gossipsub, identify}>`. Public API: `new/with_identity/with_identity_and_idle_timeout`, `listen_on_port`, `add_bootstrap`, `dht_put`, `dht_get`, `start_providing`, `subscribe`, `publish`, `find_peer`, `next_event`, `connected_peers`, `routing_table_size`, `set_dht_server_mode`, `set_dht_auto_mode`, `add_external_address`. Two documented hard-won fixes: `DEFAULT_IDLE_CONNECTION_TIMEOUT=300s` (`:60`) and forcing `kad::Mode::Server` (`:107`) — both excellent "read this before you build P2P" notes.
- `net/dht.rs:1-45` — **real local Kademlia**: 160-bit keyspace (`ID_BITS=160`, `NodeId=SHA256(x)[..20]`), XOR `Distance`, `KBucket` k=20, split-on-self / LRU-evict-otherwise, `closest_peers`. Module doc explicitly states: **no network I/O**.
- `net/dual_node.rs` (~1520 L) — `NodeSession`/`DualNodeHarness` driving **two real libp2p nodes in one runtime** (`tokio::select!` over both swarms, `:8-9` documents the classic single-poll deadlock). `bin/gsn-dual-node.rs` is the cross-host CLI (`--listen`/`--dial`).
- `net/root_seed.rs` (~589 L) — `SeedStateMachine` Root→Bootstrap→Normal with hysteresis, cooldown, bootstrap dwell, demotion combos; keeps legacy `RootSeedConfig` bit-identical.
- `nat/stun.rs` (~40 KB) — RFC 5389 Binding client over real tokio UDP; TLV attributes, XOR-MAPPED-ADDRESS, typed errors, bounded retry. `nat/holepunch.rs` (~48 KB) — real `RendezvousServer` + `PunchCoordinator` on `tokio::net::UdpSocket` (`bind`/`send_to`/`recv_from` at `:299,:475,:507`). `nat/detect.rs` — RFC 3489-style classification that **reports Unknown rather than guessing**. `nat/stats.rs` — success-rate / RTT percentiles.

**Fakes with real-sounding names (danger zone):**
- `net/libp2p_node.rs` — file named "libp2p node initialization", header says "(cross-platform lightweight version)", body is `GsnNode { peer_id: String, listen_addr: String, cards: HashMap<String,AgentCard> }`. **No libp2p.**
- `net/gossip.rs` — named GossipSub; body is `HashSet<String>` topics + `HashMap<String, Vec<Vec<u8>>>` messages. **No broadcast.**
- `storage/sqlite.rs` — named "local storage (SQLite)"; body is `HashMap<String, Vec<u8>>`.
- **All three are re-exported from the public root** (`lib.rs:35` `pub use net::{GsnNode, ..., GossipSub, ...}`), while the real types (`P2pPeer`, `RoutingTable`) are exported separately at `lib.rs:83-91`. A naive consumer taking the first-listed API gets the in-memory fakes.
- `nat/mock.rs` is honest and correct: a **mock STUN cluster over 4 real UDP sockets** with RFC 4787-style mapping/filtering profiles, used for tests. `nat/mod.rs:16-23` warns that legacy `NatTraversalManager` is a "v2.3.x synchronous placeholder API" kept for back-compat.

### 3.6 Plugin / message bus (PMB) — **ABSENT**

- **Zero occurrences of `plugin`/`Plugin`** anywhere under `agent-universe/`. There is no plugin host, no extension registry, no lifecycle/hook system. (Upstream v3.x is described as "插件化" in `docs/SUBPROJECTS.md:84` — `[documented, unverified in code]`, not in this snapshot.)
- Closest substitutes: `mcp/` tool/resource/prompt registries (`mcp/server.rs`, in-process, `handle_tools_call` returns a **placeholder** string, `:109-110`), and two hard-coded GossipSub topics `gsn/agents`, `gsn/tasks` (`bin/gsn-daemon.rs:397-398`) that are subscribed but **never consumed** by any handler.
- Transport is a hand-written HTTP/1.1 over `tokio::net::TcpListener` (`bin/gsn-daemon.rs:173-344`); actor pattern via `mpsc` commands (`PeerCommand` enum, `:24-33`) with three variants marked `#[allow(dead_code)] // reserved`.

### 3.7 Policy engine (OPA/Rego) — **ABSENT**

- No `OPA`, no `Rego`, no policy DSL, no policy files. Nothing embedded.
- Closest analogues, both plain Rust enums:
  - `marketplace/task.rs:67-74` `VerificationPolicy { BftLite{n,f}, Sampling{ratio}, None }` — a *declaration* only; `verify_result` ignores it and just tallies a caller-supplied `QaCommittee`.
  - `aca/verification.rs:78-110` `VerificationPolicy::choose_level()` — economic choice `C_verify ≤ V_task × P_fraud` with hard-coded thresholds (<1, <10, <100, <1000) and cost multipliers (0.1, 2.0, 1.15, 500, 100). This is the seed of a real policy idea, implemented as an if-chain.
  - `mode/mod.rs:23-35` `NodeMode::{Archive,Full,Light,Edge,Browser}.requires_dht_server()/max_storage_gb()` — a capability policy table. **Parsed by the daemon and then only printed** (`bin/gsn-daemon.rs:353-360,394`); never enforced.

### 3.8 Swarm / emergence — **EXISTS as statistical heuristics**

- `swarm/emergence.rs:52-101` `EmergenceDetector::detect()` — compares mean throughput and mean latency of the recent window vs the previous window against `emergence_threshold`; emits `EmergenceSignal{Collaboration|LoadBalancing|FaultTolerance|Evolution}`. `FaultTolerance`/`Evolution` are declared but **never emitted**.
- `swarm/consensus.rs:36-108` `LightweightConsensus` — stake-weighted proposal/vote/tally; `quorum = total_stake × quorum_ratio`; `accepted = reached && approve > reject`.
- `swarm/collective.rs:55-62` `AgentNode::score() = rep/10000 × success_rate × (1 + 0.1·ln(stake)/10)`; `Swarm::{join,leave,online_nodes,select_by_capability,record_task_completion}`.
- The "network Scaling Law" formula `C ≈ N_eff^α · D_collab^β · …` and the five falsifiability conditions exist **only in docs** (`docs/v2.3.4-deep-analysis.md:154-186`, `docs/INTEGRATION.md:115-127`) — `[documented, unverified in code]`, no instrumentation for `N_eff`/`D_collab`/`G_trust` exists.

### 3.9 Economy / reputation — **EXISTS, duplicated across languages**

- **Two parallel Rust reputation systems:**
  - `economy/reputation.rs:18-111` — `ReputationSystem`, single `u16` score 0..10000 (init 5000), `record_success` (+`value×0.01`), `record_failure(−penalty)`, `apply_decay()` halving every `decay_halflife_secs`.
  - `marketplace/reputation.rs:11-85` — `MarketReputation`, 4-dim f64: quality/speed/honesty/availability, EMA `α=0.1`; `overall() = 0.35q + 0.20s + 0.30h + 0.15a` (used by matching).
- `ReputationManager` (`marketplace/reputation.rs:117-231`): `register_stake` (min_stake), `slash_stake` (−honesty 0.3), `leaderboard`, `is_eligible(agent_id, min_reputation)`. `StakeStatus` = Locked | Withdrawing | Withdrawn | Slashed. **"Non-transferable" is enforced by absence of any transfer method**, not by a mechanism.
- `proof/poc.rs:20-100` — `ProofOfContribution`: SHA-256 over `(did, task_id, type, value_le, ts_le)`, N verifier confirmations. Self-reported, unauthenticated; `verify_contribution` is a counter increment.
- `economy/pricing.rs` — `TaskPricing` = base × difficulty(0.5..8) × urgency(0.8..2.5) × supply-demand(1/ratio).
- `economy/contribution.rs` — contribution types/weights.
- **Third implementation:** `js/lib/market.js` (240 L) reimplements register/publish/bid/match/settle/slash/conservation with a *different* staking model (locked account `__stake__:<id>` instead of a stake record) and a *different* conservation tolerance (1e-9 vs 0.001). **Fourth:** `interop/python/twinsearth_interop/market_bridge.py` `ReferenceMarket`.
- `marketplace/settlement.rs` docs claim `balance_sum = total_paid − total_slashed` (header, `:3-5`) while the implemented invariant is `total_budget − total_slashed` (`:159`) — internal doc/code mismatch.

### 3.10 AUSec elastic compute — **ABSENT**

- No `AUSec`. Closest, all toy-heuristic:
  - `inference/mod.rs:54-103` `ComputeScheduler` — registers `ComputeResource{node_id,gpu_model,vram_gb,cpus,ram_mb,bandwidth_mbps,available,kv_cache_shards}`, `assign_task` = **first node with `vram_gb ≥ required`** (HashMap iteration order).
  - `crowdsource/mod.rs` `CrowdsourcingMarket` — publish/accept/complete, points credited on completion, no verification.
  - `collaboration/mod.rs` — `AgentTier::{McpAgent,RouteAgent,EndAgent}` + `CollaborationGroup`/`CollaborationMessage`, all in-memory; `#[allow(dead_code)]`.
  - `scheduler/router.rs` + `load_balancer.rs` — `1.0 − 0.3·load_ratio − 0.2·min(latency/1000,1)` selection; round-robin/least-conn/least-response-time.
- No GPU/driver integration, no container/WASM sandbox, no metering. `interop` labels its executor `simulated` and writes `sandbox: in-process-simulation` into the result envelope (`interop/README.md:114,133-134`).

### 3.11 Agent security organization — **ABSENT**

- No security org, no council, no human-in-the-loop body, no key revocation, no attestation chain.
- `security/mod.rs:29-86` `SecurityEngine` — the whole of it: `report_behavior(node, SecurityFlag)` multiplies score by 0.8, bans under 0.3; flags = SybilSuspected | EclipseAttack | PollutionDetected | OfflineTooLong | InvalidSignature | ReputationManipulation; `random_neighbors()` to resist eclipse. **Nothing calls `report_behavior`** — it is a library with no integration.
- `docs/CHANGELOG.md:112` states plainly: `ReputationSync` **does not defend against Sybil** (nodes self-report and supply self-consistent evidence); staking + challenge-response is out of scope.
- Verifier is a stub: `verifier/client.rs:28-35` `verify()` unconditionally returns `{valid:true, score:0.95, reason:"verification passed"}`.

### 3.12 Genuinely differentiated concepts (worth borrowing)

- **Evidence grading**: `marketplace/evidence.rs` — `EvidenceGrade::{Verified, CpuProto, Unverified}` with `is_trustworthy()` as a *settlement gate*. Applied consistently across `MarketAgentCard`, `ResultEnvelope`, `SkillManifest`, `AgentManifest`, spec vectors, and the interop evidence table. Rare and valuable.
- **Frozen cross-language byte contract** (TEP-0/1/2/3) — see §5.
- **Trace hash chain / TraceBus** with tamper detection and two-node `merge()`.
- **Seed state machine with hysteresis** — avoids real network churn from threshold flapping.
- **Accounting invariant as a first-class, testable predicate** (`conservation_check()`, `assert_conserved()`).
- **TaskSpec six-field responsibility-transfer check** (Goal/Context/Done/Todo/Trace/Owner) as an admission gate.
- **Written, honest boundary statements** (`[RESULT NEEDED]`, `[interop-extra]`, "this is not a real NAT traversal rate").

---

## 4. Language / stack breakdown

| Language | Where | Files | Lines |
|---|---|---|---|
| **Rust** | `agent-universe/**` (gsn-core src+tests+examples + 2 Tauri shells) | 92 | **15,185** |
| **Rust** | `spec/impl/rust/twinsearth-spec` | 7 | 1,563 |
| **JavaScript** | `agent-universe/js/**` + root `index.js` | 11 | 824 |
| **JavaScript** | `spec/impl/js/twinsearth-spec` | 5 | 1,357 |
| **TypeScript** | `index.d.ts`, `js/index.d.ts` (declarations only) | 2 | 101 |
| **Python** | `interop/python/**` (interop layer + tests) | 18 | **6,173** |
| **Python** | `spec/impl/python` | 12 | 2,084 |
| **Python** | `cycle_agents/**` | 8 | 685 |
| **Python** | `demos/` | 1 | 276 |
| **Python** | `agent-universe/aip-sdk-py/**` | 8 | 243 |
| **Solidity** | `agent-universe/contracts/src` | 4 | 219 |
| **C#** | `spec/impl/csharp` (never compiled) | 4 | 1,068 |
| **C#** | `PixelToCivilization/**` (Unity, not built in CI) | many | not counted |

**Build systems:** Cargo (gsn-core + standalone spec crate); npm (root facade, `js/`, `desktop/`, `client/` via Vite + Tauri 2); pip/`pyproject.toml` (`aip-sdk-py`); pytest for `spec/impl/python`, `interop/python`, `cycle_agents`, `demos`; **no build system for Solidity**.

**Test layout & execution:**

| Suite | Location | Count | Run |
|---|---|---|---|
| Rust integration | `gsn-core/tests/{audit,integration,lib,v120_dual_node,v120_nat,v231,v232,v233,v234}_test.rs` | 148 `#[test]`/`#[tokio::test]` | `cd gsn-core && cargo test` |
| Rust inline unit | `#[cfg(test)] mod tests` in `net/{root_seed,dual_node,dht}.rs`, `nat/{stun,holepunch,detect,mod,stats}.rs` | 101 | same |
| — total | | **251 passed / 0 failed / 2 ignored** (`CHANGELOG.md:88`); baseline at v2.3.4 was 144 (`releases/v2.3.4.md:114`) | |
| JS | `js/test/test.js` | 8 hand-rolled assertions | `node js/test/test.js` |
| Python AIP | `aip-sdk-py/tests/test_sdk.py` | 6 | `cd aip-sdk-py && PYTHONPATH=. pytest tests/ -v` |
| Python interop | `interop/python/tests/*.py` | 146 | `pytest interop/python/tests -q` |
| Python cycle agents | `cycle_agents/tests/test_cycles.py` | — | `pytest cycle_agents/tests -q` |
| Spec | `spec/impl/*/test*` + `spec/conformance/run_conformance.py` | Py 31 / JS 42 / Rust 26, 69 vectors × 3 langs | `python spec/conformance/run_conformance.py --langs python,js,rust` |
| Solidity | — | **0** | CI greps `pragma` only |
| HTTP API / daemon | — | **0** | only `gsn-daemon --help` is exercised (`tests/v232_test.rs:415-423`) |

CI (`.github/workflows/ci.yml`) = 4 jobs here (rust ubuntu+macos matrix, python SDK, js, contracts-grep); `CHANGELOG.md:95` claims the monorepo CI is 11 jobs with a release gate. Clippy failures are explicitly tolerated (`ci.yml:45`).

---

## 5. Conformance / spec / wire formats

`spec/` is the strongest engineering in the repo. Normative source is **Python**; JS/Rust/C# are byte-exact ports enforced by `spec/conformance/run_conformance.py`.

**TEP-1 — deterministic JSON canonicalization** (`spec/README.md:13-97`, impl `spec/impl/python/twinsearth_spec/canonical.py`)
- Object keys ascending **UTF-8 byte order** (not UTF-16 code-unit order — the JS `Array.sort()` / C# `CompareOrdinal` trap is documented at `:32-35`).
- No whitespace. Minimal escaping; only `< U+0020` escaped, `\u00xx` lowercase; everything else raw UTF-8.
- Numbers: bool checked **before** number; integers (incl. integral floats) printed without `.0`, error beyond `2^53−1`; non-integers take the **shortest round-trip decimal string as exact input**, then **half-to-even** round to 6 decimals, strip trailing zeros; `-0.0 → 0`; NaN/Inf → error; **never exponential notation**.
- Intermediates must stay in string/BigInteger (workaround documented for `16280251454.632555`).
- 6 decimals is a deliberate cross-domain precision choice (`docs/PCE-FORMAT.md:90`).

**TEP-2 — CID** (`spec/README.md:100-118`)
`digest = SHA-256(TEP-1 bytes)`; `cid_bytes = 0x01 || 0x71 || 0x12 || 0x20 || digest`; `cid = "b" + base32_lower_nopad(cid_bytes)`. 59 chars, `bafyrei…`, isomorphic to IPFS dag-json CIDv1.

**TEP-3 — Trace hash chain** (`schema/trace-chain.schema.json`)
`frame = {index, payload, prev_hash, source, ts}`; `entry.hash = SHA-256(canonical(frame))` (64 lowercase hex); `entry.cid = CID(frame)`; genesis `prev_hash` = 64 zeros; `additionalProperties:false` on frame/entry. `prev_hash` is *inside* the hashed frame to remove concatenation ambiguity. Verification demands contiguous indices, correct genesis, consistent links, recomputable hash+cid — **"no partial trust"**; empty chain is valid.

**TEP-0 — PCE physical token** (`schema/pce-format.schema.json`)
Required: `pce_version` (const `"1.2.3"`), `pce_id` (`^bafyrei[a-z2-7]{52}$`), `tick`, `source` ∈ {`udos-reasoning-engine`, `pixel-to-civilization`, `agent-universe`}, `ts`, `observations`, `scene`, `certainty` ∈ [0,1], `evidence_grade` ∈ {`verified`,`cpu-proto`,`unverified`}. Optional `prediction`, `trace{chain,hash,cid,index}` — **optional fields participate in `pce_id`**. `additionalProperties:true` so subsystems may add private fields, which are therefore *not* silently ignored.

**Interfaces defined:** `canonical_json/canonical_bytes/cid_of/sha256_hex`, `TraceChain/TraceEntry/GENESIS_PREV`, `make_pce/pce_id/validate_pce` — mirrored as `canonicalJson/…`, `canonical_json(&v)`, `Canonical.CanonicalJson(node)` (`spec/README.md:187-197`).
**CLI contract** for the conformance runner: stdin JSON array → stdout result array, per-mode field sets for `canonical`/`trace`/`pce` (`spec/README.md:199-214`).
**Vectors:** `spec/vectors/{canonical,cid,trace,pce}_vectors.json` + `.raw.json` (raw literals preserving `-0.0`, `2^53−1`), generated — never hand-written — by `generate_vectors.py`; 31/12/9/15 cases; 10 mandatory ids; negative and fuzz verifiers present. Runner re-derives expectations from the Python reference to prevent self-confirming vectors, and exits non-zero on any divergence.

**Agent-universe does not consume this spec.** There is no PCE/CID/Trace code in `gsn-core`; the PCE↔AgentCard link lives only in `interop/python/twinsearth_interop/` and in docs.

---

## 6. Gaps, stubs and debt

**Stubbed or fake (code is a placeholder, name implies otherwise):**
1. `verifier/client.rs:28-35` — always `valid:true, score:0.95`.
2. `mcp/server.rs:105-115` — `tools/call` returns `"tool '<name>' executed"`; no execution.
3. `net/libp2p_node.rs` — `GsnNode` is a `HashMap` of cards; no libp2p.
4. `net/gossip.rs` — `GossipSub` is two collections; no gossip.
5. `storage/sqlite.rs` — `LocalStorage` is a `HashMap`; the real SQLite is `storage/persist.rs` (`PersistentStore`).
6. `ffi/uniffi.rs`, `ffi/tauri.rs` — no `uniffi` dependency, no `#[uniffi::export]`, no `#[tauri::command]`; only wrap the fake `GsnNode`.
7. `erasure/mod.rs` — **not Reed-Solomon.** Parity shards are `SHA-256(index||data)||len`; `decode` (`:87-114`) can only reassemble *data* shards and requires `data_shards` of them, so parity is useless for recovery. No finite-field arithmetic anywhere. The v2.3.1 changelog claim "真正的 Reed-Solomon 解码" is false.
8. `crdt/mod.rs` — only a `VersionVector` (`increment/get/merge`); no CRDT data types.
9. `chain/pocv.rs` — hash equality, not verifiable computation.
10. `ghost` docs: `docs/v2.3.4-deep-analysis.md:67-86` describes `ListedAgent`/`AgentMarketplace`/`AgentReview`/`CallRecord` structs that **do not exist**; `releases/v2.3.4.md:71-110` diagrams Postgres+Redis+MinIO+NATS/gVisor/Firecracker/OpenTelemetry that **do not exist**.

**Unwired subsystems (exist but unreachable):**
11. `AgentMarket` is not connected to the daemon, DHT, SQLite, or P2P; no market HTTP route. The market's 61 tests and the demo example run entirely in-process.
12. `NodeMode` parsed then discarded (`bin/gsn-daemon.rs:353-360`); `SecurityEngine` never invoked; `NatTraversalManager` is a legacy placeholder; `PeerCommand::{Subscribe,Publish}` are `#[allow(dead_code)]`.
13. GossipSub topics `gsn/agents`/`gsn/tasks` are subscribed but no handler consumes inbound messages; `DhtPut` errors are printed with `eprintln!`, never surfaced.
14. Market/Swarm/Reputation/Stake state is **not persisted**. SQLite holds only `agents(agent_id,name,skills,stake,reputation,created_at)` and `tasks(task_id,goal,state,owner,budget,created_at)` — i.e. 5 of the 19 `MarketAgentCard` fields and 5 of 12 `TaskSpec` fields.
15. Contracts have no compiler in CI, no tests, no deploy script, no bridge from Rust; `AgentCardAnchor` is never called (DIDs are not hashed to `bytes32` anywhere).

**Consistency / quality debt:**
16. **Four market implementations** of one invariant set: Rust `marketplace/`, JS `js/lib/market.js`, Python `interop/.../market_bridge.py`, and (structurally) `tests/v234_test.rs` fixtures. Different staking models and tolerances; `interop/README.md:37` admits one upstream bug is *deliberately* mirrored line-for-line.
17. **Three card types, three DID schemes, two reputation systems, two verification-policy types** in one codebase.
18. Version drift across 6 artifacts (§2); stale Identify string `/gsn/0.2.34` vs crate `0.2.35`; stale `naming` client 2.3.5.
19. `aip-sdk-py` contains **both** `aip/models.py` (64 L) and the package `aip/models/` (`__init__.py` = `from .models import *`, `models/models.py` = 84 L). Python resolves the package first, so **`aip/models.py` is dead code** that duplicates (and is a subset of) `aip/models/models.py`.
20. `gsn-core` claims a `Cargo.lock` + built `target/` (Windows binaries `gsn-daemon.exe`, `gsn-dual-node.exe` present) — so the Rust side has been compiled in this working copy; nothing else has build artifacts.
21. `README.md:170` claims "144 Rust tests + 8 JS tests" (stale; `CHANGELOG.md:88` records 251); `README.md:114` claims 1892+ UDOS tests.
22. No daemon/HTTP integration test; no end-to-end test that starts a node and drives the market over the network.
23. `docs/部署与验证报告-2026-09-23.md:15,102` states the daemon is a skeleton that does not bind ports and that the repo has no JS at all — **stale** relative to this snapshot (the daemon now binds libp2p + HTTP + SQLite, and `js/` exists). Treat that report as historical, not current.
24. `marketplace/settlement.rs:1-5` header invariant contradicts the implemented one at `:159`.
25. Deep-dive doc's theoretical core (`N_eff`, `D_collab`, `G_trust`, five falsifiability conditions, six cold-start stages) has **zero code instrumentation** — no metric is collected anywhere.

**Honest boundaries the project itself declares (trust these):** C# never compiled (`spec/README.md:325`); the 0.8000 hole-punch rate is loopback-with-silent-peers, *not* a real NAT traversal rate (`CHANGELOG.md:100-108`); no cross-host test was possible; hole-punching has no message authentication and no TURN fallback; `ReputationSync` is not Sybil-resistant; the interop world is an 8×8 toy and its sandbox is `in-process-simulation`.

---

## 7. What to reuse as a concept, and what is human-centric and must be inverted

### 7.1 Reuse as concept (no code)

1. **Frozen, byte-exact wire contract with a normative reference implementation.** TEP-1 canonical JSON (UTF-8 key order, half-to-even at fixed precision, no exponentials) + TEP-2 CIDv1/dag-json + TEP-3 hash chain. The *pattern* — one language is normative, others are ports, a conformance runner re-derives expectations and fails the build on divergence, no hand-written vectors, no C# claim until it runs — is directly transplantable and is the single most valuable artifact here.
2. **Evidence grading as a settlement gate.** `verified | cpu-proto | unverified` attached to every card, skill, result and metric, enforced by `is_trustworthy()`. For an agent-native system this is how you stop agents from asserting unverified results as fact — and how you stop yourself from shipping marketing as metrics. Extend it: make the grade a machine-checkable attestation, not a label.
3. **TaskSpec six-field responsibility transfer** (goal / context / done / todo / trace / owner) with an admission gate that *rejects* incomplete tasks. Good, cheap, and directly agent-relevant: it is the minimum handoff contract.
4. **Accounting invariant as an executable predicate.** `balance_sum == total_deposits − total_slashed`, checked after every mutation and used as a CI gate. Any agent economy needs this.
5. **BFT-lite QA committee with explicit equivocation and silence handling** (`n ≥ 3f+1`, `q = 2f+1`, equivocation voids the round, silence > f → NO_QUORUM + view change). Small enough to implement, and the failure modes are named rather than hidden.
6. **Stake-gated admission with slashing that moves value out of the system** — and the explicit design *rule* that reputation is non-transferable ("capital can buy compute, not reputation or governance").
7. **Trace bus as the universal event log**, with a single chain shared by all producers, JSONL persistence, `verify()`, and two-node `merge()`. For agent-native observability this is the right backbone (and it is exactly what humans should observe).
8. **Economic verification-level selection**: `C_verify(level) ≤ V_task × P_fraud`, with declared cost multiplier and finality per level (L0 sample → L1 2-of-3 → L2 TEE → L3 zkML → L4 committee). The *decision procedure* is reusable even though the implementation is an if-chain.
9. **Seed state machine with hysteresis/cooldown/dwell.** Any emergent agent network needs bootstrap-node degradation that does not flap.
10. **Real-Kademlia-shape local routing table** (160-bit, XOR distance, k=20, split-on-self / LRU-otherwise) as a deterministic, unit-testable component decoupled from I/O — plus the two libp2p gotchas (`idle_connection_timeout=0` kills connections; kad `Mode::Client` denies all inbound DHT subflows).
11. **Two-node harness that polls both swarms in one `select!`** as the standard pattern for P2P integration tests, plus the `gns-dual-node --listen/--dial` CLI shape for cross-host verification.
12. **Honest-boundary discipline as a first-class artifact**: `[RESULT NEEDED]`, explicit "this is not what you think it measures" notes, a per-capability evidence table. Institutionalize it.
13. **Interop layer that mirrors another language's semantics line-by-line and marks its own deltas** (`[interop-extra]`) instead of silently "improving" them.
14. **Actor-per-swarm + command channel** (`mpsc<PeerCommand>` + `oneshot` replies) to avoid sharing a `Swarm` behind a mutex. Good concurrency shape for any node runtime.

### 7.2 Avoid / must be inverted (human-centric or actively misleading)

| # | What is there | Why it is human-centric | The agent-native inversion |
|---|---|---|---|
| 1 | Daemon HTTP API is read-mostly for humans (`GET /health,/version,/peers,/agents,/tasks`, `POST /agents`), and the market has **no** endpoints | Designed for a person with a browser; agent capabilities are invisible to other agents | Make the *agent-facing* surface primary: discovery, capability negotiation, bid/offer, result submission, dispute — over a machine protocol (JSON-RPC/MCP/A2A), with HTTP+JSON only as a human observer mirror |
| 2 | Market is an in-process `HashMap` library | A market that only exists inside one process cannot be agent-native; agents cannot find each other | Market state must live on the shared substrate (DHT/CRDT/append-only log), with the local object as a cache |
| 3 | Reputation/economy **not persisted**; SQLite keeps 5 fields of a 19-field card | Fine for demos, fatal for autonomous agents that must remember counterparties across restarts | Persist the full agent record + settlement ledger + trace; make the ledger append-only and replayable |
| 4 | `NodeMode` parsed and printed but not enforced; `SecurityEngine` never called | Human-facing status labels | Modes must alter real behaviour (DHT server, storage quota, relay duty, allowed skills); safety engines must be on the critical path |
| 5 | Two "human-observer" UIs (Tauri `desktop/`, `client/`) that run the market **in-process with a hard-coded DID** | These are demos of a market, not participants in one | Humans get a *read-only projection* of the trace/ledger/market (progress, results, revenue); no human input path into agent decisions |
| 6 | `GsnNode` / `GossipSub` / `LocalStorage` fakes exported from the crate root alongside the real types | A human reading the API index picks the friendly name and silently gets a mock | Delete the fakes from public API or rename them (`InMemoryCardIndex`, `LocalTopicLog`); ship one obviously-real type per capability |
| 7 | One-shot `--help` as the only daemon test; no process-level integration test | "It starts" satisfied a human reviewer | CI must boot a node, connect two, register an agent, publish a task, settle it on the ledger, and verify the trace chain — all headless |
| 8 | Docs assert capabilities that are absent (`ListedAgent`, Postgres/Redis/MinIO/NATS, EVM light client, "true Reed-Solomon") | Written to persuade a human reader | Generate capability docs from the evidence table in CI; a claim without a test ID should fail the build |
| 9 | Version drift across six artifacts; upstream is at v3.1.0 while the vendored copy is v2.3.4 | Humans reconcile versions by reading prose | Single machine-readable version source, propagated at build time; agents must be able to detect capability skew from the wire protocol handshake |
| 10 | Contracts unbuilt/untested and never called; settlement numbers are `f64` | Human-facing tokenomics narrative | Either drop the chain or make it the actual settlement authority with compiled+tested contracts and an integer/exact-decimal money type (see TEP-1's own lesson: never let money touch a float) |
| 11 | Reputation "non-transferable" by *omission of a transfer method*; Sybil-undefended (self-admitted) | Assumes honest-human-like actors | Make non-transferability *structural* (reputation keyed to a proven-work history, not to a mutable balance) and add staking + challenge-response before any real value flows |
| 12 | Verification policy declared but ignored (`verify_result` takes a caller-supplied committee) | The human decides who verifies | Verification level must be *derived* from task value × fraud probability and executed by selected, staked agents — no caller-supplied committee |
| 13 | "Emergence detection" is throughput/latency window comparison; `FaultTolerance`/`Evolution` never emitted | A dashboard metaphor for humans | Emit only signals an agent can act on, with an action attached; or drop the word "emergence" until there is a measurable, falsifiable metric (`N_eff`, `D_collab`, `G_trust` — currently zero instrumentation) |
| 14 | Design premise: "let ordinary people participate, contribute and share revenue"; humans are the intended nodes | The whole monorepo is organized around human onboarding (Mac mini in a living room, desktop clients, mobile shells) | Invert the actor model: agents are the default principals (they hold keys, stake, bid, execute, dispute, and pay each other); humans are *stakeholders and observers* who set budget/policy and watch progress/results/revenue |
| 15 | Documentation language and comments are Chinese-first, code identifiers mixed | Onboarding is aimed at a human community | Keep prose for humans, but make all wire fields, enums and error codes a stable, documented, language-neutral contract |

### 7.3 One-paragraph design takeaway

The reusable spine is: **canonical bytes → content-addressed identity → append-only trace → capability card → task contract → staked matching → graded evidence → conservation-checked settlement → reputation that cannot be bought**, with agents as the only actors that write to any of it. The traffic to avoid is treating a library-shaped, in-process, human-watched simulation as a network: here the market is real code with real invariants but zero reachability, persistence, or counterparty independence; the fake-named P2P types sit in the public API while the real ones sit beside them; and the strongest evidence in the repo (the frozen spec) is not yet connected to the agent protocol at all.
