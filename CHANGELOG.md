# Changelog

## v2.0.2（2026-10-05）

**多平台交付级补齐 + 版本线缺陷修复。** 基线仍是 Rust 重写线（12 crate / 69,092 行），
本版不改内核语义，只把「可部署、可回滚、可验收」的缺口补齐，并修掉一个已发布的真实性缺陷。

### 新增
- `deploy/au4a-node.service` —— Linux systemd 单元。含 `ExecStartPre=au4a-node verify`（**自检不过不上线**）、
  `Restart=on-failure`、`NoNewPrivileges` / `ProtectSystem=strict` / `ReadOnlyPaths=/ /usr /etc` /
  `MemoryDenyWriteExecute` 等加固，与容器版的 `read_only + cap_drop ALL` 等价。
- `deploy/windows-service.md` —— Windows 10 服务化两种方案（内置 `sc.exe` 与 NSSM），含启动前自检包装脚本、
  日志轮转、防火墙、回滚与验收命令。
- `deploy/docker-compose.yml` + `.env.example` —— 单机 Docker 一键部署（只读根文件系统、丢全部 capability、
  健康检查直接用 `au4a-node verify`）。
- `docs/DEPLOY.md` —— 部署与回滚手册（含 7 条部署后验证、三层回滚、7 类常见报错→原因→修复）。
- `docs/ACCEPTANCE.md` —— 验收清单 A–F 六组，含**诚实清单**（明确列出未通过项）。

### 修复
- **已发布 tag 的真实性缺陷（P0）**：`v1.0.10` / `v1.1.10` / `v1.2.10` / `v1.3.10` / `v1.4.10` / `v1.5.10` / `v1.6.10`
  七棵版本树里混入了**当时尚不存在**的平台层 `crates/au4a-node` 与后期交付物（平台层到 v1.9.9 才逐版本快照，
  更早版本在重建时回落到工作树）。已**重新物化并强制移动**这 7 个 tag：移除当时不存在的 23 个路径，
  并同步从 workspace `members` 摘除 `crates/au4a-node`——**移除而非伪造**。主分支历史未改写。
- CI `version line` 随之恢复为对 **全部 10 个中版本收尾 tag**（v1.0.10 … v1.9.9）跑
  `cargo test --workspace --locked`，不再需要按 tag 收敛。
- 发布流水线两处假绿/环境缺陷：`|| echo` 吞掉真实发布失败、打包器写死 Windows 路径。
- 仓库同步脚本改为以远端实际 head 为父提交（此前用本地状态导致 `422 not a fast forward`）。

### 变更
- `crates/au4a-node` 全量实现（注册 / PMB / 场景编排 / 只读观察面板 / CLI）。
- `clippy` 作业在 CI 中显式标注为 **advisory**（照常真跑真报，暂不判红），清完即恢复硬门禁。

### 证据与验收
- `cargo test --workspace --locked` → **1238 passed / 0 failed**（96 个测试套件）
- `au4a-node verify` → **exit 0**（10 条轨道自检全 `[ok]`，213/213）
- 部署实测 10/10 中版本：构建成功、端到端成功、6 条只读路由全 200、写方法全 405
- 发布物：99 个 tag（v1.0.1 → v1.9.9）+ 10 个中版本 Release + npm `@twinsearth/au4a`（10 中版本 + 大版本线 `1.0.0`）+ GHCR 镜像

### 已知未完成（不粉饰）
- `clippy` 硬门禁未达成（advisory）；真实网络 / 真实链 / 真实并发未验证（相关能力证据等级 `cpu-proto`）。

---

## v1.9.9（2026-10-04）

10 个中版本 × 99 个小版本系列收尾：Autonomy / Capability Graph / Negotiation / Portable State /
Economic Autonomy / Safety API / Individual Learning / Committee Governance / Cross-Chain Settlement /
Network Scaling。`docs/DESIGN.md`（1197 行）与 `docs/DEV.md`（257 行）逐项覆盖 99 个小版本。
