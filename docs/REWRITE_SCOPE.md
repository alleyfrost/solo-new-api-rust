# New API Rust 重写：范围与兼容性矩阵

> 状态：设计草案（待设计评审）
> 更新日期：2026-11-06
> 源项目：`/Users/alleyfrost/projects/github/new-api`（Go，AGPL-3.0）
> 目标：个人自用 Rust 网关，单用户、多 Token、与线上 SQLite 数据无缝切换

## 1. 定位与原则

- **单用户自用**：不做多租户、注册、充值、额度拦截。
- **保留多 Token 管理**：现有 11 个 Token 的认证、分组、模型限制、IP 白名单、过期/禁用语义保持可用。
- **外部兼容，不照搬内部实现**：兼容对外 HTTP 行为与 SQLite 数据；不要求复刻 Redis、缓存结构、计费引擎、任务系统等内部机制。
- **优先复用成熟组件**：HTTP 栈用 axum/hyper/tokio，协议转换用 NVIDIA Switchyard 的 `switchyard-translation`（Apache-2.0），前端复用 new-api React 并裁剪，避免手写协议转换。
- **面向并发**：请求热路径不落库、SSE 零缓冲转发、连接池复用、日志异步批量写入。

## 2. 已确认决策

| 编号 | 决策 | 说明 |
| --- | --- | --- |
| D1 | 单用户、保留多 Token | 去掉多租户/充值/额度管理/兑换码/二次验证 |
| D2 | 复用 new-api 前端并裁剪 | 只保留登录、Token、渠道、日志、用量看板、模型 |
| D3 | 只记用量不记费用 | `logs` 继续写 token 数/耗时/渠道等，`quota=0`；不做费用与额度扣减 |
| D4 | 协议尽量全覆盖 | OpenAI 全套 + Anthropic Messages + Responses；不含 Gemini 原生/MJ/任务/插件 |
| D5 | 部署：本地编译后上传 | 本地交叉编译 linux/amd64，上传产物，替换现有 Docker 服务 |
| D6 | 不引入 Redis | 多 Key 轮询、渠道选择、令牌缓存均放进程内；单实例部署 |
| D7 | 不做内部机制复刻 | Key 轮询只需行为等价，不要求 Redis/多节点一致性 |

## 3. “无缝切换”的验收边界

1. 直接使用现有 `/data/one-api.db`，启动后不修改表结构、不重命名列。
2. 客户端使用现有 `sk-...` Token 可继续调用，无需换 Key。
3. `/v1/chat/completions`、`/v1/messages`、`/v1/models` 的请求/响应/SSE 行为与现网一致。
4. 渠道、分组、优先级、权重、多 Key、429 重试策略可用，能继续从 SenseNova 上游取数。
5. 新产生的消费日志可被现有表结构与查询界面读取（费用字段为 0）。
6. 旧服务可随时回滚：切换期间只增量写数据，不破坏 Go 版本可读性。

## 4. 功能矩阵

状态说明：`已实现` = 已完成并验证；`待实现` = 计划复刻；`不复刻` = 明确不做；`待评估` = 需进一步确认。

### 4.1 数据与认证

| 功能 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| SQLite 同库同 schema | 复刻 | 待实现 | `/data/one-api.db`，WAL |
| `users` / `tokens` / `channels` / `abilities` / `options` 读取 | 复刻 | 待实现 | 未知列/JSON 原样保留 |
| `logs` 消费日志写入 | 复刻 | 待实现 | `type=2`，`quota=0`，记录 token/耗时/渠道/Key 索引 |
| `quota_data` 统计 | 待评估 | 待评估 | 看板可直接从 `logs` 聚合，避免双写 |
| `user_sessions` 会话持久化 | 复刻 | 待实现 | 管理员登录会话 |
| 多 Token 管理 | 复刻 | 待实现 | 状态/过期/分组/模型限制/IP 白名单 |
| 单用户密码登录（argon2id） | 复刻 | 待实现 | 复用现有 hash |
| 多租户/用户管理/注册 | 不复刻 | 不复刻 | 个人使用 |
| OAuth / 邮箱验证 / 2FA / Passkey | 不复刻 | 不复刻 | 按需求去掉 |
| 额度扣减 / 充值 / 订阅 / 兑换码 / 邀请 / 签到 | 不复刻 | 不复刻 | 不做额度拦截与支付 |
| Redis 缓存 / 多节点同步 | 不复刻 | 不复刻 | 改为进程内缓存 |
| 密码登录加密（RSA） | 不复刻 | 不复刻 | 现网未启用；TLS 即可 |

### 4.2 转发协议

| 端点 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| `POST /v1/chat/completions` | 复刻 | 待实现 | 流式/非流式，OpenAI 兼容上游透传 |
| `POST /v1/messages`（含 `?beta=true`） | 复刻 | 待实现 | Anthropic↔OpenAI Chat 转换 |
| `POST /v1/responses` | 复刻 | 待实现 | Responses↔Chat 转换（现网无真实流量） |
| `POST /v1/messages/count_tokens` | 不复刻 | 不复刻 | 现网一直 404，保持 404 |
| `GET /v1/models`、`GET /v1/models/:model` | 复刻 | 待实现 | 按分组/Token 限制聚合 |
| `POST /v1/embeddings` | 复刻 | 待实现 | 透传 |
| `POST /v1/images/generations`、`/v1/images/edits` | 复刻 | 待实现 | 透传 |
| `POST /v1/audio/transcriptions`、`translations`、`speech` | 复刻 | 待实现 | multipart/二进制透传 |
| `POST /v1/rerank` | 复刻 | 待实现 | 透传 |
| `POST /v1/completions`、`/v1/moderations` | 复刻 | 待实现 | 透传 |
| Gemini 原生 `/v1beta/*` | 不复刻 | 不复刻 | 现网未使用 |
| Realtime WebSocket / Responses WebSocket | 不复刻 | 不复刻 | 现网未使用 |
| Midjourney / Task / Video / Suno / JS 插件 | 不复刻 | 不复刻 | 现网未使用 |
| Files / Fine-tunes | 不复刻 | 不复刻 | 现网未使用 |

### 4.3 渠道与路由

| 功能 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| 分组匹配（default/vip） | 复刻 | 待实现 | Token 分组决定可用渠道 |
| 优先级 + 权重选择 | 复刻 | 待实现 | 按 `abilities`/`channels` |
| 多 Key polling | 复刻 | 待实现 | 进程内轮询，不依赖 Redis |
| 单 Key 禁用/恢复 | 复刻 | 待实现 | 认证失败等场景 |
| 429/5xx 重试与跨渠道切换 | 复刻 | 待实现 | `RetryTimes=10`，流式开始后不重试 |
| 模型映射 `model_mapping` | 复刻 | 待实现 | 现网为空，读字段即可 |
| Header 覆盖/透传 | 复刻 | 待实现 | 现网用 `Authorization: Bearer {api_key}` + 通配透传 |
| Param 覆盖 | 待实现 | 待实现 | 现网为空 |
| 上游模型同步 / 余额查询 / Ollama 拉取 / VLLM 状态 | 不复刻 | 不复刻 | 管理页裁掉 |
| 标签批量操作 / 优先级拖拽排序 | 不复刻 | 不复刻 | 保留基础编辑即可 |
| 多上游适配器（Azure/Vertex/Bedrock/Gemini/…） | 不复刻 | 不复刻 | 只做 OpenAI 兼容 + Anthropic 直连 |

### 4.4 管理界面（复用 new-api React 并裁剪）

| 页面 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| 登录页 | 复刻 | 待实现 | 账号密码 |
| Token 管理 | 复刻 | 待实现 | 列表/新建/编辑/删除/批量/查看 Key |
| 渠道管理 | 复刻（核心） | 待实现 | 列表/新建/编辑/删除/启用/测试/多 Key |
| 用量日志 | 复刻 | 待实现 | 列表/筛选/统计/导出 |
| 用量看板 | 复刻 | 待实现 | 按 logs 聚合 token/请求量 |
| 模型管理 | 复刻（基础） | 待实现 | 列表/启停/基础 CRUD |
| 系统设置 | 不复刻 | 不复刻 | 配置走环境变量/少量 DB option |
| 用户管理 / 钱包 / 订阅 / 兑换码 | 不复刻 | 不复刻 | 单用户 |
| 安全（2FA/Passkey）/ 个人资料 | 不复刻 | 不复刻 | 只保留登录/登出 |
| 审计日志 / 系统信息 / 性能指标 | 不复刻 | 不复刻 | 保留 `/api/status` 健康检查 |
| 定价 / 排行榜 / 聊天 / Playground | 不复刻 | 不复刻 | 裁掉 |
| 任务插件 / MJ 日志 / 任务日志 | 不复刻 | 不复刻 | 裁掉 |

### 4.5 运维

| 功能 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| `/api/status` 健康检查 | 复刻 | 待实现 | 兼容现有 Docker healthcheck |
| 优雅停机 | 复刻 | 待实现 | SSE 最长等待，可配置 |
| 本地交叉编译 + 上传 | 复刻 | 待实现 | linux/amd64，参考现有 rust:bookworm 配方 |
| Docker 服务替换 | 复刻 | 待实现 | 保留 `./data:/data`、端口 20128→3000 |
| 回滚到 Go 版本 | 复刻 | 待实现 | 数据只增量写，保留旧镜像 |
| Prometheus/复杂可观测性 | 不复刻 | 不复刻 | 结构化 tracing 日志即可 |
| 多节点/集群 | 不复刻 | 不复刻 | 单实例 |

## 5. 线上现状（用于验收）

- 数据库：`one-api.db`（231 MB，WAL），已拷贝到本地 `data/`（不入 Git）。
- 用户：1 个 root（`alleyfrost`），角色 100。
- Token：11 个（10 启用、1 禁用），全部 `unlimited_quota=1`；分组 `default`/`vip`。
- 渠道：3 个
  - #1 Anthropic（type 14），11 Key，已手动禁用；
  - #3 OpenAI 兼容（type 1），12 Key，启用，多 Key polling；
  - #4 OpenAI 兼容（type 1），5 Key，启用，多 Key polling。
- 上游：`https://token.sensenova.cn`（OpenAI 兼容；无 Key 请求返回 401，服务可达）。
- 现网流量（约 30 天）：
  - `POST /v1/messages`（含 `?beta=true`）：约 24,500 次成功；
  - `POST /v1/chat/completions`：约 18,200 次成功；
  - `/v1/responses`：519 次全部 401（扫描流量）；
  - `/v1/messages/count_tokens`：202 次全部 404；
  - Web 面板管理接口有持续使用。
- 上游 429 极频繁：现网依赖最多 10 次重试 + 多 Key + 跨渠道切换。
- 日志：132,523 条，其中消费日志（type=2）43,522 条。

## 6. 待评估事项

1. `quota_data` 是否继续写：若看板接口直接从 `logs` 聚合，可省掉双写与聚合任务。
2. 多 Key 轮询状态是否落库：进程内轮询即可；可选择性回写 `channel_info.multi_key_polling_index` 保持回滚兼容。
3. 日志批量写入的批次/间隔：默认 100 条或 200ms，压测后调整。
4. 前端静态资源：嵌入二进制 vs 目录部署；倾向目录部署，便于单独更新前端。
5. Switchyard 转换结果的 new-api 语义差异：用现网请求样本做黄金用例比对。
6. `accessed_time`/`used_quota` 是否更新：倾向只更新 `accessed_time`，不改额度字段。

## 7. 里程碑

1. M1 基础骨架 + 数据层 + 认证（可读现有库、Token 可认证）
2. M2 转发核心 + 渠道选择/重试（chat/completions、messages 可用）
3. M3 管理 API + 前端裁剪（可登录并管理 Token/渠道/日志/看板/模型）
4. M4 全套端点 + 兼容性/并发测试 + 本地数据验收
5. M5 本地交叉编译、上传切换、回滚演练

## 8. 相关任务

任务清单由协作任务系统维护（项目脚手架、数据层、渠道选择、认证重试、转发核心、协议转换、全套端点、管理认证、管理 API、前端裁剪、日志统计、兼容测试、性能验证、部署切换、文档）。
