# solo-new-api-rust 设计

- 日期：2026-11-06
- 状态：已确认方案 A，进入实现
- 关联：[REWRITE_SCOPE](../../REWRITE_SCOPE.md)、[AGENTS](../../../AGENTS.md)
- 源码锚点：new-api `e03439033238722c0b8d4437a052445f3aeeed83`；Switchyard `c365efe832822541024704aadf6a0d5c81cc0528`

## 1. 目标与非目标

### 目标
- 单进程 Rust 网关，读写现有 `/data/one-api.db`，与线上无缝切换。
- 保留多 Token 管理、渠道/分组/优先级/权重/多 Key/429 重试。
- 转发 OpenAI 全套端点 + Anthropic Messages + Responses。
- 复用 new-api React 前端并裁剪为单用户页面。
- 高并发、流式零缓冲、内存可控，规避上游已知流式内存缺陷。

### 非目标
- 多租户/注册/充值/订阅/兑换/邀请/签到/额度扣减/计费。
- Redis/多节点/任务系统/MJ/视频/Suno/JS 插件/Gemini 原生/Realtime。
- 系统设置页/用户管理页/审计页/性能指标页/Playground/聊天。
- 渠道高级功能（余额、上游模型同步、Ollama 拉取、VLLM/SGLang 状态、标签批量、优先级拖拽、Azure/Vertex/Bedrock 适配器）。

## 2. 兼容性契约

1. 直接使用现有 SQLite，不改表结构、不重命名列、不做自动迁移；启动校验必需表/列，缺失则退出码 2。
2. 未知列与 JSON 字段（`channels.other/settings/setting/param_override/header_override/channel_info`）读写时原样保留。
3. 客户端现有 `sk-...` Token 可继续使用，鉴权行为不变。
4. `/v1/chat/completions`、`/v1/messages`、`/v1/models` 的请求/响应/SSE 行为与现网一致。
5. 新产生的 `logs` 行可被现有查询与界面读取（`quota=0`）。
6. 回滚到 Go 版本只需换镜像；数据仅增量写，不破坏旧版可读性。

## 3. 总体架构

```
客户端 ──HTTP──> axum/hyper 服务
   │  公共中间件：RequestId、CORS(relay)、日志、限流(登录)
   │  /v1/*    -> TokenAuth -> 路由解析 -> 渠道选择/重试 -> 上游转发 -> 响应/SSE
   │  /api/*   -> 管理认证(JWT+会话) -> REST 处理器 -> DB
   │  /        -> 静态前端(ServeDir + SPA fallback)
   └─ 状态层：内存缓存(tokens/channels/abilities/options/users) + 进程内 Key 轮询
   └─ 写层：sqlx SQLite(WAL)；日志 mpsc 批量写
```

单进程、单实例、无 Redis。所有缓存与轮询在进程内，行为等价即可。

## 4. Crate 结构与依赖

单一 Cargo workspace（先单 crate `solo-new-api-rust`，必要时拆子模块）。

模块：
- `config`：环境变量与默认值。
- `db`：连接池、schema 校验、仓储。
- `state`：内存缓存、刷新、失效。
- `routing`：分组/优先级/权重/多 Key/重试决策。
- `auth`：Token 认证、管理登录、会话、JWT。
- `relay`：`/v1/*` 处理器、端点分发、请求改写、流式。
- `translate`：对 Switchyard 的薄封装与同格式透传选择。
- `usage`：增量 usage 提取与估算。
- `logging`：异步批量写 `logs`。
- `admin`：`/api/*` REST。
- `web`：静态资源与 SPA。
- `error`：统一错误与响应映射。

依赖（均成熟、Apache/MIT）：
- `tokio`（multi_thread）、`axum` 0.8、`hyper` 1、`tower-http`（ServeDir、Compression、CORS、RequestDecompression、限流）。
- `reqwest` 0.13（rustls、stream、gzip）、`hyper` 用于流式 body。
- `sqlx` 0.8（sqlite、runtime-tokio-rustls）。
- `serde`/`serde_json`（preserve_order）、`bytes`、`futures`/`futures-util`。
- `argon2` 0.5、`jsonwebtoken`、`uuid`、`rand`、`sha2`、`regex`、`tiktoken-rs`（fallback 估算）。
- `tracing`、`tracing-subscriber`、`humantime`。
- `switchyard-translation` 0.3.0 + `switchyard-protocol` 0.3.0：**vendored**（`vendor/`），MSRV 补丁为 1.96.0；Apache-2.0，保留 LICENSE/NOTICE。
- 开发/测试：`wiremock`、`pretty_assertions`、`tokio-test`。

工具链：Rust 1.96.0（`rust-toolchain.toml` 锁 `1.96.0`）。Switchyard 0.3.0 在 1.96.0 vendored 后已实测编译通过；官方 1.96.1 toolchain 安装失败，故不依赖。

## 5. 配置

| 环境变量 | 默认 | 说明 |
| --- | --- | --- |
| `PORT` | 3000 | 监听端口 |
| `DB_PATH` | /data/one-api.db | SQLite 文件 |
| `WEB_DIST` | ./web/dist | 前端静态目录 |
| `JWT_SECRET_PATH` | /data/.rust_jwt_secret | HS256 密钥文件，缺失则生成 0600 |
| `MAX_REQUEST_BYTES` | 67108864 (64MB) | 请求体上限 |
| `MAX_NONSTREAM_RESPONSE_BYTES` | 33554432 (32MB) | 非流式响应捕获上限，超限透传不解析 usage |
| `MAX_FRAME_BYTES` | 1048576 (1MB) | 单个 SSE 帧累积上限 |
| `UPSTREAM_CONNECT_TIMEOUT_MS` | 10000 | |
| `UPSTREAM_NONSTREAM_TIMEOUT_MS` | 600000 | |
| `UPSTREAM_STREAM_IDLE_TIMEOUT_MS` | 300000 | 流式无数据超时 |
| `SHUTDOWN_TIMEOUT_SECONDS` | 120 | 等待在途流式结束 |
| `TRUST_PROXY_HEADERS` | true | 用 X-Forwarded-For 取客户端 IP |
| `CACHE_REFRESH_SECONDS` | 60 | 全量刷新缓存周期 |
| `LOG_QUEUE_CAPACITY` | 20000 | 日志队列容量 |
| `LOG_BATCH_SIZE` | 100 | |
| `LOG_FLUSH_MS` | 200 | |
| `LOGIN_RATE_LIMIT` | 10/300s/IP | |

`RetryTimes` 优先读 DB `options.RetryTimes`，缺失用 3。

## 6. 数据层

### 6.1 连接
- `sqlx::SqlitePool`：读池 `max_connections=8`；写池 `max_connections=1`。
- 启动执行：`journal_mode=WAL`、`synchronous=NORMAL`、`busy_timeout=5000`、`foreign_keys=ON`、`temp_store=MEMORY`。
- 不自动建表；仅执行 PRAGMA。

### 6.2 表与读写策略

| 表 | 读 | 写 | 说明 |
| --- | --- | --- | --- |
| `users` | 是 | 否 | 单用户 root；仅认证 |
| `tokens` | 是 | 是 | CRUD、`accessed_time`；不改额度字段；删除为软删（`deleted_at`） |
| `channels` | 是 | 是 | CRUD；保存未知列与 JSON 字段；硬删除 |
| `abilities` | 是 | 是 | 渠道变更时重建本渠道相关行 |
| `options` | 是 | 否 | 读取 `RetryTimes`、`SelfUseModeEnabled` 等 |
| `logs` | 是 | 是 | 插入 `type=2` 消费日志；`quota=0` |
| `user_sessions` | 是 | 是 | 登录会话（与 Go 同 schema） |
| `models` | 是 | 是 | 基础 CRUD；不自动同步上游 |
| `vendors` | 是 | 是 | 基础 CRUD |
| `quota_data` | 否 | 否 | 不写；看板直接聚合 `logs` |

### 6.3 启动 schema 校验
必需表与关键列：`users(id,username,password,role,status,group,auth_version)`、`tokens(id,user_id,key,status,expired_time,group,unlimited_quota,deleted_at,allow_ips,model_limits,model_limits_enabled,auto_groups,cross_group_retry)`、`channels(id,type,key,status,models,group,base_url,priority,weight,setting,param_override,header_override,channel_info,settings,other_info,model_mapping,auto_ban,tag)`、`abilities(group,model,channel_id,enabled,priority,weight,tag)`、`options(key,value)`、`logs(全部现网列)`、`user_sessions(sid,user_id,status,refresh_hash,expires_at,version,user_auth_version)`、`models(id,model_name,status,vendor_id)`、`vendors(id,name)`。缺失则报错退出。

### 6.4 渠道变更与 abilities 同步
- 创建/更新/删除渠道时，事务内：删除该 `channel_id` 的 `abilities`，按 `channels.models`（逗号分隔）与 `channels.group`（逗号分隔分组）笛卡尔插入：`(group, model, channel_id, enabled=channel.status==1 && model in models, priority, weight, tag)`。
- 优先级/权重取渠道字段（NULL → 0）。

## 7. 内存缓存与并发

### 7.1 缓存
- `TokensByKey: DashMap<String, TokenRow>`；`UsersById: DashMap<i64, UserRow>`；`Options: RwLock<HashMap>`；`Channels: RwLock<Vec<ChannelRow>>`；`Abilities: RwLock<HashMap<(group,model), Vec<AbilityRow>>>`。
- 启动全量加载；每 `CACHE_REFRESH_SECONDS` 全量刷新；管理端变更后立即局部刷新（受影响 token/channel/abilities）。
- 单进程内并发安全；无 Redis。

### 7.2 渠道选择算法
1. 模型名：客户端请求模型（同格式取 `model`；转换请求取转换后模型；若 `model_mapping` 命中，用映射后模型路由）。
2. 分组：`token.group`；若为 `auto` 且 `auto_groups` 非空，按列表依次尝试。
3. 候选：`abilities[group,model]` 中 `enabled=true` 且对应 `channels.status=1`。
4. 优先级分层：按 `priority` 去重降序；重试第 r 次取第 `min(r, len-1)` 层。
5. 层内加权随机：`weight`（NULL→0；全 0 时均匀）；每请求维护已用 `(channel_id,key_index)` 集合，优先未用。
6. 多 Key 轮询：`channel.key` 按换行拆分；跳过 `channel_info.multi_key_status_list` 中禁用与进程内 `disabled_until` 未到的 Key；索引 `AtomicUsize` 轮询，行为等价 polling。
7. 选中后写 `channel_id/channel_type/channel_name/key_index` 到请求上下文。

### 7.3 重试算法
- `max_retry = min(options.RetryTimes, 10)`，缺失 3。
- 响应未开始（未发任何字节）前才重试。
- 决策：
  - 2xx：成功。
  - 408/429/5xx：重试；429 立即换 Key/渠道；5xx 退避 `100ms*attempt + jitter`，上限 1s。
  - 401/403：标记当前 Key `disabled_until=now+10min`（进程内），换 Key/渠道重试；无可用 Key 则换渠道。
  - 400/404/413/422/451/其他 4xx：停止，透传错误。
  - 客户端断开：终止。
- 不自动禁用渠道行（不复刻 `auto_ban` 写库）；仅在进程内临时标 Key。
- 最多尝试 `max_retry+1` 次；失败后返回最后一次错误。

## 8. 认证

### 8.1 转发 Token
- 提取 Key：`Authorization: Bearer <key>`、`x-api-key`（`/v1/messages`、`/v1/models`）、`?key=`（`/v1/models`、`/v1beta/openai/models`）。去掉 `Bearer `/`bearer `，去 `sk-` 前缀，按 `-` 切分取首段（兼容旧 Key 形态）。
- 校验：Key 存在、`status=1`、未过期（`expired_time=-1` 或 > now）、用户 `status=1`。
- `unlimited_quota` 全为 true，不做额度校验；`remain_quota` 不拦截。
- `allow_ips` 非空时校验 CIDR（`cidr` crate）。
- `model_limits_enabled` 时校验请求模型在 `model_limits` 列表。
- 写上下文：`user_id`、`token_id`、`token_group`、`cross_group_retry`。

### 8.2 管理登录与会话
- `POST /api/user/login?turnstile=`：body `{username,password}`；argon2id 校验；失败统一消息、按 IP 限流。成功创建 `user_sessions` 行（`sid=uuid`、`refresh_hash=sha256(secret)`、`status=active`、`expires_at=now+7d`、`login_method=password`、`ip`、`user_agent`）。
- 返回前端 `AuthBundle`：`{access_token, token_type:"Bearer", access_expires_at, user:{id,username,display_name,role,status,group,...}, session:{sid,current:true,login_method,ip,user_agent,created_at,last_active_at,expires_at}}`。
- access_token：HS256 JWT，claims `{sub:user_id, sid, ver:session.version, uver:user.auth_version, iat, exp}`，TTL 15m。
- refresh：HttpOnly Cookie `new_api_refresh=sid.secret`（SameSite=Lax，Secure 当 `X-Forwarded-Proto=https`，Path=/，Max-Age=7d）；附 `new_api_has_session=1`。
- `POST /api/user/auth/refresh`：校验 cookie + 可选 `X-Auth-Session`；轮换 secret（`previous_refresh_hash` 宽限 60s）；返回新 bundle 与 cookie。
- `POST /api/user/auth/logout`：撤销会话，清 cookie。
- 管理中间件：JWT 校验 + `user_sessions` 仍有效（`status=active`、`expires_at>now`、`version`、`user_auth_version`）+ 用户 `status=1`。
- `password_login_encryption_enabled` 固定返回 false；不支持加密登录。

## 9. 转发端点与转换

### 9.1 端点矩阵

| 客户端格式 | 路径 | 上游 OpenAI 兼容(type1) | 上游 Anthropic(type14) |
| --- | --- | --- | --- |
| OpenAI Chat | /v1/chat/completions | 透传 | 翻译 chat→messages |
| OpenAI Chat | /v1/completions | 透传 | 翻译 |
| OpenAI Chat | /v1/moderations | 透传 | 不支持 |
| Embeddings | /v1/embeddings | 透传 | 不支持 |
| Images | /v1/images/generations,/edits | 透传 | 不支持 |
| Audio | /v1/audio/* | 透传 | 不支持 |
| Rerank | /v1/rerank | 透传 | 不支持 |
| Anthropic | /v1/messages | 翻译 messages→chat | 透传 |
| Responses | /v1/responses | 翻译 responses→chat | 翻译 responses→messages |
| Models | /v1/models,/v1/models/:model | 聚合返回 OpenAI 形 | 聚合返回 Anthropic 形（按头） |

`/v1/messages/count_tokens` 返回 404（与现网一致）。其余未列端点返回 404。

### 9.2 转换策略
- 同格式：直接透传请求体与响应/SSE，不进入翻译；仅做模型映射/header 改写。
- 跨格式：用 `switchyard_translation::TranslationEngine::default()`：
  - 请求：`translate_request(source, target, &json, &policy)`；policy 默认 `AllowWithDiagnostics`，记录 lossy 到日志 `other.diagnostics`。
  - 非流式响应：`translate_response`。
  - 流式：上游字节流 → `decode_stream` → `encode_stream`（或 `translate_event`）逐事件转换并即时下发；**不累积完整响应**。已核验 Switchyard `decode_stream` 仅按帧缓冲，内存常量。
- 上游格式判定：渠道 `type` ∈ {1,3,20,43,47,48,57} → OpenAI Chat；`type=14` → Anthropic Messages。

### 9.3 请求改写
- Base URL 规范：去尾 `/`；若以 `/v1` 结尾，拼接去掉 `/v1` 的路径；否则拼接 `/v1` + 路径。Anthropic type 拼接 `/v1/messages`。
- Header：
  - 移除 hop-by-hop：`Connection, Keep-Alive, Proxy-*, TE, Trailer, Transfer-Encoding, Upgrade, Host, Content-Length, Authorization, X-Api-Key`。
  - 应用 `header_override` JSON：`"*":true` 转发所有剩余头；`"re:<regex>":true` 转发匹配；`<name>:"<value>"` 设置（支持 `{api_key}`、`{client_header:<name>}`、`null` 删除）。正则按渠道缓存编译。
  - 默认设置 `Authorization: Bearer <上游key>`、`Content-Type` 保留、`Accept-Encoding: identity`（流式不压缩）。
- Body：
  - JSON 端点：读取为 `Bytes`（≤64MB），解析取 `model`/`stream`；若 `model_mapping` 命中或需 `stream_options`，最小化修改后重序列化；否则转发原始字节。OpenAI 上游流式时合并 `stream_options.include_usage=true`。
  - multipart/audio：缓冲 ≤64MB，扫描 `name="model"` 字段取模型路由；透传原始字节。
  - 非 JSON（图片二进制响应等）：透传。
- 模型映射：`model_mapping` JSON map；命中则替换请求 `model` 并以原模型名路由与日志记录（`other.is_model_mapped`、`upstream_model_name`）。

### 9.4 响应与 SSE
- 同格式：原样转发状态码、头（过滤 hop-by-hop）、字节流；按需 tee 到 usage 提取。
- 跨格式：状态码映射（错误用目标格式）；SSE 事件即时转换下发。
- 流式不设总超时，按 `UPSTREAM_STREAM_IDLE_TIMEOUT_MS` 监控无数据；非流式总超时。
- 客户端断开：drop 上游响应流 → reqwest 自动取消；日志记 `other.stream_status={status:"aborted"}` 与已获取 usage。
- 请求/响应 ID：接受或生成 `X-Request-Id`，响应头返回；记录上游 `x-request-id`。

### 9.5 错误格式
- OpenAI：`{"error":{"message","type","param","code"}}`。
- Anthropic（`/v1/messages`）：`{"type":"error","error":{"type","message"}}`。
- 上游同格式错误透传状态码与体；跨格式映射。本地错误（无渠道、超时、413）按目标格式构造。

## 10. 用量日志与统计

### 10.1 增量 usage 提取（禁止缓冲完整响应）
- 优先上游 `usage`：OpenAI 非流式从响应体；流式从末尾 `usage` chunk（已强制 `stream_options.include_usage`）。
- 流式用量 Tap：在转发字节流旁挂有界状态机，逐 SSE 帧解析 JSON，仅保留 usage 字段与计数；帧缓冲 ≤ `MAX_FRAME_BYTES`；丢弃内容。Anthropic 上游从 `message_start`/`message_delta` 的 `input_tokens`/`output_tokens`/`cache_read_input_tokens`。
- 缺失时估算（标记 `other.usage_estimated=true`）：prompt 用 `tiktoken-rs`（o200k_base）对请求 messages 估算（≤64MB）；completion 用增量 Unicode 字符数/4。
- 非流式响应 > `MAX_NONSTREAM_RESPONSE_BYTES` 或非 JSON：透传，usage 记 0。

### 10.2 日志写入
- 字段：`user_id, created_at=now, type=2, content=null, username, token_name, model_name, quota=0, prompt_tokens, completion_tokens, use_time=秒, is_stream, channel_id, channel_name, token_id, group, ip, request_id, upstream_request_id, other`。
- `other`（JSON）：`{admin_info:{is_multi_key,multi_key_index,use_channel,usage_billing_path:"upstream",request_path}, cache_tokens, cache_creation_tokens, stream_status:{status,end_reason}, usage_estimated, request_conversion:[...]}`。
- 队列：`tokio::sync::mpsc` 容量 `LOG_QUEUE_CAPACITY`；worker 每 `LOG_BATCH_SIZE` 或 `LOG_FLUSH_MS` 批量插入事务。满时 `send().await` 超时 2s 后丢弃并告警计数。关停时 flush 最多 5s。
- 不更新 `tokens.used_quota`/`users.used_quota`；仅更新 `tokens.accessed_time`（批量或随日志异步）。

### 10.3 看板聚合
- `/api/data`、`/api/data/self`：按时间桶（`default_time`：hour/day）与 `model_name` 聚合 `logs` type=2：`created_at=桶起点, model_name, token_used=prompt+completion, count, quota=0`。
- `/api/data/users`：按 `user_id`/`username` 聚合。
- `/api/data/flow`、`/api/data/flow/self`：按 `user/token/group/model/channel` 聚合，返回 `FlowQuotaDataItem`。
- 查询强制时间范围，默认 30 天、上限 90 天；利用现有 `idx_created_at_*` 索引。

## 11. 管理 REST API

响应包：成功 `{success:true, message:"", data:...}`；失败 `{success:false, message, code?}` + HTTP 状态。

公共：`GET /api/status`、`GET /api/notice`（空）、`GET /api/group`、`GET /api/user/self`、`GET /api/user/self/groups`、`GET /api/user/models`。

认证：`POST /api/user/login`、`POST /api/user/auth/refresh`、`POST /api/user/auth/logout`。

Token（与前端 `features/keys` 契约）：
- `GET /api/token/?p&size`
- `GET /api/token/search?keyword&token&p&size`
- `GET /api/token/:id`
- `GET /api/token/auto-groups`
- `POST /api/token/`
- `PUT /api/token/`
- `PUT /api/token/?status_only=true`
- `DELETE /api/token/:id/`
- `POST /api/token/batch`
- `POST /api/token/:id/key`
- `POST /api/token/batch/keys`

渠道（核心子集，与裁剪后 `features/channels` 契约）：
- `GET /api/channel`（分页/筛选）、`GET /api/channel/search`、`GET /api/channel/:id`
- `POST /api/channel`、`PUT /api/channel`、`DELETE /api/channel/:id`
- `POST /api/channel/batch`、`PUT /api/channel/?status_only=true`、`POST /api/channel/status/batch`、`DELETE /api/channel/disabled`
- `GET /api/channel/models`、`GET /api/channel/models_enabled`、`GET /api/channel/default_base_urls`
- `GET /api/channel/test/:id`（异步测试上游）
- `POST /api/channel/:id/key`、`POST /api/channel/multi_key/manage`
- 高级端点（`priority/order`、`fix`、`ollama/*`、`vllm/*`、`upstream_updates/*`、`prefill_group`）返回 404，前端入口删除。

日志：`GET /api/log`、`/api/log/self`、`/api/log/stat`、`/api/log/self/stat`、`GET /api/log/export`、`/api/log/self/export`（JSONL 流 + 结尾 `{success,data:{count}}`）。

看板：`/api/data`、`/api/data/self`、`/api/data/users`、`/api/data/flow`、`/api/data/flow/self`。

模型：`GET /api/models/`、`GET /api/models/search`、`GET /api/models/:id`、`POST/PUT /api/models/`、`PUT /api/models/?status_only=true`、`DELETE /api/models/:id`、`POST /api/models/delete`、`GET /api/models/missing`、`GET /api/vendors/`（基础 CRUD）。`deployments/*`、`sync_upstream/*` 返回 404。

响应字段以 pinned 前端类型为契约；实现时用 `features/*/types.ts` 作为源，补契约测试。

## 12. 前端裁剪

- 复制 `web/`（new-api 提交 `e034390`）到本项目 `web/`，锁定版本。
- 保留路由：`/sign-in`、`/dashboard`（仅用量统计区段）、`/keys`、`/channels`、`/usage-logs`、`/models`（metadata+vendors，删 deployments）。
- 删除路由与 feature 目录：home、pricing、rankings、about、legal、oauth、register/reset/otp、setup、chat、playground、wallet、subscriptions、redemption-codes、users、task-plugins、security、profile、system-settings、system-info、audit、perf-metrics、MJ/task 日志。
- 简化布局导航与 `routeTree.gen.ts`（由 TanStack Router 插件在构建时重生成）。
- 保留 i18n、主题、登录/登出流程；移除 2FA/Passkey/加密登录分支。
- 构建：`bun install && bun run build`（本机 bun 1.4.4 已验证可构建全量产物；裁剪后体积下降）。
- 服务：`tower-http::services::ServeDir` 指向 `WEB_DIST`，SPA fallback 到 `index.html`；`/static/*` 设 `Cache-Control: immutable, max-age=31536000`，`index.html` 设 `no-cache`；启用 `CompressionLayer`（gzip/br）。
- 前端与后端同源部署，管理接口不开 CORS。

## 13. 流式与内存安全（硬性）

- 禁止缓冲完整流式响应：转发与转换均逐帧/逐事件；usage Tap 常量内存。
- 请求体/非流式响应/翻译中间态均有硬上限；超限返回 413/502 并记录，不 OOM。
- 客户端断连立即取消上游；禁止 `tokio::spawn` 后台读取上游；日志只记已获得部分。
- 所有队列有界：日志 mpsc、上游连接池、翻译状态。
- 验收（见 §15）：长流与大批量后 RSS 回落；断连后无泄漏任务/FD。

## 14. 性能目标

- 200 并发 SSE（上游 1KB/s、60s）：RSS < 300MB 且结束后回落；无随请求数持续上涨。
- 1000 rps 短非流式（mock 上游）：p99 网关开销 < 20ms。
- 10 万条日志：队列不超 1000、对请求延迟影响 < 5ms。
- 50MB 流式响应：RSS 增量 < 10MB。
- tokio multi_thread，worker 线程 = CPU 核数；reqwest 连接池 `pool_max_idle_per_host=64`、`tcp_nodelay=true`。

## 15. 测试

- 单元：Token 解析/校验、渠道选择分层/权重、重试决策、header_override、SSE usage Tap、转换黄金用例。
- 集成（本地 DB 副本）：启动→`/api/status`→登录→Token/渠道/日志/看板接口；写一条 log 后用 `sqlite3` 校验结构与 `PRAGMA integrity_check`。
- 黄金转换：从 new-api `relaykit/relayconvert/testdata` 取 Claude↔Chat 用例，比对语义 JSON。
- Mock 上游：`wiremock` 模拟 429/5xx/401 与 SSE，验证多 Key 轮询、跨渠道重试、断连取消。
- 内存/并发：自写压测（`hey`/`oha` 或 axum 测试客户端）跑 §14 场景。
- 断连测试：100 客户端中途断开；检查上游连接取消、无 detached 任务、RSS 稳定。
- 真实上游冒烟：需用户明确授权后用测试 Token 跑 `/v1/chat/completions` 与 `/v1/messages`。
- 前端：构建产物 + 本地浏览器冒烟（chrome_devtools/browser-use）覆盖登录与各保留页面。

## 16. 部署与回滚

- 构建：
  - 前端：本机 `cd web && bun install && bun run build` → `web/dist`。
  - Rust：本机 `docker run --platform linux/amd64 -v $PWD:/src -v cargo-cache:/root/.cargo rust:1.96.0-bookworm` 编译 `cargo build --release --target x86_64-unknown-linux-gnu`。
  - 镜像：`debian:bookworm-slim` + `ca-certificates` + `tzdata` + 二进制 + `web/dist`；`EXPOSE 3000`；`HEALTHCHECK` 调 `/api/status`。
- 上传：`docker save solo-new-api-rust:<tag> | ssh aliyun docker load`；旧镜像打 `rollback` tag。
- 切换：备份 `data/one-api.db*`（停旧容器后拷贝三文件或 `sqlite3 checkpoint`）；更新 compose `image`；`docker compose up -d`；健康检查；冒烟；观察日志。
- 回滚：compose 改回旧镜像 tag 并 `up -d`；数据兼容（仅增量 logs）。
- 不允许新旧进程同时打开同一 SQLite。

## 17. 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| Switchyard pre-alpha | vendored 固定 commit；MSRV 补丁；黄金用例 + 真实冒烟；必要时手工补转换分支 |
| SenseNova thinking/429 怪癖 | 授权后真实冒烟；429 行为按现网对齐；thinking 字段透传或转换测试 |
| 前端 API 契约漂移 | 前端锁定提交；契约测试以 `features/*/types.ts` 为源 |
| SQLite 写争用 | WAL + 单写 + 批量；必要时日志落独立 DB（兼容 `LOG_SQL_DSN` 思路） |
| 模型元数据为空 | `/api/models/missing` 从 abilities 计算；UI 现有导入按钮落库 |
| 内存回归 | §13/§14 自动化内存与断连测试纳入 CI 本地门 |

## 18. 里程碑与任务映射

- M1 骨架+数据+认证：任务 #1 #2 #8（部分）
- M2 转发核心+选路/重试：#3 #4 #5 #6
- M3 全套端点+管理 API+前端：#7 #9 #10 #11
- M4 测试+并发+兼容验收：#12 #13
- M5 构建+部署+回滚：#14
- 文档贯穿：#15

## 19. 引用
- new-api 源码：`/Users/alleyfrost/projects/github/new-api` @ `e034390`
- Switchyard：`/tmp/switchyard` @ `c365efe`（vendored 到 `vendor/`）
- 线上数据快照：`data/one-api.db`（不入 Git）
