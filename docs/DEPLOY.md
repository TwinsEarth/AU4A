# AU4A 部署与回滚手册（v1.9.9 基线）

> 部署事实先讲清：**AU4A 没有数据库、没有缓存、没有中间件**。状态完全在进程内
> （`au4a-core::Ledger` 整数守恒账本 + `au4a-kernel` 注册表），人类只能通过只读 HTTP 面板观察。
> 因此「部署」= 编译一个二进制并运行它；「回滚」= 换回上一个 tag/镜像。没有迁移脚本，也没有数据丢失风险。

## 1. 系统要求

| 组件 | 版本 | 用途 | 是否必需 |
|---|---|---|---|
| Rust | **1.88.0+**（CI 用 1.88.0 实机验证 MSRV） | 编译 12 个 crate | 源码部署必需 |
| Cargo | 随 Rust | 构建/测试 | 源码部署必需 |
| Docker | 24+ | 容器部署 | 可选 |
| Docker Compose | v2 | 一键编排 | 可选 |
| Node.js | 18+ | 仅工具链（文档生成/打包/发布脚本） | 可选 |

端口：默认 **8787**（只读观察面板，容器内固定 8787）。
环境变量：见 [`.env.example`](../.env.example)（只有 `AU4A_PORT` / `AU4A_AGENTS` / `AU4A_IMAGE` / `AU4A_CONTAINER`）。

## 2. 从零部署（源码方式，任何平台）

```bash
# 1) 取代码：以 v1.9.9 为唯一基线
git clone https://github.com/TwinsEarth/AU4A.git
cd AU4A
git checkout v1.9.9

# 2) 构建（锁定依赖；只构建节点二进制）
cargo build --release --locked -p au4a-node

# 3) 部署前自检：10 条轨道自检必须全绿（非 0 退出即不要上线）
./target/release/au4a-node verify          # Linux/macOS
# .\target\release\au4a-node.exe verify    # Windows

# 4) 端到端跑一遍（8 个 Agent 自主运行，确定性）
./target/release/au4a-node run --agents 8 --json > run.json

# 5) 起服务：Agent 自主跑 + 人类只读面板
./target/release/au4a-node run --agents 8 --observe 0.0.0.0:8787
```

## 3. 从零部署（Docker / Compose）

```bash
cp .env.example .env
docker compose -f deploy/docker-compose.yml build      # 本地构建
# 或直接用 GHCR 上已发布的镜像（Publish Packages 工作流推送）
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml ps
```

容器特性（`deploy/docker-compose.yml`）：`read_only: true`、`cap_drop: ALL`、`no-new-privileges`、
健康检查直接用 `au4a-node verify`。这三点合起来是「**只跑内存、人类只能读**」的可执行证明。

## 4. 可选：Nginx（TLS 终止 + 显式只放行 GET）

`deploy/docker-compose.yml` 末尾附了完整 server 片段。要点：`limit_except GET { deny all; }`
——观察层自身已经「非 GET 一律 405」，Nginx 是第二道门。

## 5. 运行验证（部署后必须逐条通过）

| # | 命令 | 预期输出 |
|---|---|---|
| 1 | `au4a-node version` | `au4a-node v1.9.9` |
| 2 | `au4a-node verify` | 退出码 **0**；10 条轨道自检全 `[ok]` |
| 3 | `au4a-node run --agents 8 --json` | 退出码 0；JSON 里 `tracks_total=10`、`tracks_ok=10` |
| 4 | `curl -fsS http://127.0.0.1:8787/api/progress` | HTTP 200，JSON 含 `"read_only":true` |
| 5 | `for r in / /api/progress /api/results /api/revenue /api/selfcheck /api/tracks; do curl -s -o /dev/null -w "$r %{http_code}\n" http://127.0.0.1:8787$r; done` | 6 行全部 **200** |
| 6 | `curl -s -o /dev/null -w '%{http_code}\n' -X POST http://127.0.0.1:8787/api/revenue` | **405**（写路径不存在，不是被禁用） |
| 7 | `curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8787/api/nope` | **404** |

## 6. 回滚方案

三层，任选，全部无需数据迁移（因为无持久化状态）：

### 6.1 代码级回滚（源码部署）
```bash
git fetch --tags
git checkout v1.8.10          # 上一中版本收尾点（或任一已验证 tag）
cargo build --release --locked -p au4a-node
./target/release/au4a-node verify && ./target/release/au4a-node run --agents 8 --observe 0.0.0.0:8787
```

### 6.2 镜像级回滚（容器部署）
```bash
# 用不可变 digest 回滚，避免 tag 被移动带来的歧义
docker image ls ghcr.io/twinsearth/au4a --digests
AU4A_IMAGE=ghcr.io/twinsearth/au4a@sha256:<上一个 digest> \
  docker compose -f deploy/docker-compose.yml up -d
```
镜像标签约定：`1.9.9`（精确版本）、`1.0.0`（v1 大版本线）、`latest`、`sha-<commit>`。**回滚请用 digest 或精确版本号，不要用 `latest`。**

### 6.3 npm 包回滚
```bash
npm install @twinsearth/au4a@1.8.10     # 精确版本，避开 latest
npx @twinsearth/au4a@1.8.10 version
```

### 6.4 回滚验收（每次回滚后必做）
```bash
au4a-node version           # 必须显示回滚目标版本
au4a-node verify            # 必须退出码 0
curl -s -o /dev/null -w '%{http_code}\n' -X POST http://127.0.0.1:8787/api/revenue   # 必须 405
```

## 7. 常见报错 → 原因 → 修复

| 报错 | 原因 | 修复 |
|---|---|---|
| `error: package ... requires rustc 1.88` | 工具链过旧 | `rustup toolchain install 1.88.0 && cargo +1.88.0 build --release -p au4a-node` |
| `error: failed to select a version ... --locked` | `Cargo.lock` 与 `Cargo.toml` 不一致 | 用仓库提交的 lock：`git checkout v1.9.9 -- Cargo.lock` |
| `cannot start observer on 0.0.0.0:8787: address already in use` | 端口被占 | `AU4A_PORT=8788` 或用 `--observe 127.0.0.1:0`（系统分配端口） |
| `verify` 非 0 退出但 `cargo test` 全绿 | 你在 **v1.0.10–v1.6.10** 这些早期 tag 上跑 verify（当时平台层尚未逐版本快照，自检树混入后续版本） | 用 `v1.7.10 / v1.8.10 / v1.9.9`，或只跑 `cargo test`；详见 `docs/VERIFICATION.md` 诚实清单 |
| `docker compose up` 报 `read-only file system` | 容器根文件系统只读，而某进程试图写盘 | 这是设计意图；AU4A 运行期不写盘。若你加了自定义写入，请挂 `tmpfs` |
| GHCR `denied: permission_denied` | 拉取私有包未登录 | `echo $CR_PAT \| docker login ghcr.io -u <user> --password-stdin` |

## 8. 升级流程（不可变 + 可回滚）

1. `git fetch --tags` → `git checkout v1.9.9`；
2. `cargo test --workspace --locked` 与 `au4a-node verify` 全绿；
3. 构建/拉取新镜像，**先起在备用端口**（`AU4A_PORT=8788`）跑第 5 节 7 条验证；
4. 切换端口/流量；观察 `/api/progress` 与 `/api/revenue` 两个只读面板 5 分钟；
5. 异常则按第 6 节回滚（digest 或精确版本），无需数据恢复。
