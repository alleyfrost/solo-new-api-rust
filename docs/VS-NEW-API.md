# 与 new-api 的区别

对比对象：

- 上游：new-api（Go，AGPL-3.0），前端 fork 锚点 `e03439033238722c0b8d4437a052445f3aeeed83`
- 本项目：`solo-new-api-rust`（Rust，单用户自用裁剪版）

## 一句话结论

同一份 SQLite 数据库、同一套客户端 Token、同一组转发端点的**单用户子集**：保留网关最常用的转发、渠道、令牌、日志与看板；去掉多租户、计费、任务系统与 Redis；在流式内存、生图调用方式和部署形态上做了自己的取舍。

## 定位对比

| 维度 | new-api | solo-new-api-rust |
| --- | --- | --- |
| 使用者 | 多用户平台，面向对外售卖/团队 | 单账号自用，保留多 Token |
| 计费 | 额度、充值、订阅、兑换码、分组倍率 | 不记金额、不扣额度，只写用量（`quota=0`） |
| 状态存储 | SQLite/MySQL/PostgreSQL + Redis | 只用现有 SQLite，进程内缓存 |
| 部署 | 可多节点、多种适配器与插件 | 单实例 Docker |
| 前端 | 完整控制台 | 复用并以单用户场景裁剪 |
| 配置面 | options 表覆盖大量运行策略 | 11 个白名单键，其余原样保留不读不写 |

## 已复刻

### 数据与认证

| 能力 | 说明 |
| --- | --- |
| SQLite 同库同 schema | 直接读现有 `one-api.db`，WAL + 5s `busy_timeout`；启动校验必需表/列，不改结构、不迁移 |
| 用户与密码登录 | argon2id `m=19456 KiB, t=2, p=1`，与 Go 侧哈希格式兼容；会话 JWT + refresh cookie，可查看/撤销会话 |
| 多 Token 管理 | 列表/搜索/新建/编辑/删除/批量；分组、模型限制、过期/禁用；查看密钥。IP 白名单字段原样保存，转发时不校验 |
| 客户端密钥兼容 | 现有 `sk-...` 直接可用；新建 Token 可自填 16–64 位密钥（写库前剥掉 `sk-` 前缀），编辑不改密钥 |
| 未知字段保留 | `channels.other`、`settings`、`setting`、`param_override`、`header_override`、`channel_info` 读写原样保留 |

### 转发端点

| 端点 | 行为 |
| --- | --- |
| `/v1/chat/completions` | 流式/非流式透传；注入 `stream_options.include_usage` 收集用量；断连取消上游 |
| `/v1/messages` | Anthropic ↔ OpenAI Chat 双向转换（流式 + 非流式），`?beta=true` 兼容 |
| `/v1/responses` | Responses ↔ Chat 双向转换 |
| `/v1/models`、`/v1/models/{model}` | 按分组与 Token 模型限制聚合 |
| `/v1/embeddings` | JSON 透传，记录 usage |
| `/v1/images/generations`、`/v1/images/edits` | JSON/multipart 透传，从 body 提取 `model` 选路 |
| `/v1/audio/transcriptions`、`translations`、`speech` | multipart 提取 `model`；speech 二进制回传 |
| `/v1/rerank`、`/v1/completions`、`/v1/moderations` | JSON 透传，前两者记录 usage |

### 渠道与路由

| 能力 | 说明 |
| --- | --- |
| 分组匹配 | Token 分组决定可用渠道（如 `default`/`vip`） |
| 优先级 + 权重 | 优先级分层后按权重无放回抽样 |
| 多 Key 轮询 | 进程内轮询，不依赖 Redis |
| 单 Key 熔断 | 401/403/429 后一段时间内跳过该 Key |
| 重试与切换 | 读取现有 `RetryTimes`（上限 10），429/5xx 重试并跨渠道切换；响应已开始时不再重试 |
| 模型映射 | `model_mapping` 链式映射 + 循环保护 |
| Header 覆盖 | 通配/正则/删除/占位符；Anthropic 渠道用 `x-api-key` |

### 管理界面与接口

| 页面/接口 | 说明 |
| --- | --- |
| 登录 | 账号密码，注册/找回密码入口已删 |
| 概览 / 数据看板 | 读 `quota_data` 小时聚合（写日志时累计、启动时回填）；金额口径全部改为用量口径（Token、次数、耗时） |
| API 密钥 | 含自定义密钥、批量操作、查看明文密钥 |
| 使用日志 | 列表/筛选/统计/用户详情/导出/按时间分批清理 |
| 渠道 | 列表/编辑/启停/查看密钥/测试/拉取模型/优先级拖拽/复制/一致性修复 |
| 模型 | 列表/编辑/启停/软删除；渠道派生模型并集、绑定渠道与分组（无模型广场） |
| 系统设置 | 站点信息 + 日志维护（保留天数） + 转发并发 + 生图模型 + new-api 迁移（11 个白名单键） |
| 个人资料 | 基本资料、改密码、会话管理 |
| 游乐场 | `/pg/chat/completions` 复用同一套选路与转发 |
| 运维 | `/api/status` 健康检查、SIGINT/SIGTERM 优雅停机（含日志落盘） |

## 未复刻

| 类别 | 具体项 | 原因 |
| --- | --- | --- |
| 多租户 | 注册、用户管理、用户额度、钱包 | 单用户 |
| 认证扩展 | OAuth、邮箱验证、2FA、Passkey、RSA 登录加密 | 本项目未使用；TLS + argon2id 足够 |
| 计费 | 充值、订阅、兑换码、邀请、签到、额度扣减、分组倍率 | 只记用量不记金额 |
| 存储中间件 | Redis、多节点同步、分布式锁 | 单实例进程内缓存 |
| 任务与插件 | Midjourney、Task、Suno、视频、JS 插件、Realtime/Responses WebSocket | 本项目未使用；依赖任务系统 |
| 其他协议 | Gemini 原生 `/v1beta/*`、Files、Fine-tunes、`/v1/messages/count_tokens` | 本项目未使用；`count_tokens` 与上游当前行为一致保持 404 |
| 上游适配器 | Azure、Vertex、Bedrock、Gemini 等专用适配器 | 只保留 OpenAI 兼容 + Anthropic 直连 |
| 渠道运维 | 上游模型自动同步、余额查询、Ollama 拉取、VLLM 状态、标签批量操作 | 依赖 Go 任务系统/Redis，管理页一并裁剪 |
| 请求策略 | 参数覆盖执行、限流、敏感词、SSRF 防护、令牌数量上限 | `param_override` 只保留数据不生效；路由与重试沿用内置策略 |
| 可观测性 | 审计日志、系统信息、Prometheus、复杂性能指标 | 结构化 tracing 日志 + `/api/status` |
| 前端页面 | 用户管理、钱包、订阅、兑换码、公告/FAQ/API 信息/运行时间卡片、模型广场、看板用户统计 | 单用户/未实现，入口与开关一并删除 |

## 独有 / 增强

| 项 | new-api 或上游实现 | 本项目 |
| --- | --- | --- |
| chat → 生图桥接 | 纯生图模型（如 `sensenova-u1.5-lite`）收到 chat 请求会报错 | `options.ImageModels` 白名单命中的模型，自动把最后一条 user 文本转成 `/v1/images/generations`，把图片包成 chat completion（流式退化为单块 SSE）返回 |
| 游乐场生图呈现 | 上游 Playground 只做聊天，纯生图模型需要在外部直接调 `/v1/images/generations` | 统一为普通聊天界面：模型/分组选择器不变，生图模型返回的图片直接出现在对话消息中；历史只保留占位标记，避免 7 MB 级 base64 撑爆 localStorage |
| 流式内存纪律 | 上游存在为统计 completion tokens 缓冲整段流式的已知缺陷（部分 issue 仍 Open） | 只保留一个 SSE 事件的缓冲，usage 增量提取；断连立即取消上游；队列有界（日志 20,000）、请求体 32 MiB、SSE 事件 1 MiB、超大非流式响应改走流式透传 |
| new-api 迁移 | 无 | 红色迁移按钮：可选在线备份，清除历史 `request_policy`/计费字段，重建 `quota_data` 并 VACUUM |
| 大请求体背压 | 无显式闸门，并发大请求线性占用内存 | 默认 2–8 MiB 最多 8 并发、>8 MiB 最多 4 并发，系统设置可调；大块缓冲 mmap 后直接归还；日志批、连接池与空闲堆都有上界 |
| 登录内存 | Go 版本按需分配通过 GC 归还 | glibc `M_MMAP_THRESHOLD=16 MiB`、`M_TRIM_THRESHOLD=128 KiB`、`M_ARENA_MAX=2`，KDF 放 `spawn_blocking` 且信号量限 2；12 路并发登录从 303 MiB 降到 24.8 MiB |
| 无 Redis 的轮询与缓存 | Redis 承担多 Key 轮询、令牌缓存、跨节点一致性 | 进程内缓存；`channel_info.multi_key_mode` 的 `random` / `polling` 都在进程内完成，轮询游标加锁后 O(1) 推进，不落库、重启从第一个 Key 开始（不影响正确性） |
| 用量口径 | 余额/金额列贯穿日志与看板 | 日志 `quota=0`，前端金额列替换为 Token/耗时/次数；`quota_data` 与用户 `used_quota` 不写 |
| 设置白名单 | 大量 options 可写 | 只允许 8 个键，读取按白名单合成、缺行回落默认；其余行不覆盖 |
| 日志清理 | 按行绑定，批次过大时触及 SQLite 变量上限 | 单语句子查询分批删除，默认批次 5,000、上限 50,000；默认保留 14 天，启动后自动清理 |
| 品牌与信息架构 | 官方命名与图标 | 蓝紫新月+星芒图标；侧边栏「聊天/游乐场」置顶；「模型调用分析」改名「数据看板」；概览删除四张恒空卡 |

## 兼容与迁移

- **数据库**：直接使用现有 `one-api.db`；不建表、不改列、不迁移。切换前备份数据库与原 compose/镜像。
- **Token**：现有 `sk-...` 无需更换；密钥格式与校验规则不变。
- **日志**：新写入按 Go 版可读的形状落 `logs`（`type=2` 消费 / `type=5` 错误、`quota=0`、`request_id=req_*`）；`other` 内含 `cache_tokens` 与 `admin_info`（Key 序号、重试链），Go 版本回滚后列表与统计可读。
- **回滚方式**：恢复切换前的数据库备份与 compose/镜像，重新启动原 Go 服务。兼容性验收需确认：`PRAGMA integrity_check` 通过、表列与索引无差异、Rust 期间增量写入的日志可被 Go 版本读取。

## 已知行为差异与限制

1. `param_override` 只做数据保留，转发时不应用（现有部署中该字段为空）。
2. `/api/perf-metrics/summary` 从现有 `perf_metrics` 小时桶聚合，新的请求/成功/耗时/输出 token 由日志批增量写入；TTFT 未采集，头部延迟以总耗时近似，不做假数据。
3. 管理端错误本地化只覆盖三条密钥错误，其余管理 API 错误仍显示英文原文（`message_key` 支持已具备，尚未逐个补齐）。
4. 多 Key 轮询下标不落库，重启从 0 开始；`quota_data`、用户 `used_quota/request_count` 不更新（登录后个人资料的用量卡改为从 `logs` 统计）。
5. 生图桥接只处理 **chat 请求**（`/v1/chat/completions` 与游乐场），且只取第一张图；`/v1/messages` 打到生图模型不会桥接。
6. 生图/大响应口径：上游图片响应超过 4 MiB 时走流式透传、不解析 usage，日志 token 记 0（客户端收到的 usage 仍正确）；桥接单次最多整包读取 32 MiB，超限返回 502。
7. 流式 SSE 单事件上限 1 MiB；超过会终止该流并记录错误，防止单个畸形事件撑爆内存。
8. 上游未返回 usage 时日志记 0，不做 token 估算。
9. 单实例：不支持多节点共享缓存/轮询状态；进程重启会重新加载缓存。
10. 前端仍保留 new-api 的部分英文 key 与管理端文案；7 个语言文件按本仓库实际用到的 key 维护。
11. Token 的 `allow_ips` 只在管理页保存与回读，转发路径未做 IP 校验；上游 new-api 会按该字段拒绝请求。若现有 Token 依赖该字段，应在 `token_auth` 中补校验后再切换。

## 验证摘要（本地 Docker，2026-11-07）

- 启动读现有库快照、`/api/status`、SPA 回退、健康检查通过。
- Token 认证：现有库中的 `sk-` key 直接 200，错误 key 401。
- 转发：非流式 usage 与日志正确；Chat 流式 2.69 MB / 4001 事件零缓冲；Anthropic/Responses 跨协议转换可用；24 路并发流式 24/24 完成、峰值 47.0 MiB 后回落；客户端断连后上游 0.09 s 内取消。
- 限制：70 MB 请求返回 413（默认上限 32 MiB）。
- 登录：12/12 并发成功、错误密码 6/6 401，内存 24.8 MiB。
- 生图：真实上游一次 `sensenova-u1.5-lite` 调用 200、22.3 s、2048×2048；chat→生图桥接返回 `chat.completion` 内嵌图片；游乐场统一对话内联显示，刷新显示本地历史提示。
- 门禁：`cargo test` 167 通过 / 1 忽略；`bun run typecheck` 725 文件 0 错；`bun run test` 105 文件 1052 例。

## 上游与维护

- 当前对齐 new-api `e03439033238722c0b8d4437a052445f3aeeed83`（2026-11-06，`main`，`relaykit/v0.3.7-4-ge03439033`）；前端 `web/` 以此为基线裁剪，后续升级按文件比对而非直接覆盖。
- `switchyard-translation` 固定 0.3.0（Switchyard 锚点 `c365efe832822541024704aadf6a0d5c81cc0528`），以 vendored 形式放在 `vendor/`，升级时同步 golden 转换测试。
- 升级步骤、必须保留的本地改造与门禁清单见 [UPSTREAM.md](UPSTREAM.md)；升级完成后同时更新该文的对齐表与本节。
- 兼容性任何改动都应先跑 `cargo test` 与本地 Docker 验收；内部验收表保留在开发环境，不随仓库发布。
