# solo-new-api-rust

用 Rust 重写的轻量 [new-api](https://github.com/QuantumNous/new-api) 网关：**单用户、多 Token、直接复用现有 `one-api.db`**，可作为现成 new-api 部署的替代实现。前端复用 new-api 的 React 代码并按单用户场景裁剪，后端仅手写认证、选路重试、管理 API、日志统计与静态服务。

- 与 new-api 的功能差异、未复刻项和独有改进见 [docs/VS-NEW-API.md](docs/VS-NEW-API.md)
- 对齐的上游版本与升级同步流程见 [docs/UPSTREAM.md](docs/UPSTREAM.md)
- 内部设计、计划与逐轮验收记录保留在开发环境，不随开源仓库发布。

## 状态

- 功能已实现，并通过本地 Docker 验收；尚未发布正式版本，接口与配置仍可能调整。
- 本地门禁：`cargo test` 167 通过 / 1 忽略；前端 `bun run typecheck` 725 文件 0 错、`bun run test` 105 文件 1052 例。

## 上游对齐版本

- new-api：`e03439033238722c0b8d4437a052445f3aeeed83`（2026-11-06，`main`，`relaykit/v0.3.7-4-ge03439033`）
- Switchyard：`c365efe832822541024704aadf6a0d5c81cc0528`（`switchyard-translation` 0.3.0，vendored）
- 前端复制自 new-api `web/` 后裁剪改造；后端 `src/` 为独立实现。升级步骤、必须保留的改造点与门禁见 [docs/UPSTREAM.md](docs/UPSTREAM.md)。

## 定位

**是**：一个跑在单个进程里的 OpenAI/Anthropic 兼容网关，服务一个账号、多个客户端 Token，覆盖聊天、协议转换、Embedding、图像、音频、Rerank 等转发，附带渠道/令牌/日志/看板管理界面。

**不是**：多租户 API 平台。没有注册、用户管理、充值、订阅、兑换码、额度扣减、OAuth/2FA/Passkey、任务系统、Redis/多节点。

## 主要能力

**转发**

- OpenAI：`/v1/chat/completions`、`/v1/completions`、`/v1/embeddings`、`/v1/images/generations|edits`、`/v1/audio/transcriptions|translations|speech`、`/v1/rerank`、`/v1/moderations`、`/v1/models[/{model}]`。
- Anthropic：`/v1/messages`（含 `?beta=true`），与 OpenAI Chat 双向转换（流式 + 非流式）。
- OpenAI Responses：`/v1/responses`，与 Chat 双向转换。
- 同格式请求直接透传；跨协议走 vendored `switchyard-translation` 0.3.0。
- 流式 SSE 零缓冲转发；客户端断连立即取消上游；usage 增量提取，绝不缓冲整段流式响应。

**渠道路由**

- 分组（`default`/`vip` 等）匹配、优先级分层 + 权重抽样。
- 多 Key 轮询（进程内，不依赖 Redis）、429/5xx 重试与跨渠道切换（读取现有 `RetryTimes`，上限 10）。
- `model_mapping` 链式映射与循环保护、`header_override` 通配/正则/删除/占位符。
- 渠道测试、拉取上游模型、优先级拖拽重排、复制、`abilities` 一致性修复。

**认证与令牌**

- 会话：密码登录（argon2id `m=19456 KiB, t=2, p=1`，与 Go 侧哈希格式兼容）+ access JWT + refresh cookie，会话可查看/撤销。
- 客户端 Token：现有 `sk-...` 直接可用；支持分组、模型限制、过期/禁用；新建时可自填 16–64 位密钥（编辑不改密钥），留空生成 48 位。IP 白名单字段原样保存，但转发路径暂不校验。

**日志与看板**

- 复用 `logs` 表异步批量写入（每批最多 100 条），`quota=0`，只记 token 数/耗时/渠道/令牌等用量信息，不记金额。
- 概览、数据看板、桑基图（用户→令牌→分组→模型→渠道）全部按 `logs` 聚合。
- 日志导出、按时间分批清理（单批上限 5,000、总上限 50,000，规避 SQLite 绑定变量上限）。

**管理界面**

- 页面：登录、概览、数据看板、API 密钥、使用日志、渠道、模型、系统设置、个人资料、游乐场。
- 系统设置只暴露 8 个白名单键：站点名称/徽标/通知/服务器地址、用量日志开关、用户导出开关、最大导出跨度、生图模型；非白名单键原样保留、不覆盖。
- 游乐场统一为聊天界面：模型/分组选择器与普通对话一致，生图模型返回的图片直接显示在对话消息里；本地历史不保存 base64 图片，只留占位标记。

**运行与内存**

- SQLite WAL + 5s `busy_timeout`，只读现有表结构，不做迁移。
- 请求体默认上限 32 MiB（`MAX_REQUEST_BYTES`）、单条 SSE 事件 1 MiB、超大非流式响应走流式透传。
- glibc arena 调优 + argon2 并发限流（信号量 2）+ `spawn_blocking`，登录不再按 19 MiB × 并发抬高 RSS。
- 实测：空载约 23 MiB；12 路并发登录 24.8 MiB；24 路并发流式峰值 47.0 MiB 且结束后回落。

## 快速开始

依赖：Rust 1.96（见 `rust-toolchain.toml`）、Bun 1.4.4、Docker（本地验收/交叉编译用）。

```bash
# 1. 后端：指向一份现有 new-api 数据库（只读结构）
DB_PATH=/path/to/one-api.db \
JWT_SECRET_PATH=/path/to/.rust_jwt_secret \
WEB_DIST=./web/dist \
cargo run

# 2. 前端（另开终端）
cd web
bun install
bun run build        # 产物 web/dist，由后端静态托管
bun run dev          # 或开发服务器
```

首次启动会校验必需表/列；缺表直接退出并打印缺失项，不会改动数据库。

### 配置项

| 环境变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PORT` | `3000` | 监听端口 |
| `DB_PATH` | `/data/one-api.db` | SQLite 路径 |
| `WEB_DIST` | `./web/dist` | 前端静态目录 |
| `JWT_SECRET_PATH` | `/data/.rust_jwt_secret` | 会话签名密钥，缺失时自动生成 |
| `TRUST_PROXY_HEADERS` | `true` | 是否信任反代传来的客户端 IP（IP 白名单用） |
| `MAX_REQUEST_BYTES` | `33554432`（32 MiB） | 请求体硬上限，超限返回 `413` |

站点级配置通过管理界面或 `PUT /api/option` 写入 `options` 表（白名单见上）。

## 测试

```bash
cargo test                                   # 单元 + 集成（wiremock 模拟上游）
REAL_DB=/path/to/one-api.db cargo test -- --ignored   # 用真实库做 schema/缓存校验

cd web
bun run typecheck
bun run test
```

集成测试覆盖：现有库启动与读写兼容、Token 认证、渠道路由/优先级/重试、跨协议转换、断连取消、并发流式内存、日志导出与清理、系统设置白名单、生图桥接等。

## 构建与部署

```bash
bash deploy/build.sh     # linux/amd64 release + 前端构建 + 运行时镜像
bash deploy/package.sh   # docker save 归档，便于上传服务器
bash deploy/run-local.sh # 本地验收容器：:3006、--memory 450m、/tmp/solo-deploy-data
```

`deploy/run-local.sh` 需要一份 new-api 数据库快照 `data/one-api-consistent.db`（不入库），或通过 `SNAPSHOT=/path/to.db` 指定；首次运行会复制到 `/tmp/solo-deploy-data/one-api.db`。做无上游验收时把渠道 `base_url` 指到本地 mock（例如 `http://host.docker.internal:3007`），不要打真实上游。

部署到服务器时沿用原 Docker 服务约定：挂载 `./data:/data`、容器内端口 `3000`、`mem_limit 450m`、`wget` 健康检查。切换前备份数据库与原 compose/镜像；回滚即恢复备份并重启原服务。

## 目录

```
src/
  auth/        会话、Token 认证、密码哈希
  relay/       /v1 转发、协议转换、选路重试、生图桥接
  routes/      公开/自助/管理 API
  state.rs     users/tokens/channels/abilities/options 进程内缓存
  logging.rs   日志有界队列 + 批量写
  db.rs        SQLite 连接与 schema 校验
  settings.rs  options 白名单与校验
web/           new-api React 前端 fork（裁剪 + 单用户改造）
deploy/        交叉编译、打包、本地/服务器运行脚本
tests/         集成测试（wiremock）
vendor/        switchyard-translation（Apache-2.0，固定版本）
```

## 兼容性红线

1. 直接使用现有 `one-api.db`，不修改表结构、不重命名列、不做自动迁移。
2. 未知列与 JSON 字段（`channels.other`、`settings`、`setting`、`param_override`、`header_override`、`channel_info`）读写原样保留。
3. 现有 `sk-...` Token 继续可用；新日志可被 Go 版本与现有查询界面读取（`quota=0`）。
4. 不引入 Redis、多节点、任务系统、计费引擎。

## 已知限制

- `param_override` 只保留数据，不参与请求改写（现有部署中该字段为空）。
- 性能健康按 `perf_metrics` 小时桶聚合（请求数、成功率、平均耗时、吞吐）；TTFT 未采集，头部延迟以总耗时近似。
- 管理端错误本地化只覆盖三条密钥错误，其余管理 API 错误仍显示英文原文。
- 多 Key 轮询下标不落库，进程重启后从 0 开始；`quota_data`、用户 `used_quota/request_count` 不写。
- 生图桥接每个请求只取第一张图；上游响应超过 4 MiB 时日志 usage 记 0（客户端拿到的 usage 正确）。
- Token 的 `allow_ips` 未在转发路径校验，只在管理页保存/回读。

更完整的差异清单见 [docs/VS-NEW-API.md](docs/VS-NEW-API.md)。

## 许可

本仓库为个人重写。`web/` 复制自 new-api（AGPL-3.0），继续遵循其许可；`vendor/switchyard-translation` 为 Apache-2.0。分发前请自行确认上游许可义务。
