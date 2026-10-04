<!-- 本文件是 AU4A 立项阶段的只读架构调查记录，来源为参考仓库的本地克隆。 -->
<!-- 参考仓库：https://github.com/TwinsEarth/NewAgentUniverseByDeepSeek -->
<!-- 调查方式：只读浏览（glob/grep/read）+ 只读构建检查；未修改任何参考仓库。 -->

# REF2 — Architecture Brief: `TwinsEarth/NewAgentUniverseByDeepSeek`

**Surveyed tree:** `E:\DS\NewAgentUniverseByDeepSeek` (local copy of <https://github.com/TwinsEarth/NewAgentUniverseByDeepSeek>)
**Method:** read-only. `glob`/`grep`/`read` only; no file in the tree was modified, created or deleted.
**Build evidence:** `cargo +1.85.0 check --workspace --all-targets` was run with `CARGO_TARGET_DIR` redirected outside the tree. It **exited 0** (3 m 25 s).
**Date of survey:** 2026-10-04.
**Purpose:** evidence base for designing a NEW agent-native system (v1.0.1 → v1.9.9 rebuild) that *reuses* this project's engineering (ABI, plugin kernel, PMB, policy engine, AUSec, settlement) while *inverting* its human-centric assumptions.

> **Provenance warning, stated once.** This project is itself a clean-room rewrite of `TwinsEarth/agent-universe`, produced from a line-by-line audit of upstream v2.5.6 / v2.8.2 / v3.5.0. It carries an unusual amount of *self-audit apparatus* (`docs/GAP-ANALYSIS.md`, `docs/VERIFICATION.md`, `scripts/check-*.mjs`, tripwire tests). Read that apparatus as part of the estate: it is the most reusable thing here, and also the thing most likely to be quoted as if it were runtime behaviour.

---

## 1. Repository size and version state

| | |
|---|---|
| `VERSION` (file) | `3.8.0` — 6 bytes, the single machine-readable source of truth |
| `[workspace.package] version` | `3.8.0` (`Cargo.toml:11`) |
| `contracts/VERSION` | mirrors root `VERSION` (CI asserts `diff -u`, `ci.yml:373-382`) |
| MSRV | `rust-version = "1.85"` (`Cargo.toml:17`) — justified in-file (zeroize 1.9.0 is edition-2024) |
| Edition | 2021, `resolver = "2"` |
| Rust crates | **19** (`crates/*`), ~**89 000 LOC** in `src/`, + ~11 000 LOC in `tests/` |
| Docs (files) | 26 in `docs/`, 611 KB total |
| Biggest dirs | `integrations/` 6 626 files / 45.8 MB (46 MB of that is `node_modules`), `contracts/` 131 files / 4.1 MB, `crates/` 289 files / 5.3 MB |

**Version history is a single 3.x line, not a fresh start.** `CHANGELOG.md` is 3 133 lines and covers, newest first: `3.8.0`, `3.7.7`, `3.7.6`, `3.7.5`, `3.7.4`, `3.7.3`, `3.7.2`, `3.7.1`, `3.7.0`, `3.6.8`, `3.6.7`, `3.6.6`, `3.6.5`, `3.6.4`, `3.6.3`, `3.6.2`, `3.6.1`, `3.6.0`, `3.5.9`, `3.5.8`, `3.5.7`, `3.5.6`, `3.5.5`, `3.5.4`, `3.5.3`, `3.5.2`, `3.5.1`, `3.5.0`, `3.4.5`, `3.2.1`, `2.2.2`, `1.2.3`, `1.1.1`, `1.0.1`. So the **`1.0.1 → 1.9.9` series named in the new project's brief corresponds to this project's `1.0.1 → 3.8.0` history** — the numbering is different but the *content progression* overlaps substantially (see §9).

**Stale-version landmine.** `README.md:1` says V3.8.0, but `docs/ARCHITECTURE.md:1`, `docs/VERIFICATION.md:1`, `docs/SELF-AUDIT.md:1`, `ATTRIBUTION.md:3` and `NOTICE:4` all still say **V1.2.3**. `README.md:21` and `:55` still say V3.2.1. Nothing generated reconciles them; the version *checkers* only assert that `VERSION`, `Cargo.toml`, the SDKs and `contracts/VERSION` agree, not prose.

---

## 2. Top-level layout

| Path | Files | What it is |
|---|---|---|
| `.cargo/` | 1 | `config.toml` only: `jobs = 2`, `term.color = "auto"`. 30 lines of justification — upstream's libp2p+rusqlite tree OOM'd rustc on a 16 GB box (`rustc-LLVM ERROR: out of memory`), so concurrency is bounded. Mirrored into `[profile.dev] debug = 0`. |
| `.github/` | 6 | `dependabot.yml` + 5 workflows: `ci.yml` (563 lines, the real gate), `client.yml`, `codeql.yml`, `packages.yml`, `release.yml`. All actions pinned to commit SHAs, not tags. |
| `client/` | 26 | Browser GUI + Tauri shell. `app.js` (22 KB), `index.html`, `style.css`, `serve.mjs` (static server), `lib/nau.mjs` (25 KB — the whole client-side protocol: Ed25519 via WebCrypto, canonical JSON, `NauClient` REST wrapper, `Identity`), `lib/canonical.mjs`, `src-tauri/` (unbuilt shell), `platforms/{linux,macos,windows}.md`, `test/e2e.mjs` (41 KB, 81 assertions against a real daemon). |
| `conformance/` | 2 | `generate.mjs` (independent Node canonicalizer + OpenSSL Ed25519) and `vectors.json`. CI regenerates and `git diff --exit-code`s it — this is the cross-language byte-equality guarantee. |
| `contracts/` | 131 | Foundry project. 4 real contracts + 3 base mixins + 5 test files + `script/Deploy.s.sol`; the other ~115 files are `lib/forge-std` / `node_modules`-style vendored deps. 4.1 MB, of which ~93 KB is first-party Solidity. |
| `crates/` | 289 | The 19-crate Rust workspace. 5.3 MB. |
| `docs/` | 26 | See §6. 611 KB. Mixed authoritative and stale. |
| `DSH插件/` | 1 | A single file, `安装指引.txt` (5 391 bytes, **GB18030-encoded, not UTF-8**). It is an *install guide*, not a plugin: DeepSeek Harness plugin `@twinsearth/nau-dsh-plugin@3.4.5`, 5 696 B tarball, sha256 `BE08A2B9…620F`, 5 read-only tools. The actual source is `integrations/dsh/`, not here. |
| `integrations/` | 6 626 | **One** integration: `integrations/dsh/` (the DeepSeek Harness Cordis plugin). 7 real files (`lib/tools.js` 10 KB, `lib/index.js` 624 B deliberately empty, `cordis.patch.yml`, `market-entry.yml`, `package.json`, `smoke.mjs`, `README.md`); the remaining ~6 600 files are `node_modules`. |
| `scripts/` | 10 | All `.mjs`, no shell. Seven of them are *gates*: `check-no-panics.mjs`, `check-unsafe-containment.mjs`, `check-version-consistency.mjs`, `check-plugin-invariants.mjs`, `check-metric-claims.mjs`, plus `verify-all.mjs` (the 23-gate driver), `deploy-local.mjs` (19-check install/run/restart), `bump-version.mjs`, `publish-github.mjs`, `repair-encoding.mjs`. |
| `sdks/` | 58 | Python + JS, both zero-dependency. See §7. |
| `target/` | — | Build artifacts. Contains `debug/` (all 19 `.rlib`, 17 binaries, 31 397 files in `deps/`), plus prebuilt `aarch64-apple-darwin/`, `x86_64-apple-darwin/`, `x86_64-unknown-linux-gnu/` trees. `.rustc_info.json` records **rustc 1.85.0, x86_64-pc-windows-msvc, LLVM 19.1.7**. |
| `Cargo.toml` | — | 99 lines. Workspace, 19 members, pinned `[workspace.dependencies]`, deliberate small-dep policy, `[profile.release] panic = "abort"` + `lto = "thin"`. |
| `VERSION` | — | `3.8.0` |
| `CHANGELOG.md` | — | 213 KB / 3 133 lines. Chinese, very dense, one entry per release with a named *thesis* per version. |
| `ATTRIBUTION.md` | — | 13 KB. Upstream provenance, what was preserved / replaced / dropped. Headed *V1.2.3* (stale). |
| `NOTICE` | — | 3 KB. MIT notice + what was derived + explicit disclaimer of affiliation. Headed *V1.0.1* (stale). |
| Other | — | `README.md` (38 KB), `CONTRIBUTING.md`, `SECURITY.md`, `LICENSE` (MIT, dual copyright line), `.gitignore`, `Cargo.lock` (95 KB). |

---

## 3. The Rust workspace

Dependency direction is a strict DAG. `nau-core` is the frozen leaf: `#![forbid(unsafe_code)]`, no I/O, no async (`crates/nau-core/src/lib.rs:30`). Only `nau-sandbox` allows `unsafe`, confined to its private `platform` module.

### 3.1 `nau-core` — 5 544 LOC, 16 files — the frozen contract
Modules: `identity/` (canonical, mod), `domain/` (agent, checkpoint, consistency, fork, money, org, task, mod), `image/` (mod, signing), `clock`, `error`, `version`.

Public surface (`crates/nau-core/src/lib.rs:41-52`):
`Clock, ManualClock, SystemClock`, `AgentCard, AgentCategory, Bid, Dispute, DisputeOutcome, EvidenceGrade, Money, NonceGuard, Pricing, PricingModel, PricingUnit, ReputationScore, ResultEnvelope, Skill, Sla, Task, TaskId, TaskSpec, TaskState, Verifiable, VerificationPolicy, MAX_CLOCK_SKEW_SECS`, `NauError, Result`, `canonical, Did, Identity, Keypair, PublicKey, Signature64, DID_PREFIX, DID_PREFIX_LEGACY`, `ChunkDigest, ChunkRef, ImageManifest`, `PROJECT, PROTOCOL_VERSION, UPSTREAM_PROJECT, UPSTREAM_VERSION, VERSION`.

Hard invariants worth reusing verbatim:
- `Money(i64)` in minor units (10⁻⁶), **no floats, no `Add`/`Sub` operator impls** (`domain/money.rs:22`) — arithmetic is a `Result`.
- `DID_PREFIX = "did:nau:"`, `DID_PREFIX_LEGACY = "did:aip:"`; `Did::parse` accepts both. `PublicKey::from_hex` rejects small-order points via `is_weak()`. Known limit recorded honestly: 8-byte fingerprint ⇒ ~2³² birthday collision (`docs/ARCHITECTURE.md:101`).
- `TaskSpec` is the six-field contract `["goal","context","done","todo","trace","owner"]` (`domain/task.rs:35`); `TaskState` has 12 variants with a total transition table (`:282`); `EvidenceGrade` is `{Verified, CpuProto, Unverified}` defaulting closed (`:307`); `ResultEnvelope::validate_for_settlement()` is the settlement gate (`:627`).
- `VerificationPolicy` is `Committee{n,f}` | `RequesterOnly` (`:159`), with `quorum()` doing checked arithmetic.

### 3.2 `nau-plugin` — 12 300 LOC, 17 files — **the plugin kernel** (ABI `3.2`)
The crate the new project should mine hardest. `crates/nau-plugin/src/lib.rs:110,117`:
```rust
pub const ABI_MAJOR: u32 = 3;
pub const ABI_MINOR: u32 = 2;
pub const HOT_SWAP_SUPPORTED: bool = true;
```

| Module | LOC | What it owns |
|---|---|---|
| `tier.rs` | 463 | Five-level classification derived **from the signed plugin name alone**. `VENDOR_PREFIX="com.twinsearth."`, `SYSTEM_PREFIX="com.twinsearth.sys."`, `OFFICIAL_PREFIX`, `CERTIFIED_PREFIX`. `Tier::{System, Official, Certified, ThirdParty, Blacklisted}`; `Tier::from_name`, `from_label`, `is_loadable`, `runs_in_process` (System only), `is_hot_pluggable`, `requires_counter_signature`. |
| `capability.rs` | 900 | **21 capabilities**, 3 of them `BASIC` (`plugin:lifecycle:read`, `plugin:message:send`, `plugin:storage:own`); 5 are `is_kernel()` and are *unapprovable* outside `Tier::System`. `Approval::{Host, VendorTeam, CertificationCommittee, Operator}`; `Grant::{Always, RequiresApproval(Approval), Refused{reason}}`; `Capability::decision(tier)` is **total over 21 × 5** with a test that walks the cross product. `resolve_with_approvals` exists because without it the approval branch was unreachable — see the module's own account at `capability.rs:377-401`. |
| `manifest.rs` | 1 216 | Signed manifest: parse → validate → digest → verify against the module it names. `Manifest, VerifiedManifest, TrustStore, Limits, PriorityClass, SignatureSection`. |
| `arbiter.rs` | 2 021 | The load pipeline: manifest in, running plugin **or typed refusal** out. `Arbiter, LoadRequest, LoadFailure, Loaded`. |
| `registry.rs` | 346 | Who is registered, at what version, depending on what. `Registry::record_violation`; violations reach `VIOLATION_THRESHOLD = 3` ⇒ quarantine. |
| `lifecycle.rs` | 689 | **12-state** machine, exactly one `self.state = …` assignment site. `PluginState::{Discovered, Verified, Loaded, Running, Paused, Unhealthy, Stopping, Stopped, Refused, Quarantined, Archived, Blacklisted}`; `TERMINAL = [Archived, Blacklisted]`; `X → X` is not an edge; `start()` (Loaded→Running, **increments `init_runs`**) and `restore(snapshot, at)` (Paused→Running, **does not**) are deliberately separate — this is the "restore is not restart" rule. |
| `bus.rs` | 997 | **The PMB.** See §4. |
| `runtime.rs` | 1 262 | The isolation port. `RuntimeKind::{Native, Process, Container, Wasm, FnCall, MicroVm, FullVm}` (**7**, `ALL` at `:181`), `Boundary` (**15** variants, `ALL` at `:376`), `RuntimeCapabilities`, `StartupCost` with `CostBasis::{Measured, DesignTarget}` so a target can never be printed as a measurement. `unavailability()` returns the typed reason for the 5 kinds this build lacks. |
| `hot.rs` | 900 | Hot update / hot plug / **ABI adapters**. `Abi{major,minor}`, `trait AbiAdapter{accepts,produces,name,adapt}`, `Abi2To3` (the shipped 2.x→3.x adapter, `hot.rs:118`), `AdapterRegistry::with_shipped_adapters()` (empty registry is the fail-closed default), `Compat::{Direct, Adapted{adapter,from}}`, `PluginSlot`, `RoutingTable`, `HotSwapper` (double-buffered swap with health check + drain + rollback), `HotPlug::{start_order, stop_plan}`. |
| `secure.rs` | 431 | Opt-in end-to-end channel the host cannot read. X25519 + HKDF-SHA256 + ChaCha20-Poly1305; needs `crypto:channel`, refused outright at ThirdParty. |
| `certify.rs` | 753 | Certification: `ReviewStage::{Submitted, AutoScanned, **ManualReview**, GreyRun, Certified, Rejected}`, `Finding::{Clean, Note, Blocker}`, `Certification` = the **scope** a review grants (a certified plugin may not exceed it even with a valid counter-signature). |
| `blacklist.rs` | 571 | Quarantine with fingerprint, evidence, appeal state machine, emergency broadcast. Self-auditing: it documents that it cannot stop an operator with write access to the store. |
| `scheduler.rs` | 518 | `Scheduler, Distribution, TaskId, SubmitError`. |
| `snapshot_audit.rs` | 537 | `SnapshotAudit, SnapshotOperation` — the three questions an operator must answer after the fact. |
| `update_path.rs` | 363 | `UpdatePath, PathTiming, SnapshotCost` — incremental-snapshot cost accounting. |

**Exports** (`nau-plugin/src/lib.rs:90-100`): `Arbiter, LoadFailure, LoadRequest, Loaded`, `Approval, Capability, CapabilityToken, Grant`, `Certification, Finding, Review, ReviewStage, ScanReport`, `LoadRefusal, PluginError, Result`, `Limits, Manifest, PriorityClass, SignatureSection, TrustStore, VerifiedManifest`, `Distribution, Scheduler, SubmitError, TaskId`, `SnapshotAudit, SnapshotOperation`, `PluginId, Tier`, `PathTiming, SnapshotCost, UpdatePath`.

### 3.3 `nau-plugins` — 27 531 LOC, 47 files — the plugin implementations and the T0 host
Split from the kernel on purpose: *"The kernel decides who may do what … it deliberately contains no plugin."* (`nau-plugins/src/lib.rs:3-6`)

- `host.rs` (1 501): `SystemPlugin` trait, **the one door** `HostContext`, `SystemPluginHost`, `PluginGrant`, `PluginLimits`, `BusHandle`, `LogRecord`. A T0 plugin gets exactly three things: its read-only token, a *request* to send on the bus (the host performs it), and a bounded log sink. It cannot see the registry, sandbox manager, arbiter, blacklist, or other plugins' state.
- `frame.rs` (462): the host ABI frame. `MAX_FRAME_BYTES = 1 MiB`, `LENGTH_PREFIX_BYTES = 4`, `Request`, `Response`, `OutboxRequest`, `encode_frame`/`write_frame`/`read_frame`/`decode_request`/`decode_response`, and 9 stable error codes `abi_payload_empty`, `abi_frame_too_large`, `abi_frame_truncated`, `abi_payload_not_json`, `abi_version_mismatch`, `abi_unknown_operation`, `abi_payload_not_object`, `abi_missing_field`, `abi_field_type`.
- `payload.rs` (256), `sign.rs` (561, vendor-side manifest signing, deliberately **not** feature-gated), `official.rs` (438 — a catalogue of **14** `com.twinsearth.official.*` plugins: shard, bridge, mesh, swarm, market, economy, scheduler, mcp, crdt, agent, test-runner, skill, chain-anchor, agent-council).
- **25 T0 system plugins** assembled in `host.rs:1183-1256` and declared in `host.rs:1265`:
  `sys.identity`, `sys.storage`, `sys.policy`, `sys.orchestrator`, `sys.blacklist`, `sys.lifecycle`, `sys.arbiter`, `sys.sandbox`, `sys.erasure`, `sys.ledger`, `sys.attest`, `sys.http`, `sys.migrate`, `sys.net.transport`, `sys.net.dht`, `sys.net.gossip`, `sys.chain`, `sys.ausec`, `sys.resource`, and the six `sys.security.*` bodies.
- **17 process-plugin binaries** in `src/bin/`: `nau-plugin-{echo, agent, agent-council, bridge, chain-anchor, emergence, market, mcp, reputation, scheduler, skill, swarm}` + the echo/refusal fixtures.
- `plugins/security/mod.rs` is a *least-privilege table* worth copying wholesale: police holds `kernel:plugin:manage` but **not** `kernel:policy:write`; tribunal holds the opposite; surveillance and audit hold **no kernel capability at all** ("they can watch and cannot act"). Six bodies rather than one because *a plugin's capability set is its authority*.

### 3.4 `nau-sandbox` — 6 770 LOC, 17 files — the isolation boundary (AUSec)
The rule the crate exists to enforce (`nau-sandbox/src/lib.rs:26-28`): *"A boundary that is documented but not enforced is worse than no boundary, because it launders trust. Every policy field must either be enforced in code, or make the request FAIL."*

- `SandboxSpec` has **no `Option` and no `Default`** (`spec.rs:368`) — `interpreter`, `limits`, `network`, `filesystem`, `env`, `waivers`. A missing limit is a compile error.
- `Limits` is all-required integers: `timeout_ms, memory_bytes, cpu_ms, disk_bytes, max_processes, max_open_files, max_output_bytes`. Zero is refused, so "unlimited" cannot be expressed by accident.
- `NetworkPolicy::{DenyAll, AllowList{hosts}, Unrestricted{justification}}` — `justification` is a `String`, not a bool, and the audit log refuses an empty one.
- `Confinement::{Required, WholeHost}`; `EnvPolicy{inherit: InheritPolicy::Nothing, vars}` — **there is no "inherit" variant**, because inheriting is the defect.
- `Waivers{filesystem_confinement, disk_bytes, cpu_ms, max_open_files}` — each an `Option<String>` reason.
- `Capability` (`capability.rs:37`, `#[non_exhaustive]`, **14** variants) is what a *backend* can enforce: `EnvAllowlist, OutputCap, Timeout, WorkDirIsolation, NetworkDenyAll, NetworkAllowList, MemoryLimit, CpuLimit, DiskQuota, ProcessCountLimit, OpenFileLimit, FilesystemConfinement, SharedPageReadOnly, MemoryReclaim`. A spec field no backend publishes makes `validate()` fail with `PolicyNotEnforceable`, **naming the boundary**.
- `trait SandboxExecutor` (`executor.rs:49`) — object-safe, stateless. `NullExecutor` is the **default** and refuses with `ExecutionDisabled`. `RealProcessExecutor` is the only real backend: **`windows-jobobject` on Windows, `unix-setrlimit` elsewhere** (`platform.rs:67`, `lib.rs:112`).
- OS-level enforcement (`enforcement.rs`) — `Mechanism::{AppArmor, EbpfWhitelist}`, `EnforcementSupport::{Available{via}, Refused{reason}}`, and a `PREMISES: [Premise; 3]` table that states *what each security claim depends on*. Non-Linux is a typed refusal, never a skip. Nothing generates profiles or loads eBPF: this module is declaration + refusal only.
- Also: `manager.rs` (867, identity/ownership/startup sweep/audit log), `component.rs` (`SafeComponent` — the only way to obtain a path component), `snapshot.rs` (`Layer, Snapshot, SnapshotId, SnapshotStore, StoreReport`), `reclaim.rs`, `shared_page.rs`, `process.rs`.

### 3.5 `nau-market` — 3 198 LOC, 7 files — market, matching, reputation, resources
`Market` owns all state in `BTreeMap`s (`service.rs:139`). `MarketConfig{min_stake, min_reputation_bps, max_bids_per_task, fault_severity_bps, fault_slash_bps}` (`:70`). Full lifecycle API: `deposit, balance, register_agent, discover, search, reputation, leaderboard, publish_task, submit_bid, match_task, start_task, submit_result, verify_result, settle, open_dispute, arbitrate, escrowed_for, conservation, audit, stats, persist, restore*`.

`matching.rs` — `rank_bids`, `MatchOutcome`. Integer scoring `reputation_bps × 1e6 / price_minor` with a latency penalty measured against **the agent's own committed p95**; total order `(score desc, price asc, eta asc, did asc)`.

`resource.rs` (new in 3.8.0, 596 LOC) — the commodity vocabulary:
`ResourceKind::{Cpu, Memory, Storage, Network, Snapshot, AgentCapability}` with **six distinct units** (`cpu-milliseconds`, `mib-seconds`, `gib-seconds`, `bytes`, `snapshots`, `invocations`); `is_rate()` is true only for Memory and Storage. `ResourceAmount` is a **different type from `Quota` with no `From`, no method, no `as`** — the compiler, not a comment, enforces that a quota cannot be handed over as a price. `checked_add` **refuses to add two different kinds** and names both. `ResourceBundle` has `total_of(kind)` and deliberately **no `total()`**.

`reputation.rs` — four-dimensional integer bps with integer exponential smoothing (α = 1/10); honesty moves only on slashing or fault-free confirmation, never on settlement.
`persistence.rs` — restore report; `actor.rs` — `Actor`, `Authority::{Party, Arbitrator, **Operator**}`.

### 3.6 `nau-ledger` — 4 031 LOC, 8 files — exact-integer double-entry with escrow
`Ledger` (`ledger.rs:78`) with `deposit, withdraw, escrow, release, refund, slash, stake, unstake, balance, escrow_record, live_escrows, escrow_shortfall, total_paid, total_refunded`. Two conservation views: `conservation()` is **O(1)** off incremental counters; `audit()` is **O(N)**, re-walking every account and recomputing every entry — and tests prove the O(N) audit detects corruption the O(1) check cannot.

Reserved namespaces (upstream's one good idea, kept): `__escrow__:<task>`, `__stake__:<did>`. Hash-chained journal: `verify_journal`, `journal_break`, `verify_journal_against(anchor)`, `anchor`, `relink_journal_for_recovery`, `from_journal`. `fork.rs` — `ForkedLedger`, `Contested`: fork semantics where **three branches spend one escrow, not three**.

### 3.7 `nau-consensus` — 1 418 LOC, 5 files — BFT-lite with authenticated votes
`CommitteeSpec::new(n, f)` with checked arithmetic (`n ≥ 3f+1`, `q = 2f+1`); `Committee::assign(spec, task_id, members)` enforces `members.len() == n` and dedupes; `Committee::cast(vote, now)` verifies signature **and membership**; `tally()` returns `TallyResult` with `reason`, `equivocators: Vec<Did>`, `safety_violation`. Equivocation voids the whole round; both sides reaching quorum ⇒ `SafetyViolation` + `ConflictingQuorums`; `reset_for_next_round()` keeps the equivocation record. `Vote`, `Decision`, `MAX_PROPOSAL_LEN`.

### 3.8 `nau-agent` — 2 891 LOC, 11 files — memory / LLM ports / emergence
`provider.rs` (715) — `ProviderKind` (OpenAI/Anthropic/Gemini shapes), `HttpProvider<T>`, `ProviderProfile`, `build_request`, `parse_response`; empty completions are typed errors, never panics. `llm.rs` (589), `memory.rs` (`AgentMemory`, `MemoryRecord`), `experience.rs` (`Experience, SharedMemory, MAX_QUALITY_BPS, MIN_SHARED_OBSERVATIONS`), `chain.rs` (`HashChain`, `link_digest`, `GENESIS_DIGEST`), `transfer.rs` (`TransferBundle` — portable memory), and **`layered.rs`**: `MemoryTier::{Individual, Group, Generational}`, each tier a real verifiable `HashChain`; the generational head digest is the *cross-generation pointer* naming exactly what a later generation inherits.

### 3.9 `nau-store` — 2 085 LOC, 7 files — the persistence port
`trait Store`, `FileStore` (append JSONL ×3 + atomic `meta.json`), `MemoryStore`, `journal.rs` (hash-chained journal), `jsonl.rs` with `SCHEMA_VERSION: u32 = 1` per record and `MAX_RECORD_BYTES`. A truncated trailing line (simulated crash) is **skipped with a warning**, not fatal. `compact()` = temp file → fsync → rename.

### 3.10 `nau-net` — 4 718 LOC, 10 files — transport, NAT, relay, topology
`trait Transport`, `TcpTransport` (real: bind/connect, 4-byte big-endian length prefix, 8 MiB frame cap that **errors instead of allocating**, read timeout), `MemoryTransport` (named as a double, never as a network service). `stun.rs` is a real RFC 5389 codec (`MAGIC_COOKIE = 0x2112_A442`); `nat.rs` is RFC 4787 mapping/filtering classification as a pure function + `NatProbe` port; `Unknown` is a first-class answer. `RelayPool` with `RelayClass::{Dedicated, SelfHosted, ThirdParty, General}`, capacity enforced in the same call as insertion, health mutable **only** via `record_failure`/`record_success`. `LayeredTopology` Lv1–Lv7 where room membership is a **pure deterministic function of node id** (insertion-order independent) and `route_hops` is derived from the real tree, never a constant.

### 3.11 `nau-libp2p` — 5 986 LOC, 8 files — the optional real swarm
`swarm.rs` (1 937), `config.rs` (1 241), `identity.rs` (792), `behaviour.rs` (585), `transport.rs` (414), `naming.rs` (379), `codec.rs` (374, `PROTOCOL_VERSION: u8 = 1`). Composes TCP + Noise + Yamux + Kademlia + GossipSub + Circuit Relay v2 + AutoNAT + DCUtR and adapts it to `nau_net::Transport`. Everything libp2p is behind `#[cfg(feature = "libp2p")]` (`lib.rs:164-184`), **off by default** — the three-OS CI matrix never compiles it. Cut for MSRV: `dns`, `quic`, `tls`, `websocket`. DCUtR hole-punching and AutoNAT reachability are explicitly **not verified** (loopback tests would pass while testing nothing).

### 3.12 `nau-node` — 13 254 LOC, 13 files — the composition root
`api.rs` (2 547), `plugin_review.rs` (2 900), `plugin_blacklist.rs` (1 646), `api/sandbox_routes.rs` (1 621), `auth.rs` (889), `plugin_cli.rs` (850), `plugin_host.rs` (656), `plugin_process.rs` (625), `lib.rs` (449), `p2p.rs` (281). Binaries: `nau`, `nau-daemon`, `nau-p2p-daemon`.

**REST routes** (`api.rs:747-1560`, sandbox routes under `api/sandbox_routes.rs`):
- `GET /` | `/health` | `/version` → node snapshot
- `GET /plugins` → ids, states, `outbox_pending` per plugin, `host_key`
- `POST /plugins/<id>/call` → `{capability, …payload}`; the *plugin's own* `require_declared` decides. Drains the plugin's bus outbox on the way out.
- `GET|POST /agents`, `GET /agents/<did>`
- `GET /p2p/peers`, `GET /p2p/agents` (read-only; a card that arrived over the wire is shown but never enters the staked registry)
- `GET|POST /tasks`, `GET /tasks/<id>`, `POST /tasks/<id>/{bids,match,start,results,verify,settle}`
- `GET /accounts/<a>/balance`, `POST /accounts/<a>/deposit`
- `GET /conservation`, `GET /audit`, `GET /stats`, `GET /leaderboard`
- `GET|POST /disputes`, `GET /disputes/<id>`, `POST /disputes/<id>/arbitrate`
- `POST /plugins/com.twinsearth.sys.security.police/report`, `POST /plugins/com.twinsearth.sys.security.audit/assess`
- Sandbox: `POST|GET /api/v1/sandboxes`, `GET|DELETE /api/v1/sandboxes/<id>`, `POST …/{exec,commands,run,pause,resume}` (aliases without the `/api/v1` prefix)

**Auth model** (`auth.rs`) — unconfigured **means refuse** (`Authenticator::deny_all` is the default; there is no "allow if nothing configured" branch); `Scope::{Read, Write, Admin}`; `Principal` has no public write-capable constructor; a credential bound to a DID cannot set another actor in the body (`api::require_actor`); no wildcard CORS, `Origin` allow-listed and echoed exactly with `Vary: Origin`, `Host` checked (DNS rebinding).

### 3.13 Remaining crates

| Crate | LOC | Purpose / surface |
|---|---|---|
| `nau-attest` | 4 469 | Attestation envelopes + Merkle inclusion proofs. Verifies structure, pinned root signature, nonce binding, freshness, payload binding. **All four proof formats return `ChainNotImplemented`; `HardwareAttested` is unreachable by construction.** `grade_of`, `label`, `EvidenceGrade`, `MAX_ACHIEVABLE_GRADE`. |
| `nau-erasure` | 3 322 | Systematic GF(256) Reed–Solomon, generator-polynomial/remainder construction. `ErasureCoder`, `MAX_TOTAL_SHARDS`, `Matrix`, `PRIMITIVE_POLYNOMIAL`. **No error correction** — corrupt-but-not-declared-lost shards silently yield wrong data; asserted by test. |
| `nau-migrate` | 5 622 | Upstream v2.5.6 data import. Amounts converted by exact decimal text, never through `f64`; refuses when not exactly representable. **Reads JSON/JSONL only, no DB reader.** Binary + library. `MAX_ACHIEVABLE_GRADE`-adjacent refusal taxonomy in `warning.rs`. |
| `nau-http` | 1 709 | Real HTTP/1.1 client: `tcp.rs` (654), `tls.rs` (320, behind `tls` feature, **never test-executed**; `https://` without the feature is a typed error *before* a socket opens — no silent downgrade), `message.rs` (credential-shaped header redaction in `Debug`), `transport.rs` (`CannedTransport`, `RecordingTransport`). |
| `nau-mcp` | 3 044 | MCP server. `SUPPORTED_PROTOCOL_VERSIONS = ["2025-06-18", "2024-11-05"]`, `DEFAULT_PROTOCOL_VERSION = "2024-11-05"` (`protocol.rs:16,19`), explicit negotiation. Each tool defined **once** (name, description, typed params, handler); JSON Schema derived from that definition; args validated by declared type before dispatch, `-32602` naming the parameter. Threshold params (`approvals`, `committee_size`) **do not exist** in the tool surface. |
| `nau-image` | 1 748 | Content-addressed image chunks, on-demand reading, P2P chunk exchange. `ImageManifest`, `ChunkRef`, `ChunkDigest`, `ChunkReader`, `ReadReport`, `Metrics`, `ChunkSource`, `VerifiedSource`, `LocalSource`, `PeerSource`, `ChunkServer`, `SeedingRatio`, `SeedingStats`, `UdosSource`/`SourceKind` (declared + refused). |
| `nau-node` | 13 254 | See §3.12. |

---

## 4. PMB — the plugin message bus, and every wire format

### 4.1 PMB message types — `crates/nau-plugin/src/bus.rs`
```rust
pub enum PmbKind { Request, Response, Event }          // bus.rs:47,  snake_case on the wire
pub enum Priority { Low, Normal, High, Critical }      // bus.rs:74; Critical is host-only
pub enum Target { Plugin(String), Broadcast, Host }    // bus.rs:118

pub struct PmbMessage {                                 // bus.rs:142, #[serde(deny_unknown_fields)]
    id: String, corr_id: Option<String>, source: String, target: Target,
    capability: String, kind: PmbKind, topic: Option<String>,
    payload: serde_json::Value, issued_at: u64, ttl_ms: u64, priority: Priority,
}
pub struct BusLimits { max_message_bytes, max_messages_per_minute, max_audit_records }  // bus.rs:219
pub struct AuditRecord { message, source, delivered_to, capability, delivered, refusal, at }
pub struct Delivery { recipients, audit_index }
pub trait BusMembership { fn state_of(&self, name: &str) -> Option<PluginState>; }  // bus.rs:720
pub struct Bus { … }                                    // bus.rs:319
```
Defaults: `max_message_bytes = 256 KiB`, `max_messages_per_minute = 600`, `max_audit_records = 4096`; `ttl_ms = 5_000` on `PmbMessage::new`. `BusLimits::validate()` refuses zero, because zero would *look* like unlimited and behave like "nothing works".

**The five checks in `Bus::send`** (`bus.rs:445-605`), all of which must hold:
1. `message.source == caller.plugin()` — the caller must **present its token**; a mismatch is `bus_source_forged`. The doc at `bus.rs:428-438` records the real bug this closes (the first version looked the sender up *by a field inside the message*, so any caller could act under another plugin's capabilities).
2. the token holds the declared `capability` (`bus_capability_refused`) — the declared label is checked, not trusted.
3. sender is `Running` (`bus_sender_not_running`).
3b. `Critical` priority only from `com.twinsearth.sys.*` (`bus_priority_reserved`).
4. non-zero TTL, and the encoded frame is within the size cap (`bus_no_ttl`, `bus_too_large`).
4b. **every** recipient is running (`bus_recipient_not_running`) — a broadcast is refused *as a whole* if any subscriber is not, because partial delivery of one logical message is split state; self-send refused.
5. rate limit (`bus_rate_limited`), sliding 60 s window with saturating arithmetic.

**`refusal_is_misconduct` (`bus.rs:623`) is one of the sharpest ideas in the tree.** Exactly three refusals escalate to a recorded violation → quarantine on the 3rd: `bus_source_forged`, `bus_capability_refused`, `bus_priority_reserved`. Everything else explicitly must **not** escalate — *"`bus_rate_limited` is backpressure, and turning 'busy' into 'banned' would make the bus punish load"*. `send_checked(registry, …)` joins the bus refusal to the lifecycle violation count; the doc notes both rules were separately true and jointly unenforced before it existed.

### 4.2 Encoding: **canonical JSON, deliberately — not bincode, not CBOR**
`bus.rs:23-30`: the draft spec said bincode in one section and CBOR in another. The bus uses this project's **canonical JSON** because it is already byte-identical across Rust, Python and JavaScript and pinned by conformance vectors, so a plugin in another language is a first-class citizen and the audit log stays readable. Cost: size, bounded by `BusLimits::max_message_bytes`. Encoding is `serde_json::to_vec(message)` at `bus.rs:521` — the size check is done on the real encoded bytes.

Canonical JSON rules (`docs/CONFORMANCE.md`, `nau-core/src/identity/canonical.rs`, `conformance/generate.mjs`): root must be an object; `signature` dropped at **any** depth (upstream only dropped the top level); keys sorted by **Unicode code point** (not UTF-16 code unit); no whitespace, separators exactly `,` and `:`; raw UTF-8 strings escaping only `"`, `\` and 7 short control escapes, other controls as **lowercase** `\u00xx`; **integers only** — float/exponent/out-of-range rejected; nesting depth ≤ 64; serialization failure is an **error**, never a signed `null`.

### 4.3 Host ABI frame (process plugins) — `crates/nau-plugins/src/frame.rs:1-16`
```
frame         := length-prefix || payload
length-prefix := 4 bytes, big-endian unsigned, payload length
payload       := UTF-8 JSON, one of Request | Response, at most 1 048 576 bytes
```
Request  `{ "abi": "3.2", "id", "op", "payload" }`
Response `{ "abi", "id", "plugin", "version", "ok", "payload"?, "code"?, "message"?, "outbox"? }`
One request, one response, then exit. Process exit codes: `0` answered, `1` answered `ok:false`, `2` the frame itself could not be read/written — so a host can distinguish "the plugin refused the call" from "the plugin is not speaking this ABI at all".

`abi_is_compatible()` (`frame.rs:98`) accepts an **older major** (because hot compatibility via adapters exists) and refuses only an **ABI from the future**. The envelopes deliberately do **not** use `deny_unknown_fields` — the bus is additive within a major, so refusing unknown keys would break every deployed plugin the first time an optional field was added. (The manifest schema *does* use `deny_unknown_fields`; the asymmetry is intentional.)

`OutboxRequest{to?, topic?, capability, payload}` lets a process plugin **declare an intent** to send on PMB. It holds no token, so the host presents the plugin's own token and every bus check runs on the way out. A declaration naming an unheld capability is refused exactly as a system plugin's queued message is.

### 4.4 Versioned protocol constants (complete list)

| Constant | Value | File:line |
|---|---|---|
| `PROTOCOL_VERSION` | `"nau/1"` | `crates/nau-core/src/version.rs:24` |
| `ABI_MAJOR` / `ABI_MINOR` | `3` / `2` → wire string `"3.2"` | `crates/nau-plugin/src/lib.rs:110,117`; formatted at `crates/nau-plugins/src/frame.rs:65` |
| `SCHEMA_VERSION` | `1` (per JSONL record, `"v": 1`) | `crates/nau-store/src/jsonl.rs:36` |
| `SCHEMA_KEY` / `SCHEMA_VALUE` | `"nau-migrate.schema"` / `"nau-migrate/1"` | `crates/nau-migrate/src/plan.rs:71,73` |
| `PROTOCOL_VERSION` (libp2p wire) | `1u8` | `crates/nau-libp2p/src/codec.rs:46` |
| `MAGIC_COOKIE` (STUN) | `0x2112_A442` | `crates/nau-net/src/stun.rs:62` |
| MCP versions | `["2025-06-18","2024-11-05"]`, default `2024-11-05` | `crates/nau-mcp/src/protocol.rs:16,19` |
| `DID_PREFIX` / `_LEGACY` | `"did:nau:"` / `"did:aip:"` | `crates/nau-core/src/identity/mod.rs` |
| Vendor prefixes | `com.twinsearth.` / `.sys.` / `.official.` / `.certified.` | `crates/nau-plugin/src/tier.rs:32-41` |
| `UPSTREAM_AUDITED` | `"agent-universe v2.5.6"` / `"v2.8.2"` / `"v3.5.0"` | `nau-plugin/src/lib.rs:130`, `nau-sandbox/src/lib.rs:108` |

**There is no binary serialization anywhere in the crate graph.** `serde` + `serde_json` only; `bincode` and `cbor` appear in the tree as *rejected alternatives* in prose.

---

## 5. Build and test reality

### 5.1 Does the tree build? — **Yes, verified**
```
cd E:\DS\NewAgentUniverseByDeepSeek
$env:CARGO_TARGET_DIR="<path outside the tree>"
cargo +1.85.0 check --workspace --all-targets --message-format short
→ Finished `dev` profile [unoptimized] target(s) in 3m 25s   EXIT=0
```
Zero warnings, zero errors across all 19 crates **and all targets** (lib, bins, integration tests, doctests). `target/debug/` also holds 19 `.rlib`s, 17 `.exe`s (all `nau-plugin-*` binaries, `nau`, `nau-daemon`, `nau-p2p-daemon`, `nau-migrate`) and 31 397 files under `deps/`, built with rustc 1.85.0 / LLVM 19.1.7 on `x86_64-pc-windows-msvc`.

### 5.2 Stubs, ignores, gates — **the tree is unusually clean**
Exhaustive grep over `crates/**/*.rs`:
- `todo!()` — **0**
- `unimplemented!()` — **0**
- `unreachable!()` — **0** (one *comment* at `nau-sandbox/src/reclaim.rs:294` explains why it was written out instead)
- `#[ignore]` — **0**, in `src/` and in `tests/`
- `#[allow(dead_code)]` — **1**, on `nau-sandbox/src/platform/rt.rs:9` (the raw Win32 FFI decls)
- `#[cfg(feature = …)]` — only two features exist: `tls` (`nau-http/src/lib.rs:52`, `tcp.rs:632`) and `libp2p` (`nau-libp2p/src/lib.rs:164-184`, `config.rs:314,320`). Both **off by default**.

This is not an accident of style: `scripts/check-no-panics.mjs` and `scripts/check-unsafe-containment.mjs` are CI-adjacent gates, and `cargo clippy --workspace --all-targets -- -D warnings` is a hard gate with no `|| echo "tolerated"`.

### 5.3 What is genuinely *not implemented* (declared and refused, not stubbed)
These are the honest gaps, all in-code as typed refusals rather than missing branches:
- `RuntimeKind::{Wasm, MicroVm, FullVm, Container, FnCall}` — 5 of 7 have no backend; `unavailability()` returns the reason (`nau-plugin/src/runtime.rs:276`).
- AppArmor profiles and eBPF programs are **declared only** (`nau-sandbox/src/enforcement.rs:1-11`).
- TLS path is compiled but **never test-executed**; `https://` without the feature is a typed error before a socket opens.
- Unix sandbox backend (`setrlimit`/`setsid`) is **unverified on the survey host** (Windows).
- Tauri desktop shell is source-only, never compiled here.
- All four attestation formats return `ChainNotImplemented`; no Intel/AMD cert chain ⇒ `HardwareAttested` unreachable.
- Erasure coding does **no** error correction.
- `nau-migrate` has **no database reader** (JSON/JSONL only).
- DCUtR hole-punching / AutoNAT reachability unverified; no QUIC, no DNS resolution.
- Application-layer P2P data plane has **no consumer** (`docs/VERIFICATION.md:88`).
- The ten economy nouns `ERC-8004, x402, L402, Lightning, Taproot, RGB, HTLC, USDC, ERC-4337, Paymaster` are **0 hits** in `crates/`, `contracts/src/`, `docs/` — and `com.twinsearth.sys.resource` **refuses all ten by name** (that is the D-02 deliverable, and there is a test asserting the count is 10).

Additionally, `docs/VERIFICATION.md:74` concedes that "three-platform deployment verification" was *asserted* for a long time and only became *executed* when `deploy-local.mjs` was added to the CI rust job — a good example of the project's own failure mode being caught by its own apparatus.

### 5.4 CI workflows (`.github/workflows/`)

| File | Jobs | Gate |
|---|---|---|
| `ci.yml` (563 lines) | `rust` (ubuntu/macos/windows matrix), `python`, `js`, `conformance`, `contracts`, `version`, `shellcheck`, `libp2p` | The real gate. Every action pinned to a commit SHA; every step fails the job. |
| `client.yml` | Tauri bundle | Reuses `ci.yml` via `workflow_call`, so the bundle cannot be built on a red tree. |
| `codeql.yml` | CodeQL | — |
| `packages.yml` | npm / GitHub Packages publish | Injects the version from `VERSION` at publish time. |
| `release.yml` | Release | Reuses `ci.yml`; `libp2p` job opts out with an `if: github.event_name != 'workflow_call'` and the reason is written down. |

`ci.yml` concurrency key includes `github.workflow` — because without it, `Client` and `Release` cancelled each other through `workflow_call` (`ci.yml:27-34`). `RUST_TOOLCHAIN = "1.85.0"` is pinned to the MSRV so `rust-version` is *verified* rather than asserted. `CARGO_BUILD_JOBS = "2"`.

### 5.5 `scripts/verify-all.mjs` — 23 gates in one command
```bash
node scripts/verify-all.mjs                    # all 23 gates
node scripts/verify-all.mjs --quick            # skip slow gates
node scripts/verify-all.mjs --only=no-panics,unsafe-containment
node scripts/verify-all.mjs --allow-missing-tools
```
Design contract, and it is the right one: a missing tool is **`SKIP`**, listed separately under `NOT VERIFIED`, **and makes the exit code non-zero** unless `--allow-missing-tools` is passed; `NOT VERIFIED` and `FAILED` are printed as different things. Named gates include `rust-test`, `rust-lock`, `rust-clippy`, `rust-fmt`, `no-panics`, `unsafe-containment`, `version-consistency`, `conformance`, `client`, `libp2p`, `contracts-static`/`contracts-build`/`contracts-test`, `python`, `javascript`, `deploy`, `cross-target`, `plugin-invariants`, `doc-counts`, `metric-claims`.

### 5.6 Exact commands to run
<!-- markdownlint-disable MD022 -->
```bash
## Build & test (the surveyed tree's own pinned toolchain)
cargo +1.85.0 build --workspace --locked
cargo +1.85.0 test  --workspace --locked
cargo +1.85.0 fmt   --all --check
cargo +1.85.0 clippy --workspace --all-targets -- -D warnings

## Dependency-resolution sanity (one second, readable failure)
cargo metadata --locked --format-version 1

## Optional feature trees (both off by default)
cargo +1.85.0 build -p nau-libp2p --features libp2p --locked
cargo +1.85.0 test  -p nau-libp2p --features libp2p -- --test-threads=1
cargo +1.85.0 test  -p nau-libp2p            ## proves the feature is genuinely optional
cargo +1.85.0 test  -p nau-http              ## TLS feature off

## Everything, the project's own way
node scripts/verify-all.mjs --quick
node scripts/deploy-local.mjs --prefix "$TEMP/nau-deploy"   ## NAU_PROFILE=debug
node conformance/generate.mjs && git diff --exit-code -- conformance/vectors.json
python sdks/python/run_tests.py
node sdks/js/test/run.js
node client/test/e2e.mjs

## Contracts
python contracts/verify_static.py && python contracts/verify_api.py
forge build --sizes
forge test -vvv --no-match-path '*lib/forge-std*'
```
<!-- markdownlint-enable MD022 -->
> **Read-only caveat honoured:** none of the above was executed inside the tree. `cargo check` was run with `CARGO_TARGET_DIR` pointed outside it; `cargo metadata --locked` alone would write nothing.

---

## 6. Documentation state

26 files, 611 KB. Three families, and the split matters for a rebuild.

**A. Authoritative, hand-written, still accurate (design/spec — reuse as specification):**

| File | Size | Content | Version it claims |
|---|---|---|---|
| `PLUGIN-ARCHITECTURE.md` | 28 KB | The plugin programme: version map V2.2.2→V3.0.0, principles, architecture diagram, lifecycle, five tiers, capability model, manifest/signature, **PMB (§7)**, external interface (§8), isolation port and its three backends (§9.1–9.4 incl. the honest WASM position and the V3.0.0 seams), quota matrix, SOP stages, migration path, verification plan, **per-item deviations from the draft (§14)**, V3.0.0 roadmap. | V2.2.2 → V3.0.0 |
| `DEVELOPMENT-PLAN-v3.5-v3.7-AUSec.md` | 30 KB | AUSec plan. Same house style: every named mechanism must be findable in the repo, and anything unfindable is **named as absent** rather than written as "already exists". | v3.5–v3.7 |
| `DEVELOPMENT-PLAN-v3.8-v3.9-Economy.md` | 23 KB | §0 is the model to copy: a *verified* reuse table (with evidence), an explicit **0-hit** table for the ten economy nouns, and a **conflict table** against hard baselines (integer ledger vs second unit of account; evidence gate fail-closed; sandbox refuses egress ⇒ payment must be proxied by a host signing service so an in-sandbox agent **never holds a private key**). | v3.8–v3.9 |
| `DEVELOPMENT-GUIDANCE-v3.5.x.md` | 20 KB | Merged 43-item security/engineering audit + release-doc verification. | v3.5.x |
| `CONFORMANCE.md` | 7.7 KB | Canonical JSON rules, cross-language vectors. | — |
| `DEFENCE-IN-DEPTH.md` | 4.6 KB | Three-layer enforcement + the place all three miss. | — |
| `DEPLOYMENT.md` | 12 KB | Install/run/restart. (Its own text quotes the V1.0.1-vs-1.1.1 drift as a defect.) | V1.0.1 → 1.1.1 |
| `METRICS-REPORT-v3.5.9.md` | 8.5 KB | Reproducibility report for AUSec quantitative claims. | v3.5.9 |
| `UPSTREAM-AUDIT-v3.5.0.md` | 14 KB | Audit of upstream's release documentation. | v3.5.0 |
| `00-issue-body.md` | 7.5 KB | Independent doc-verification result + merged 43-item action list. | v3.5.0 |

**B. Audit / verification apparatus (the most reusable artefact class — reuse as *method*):**

| File | Size | Content |
|---|---|---|
| `GAP-ANALYSIS.md` | 74.6 KB | 76 upstream defects with file:line evidence and quoted upstream source. |
| `GAP-ANALYSIS-v2.8.2.md` | 36.7 KB | Incremental audit of upstream's *fixes* — and the finding that a batch of fixes introduced **more severe** defects than they removed. |
| `VERIFICATION.md` | 46.2 KB | Claim → gate map for all 23 gates, then three tables of **hard limits** (platform, capability, and *the limits of the verification method itself* — including "`no-panics` is text analysis, not semantic analysis" and "cross-target only type-checks, never links"). |
| `SELF-AUDIT.md` | 9.9 KB | Self-audit. |
| `PLUGIN-MIGRATION.md` | 20.6 KB | Full re-audit and migration plan for the everything-is-a-plugin move. |
| `shard-{A..G}-*.md` | 3.3–6.4 KB each | Seven parallel audit shards against upstream v3.5.0 (`gsn-core 0.3.50`): ledger, consensus+identity, plugins, sandbox, api+mcp, node+runtime, non-Rust+docs. |

**C. Generated / report-shaped / mechanically checked:**
`METRICS-REPORT-v3.5.9.md` (reproducibility annex), and the *count tripwires* — `scripts/check-plugin-invariants.mjs`, `check-metric-claims.mjs`, and the `doc-counts` gate keep 7 documents' numbers (23 gates, 41 deployment checks, 25 T0 plugins, 418 `nau-plugins` tests) synchronized. `docs/README.md` (1.4 KB) is an index. Two `.docx` binaries (`agent-universe-v3.5.x-development-guidance.docx`, `agent-universe-v3.5-v3.7-AUSec-development-plan.docx`) duplicate the Markdown plans.

**Stale — do not trust without cross-checking:**
`ARCHITECTURE.md` (20 KB, the best single overview in the tree, but headed **V1.2.3** and its §7 "not implemented" list is explicitly annotated as *superseded* by §7.1), `VERIFICATION.md`, `SELF-AUDIT.md`, `GAP-ANALYSIS*.md`, `ATTRIBUTION.md`, `NOTICE` — all V1.2.3-headed while `VERSION` is 3.8.0; `README.md` body still V3.2.1-headed while its title says V3.8.0.

**Note on encoding.** `DSH插件/安装指引.txt` is **GB18030, not UTF-8** — a `read`-tool call on it fails. `scripts/repair-encoding.mjs` exists and suggests this is a known recurring class of problem in this repo. All `docs/*.md` and all `crates/**/*.rs` checked are valid UTF-8 without BOM.

---

## 7. SDKs, contracts, integrations, `DSH插件/`

### 7.1 `sdks/python` — zero-dependency, stdlib only
`nau_sdk/`: `canonical.py` (9.2 KB), `identity.py` (14.1 KB), `_ed25519.py` (8.7 KB — **own Ed25519 implementation**, hence "no `cryptography`" is CI-enforced), `models.py` (46.5 KB), `ledger.py` (13.4 KB, `Ledger`, `LedgerEntry`, `GENESIS_HASH`), `market.py` (13.0 KB, `MarketClient`, `DEFAULT_BASE_URL`, `DEFAULT_PATHS`, `DEFAULT_TIMEOUT`), `mcp.py` (10.0 KB, `McpHttpClient`, `MCP_PROTOCOL_VERSION`), `errors.py` (typed: `SDKError, ValidationError, AmountError, OverflowError, DidError, SignatureError, AuthorizationError, TransitionError, MarketError, McpError`), `version.py` (reads `VERSION`, never restates it). Tests: 4 files, 77 KB. Runner `run_tests.py` exits non-zero when a test cannot run — a skip-only run cannot masquerade as a pass. Talks to a daemon over HTTP (`MarketClient`) and MCP-over-HTTP (`McpHttpClient`).

### 7.2 `sdks/js` — zero-dependency ESM, Node ≥ 18
`index.js` + `index.d.ts` (26.6 KB of types) + `lib/{canonical,errors,identity,ledger,market,mcp,models,version}.js`; 12 test files (~112 KB) driven by `test/run.js`, **223 tests**. `package.json` is `@nau/sdk` and **deliberately declares no `version` field** — CI fails if it appears, because the version must be injected from `VERSION` at publish time. Same client shape as Python.

Both SDKs are **clients of the HTTP/MCP surface**, not of PMB. Neither has a plugin-side entry point.

### 7.3 `contracts/` — Foundry, 4 real contracts (~93 KB of first-party Solidity)

| Contract | Lines/bytes | Interface |
|---|---|---|
| `Settlement.sol` | 33 KB, ~690 lines | `contract Settlement is Ownable, ReentrancyGuard`. `Task` struct, `Status` enum with an explicit transition table (`_setStatus`), `createTask`, `acceptTask` (payable, required stake), `assignTask`, `submitResult`, `attestVerified` (verifier quorum), `disputeTask`, `resolveDispute(onlyOwner)`, `settleTask`, `refundTask`, `withdrawCredits`/`withdrawAllCredits`, `addVerifier`/`removeVerifier`/`setQuorum`/`setRequiredStake` (all `onlyOwner`). 25 custom errors, 21 events. `totalObligations()` / `isSolvent()` expose the solvency invariant. |
| `ReputationRegistry.sol` | 12.9 KB | `Reputation` + `Snapshot` structs, `recordReputation(agent, epoch, r)` verifier-gated with `EpochNotIncreasing` / `EpochAlreadyRecorded` / `ScoreAboveBps` guards, `getLatestReputation`, `snapshotByEpoch`. |
| `AgentCardAnchor.sol` | 10.4 KB | `anchor(cidHash, agentDidHash)`, `verify`, `isAnchorable`, `getAnchor`/`tryGetAnchor`, `anchorerOf`, paginated `anchorsOf` with `PageOutOfRange`. |
| `GovernanceToken.sol` | 11.3 KB | ERC20 + `Checkpoint`-based vote history: `delegate`, `delegateBySig`, `getVotes`, `getPastVotes`, `numCheckpoints`, `clock`/`CLOCK_MODE`. |
| `base/{ERC20,Ownable,ReentrancyGuard}.sol` | 1.5–4.9 KB | Hand-rolled mixins — no OpenZeppelin dependency. |
| Tests | 5 files, 77 KB | `Settlement.t.sol` (32.8 KB), `SettlementInvariant.t.sol` (13.8 KB — **invariant/fuzz testing**), plus per-contract suites. |
| Build/verify | `foundry.toml` (6.8 KB), `verify_static.py` (28.4 KB), `verify_api.py` (11.3 KB), `script/Deploy.s.sol` (8.1 KB) | `Deploy.s.sol` **reads `VERSION` at run time** and refuses to deploy on a mismatch — a behavioural check CI greps for (`ci.yml:390-392`). |

Two things to carry over: the deploy script refuses on a version mismatch rather than trusting a config file, and ownership is `onlyOwner` throughout — i.e. **every governance action on-chain is a human keyholder action** (§8).

### 7.4 `integrations/dsh/` — DeepSeek Harness plugin
`@twinsearth/nau-dsh-plugin`, Cordis-style. `lib/index.js` is **deliberately empty** (624 B): the mount point is the `tools` sub-path, declared explicitly in `cordis.patch.yml` via an `insert` patch, so reading the profile tells you which module becomes the plugin instead of relying on a "root export is the plugin" convention. `lib/tools.js` exports `inject = ['tools']` and registers **5 read-only tools**: `nau_version`, `nau_inspect`, `nau_conformance`, `nau_amount`, `nau_node_status`. Fail-closed choices stated in its README: arguments passed as an **array**, never a shell string; every invocation bounded by timeout and output cap; a non-zero exit is **reported with its stderr**, never swallowed; `nau_node_status` says only whether a TCP connect succeeded and says so *in the answer*. It deliberately does **not** expose driving the node.

### 7.5 `DSH插件/安装指引.txt`
GB18030 install guide for the above: four install channels (npmjs, GitHub Release asset, GitHub Packages, raw tarball), the exact `dsh plugin --profile <p> add @twinsearth/nau-dsh-plugin@3.4.5` syntax, the manual pnpm + `package.json` `dsh.profile.bundles[]` equivalent, and a verification snippet that proves `apply()` really registers 5 tools rather than merely importing. It ends by stating precisely what was verified and what was **not** ("I have not seen these 5 tools inside a running harness session" — *未验证*).

---

## 8. Human-centric assumptions to invert

This is the load-bearing section. Every entry is a place where the code *requires* a human to decide, approve, review, or hold a key — with the exact site.

**H1 — The tier system's only escape hatch from `Refused` is a named human authority.**
`Approval::{Host, VendorTeam, CertificationCommittee, **Operator**}` (`nau-plugin/src/capability.rs:39-48`) and `Grant::RequiresApproval(Approval)` (`:77`). `Capability::resolve_with_approvals` (`:402`) grants an Official/Certified capability **only** if the supplied approval names *exactly* the required authority — `Approval::Operator` explicitly **cannot** substitute for `VendorTeam` or `CertificationCommittee` (`:390-396`, asserted at `:861-863`). A rebuild that wants autonomy must replace this with machine-verifiable standing, not a broader approval set — widening it is exactly the failure the module documents.

**H2 — Certification has a mandatory human stage.**
`ReviewStage::ManualReview` (`nau-plugin/src/certify.rs:47-48`, *"A human is reading the code"*), and the pipeline is **sequential and non-skippable**: `Submitted → AutoScanned → ManualReview → GreyRun → Certified` (`:97-103`), with `Certification` as the *scope* the review grants. Label `manual_review` is on the wire (`:79`). The whole review journal is described as *"the human record of a review"* (`nau-node/src/plugin_review.rs:1433`); a scan at `manual_review` "cannot change the decision" (`:3078`).

**H3 — The operator's trust store is the only way a third-party plugin loads at all.**
`TrustStore` (`nau-plugin/src/manifest.rs`), checked at `manifest.rs:512-518`: a third-party publisher not in the operator's trusted key list is refused. `nau-node/src/plugin_review.rs:23-24`: *"The one check a review cannot decide is the operator's trust store: a third-party manifest is refused at load until the operator lists its publisher key."* `plugin_review.rs:1046-1047`: *"what it needs is the operator trusting this key, which is the admission decision this review is making on the operator's behalf — recorded as a … `operator-trust`"* (`:1109`). `nau-node/src/plugin_cli.rs:447`: *"refused until the operator names a key."*

**H4 — Quarantine appeal and unblock paths are human-authored.**
`nau-node/src/plugin_blacklist.rs:499` — `--reason` defaults to the literal string `"operator unblock"`; the module says outright (`:72`) that it cannot stop *"an operator with write access"*. `nau-plugin/src/blacklist.rs:128`: *"documents the appeal path instead, and an operator who could delete an entry…"*.

**H5 — HTTP authorization is a static, human-configured token list with human scopes.**
`nau-node/src/auth.rs` — `Scope::{Read, Write, **Admin**}` (`:47-54`), `Admin` = *"Decide disputes, which can slash another account's stake"* (`:52`). Principals come from configured tokens; `Authenticator::deny_all` is the default. `api.rs:452,458` require `Scope::Admin` for `POST /disputes/<id>/arbitrate` **and** for `POST /plugins/<id>/call`. An unconfigured node refuses everything and a human must edit the token configuration to make anything happen.

**H6 — Dispute arbitration and market authority are role-typed humans.**
`nau-market/src/actor.rs:26-27` — `Authority::Operator` / *"The node operator, acting for the market itself"*, with `Actor::operator(did)` (`:71`). `Market::arbitrate(actor, ruling, at)` (`service.rs:974`) is the only slashing path for a dispute. Upstream's own fix story (`GAP-ANALYSIS`) notes that taking a caller-supplied `slash_amount` was a defect — the current code takes a *ruling* from an authorized actor instead. Invert by making the arbiter a committee (see §9 / v1.7) rather than an Operator actor.

**H7 — On-chain governance is `onlyOwner` everywhere.**
`contracts/src/Settlement.sol`: `addVerifier`, `removeVerifier`, `setQuorum`, `setRequiredStake`, `resolveDispute` are all `onlyOwner` (`:272,288,303,311,504`). `ReputationRegistry.sol`: `addVerifier`/`removeVerifier` `onlyOwner` (`:124,140`). `GovernanceToken.sol`: `mint` `onlyOwner` (`:89`). `Ownable` itself is a single human key. There is no timelock, no multisig, no on-chain vote-to-execute in the surveyed tree.

**H8 — Sandbox waivers are free-text human justifications.**
`nau-sandbox/src/spec.rs:313-322` — `Waivers{filesystem_confinement: Option<String>, disk_bytes, cpu_ms, max_open_files}`; `NetworkPolicy::Unrestricted{justification: String}` (`:173-177`). The manager writes the reason to the audit log before the sandbox exists. The test fixture literally uses `"operator accepted unbounded disk for this task"` (`nau-sandbox/tests/policy.rs:474`) and `"operator decision"` (`spec.rs:879`), and `nau-plugin/src/runtime.rs:1172` uses `"operator accepted this on a loopback-only host"`. Good design — but the *authority* is a human sentence.

**H9 — Every refusal message is written for a human reader.**
There are dozens of sites where the code's own comment says the message exists so an operator knows where to look: `arbiter.rs:6` (*"the operator's …"*), `arbiter.rs:368` (*"the one fact an operator needs"*), `arbiter.rs:773`/`:1601` (*"the two tell an operator different things — fix the name, versus fix the document"*), `arbiter.rs:1715` (*"operator to the wrong document"*), `runtime.rs:473` (*"an operator learns the whole list in one refusal"*), `plugin_process.rs:92` (*"for an operator who has to look it up in the audit log"*), `migrate/src/main.rs:9,138` (stdout is machine-readable, **stderr is the human half**). A machine-deciding system needs the same information as structured data, with prose as a rendering — not instead of it.

**H10 — The interactive/CLI surface is the assumed control plane.**
`crates/nau-node/src/bin/nau.rs:104-114` reads JSON on **stdin** to exercise canonical signing; `:180` prints the identity *"so an operator can"* compare; `nau-node/src/plugin_cli.rs` is the operator's interface to tiers/runtimes/verify/system/blacklist, and `plugin_cli.rs:236` states the design concern outright — *"A whole set of tiers that can pass every gate and still not run is the … operator has"* (i.e. T1/T2/T3 were loadable and reviewable but nothing ever ran one). `nau-node/tests/plugin_cli.rs:1088` and `:1259` describe tests that *"drive the whole thing the way an operator would"*.

**H11 — The trust assumption is "an operator with write access to the store".**
`plugin_blacklist.rs:72`, `plugin_review.rs:340` (*"A journal is durable state a human can edit"*), `nau-plugin/src/snapshot_audit.rs:9` (*"what an operator has to be able to answer for after the fact"*). The durable stores are local files; the security model is honest that a local human with write access wins.

**H12 — Local-file trust means the daemon is a single-tenant, single-machine assumption.**
`nau-node/src/auth.rs:11` frames the threat as *"A web page the operator merely visits can therefore drive the daemon"* — i.e. the daemon is modelled as a **privileged local desktop service**, not a network node. `Deployment.md` and `deploy-local.mjs` install to a prefix and restart a local process. A network-native rebuild inverts this: every peer is potentially hostile and there is no local operator to fall back on.

**Inversion summary.** Across H1–H12 the same shape recurs: *a place where the system could not decide, so it returned a typed refusal and handed the decision to a human.* That refusal discipline is the reusable asset; the *human* is the thing to replace — with (a) machine-checkable standing instead of named authorities (H1, H3, H6, H7), (b) deterministic automatic review with the human stage kept only as an override (H2), (c) capability-scoped machine credentials instead of a configured token file (H5), and (d) structured refusal data instead of prose (H9).

---

## 9. Kernel concepts to preserve

Ordered by how expensive each would be to retrofit.

1. **One channel, one choke point.** PMB is the *only* path between plugins (`bus.rs:1-9`). Upstream let components call each other directly, so a policy check had to be repeated at every call site — and the audit found the checks existed with no callers. A single channel turns policy from a convention into a chokepoint.
2. **The caller presents its token; the message's `source` field is not trusted.** `Bus::send` checks `message.source == caller.plugin()` **first**, because everything below reasons about `source` as if it were true (`bus.rs:428-473`). This is a real vulnerability class, closed with a named refusal code.
3. **Tier derived from the signed name alone, never from a separate field.** `Tier::from_name` (`tier.rs:95`), plus a reserved vendor prefix so a name collision cannot be used to climb tiers.
4. **Three-way decisions, not booleans.** `Grant::{Always, RequiresApproval(who), Refused{reason}}` — the middle outcome is the honest one, and the matrix is *total* over 21×5 with a test that walks the cross product so a new variant cannot be added without a decision (`capability.rs:297-341`).
5. **Kernel authority has no approval path.** `Capability::decision`'s kernel branch *ignores the tier* rather than consulting an approval table (`capability.rs:313-324`); the `Refused` branch of `resolve_with_approvals` never reads the approvals argument (`:435-441`). This is what makes it *unapprovable* rather than merely *unapproved*.
6. **Privilege pairs over privilege bundles.** `SandboxCreate` ⊥ `SandboxConfigure` (`capability.rs:140-152`) and `SandboxSnapshot` ⊥ `SandboxRestore` (`:153-182`): create+configure is a privilege-escalation path, and read-snapshot vs write-snapshot are disclosure vs substitution. Also `police` (`kernel:plugin:manage`, not `kernel:policy:write`) vs `tribunal` (the reverse) — the body that can change the law must not also enforce it.
7. **Docs that watch and cannot act.** Surveillance and audit hold **no kernel capability at all** (`nau-plugins/src/plugins/security/mod.rs:14-25`, asserted by test).
8. **One assignment site for state.** `Lifecycle::state` is private with exactly one `self.state = …` in the module, inside `transition()`, and a test reads the file back to fail if a second appears (`lifecycle.rs:3-11`). Terminal states have no inbound edges; `X → X` is not an edge.
9. **Restore is not restart.** `start()` (Loaded→Running, increments `init_runs`) and `restore(snapshot, at)` (Paused→Running, must name its snapshot, does **not** increment) are separate methods, and `init_runs` is the *measurement* that proves it (`lifecycle.rs:253-313`).
10. **Escalation must distinguish misconduct from backpressure.** `refusal_is_misconduct` (`bus.rs:623`) escalates exactly 3 of 10 refusal codes; *"A rule that quarantines on any refusal is a rule that quarantines on a race."*
11. **A boundary is either enforced in code or the request FAILS — never silently skipped.** `nau-sandbox/src/lib.rs:26-28`; mechanically implemented as `Capabilities` that each backend publishes, with `PolicyNotEnforceable` naming the boundary. Non-Linux → typed refusal, never a skip.
12. **No optional limits; no `Default`.** `SandboxSpec` has no `Option` and no `Default`; `Limits` values of `0` are refused so "unlimited" cannot be expressed by accident. `EnvPolicy` has **no inherit variant**, because inheriting is the defect.
13. **Waivers and unrestricted modes require a *reason*, recorded in the audit log before the thing exists.**
14. **A claim whose condition is not written beside it will be read as unconditional.** The `PREMISES: [Premise{claim, depends_on, otherwise}; 3]` table (`nau-sandbox/src/enforcement.rs:157`) and `CostBasis::{Measured, DesignTarget}` where the string `(target, not measured)` is *part of the rendering* so a report cannot present a target as a measurement by forgetting (`runtime.rs:108-119`).
15. **Adapters are named and fail-closed.** `AdapterRegistry::new()` is empty; `with_shipped_adapters()` is not the default; an ABI with no adapter is refused **with the migration path named** (`hot.rs:5-10,184-193`). And `Abi2To3::adapt` **refuses** a 2.x broadcast with no topic rather than inventing one, because choosing a topic would silently change who receives the message (`hot.rs:133-142`).
16. **Double-buffered hot swap.** Prepare and health-check beside the running version → switch the routing table by **one pointer replacement** → only then drain the old instance; a drain failure does not resurrect the old version (`hot.rs:12-16`, `HotSwapper::swap` at `:500`).
17. **Hot plug is a dependency graph, not a hope.** `HotPlug::start_order` / `stop_plan` compute order; stopping something others depend on pauses the dependents **first, in reverse order**.
18. **Exact integer money, no floats, no operator overloading.** `Money(i64)` with `Result`-returning arithmetic and O(1) `conservation()` *plus* an independent O(N) `audit()` that tests prove catches what O(1) cannot.
19. **Two different questions must not share a type.** `Quota` ("is this permitted?") vs `ResourceAmount` ("what is it worth?"): no `From`, no method, no `as`. `checked_add` refuses to add different `ResourceKind`s and names both. `ResourceBundle` deliberately has no `total()`.
20. **Six units means six units.** cpu-ms, MiB·s, GiB·s, bytes, snapshots, invocations — and `is_rate()` marks the two that are quantities *over time*, because a market that priced a MiB-second as a MiB is selling something other than what it delivers.
21. **Signed votes with a fixed committee.** `Committee::assign` forces `members.len() == n` and dedupes; equivocation voids the round and yields `equivocators: Vec<Did>` (attribution, not a bool); simultaneous quorums ⇒ `SafetyViolation` + `ConflictingQuorums`.
22. **`#[cfg(test)]`-outside-no-`unwrap`/`panic!`, and `unsafe` confined with `// SAFETY:`.** Enforced by `check-no-panics.mjs` (strips comments and strings first) and `check-unsafe-containment.mjs`, not by convention.
23. **Refusals are structured and stable.** Every refusal has a machine-readable code (`bus_*`, `abi_*`, `identity_*`, `policy_not_enforceable`, …) and the audit log keeps **the refused attempt**, because *"a refused message is the interesting one"*.
24. **Tripwire counts.** `Capability::ALL: [Capability; 21]`, `Tier::ALL: [Tier; 5]`, `PluginState::ALL: [PluginState; 12]`, `RuntimeKind::ALL: [RuntimeKind; 7]`, `Boundary::ALL: [Boundary; 15]`, `Capability::ALL: [Capability; 14]`, `SECURITY_PLUGINS: [&str; 6]` — the length is written into the type, so adding a variant without deciding its edges is a **compile error**.

---

## 10. Concrete reuse map: this tree → the new project's ten medium versions

Legend: **[ADOPT]** take the code/design essentially as-is · **[ADAPT]** keep the mechanism, invert the human assumption · **[MINE]** read for the pitfalls and the refusal taxonomy, write fresh.

### v1.0 — Autonomy kernel
| Source | Stance | What exactly |
|---|---|---|
| `crates/nau-core` (whole crate) | **ADOPT** | The frozen leaf: `Did`/`Keypair`/canonical/`Money`/`Task`/`TaskState`/`EvidenceGrade`/`Verifiable`/`NonceGuard`/`Clock`. Keep `#![forbid(unsafe_code)]`, no I/O, no async. |
| `crates/nau-plugin`: `tier.rs`, `capability.rs`, `manifest.rs`, `registry.rs`, `lifecycle.rs`, `bus.rs`, `error.rs` | **ADOPT** | The kernel's four verbs — *register, route, arbitrate, transition*. Keep the 21-capability matrix and the total-over-cross-product test. |
| `crates/nau-plugin`: `arbiter.rs` | **ADAPT** | Keep the load pipeline and typed refusals; replace the `Approval::Operator`/trust-store gate (H1/H3) with machine-verifiable standing. |
| `crates/nau-plugins`: `host.rs` (`SystemPlugin`, `HostContext`, `SystemPluginHost`) | **ADOPT** | *The one door*: three things and nothing else. This is the single best idea for a v1.0 kernel boundary. |
| `crates/nau-plugins/src/plugins/{identity,storage,policy,orchestrator,lifecycle,arbiter,blacklist}.rs` | **ADOPT** | Seven of the 25 T0 plugins are pure kernel mechanics and are directly reusable. |
| `crates/nau-node/src/{plugin_host,plugin_process}.rs` | **ADAPT** | Process supervision and the frame protocol; drop the CLI-shaped entry points. |
| `crates/nau-plugin/src/hot.rs` | **ADOPT** | `Abi`, `AbiAdapter`, `AdapterRegistry` (fail-closed default), `HotSwapper`, `HotPlug`. |
| `scripts/check-plugin-invariants.mjs` | **ADOPT** | The roster invariant ("every implementation is constructed *and* declared") is cheap and catches a real class of bug. |

### v1.1 — Capability Graph
| Source | Stance | What exactly |
|---|---|---|
| `nau-plugin/src/capability.rs` | **ADOPT** | `Capability::ALL` (21), `BASIC` (3), `is_kernel()`, `decision()`, `resolve()`/`resolve_with_approvals()`. This *is* the capability graph's node set. |
| `nau-plugin/src/capability.rs` privilege pairs | **ADOPT** | `SandboxCreate`⊥`SandboxConfigure`, `SandboxSnapshot`⊥`SandboxRestore`, `police`⊥`tribunal`. |
| `nau-sandbox/src/capability.rs` | **ADOPT** | 14 backend-enforceable boundaries + `BoundaryRequest::{Require, Waived}` + `PolicyNotEnforceable`. Different axis from plugin capabilities: *what a substrate can enforce*. |
| `nau-plugin/src/runtime.rs` | **ADOPT** | `RuntimeKind` (7) × `Boundary` (15) × `RuntimeCapabilities`, with `unavailability()` reasons. |
| `nau-plugins/src/plugins/security/mod.rs` | **ADOPT** | The least-privilege table as an executable roster test. |
| `nau-sandbox/src/component.rs`, `spec.rs` | **ADOPT** | `SafeComponent` (the only way to get a path component) and the no-`Option` spec. |
| The *graph* itself (capability → capability implications, transitive ceilings) | **MINE** | **This does not exist here.** Capabilities are a flat set with a tier matrix; there are no capability-to-capability edges, no delegation, no attenuation, no expiry. `CapabilityToken` is bound to a plugin id + manifest digest with no narrowing. That is the new project's contribution. |

### v1.2 — Negotiation
| Source | Stance | What exactly |
|---|---|---|
| `nau-market/src/matching.rs` | **ADOPT** | `rank_bids`, `MatchOutcome`; integer scoring; latency penalty against the bidder's **own committed p95**; total order `(score desc, price asc, eta asc, did asc)` so ties are never decided by arrival order. |
| `nau-market/src/service.rs` | **ADOPT** | The six-step discipline per mutating method: load → authorize → anti-replay → validate → transition → mutate. `submit_bid`, `match_task`, `start_task`, `submit_result`, `verify_result`, `settle`. |
| `nau-core/src/domain/task.rs` | **ADOPT** | `Bid`, `ResultEnvelope`, `VerificationPolicy::{Committee{n,f}, RequesterOnly}`, `validate_for_settlement()`. |
| `nau-market/src/actor.rs` | **ADAPT** | Keep `Actor`/`Authority` as the typing of "who is speaking"; replace `Authority::Operator` (H6) with machine roles. |
| `nau-consensus/src/{committee,vote,spec}.rs` | **ADOPT** | Signed votes, fixed membership, `quorum()`, `Equivocation`, `TallyResult::equivocators`. |
| Multi-round / multi-party protocol negotiation (offer→counter→accept, capability exchange, metering handshakes) | **MINE** | The negotiation here is one-shot bid-and-match. There is no iterative protocol, no counter-offer, no session state machine beyond `TaskState`. |
| `docs/CONFORMANCE.md` + `conformance/` | **ADOPT** | Any new wire message must be added to the cross-language vector generator, or byte-equality is a claim, not a fact. |

### v1.3 — Portable State
| Source | Stance | What exactly |
|---|---|---|
| `nau-sandbox/src/snapshot.rs` | **ADOPT** | `Layer`, `Snapshot`, `SnapshotId`, `SnapshotStore`, `StoreReport` — incremental snapshot with per-layer accounting. |
| `nau-plugin/src/{snapshot_audit,update_path}.rs` | **ADOPT** | `SnapshotAudit`/`SnapshotOperation`; `UpdatePath`/`PathTiming`/`SnapshotCost`. |
| `nau-plugin/src/lifecycle.rs` `restore()` + `init_runs` | **ADOPT** | "Restore is not restart", with a counter as the evidence. |
| `nau-core/src/domain/{checkpoint,fork,consistency}.rs` | **ADOPT** | Execution checkpoints, fork semantics, convergence model. |
| `nau-ledger/src/fork.rs` | **ADOPT** | `ForkedLedger`/`Contested`: three branches spend one escrow, not three. |
| `nau-agent/src/transfer.rs` + `layered.rs` + `chain.rs` | **ADOPT** | `TransferBundle` (portable memory), `MemoryTier::{Individual,Group,Generational}` each a real `HashChain`, and the generational head digest as the **cross-generation pointer**. |
| `nau-store/src/{journal,jsonl,file}.rs` | **ADOPT** | Hash-chained append-only journal, `SCHEMA_VERSION` per record, truncated-trailing-line tolerance, atomic `compact()`. |
| `nau-sandbox/src/{reclaim,shared_page}.rs` | **ADOPT** | `Reclaimer`/`MemoryStats`/`PageState`; `SharedPageReadOnly` as an enforceable boundary (a writable shared page is a **cross-tenant write primitive**). |
| Cross-machine / cross-implementation state transfer with a portable envelope | **ADAPT** | `TransferBundle` is capped (`MAX_ENTRIES`, `MAX_FIELD_BYTES`) and local. The envelope shape is right; the addressing, integrity and resumption story is new. |

### v1.4 — Economic Autonomy
| Source | Stance | What exactly |
|---|---|---|
| `crates/nau-ledger` (whole crate) | **ADOPT** | `Money(i64)` minor units; double-entry; `escrow/release/refund/slash/stake/unstake`; O(1) `conservation()` **plus** independent O(N) `audit()`; reserved `__escrow__:`/`__stake__:` namespaces; hash-chained journal with anchors. |
| `nau-market/src/service.rs` `MarketConfig` | **ADOPT** | `min_stake`, `min_reputation_bps`, `fault_severity_bps`, `fault_slash_bps` — and the rule that the **server** decides the slash amount, never the caller. |
| `nau-market/src/reputation.rs` | **ADOPT** | Four-dimensional integer bps; honesty moves only on slashing or fault-free confirmation, never on settlement. |
| `nau-market/src/resource.rs` | **ADOPT** | `ResourceKind` (6) with 6 distinct units; `ResourceAmount` vs `Quota` as **unbridgeable types**; `checked_add` refusing cross-kind sums; `is_rate()`; `ResourceBundle` with no `total()`. |
| `nau-market/src/persistence.rs` + `Market::restore*` | **ADOPT** | Replay the ledger journal by `EntryKind` so balances are exactly restored; report records-expected vs records-restored. |
| `contracts/src/Settlement.sol` + `SettlementInvariant.t.sol` | **ADAPT** | Escrow/quorum/solvency invariant is directly reusable; `onlyOwner` governance (H7) must become programmatic. |
| Autonomous *pricing* and *budget* decisions | **MINE** | `resource.rs:39-43` says it explicitly: the module **does not price anything** — pricing rules are a separate unreleased plan item (D-05). Nothing here decides what something is worth. |

### v1.5 — Safety API
| Source | Stance | What exactly |
|---|---|---|
| `crates/nau-sandbox`: `capability.rs`, `spec.rs`, `executor.rs`, `manager.rs`, `component.rs`, `enforcement.rs` | **ADOPT** | The whole "enforced or it fails" architecture: `SandboxExecutor` port, `NullExecutor` as the run-nothing default, `Capabilities` published per backend, `PolicyNotEnforceable` naming the boundary, `PREMISES`, `Waivers` with reasons, startup sweep + no id reuse, per-sandbox lock instead of a global one. |
| `nau-plugin/src/runtime.rs` `Boundary` (15) | **ADOPT** | The boundary vocabulary, incl. `PmemSharedReadOnly`, `IoctlFilter` (`XFS_IOC_SWAPEXT` by name), `NetworkEgressAllowlist`, `PriorityClass`. |
| `nau-plugins/src/plugins/security/*` (6 bodies) | **ADOPT** | Six bodies with disjoint authority; observers hold no kernel capability; the police/tribunal split. |
| `nau-plugin/src/{blacklist,certify}.rs` | **ADAPT** | Keep the quarantine state machine, evidence, appeal, emergency broadcast, and the `Certification`-as-scope idea. Replace `ManualReview` (H2) with automatic decision + human override. |
| `nau-plugin/src/secure.rs` | **ADOPT** | The opt-in end-to-end channel the host cannot read, gated on `crypto:channel` and refused at ThirdParty — a rare correct treatment of "the host must not see this". |
| `nau-attest` | **ADAPT** | Keep the envelope/nonce/freshness/payload-binding verification and the Merkle inclusion proof; the grade ladder is honest (`MAX_ACHIEVABLE_GRADE`, `ChainNotImplemented`). |
| A **machine-facing** safety API (policy as data, decisions queryable, reasons structured) | **ADAPT** | `scripts/check-no-panics.mjs`, the refusal-code taxonomy and `verify-all.mjs`'s `NOT VERIFIED` vs `FAILED` split are the *shape* to keep; the *audience* changes from operator to peer. |

### v1.6 — Individual Learning
| Source | Stance | What exactly |
|---|---|---|
| `nau-agent/src/layered.rs` | **ADOPT** | Three memory tiers, each a verifiable `HashChain`; individual / group / generational; the generational head digest as the cross-generation pointer. |
| `nau-agent/src/{memory,experience,chain,transfer}.rs` | **ADOPT** | `AgentMemory`/`MemoryRecord`; `Experience`/`SharedMemory` with `MIN_SHARED_OBSERVATIONS` and `MAX_QUALITY_BPS`; `HashChain`; `TransferBundle`. |
| `nau-agent/src/provider.rs` + `llm.rs` | **ADOPT** | `ProviderKind` (OpenAI/Anthropic/Gemini shapes), `HttpProvider<T>` over the `nau_http::Transport` port, `ProviderProfile`; empty completions are typed errors, never panics. |
| `nau-market/src/reputation.rs` | **ADOPT** | The learning signal at the market level, with the anti-saturation fix (no `reward_honesty()` on every settlement). |
| `nau-core/src/domain/task.rs` `EvidenceGrade` | **ADOPT** | `verified > cpu-proto > unverified`, defaulting closed — the gate for *what counts as having learned something*. |
| Per-agent adaptation / weight updates / self-modification | **MINE** | Memory is append-only hash chains plus LLM calls. There is no model updating, no credit assignment, no per-agent policy. |

### v1.7 — Committee Governance
| Source | Stance | What exactly |
|---|---|---|
| `crates/nau-consensus` (whole crate) | **ADOPT** | `CommitteeSpec::new(n,f)` checked; `assign` forcing `members.len()==n` + dedupe; `cast` verifying signature **and** membership; `tally()` with `reason`/`equivocators`/`safety_violation`; equivocation voiding the round; `reset_for_next_round()` preserving the equivocation record. |
| `nau-market/src/service.rs` `arbitrate` | **ADAPT** | Replace `Actor`/`Authority::Operator` (H6) with a committee verdict that consumes a `TallyResult`. |
| `contracts/src/GovernanceToken.sol` | **ADAPT** | `Checkpoint`-based delegation and past-vote lookup are correct; `mint` being `onlyOwner` (H7) is not. |
| `nau-plugins/src/plugins/security/{tribunal,report,audit}.rs` | **ADOPT** | The write-policy / record / assess split, with the authority table asserted by test. |
| `nau-plugin/src/{certify,blacklist}.rs` review + appeal state machines | **ADAPT** | Reuse the state machine; replace the `ManualReview` human stage (H2) with committee decision. |
| Proposal lifecycle, voting windows, quorum policy as a *governed* parameter, execution of passed proposals | **MINE** | `Committee` tallies a vote on a task result. There is no proposal type, no epochs, no binding execution, no parameter governance. |

### v1.8 — Cross-Chain Settlement
| Source | Stance | What exactly |
|---|---|---|
| `nau-ledger` escrow + conservation + journal anchors | **ADOPT** | On-chain settlement must be *execution* of a ledger escrow, never a second book of account. The plan states the rule (`DEVELOPMENT-PLAN-v3.8-v3.9-Economy.md:62`): all on-chain settlement goes through `nau-ledger` escrow first; a reconciliation mismatch is **fail-closed**. |
| `contracts/src/Settlement.sol` | **ADAPT** | The reference implementation of the on-chain half: task escrow, quorum attestation, dispute resolution, credits, `totalObligations`/`isSolvent`. Reusable as the target ABI. |
| `contracts/src/{ReputationRegistry,AgentCardAnchor}.sol` | **ADOPT** | Reputation epochs and card anchoring, both verifier-gated with explicit stale/duplicate guards. |
| `contracts/script/Deploy.s.sol` + `contracts/VERSION` + the CI diff check | **ADOPT** | The deploy script reads `VERSION` and **refuses to deploy on a mismatch**. |
| `nau-plugins/src/plugins/{chain,http}.rs` | **ADAPT** | `sys.chain` is a *configuration contract* and explicitly sends nothing (`chain.rs:34`: "`sys.http` is the door that does it"); `sys.http` is one real GET. Payment is meant to be proxied by a **host signing service** so an in-sandbox agent only holds a capability token and **never a private key** (`DEVELOPMENT-PLAN-v3.8-v3.9-Economy.md:64`). |
| `nau-attest` Merkle inclusion proofs | **ADAPT** | The right primitive for "this happened on the other chain"; honest about not being ZK, not succinct, and not proving computation. |
| Actual cross-chain settlement | **MINE — genuinely absent.** | `ERC-8004, x402, L402, Lightning, Taproot, RGB, HTLC, USDC, ERC-4337, Paymaster` are **0 hits** across `crates/`, `contracts/src/` and `docs/`. There is no light client, no bridge, no relay, no HTLC, no stablecoin, no account abstraction. The reusable asset is the **refusal-by-name** plugin that answers all ten, plus the "settlement is execution, ledger is truth, mismatch is fail-closed" rule. |

### v1.9 — Network Scaling
| Source | Stance | What exactly |
|---|---|---|
| `crates/nau-libp2p` (whole crate, `--features libp2p`) | **ADOPT** | TCP + Noise + Yamux + Kademlia + GossipSub + Circuit Relay v2 + AutoNAT + DCUtR composed into a real swarm and adapted to the `nau_net::Transport` port. `naming.rs` namespaces DHT record keys (`nau/rec/…`) so this application cannot pollute the shared DHT. |
| `crates/nau-net`: `transport.rs`, `tcp.rs`, `topology.rs`, `relay.rs`, `stun.rs`, `nat.rs` | **ADOPT** | The `Transport` port with one real impl and one honestly-named double; `LayeredTopology` Lv1–Lv7 where room membership is a **pure deterministic function of node id** (insertion-order independent) and `route_hops` is derived, never constant; `RelayPool` with capacity enforced at insert and health mutable only through `record_failure`/`record_success`; real RFC 5389 STUN; RFC 4787 classification with `Unknown` as a first-class answer. |
| `crates/nau-erasure` | **ADOPT** | Systematic GF(256) Reed–Solomon, 504 erasure subsets exhaustively tested. **Carry the boundary:** no error correction — corrupt-but-not-declared-lost shards yield silently wrong data, so an upper-layer checksum/MAC is mandatory. |
| `crates/nau-image` | **ADOPT** | Content addressing, on-demand chunk reading with `ReadReport`/`Metrics`, P2P chunk exchange (`ChunkServer`/`PeerSource`), `VerifiedSource`, and **`SeedingRatio` as a measured value** rather than a claim. |
| `crates/nau-store` + `nau-migrate` | **ADOPT** | Persistence port and the migration tool, including its exactness rules (money by decimal text, never `f64`; refuse when not exactly representable). |
| `docs/METRICS-REPORT-v3.5.9.md` + `scripts/check-metric-claims.mjs` | **ADOPT** | A number may not be written into a document without its basis. This is the discipline that keeps a scaling claim from becoming marketing. |
| DCUtR hole-punching, real NAT classification, QUIC, DNS | **MINE / keep refused** | Declared unverified and honestly so: loopback hole-punch tests "would pass while testing nothing". A production network must either verify them on the real internet or keep them refused with a reason. |

---

## 11. Evidence-strength ledger

**Verified by reading code (this survey):** the crate inventory and LOC; every public API list in §3; the PMB types, checks, refusal codes and JSON encoding; the ABI frame grammar and version constants; the 12-state lifecycle and its single assignment site; the 21×5 capability matrix and its totals; the 14 sandbox boundaries and the `NullExecutor` default; the 25-plugin T0 roster and the 14-plugin official catalogue; the REST route table and the auth model; `Limits`/`Waivers`/`SandboxSpec` field-by-field; the Solidity function/event/error surface; `SUPPORTED_PROTOCOL_VERSIONS`; the CI job list and the `verify-all.mjs` contract; the absence of `todo!()`/`unimplemented!()`/`#[ignore]`; the 10 zero-hit economy nouns.

**Verified by execution (outside the tree):** `cargo +1.85.0 check --workspace --all-targets` → exit 0 in 3 m 25 s with zero warnings; the presence of 19 `.rlib`s and 17 `.exe`s in `target/debug/` built by rustc 1.85.0 / LLVM 19.1.7.

**Documented, unverified in code by this survey** — treat as claims to re-check before relying on them:
- All test *counts* (418 `nau-plugins`, 97 `nau-net`, 82+9 `nau-erasure`, 65 `nau-migrate`, 63 `nau-attest`, 223 JS, 34 `nau-agent` provider, 81 client assertions, 41 deployment checks, 19 deploy checks). No `cargo test` was run.
- "23 gates green" and every per-gate result in `docs/VERIFICATION.md`.
- The **cross-platform** claims: macOS and Linux runtime behaviour, three-platform *deployment*, and cross-target linking correctness. `docs/VERIFICATION.md:98` concedes that cross-target only type-checks and that a `#[link(name="kernel32")]` bug once passed every local gate while failing CI.
- AppArmor / eBPF behaviour (declaration-only in code), the Unix `setrlimit`/`setsid` backend (unverified on this Windows host), TLS execution.
- That `npm`/GitHub Packages releases at `v3.4.5` exist and match the stated sha256 (`DSH插件/安装指引.txt` asserts byte-equality; no network check was performed).
- `README.md`'s upstream-audit figures (648 files, 15 735 Rust lines, 76 defects, 52 `GAP §` hits in upstream).

**Known-stale / self-contradicting in-tree artifacts** (do not propagate):
- `docs/ARCHITECTURE.md`, `docs/VERIFICATION.md`, `docs/SELF-AUDIT.md`, `docs/GAP-ANALYSIS*.md`, `ATTRIBUTION.md`, `NOTICE` are V1.2.3-headed; `README.md` body is V3.2.1-headed; `VERSION` is 3.8.0.
- `docs/ARCHITECTURE.md:275` "not implemented" list is explicitly superseded by its own §7.1.
- `docs/VERIFICATION.md` still says "15 crates"; there are 19.
- `nau-plugins/src/lib.rs:129` says, of the **system-plugin tier**, "cannot be hot-plugged (`HOT_SWAP_SUPPORTED` is `false`)" while the constant in `nau-plugin/src/lib.rs:143` is `true`. Not a contradiction in the constant — the kernel's own doc at `nau-plugin/src/lib.rs:140-142` explains that `true` covers the routing table and lifecycle, and that a **system plugin is compiled into the kernel and cannot be replaced at all** (it is hot-*configurable*). Read as a stale cross-reference in the dependent crate's doc comment, not as a behavioural conflict.

---

*End of brief. The surveyed tree was not modified by this survey. The temporary `cargo check` target directory lives outside the tree.*

---

## Appendix — read-only compliance and one unexplained concurrent write

Every file in the surveyed tree was touched by `glob`/`grep`/`read` or by read-only PowerShell cmdlets (`Get-ChildItem`, `Get-Content`, `Select-String`, `Get-FileHash`, `[System.IO.File]::ReadAllBytes`). No `write`/`edit` call, no `New-Item`, no `Remove-Item`, no `cargo build`, and no `node` script was executed against the tree. `cargo +1.85.0 check` was run with `CARGO_TARGET_DIR` redirected to `E:\DS\_forangent\recon\_nau-target` (outside the tree), so not even `target/` was touched.

**One file nonetheless changed while this survey was running**, and it is recorded here rather than glossed over:

| | |
|---|---|
| Path | `conformance/vectors.json` (9 030 bytes — the cross-language conformance fixture) |
| Created | 2026-09-30 01:08:15 |
| **Last written** | **2026-10-04 13:48:11** — i.e. ~10 minutes into this survey |
| Current SHA-256 | `4F24CDD75FAA5B75A2A1F0139349278492B39EA77B29C70694B217DBE466343A` |
| Other tree files written after 13:44 | **none** (checked across `crates/`, `docs/`, `sdks/`, `contracts/`, `scripts/`, `.github/`, `.cargo/`, `conformance/`, `client/`, `integrations/dsh/`, and the repo root) |

This survey did not write that file, and no command it ran has a write path to it: the only scripts capable of regenerating it are `node conformance/generate.mjs` (which the survey never ran) and `scripts/verify-all.mjs` (likewise never run). The most likely explanation is a **concurrent process in the same shared workspace** — this session runs as a subagent, and the parent or a sibling task may have been running the project's own verification suite at that time. The practical consequence for this brief is nil: `vectors.json` is cited only as *existing* and as the mechanism CI uses to prove byte-equality; none of its contents are relied on above. It is flagged so that a later reader diffing the tree does not attribute the change to this survey.
