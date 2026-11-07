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
| D2 | 复用 new-api 前端并裁剪 | 只保留登录、Token、渠道、日志、用量看板、模型、个人资料、Playground 与裁剪后的系统设置（站点信息 + 日志维护） |
| D3 | 只记用量不记费用 | `logs` 继续写 token 数/耗时/渠道等，`quota=0`；不做费用与额度扣减 |
| D4 | 协议尽量全覆盖 | OpenAI 全套 + Anthropic Messages + Responses；不含 Gemini 原生/MJ/任务/插件 |
| D5 | 部署：本地编译后上传 | 本地交叉编译 linux/amd64，上传产物，替换现有 Docker 服务 |
| D6 | 不引入 Redis | 多 Key 轮询、渠道选择、令牌缓存均放进程内；单实例部署 |
| D7 | 不做内部机制复刻 | Key 轮询只需行为等价，不要求 Redis/多节点一致性 |
| D8 | 只保证外部兼容 | 不逐行移植 Go 实现；接口行为、数据可读性、故障转移结果等价即可 |
| D9 | 不复刻上游流式内存缺陷 | 禁止缓冲完整流式响应统计 token；断连即取消上游；队列有界、内存可控 |

## 2.1 可行性验证（2026-11-06）

| 项目 | 结果 | 说明 |
| --- | --- | --- |
| 前端独立构建 | 通过 | 在 `/tmp` 用 bun 1.4.4 + rsbuild 构建成功，未裁剪产物 68 MB |
| 协议转换库 | 通过 | `switchyard-translation` 0.3.0 源码 vendored 后在 Rust 1.96.0 编译通过；Anthropic Messages → OpenAI Chat 请求转换实测可用 |
| Rust 1.96.1 | 受阻 | 官方 toolchain 安装报 “Missing manifest”；0.3.0 仅补丁级要求，vendored 后可降到 1.96.0 |
| 上游可达性 | 通过 | `https://token.sensenova.cn` 可达，无 Key 请求 `/v1/models` 返回 401 |

## 2.2 上游已知缺陷（禁止复刻）

来自 new-api v1.0.0-rc.47（2026-10-25）的已知问题，Rust 实现必须规避：

| 上游问题 | 上游状态 | 本项目的处理 |
| --- | --- | --- |
| 流式 `/v1/responses` 大输入上下文内存升至 4 GB，输出结束后不回落 | 已关闭，`not_planned`；建议设 GOMEMLIMIT | 不缓存完整响应；请求体/非流式响应设硬上限；流式零缓冲 |
| 流式转发为统计 completion tokens 把完整响应缓冲在内存 | 仍 Open，是内存上涨的核心机制 | 增量提取 usage；缺失时用有界状态估算，绝不保留完整文本 |
| 旧版流式渠道客户端断开后泄漏上游读取 goroutine | 仍 Open | 断连立即 drop 上游响应流并取消请求，不派生 detached 读取任务 |
| 内存限流器在高 `GLOBAL_API_RATE_LIMIT` 下 OOM | 已修复 | 不实现全局内存限流器；仅做有界队列与背压 |

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
| SQLite 同库同 schema | 复刻 | 已实现 | M1：WAL 连接 + 必需表/列校验 |
| `users` / `tokens` / `channels` / `abilities` / `options` 读取 | 复刻 | 已实现 | M1：内存缓存加载，未知列不解析 |
| `logs` 消费日志写入 | 复刻 | 已实现 | M2：异步批量写，`type=2`、`quota=0`，同事务更新 `tokens.accessed_time` |
| `quota_data` 统计 | 待评估 | 待评估 | 看板可直接从 `logs` 聚合，避免双写 |
| `user_sessions` 会话持久化 | 复刻 | 已实现 | M1：JWT + refresh cookie |
| 多 Token 管理 | 复刻 | 已实现 | M3：列表/新建/编辑/删除/批量/查看 Key，分组、模型限制、IP 白名单；M5 新建支持自定义密钥（留空回落随机 48 位，编辑不改密钥） |
| 单用户密码登录（argon2id） | 复刻 | 已实现 | M1：登录/刷新/登出 |
| 多租户/用户管理/注册 | 不复刻 | 不复刻 | 个人使用 |
| OAuth / 邮箱验证 / 2FA / Passkey | 不复刻 | 不复刻 | 按需求去掉 |
| 额度扣减 / 充值 / 订阅 / 兑换码 / 邀请 / 签到 | 不复刻 | 不复刻 | 不做额度拦截与支付 |
| Redis 缓存 / 多节点同步 | 不复刻 | 不复刻 | 改为进程内缓存 |
| 密码登录加密（RSA） | 不复刻 | 不复刻 | 现网未启用；TLS 即可 |

### 4.2 转发协议

| 端点 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| `POST /v1/chat/completions` | 复刻 | 已实现 | M2：流式/非流式透传，增量 usage，断连取消上游 |
| `POST /v1/messages`（含 `?beta=true`） | 复刻 | 已实现 | M2：Anthropic↔OpenAI Chat 转换（非流式+流式） |
| `POST /v1/responses` | 复刻 | 已实现 | M2：Responses↔Chat 转换（现网无真实流量） |
| `POST /v1/messages/count_tokens` | 不复刻 | 不复刻 | 现网一直 404，保持 404 |
| `GET /v1/models`、`GET /v1/models/:model` | 复刻 | 已实现 | M1：按分组/Token 限制聚合 |
| `POST /v1/embeddings` | 复刻 | 已实现 | M4：JSON 透传，记录 usage |
| `POST /v1/images/generations`、`/v1/images/edits` | 复刻 | 已实现 | M4：JSON/multipart 透传，按 model 路由 |
| `POST /v1/audio/transcriptions`、`translations`、`speech` | 复刻 | 已实现 | M4：multipart 提取 model 路由；speech 二进制回传 |
| `POST /v1/rerank` | 复刻 | 已实现 | M4：JSON 透传，记录 usage |
| `POST /v1/completions`、`/v1/moderations` | 复刻 | 已实现 | M4：JSON 透传，completions 记录 usage |
| Gemini 原生 `/v1beta/*` | 不复刻 | 不复刻 | 现网未使用 |
| Realtime WebSocket / Responses WebSocket | 不复刻 | 不复刻 | 现网未使用 |
| Midjourney / Task / Video / Suno / JS 插件 | 不复刻 | 不复刻 | 现网未使用 |
| Files / Fine-tunes | 不复刻 | 不复刻 | 现网未使用 |

### 4.3 渠道与路由

| 功能 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| 分组匹配（default/vip） | 复刻 | 已实现 | M2：Token 分组决定可用渠道 |
| 优先级 + 权重选择 | 复刻 | 已实现 | M2：优先级分层 + 权重无放回抽样 |
| 多 Key polling | 复刻 | 已实现 | M2：进程内轮询，不依赖 Redis |
| 单 Key 禁用/恢复 | 复刻 | 已实现 | M2：401/403/429 后 600s 内跳过 |
| 429/5xx 重试与跨渠道切换 | 复刻 | 已实现 | M2：`RetryTimes` 上限 10，响应开始后不重试 |
| 模型映射 `model_mapping` | 复刻 | 已实现 | M2：链式映射 + 循环保护 |
| Header 覆盖/透传 | 复刻 | 已实现 | M2：通配/正则/删除/占位符；Anthropic 用 `x-api-key` |
| Param 覆盖 | 待实现 | 待实现 | 现网为空 |
| 上游模型同步 / 余额查询 / Ollama 拉取 / VLLM 状态 | 不复刻 | 不复刻 | 管理页裁掉 |
| 标签批量操作 / 优先级拖拽排序 | 不复刻 | 不复刻 | 保留基础编辑即可 |
| 多上游适配器（Azure/Vertex/Bedrock/Gemini/…） | 不复刻 | 不复刻 | 只做 OpenAI 兼容 + Anthropic 直连 |

### 4.4 管理界面（复用 new-api React 并裁剪）

| 页面 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| 登录页 | 复刻 | 已实现 | 账号密码登录，已移除注册/找回密码入口 |
| Token 管理 | 复刻 | 已实现 | M3：列表/筛选/新建/编辑/删除/批量/查看 Key。M5 增加“新建时可自填密钥”：16–64 位字母/数字/下划线（粘贴的 `sk-` 前缀写库前剥掉，因为认证按第一个 `-` 截断），留空由服务端生成 48 位；`idx_tokens_key` 不看软删行，所以先查全表再把 UNIQUE 冲突映射成同一句业务错误；编辑一律忽略传入的 `key`——换密钥会让已下发的客户端配置静默失效，要改就新建再删旧的。批量数量 >1 与自填密钥互斥 |
| 渠道管理 | 复刻（核心） | 已实现 | M3：列表/新建/编辑/删除/启停/查看密钥/多 Key 轮询；M4 补齐上游测试、获取模型、优先级拖拽重排（`/api/channel/priority/*`）、复制（`/api/channel/copy/:id`）、修复一致性（`/api/channel/fix`）。多 Key 管理面板、Ollama 模型管理、推理状态、上游模型自动同步入口已裁剪（依赖 Go 任务系统/Redis） |
| 用量日志 | 复刻 | 已实现 | M3：列表/筛选/统计/用户详情；导出沿用前端流式接口。M4 收敛为通用日志：任务/MJ 日志、金额与定价展示、审计与请求策略详情随对应后端一起裁剪 |
| 用量看板 | 复刻 | 已实现 | M3：概览与模型调用分析按 logs 聚合；M4 金额口径全部改为用量口径（tokens/次数/耗时） |
| 模型管理 | 复刻（基础） | 已实现 | M3：列表/新建/编辑/启停/软删除；M4 补齐 `include_channel_models` 的渠道模型并集与广场状态派生（`square_state` 按 abilities/channels 计算，含按状态筛选后的分页）、`configured_channel_count`/`bound_channels`/`enable_groups`/`matched_models` 富化、`vendor_counts` 与 `/api/vendors` 真实读取；裁剪部署、供应商运营与上游同步。与 Go 的唯一可见差异：渠道派生行（`id=0`）的“展示策略”列显示 `—`，Go 会按 `status=0` 显示“未展示”，但那是对一条本就没有的规则做断言 |
| 系统设置 | 复刻（裁剪） | 已实现 | M5：站点信息（SystemName/Logo/Notice/ServerAddress）+ 日志维护（记录用量日志开关、登录用户导出开关、最大导出跨度、按时间分批清理历史日志）。`GET/PUT /api/option` 走 7 键白名单：读取按白名单合成（缺行回落内置默认），写入只落白名单键，非白名单键与非法值返回 Go 风格业务错误；`/api/status`、`/api/notice` 与导出跨度都改读这份设置。请求策略（重试次数/auto_ban/会话亲和/敏感词）与安全与限制（限流/SSRF/令牌上限）不复刻，路由与重试沿用内置策略并在各渠道内配置 |
| 用户管理 / 钱包 / 订阅 / 兑换码 | 不复刻 | 不复刻 | 单用户 |
| 安全（2FA/Passkey）/ 个人资料 | 个人资料复刻，2FA/Passkey 不复刻 | 已实现 | 个人资料 = 基本资料 + 改密码 + 会话管理 |
| Playground | 复刻 | 已实现 | M4：`/pg/chat/completions` 直连本地转发，不做聊天历史持久化 |
| 审计日志 / 系统信息 / 性能指标 | 不复刻 | 不复刻 | 保留 `/api/status` 健康检查；看板的性能健康面板见 §6 第 7 条 |

### 4.5 运维

| 功能 | 决策 | 状态 | 备注 |
| --- | --- | --- | --- |
| `/api/status` 健康检查 | 复刻 | 已实现 | M1：兼容现有 Docker healthcheck |
| 优雅停机 | 复刻 | 已实现 | M1：SIGINT/SIGTERM |
| 本地交叉编译 + 上传 | 复刻 | 已实现 | `deploy/build.sh`：`rust:1.96-bookworm` + `--platform linux/amd64` + `RUSTUP_TOOLCHAIN=1.96.1`（避免现网拉取 1.96.0 toolchain 卡住）；`deploy/package.sh` 产出 `docker save` 归档 |
| Docker 服务替换 | 复刻 | 已演练，待确认后上线 | `deploy/Dockerfile` 保留 `./data:/data`、端口 20128→3000、`mem_limit 450m`、`wget` 健康检查；2026-11-06 现网切换 34 分钟后按用户要求回退，后续验证只在本地 Docker 进行 |
| 回滚到 Go 版本 | 复刻 | 已演练 | 恢复 `docker-compose.yml.bak.rust-20261106` + `docker compose up -d`；旧镜像 `calciumion/new-api:v1.0.0-rc.47` 与 `data/one-api.db.pre-rust-20261106` 保留。回退后核验：`PRAGMA integrity_check` ok，39 张表的列与索引与切换前零差异，Rust 期间增量写入的 23 条消费日志（`quota=0`、`request_id=req_*`、`other={}`）Go 侧读取与列表正常，`channels/tokens/abilities/users/options` 行数与最大 id 未变，无测试残留行 |
| Prometheus/复杂可观测性 | 不复刻 | 不复刻 | 结构化 tracing 日志即可 |
| 多节点/集群 | 不复刻 | 不复刻 | 单实例 |

### 4.6 本地 Docker 验收（上线前置门）

`bash deploy/run-local.sh` 用线上快照播种 `/tmp/solo-deploy-data/one-api.db`，以 `--memory 450m`、端口 3006 跑 `solo-new-api-rust:local`；渠道 3/4 的 `base_url` 改成 `http://host.docker.internal:3007` 指向本地 mock 上游，全程不访问真实上游。

| 验收项 | 结果 | 读数 |
| --- | --- | --- |
| 启动读现有库 + `/api/status` + SPA/未知路径 | 通过 | healthcheck `grep` 命中；`/api/nope`、`/v1/nope`、`/mj/submit` 均 `404 application/json`；`/token` 返回 index |
| 线上 argon2id 口令登录 | 通过 | `/api/user/login` 发 access_token + session，`/api/user/self` 200 |
| Token 认证 | 通过 | 快照里的 48 位 key 加 `sk-` 前缀直接可用；错误 key `401 Invalid token` |
| 非流式转发 + 日志 | 通过 | `prompt=11 completion=7 quota=0`，`other={}`，`request_id=req_*` |
| Chat 流式零缓冲 | 通过 | 2.69 MB / 4001 事件全量透传；容器内存 18.4 → 峰值 23.6 → 回落 18.6 MiB |
| Anthropic 流式转换 | 通过 | `/v1/messages` 输出 `message_start`…`message_stop`，2.46 MB |
| 跨协议非流式 | 通过 | `/v1/messages`、`/v1/responses` 对 OpenAI 渠道互相转换并给出 usage |
| 客户端断连取消 | 通过 | curl 2s 断开后 mock 在 +0.09s 记录 `stream-aborted BrokenPipeError`，无 detached 续读 |
| 24 路并发流式 | 通过 | 24/24 完成、9.69 MB 中转；基线 30.4 → 峰值 47.0 → 结束 33.0 MiB；24 条日志 usage 正确 |
| 请求体硬上限 | 通过 | 70 MB 请求返回 `413`；默认上限由 64 MiB 收紧为 32 MiB（`MAX_REQUEST_BYTES`） |
| 管理 API | 通过 | token/channel/models/group/log/stat/data 全部 `success:true` |
| 浏览器逐页实测（2026-11-07，宿主机直跑 + 最新 `web/dist`） | 通过 | `/keys` 分组列只显示 `default`/`vip`（臆造的 1x 倍数已移除）；`/playground` 空状态与输入区（模型/分组选择器、发送）正常；`/profile` = 基本资料 + 改密码 + 会话管理，用量卡为 API 请求 / 输入 Token / 输出 Token；`/dashboard/flow` 桑基图按 logs 聚合出 用户→节点→令牌→分组→模型→渠道；`/models/metadata` 列出 6 个渠道派生模型，广场状态、绑定渠道与分组正确；页面零 console error，XHR 全 200 |
| i18n 覆盖对账 | 通过 | 提取 `web/src` 全部 `t('…')` 字面量与 `zh.json` 比对，仅个人资料页两条长度提示缺 key，已补齐 7 个语言后复跑归零 |
| 镜像重建 + 产物一致（2026-11-07，本轮两次重编后） | 通过 | `deploy/build.sh` 重出 linux/amd64 镜像：镜像内 `web/dist` 入口哈希与浏览器实测加载的那份一致（`index.9d2c7d4d62.js` / `index.c9a0673030.css`，由页面 resource 条目核对），二进制构建时间晚于全部提交；容器 19.8–25.2 MiB / 450 MiB |
| Docker 登录态逐页实测（:3006，2026-11-07） | 通过 | `/models/metadata` 总计 6、全部 缺元数据 / 正常展示 / 渠道 2·分组 2；`/keys` 分组列只显示 `default`/`vip`；`/channels` 批量测试 6/6 成功（164–169 毫秒，打到本地 mock）；`/api/channel/fetch_models/3` 给出 2 个新模型 + 6 个已移除；`/playground`、`/profile`（含新译的“使用 1–20 个字符。”）、`/dashboard/flow`、`/usage-logs/common` 零 console error、XHR 全 200；日志表头为 时间/渠道/用户/令牌/模型/流/Token/总 Token 数/耗时/详情，无金额列 |
| 系统设置接口（Docker :3006） | 通过 | 写入只落白名单键：`options` 行数 24 → 29（`LogConsumeEnabled` 保持内置默认未落库，共 6 行落库）；Go 侧原有 23 行零改动（`RetryTimes=10`、`HeaderNavModules`、`SidebarModulesAdmin`、`theme.frontend`、`passkey.*`、`payment_setting.*`、`AutomaticEnableChannelEnabled` 等），`PRAGMA integrity_check` ok。`GET /api/option/` 只返回 7 个白名单键；非白名单键与非法值给 Go 风格 `success:false` |
| 设置页浏览器实测（Docker :3006，2026-11-07） | 通过 | 站点信息四项可输入并保存，`/api/status` 与顶栏品牌即时更新，刷新后回读到 `SystemName=Solo 本地验收站`、`ServerAddress=https://api.example.com`；日志卡两个开关与最大导出跨度=45 保存后回读一致，跨度即时生效——60 天跨度导出返回“The time range cannot exceed 45 days”；“24 小时前”快捷钮写入截止时间；清理确认对话框（AlertDialog）确认后 23 批删除 113,666 条，日志总数 128,210 → 14,544 与 `/api/log/` 读数一致，单批最慢 91ms、全部 200，容器内存 25.2 MiB / 450 MiB |
| 本轮实测发现并修复的两个缺陷 | 已修复 | ① 设置页无限重渲染：`useOptions()` 每帧返回新的 `values` 对象，两张卡的 `useEffect(…, [loaded])` 因此每帧 `setValues` 新对象，输入在同一 tick 被回滚、该标签页渲染进程 100% CPU。改为 `useMemo` 依赖 `query.data`（loading 期用固定空数组引用）并加 `web/src/features/settings/__tests__/use-settings.test.tsx`（改前 2 例红、改后 3 例绿），顺带把站点卡的 label 与输入框拆成兄弟节点修掉提示语换行。<br>② 历史清理返回 500：默认批次 50_000 行按 id 逐行绑定，超过该 SQLite 构建的 32766 绑定变量上限，报 `too many SQL variables`（现网快照 128,210 行、早于 24 小时的有 113,666 行时才暴露，原来的集成测试只造 200 行看不见）。删除集改为单语句子查询 `DELETE FROM logs WHERE id IN (SELECT … LIMIT ?)`（只绑 2 个变量），默认批次 5_000、上限 50_000，并加 `cleanup_round_can_delete_more_rows_than_sqlite_allows_binds`（100_000 行夹具、40_000 行匹配，改前复现同一条 500 报文、改后绿） |
| 转发/流式/内存矩阵本轮未重跑 | 说明 | 本轮后端改动只在设置白名单模块（`src/settings.rs`、`src/routes/admin/option.rs`）、日志清理批次（`src/routes/admin/log.rs` 的 `cleanup`）与 status/notice 读取；`src/relay/*` 与上一轮通过本节全部转发、流式、断连、并发内存项时同源。全量 `cargo test` 152 例通过、`bun run test` 与 `bun run typecheck` 通过；渠道测试 6/6 走的是同一套上游客户端与协议转换，只作为新二进制的间接证据 |
| 新建令牌自定义密钥（API，Docker :3006，2026-11-07） | 通过 | `solo-new-api-rust:local` 由 `deploy/build.sh` 重编，镜像内入口 `index.bd05961477.js` 与容器页面加载、本地 `web/dist` 三方一致；容器内存 43.78 MiB / 450 MiB。`POST /api/token/` 传 `sk-docker_custom_key_2026` → `POST /api/token/12/key` 回读 22 位裸密钥（前缀已剥），`Authorization: Bearer <裸>` 与 `Bearer sk-<裸>` 打 `/v1/models` 均 200；`key="   "` 回落 48 位随机（`uE1S…IxAp`，同样 200）；重复密钥 400 `Token key is already in use`；`tiny` 400 `Token key must be 16-64 characters`；被拒的两次没有落库（列表只剩 dk1/dk4） |
| 编辑不改密钥 + 浏览器中文实测（Docker :3006） | 通过 | PUT 传 `key=totally_different_key_99` 后回读仍是 `docker_custom_key_2026`，旧客户端 key 继续 200——已下发配置不会静默失效。中文界面：新建抽屉标签为 名称/自定义密钥（可选）/分组/过期时间/数量，描述显示“仅支持字母、数字和下划线，长度 16-64 位。粘贴的 sk- 前缀会被去掉。密钥创建后不可修改。”；提交与既有令牌重键的表单 → toast 显示“令牌密钥已被使用”（非英文原文）；编辑抽屉标题“更新 API 密钥”只有 名称/分组/过期时间，无密钥输入。console 除预期的 401（未登录首屏）/400（重键）外，只有我注入脚本自身的一次 TypeError，页面代码无报错 |
| 服务端错误本地化（`message_key`） | 部分实现 | `ApiError` 新增可选 `message_key`，只在显式标注时写入 JSON，其余报文形状不变（集成测试断言 `Token name is too long` 仍不带该字段）；本轮只标注三条密钥错误，7 个语言各补 7 个 key（每份 7454 key，缺失 0）。前端按 Go 的规则只用 `message_key`/`code` 翻译，所以其余管理端错误仍显示英文——见 §6 第 8 条 |
| 本轮测试与门禁 | 通过 | `tests/admin_token.rs` 新增 5 例（HEAD 实现下 4 例红，改后 7 例绿）；前端新增 `custom-key-form.test.ts` 9 例 + 抽屉用例 4 例（HEAD 源码下 2 例红）。`cargo test` 157 通过 / 1 忽略，rustfmt 限定 `src/error.rs`、`src/routes/admin/token.rs`、`tests/admin_token.rs` 检查归零（三者 BASE 版本就 clean），clippy 无指向这三处；`bun run typecheck` 729 文件 0 错误、`bun run test` 103 文件 1044 例通过、oxfmt 限定的 5 个改动文件已合规、`copyright:check` 的 11 个 offender 全为既有文件 |
| 侧边栏 / 命名 / 概览 / 新图标（Docker :3006，2026-11-07） | 通过 | 侧边栏顶层新增「聊天」组，`游乐场` 置顶（沿用 `chat` 组 id，`chat.playground` 开关仍生效，组内为空时整组隐藏）；`模型调用分析` 入口与页面标题改为「数据看板」，页内图表卡仍用 `Model Call Analytics` 与上游一致；概览页删掉 常见问题解答 / 公告 / API 信息 / 运行时间 四块及其组件、类型、`getUptimeStatus`、`useAnnouncements`/`useFAQ`/`useDashboardContentVisibility`（Rust `/api/status` 从不返回 `faq*`/`announcements*`/`api_info*`/`uptime_kuma*`，前端按「缺省即显示」渲染，等于四张恒空卡），管理员只剩「性能健康」；新品牌标（蓝→紫渐变的新月 + 星芒，替代原青-粉双月牙）：`favicon.ico` 为 6 个 PNG 条目 16/32/48/64/128/256、透明底、8,857 B（每个尺寸按目标像素原生光栅化 + 超采样，不是缩放大图），`logo.png` 为 180×180 RGBA、2,989 B，容器 `/favicon.ico`、`/logo.png` 的 SHA-256 与 `web/public` 一致，页面 `<link rel="icon">` 只引用 `/favicon.ico`、顶栏徽标 naturalWidth=180；i18n 未新增 key（`Chat`/`Playground`/`Dashboard` 7 语言已有）。浏览器实测：登录后侧边栏顺序 = 聊天 游乐场 / 常规 概览·数据看板·API 密钥·使用日志 / 管理员 渠道·模型·系统设置，`/dashboard/models` 标题「数据看板」三个 tab 正常，`/playground` 与 `/dashboard/overview` 渲染正常 |
| 登录内存占用（Docker :3006，2026-11-07） | 已修复 | 现象：本地容器 146 MiB（Go 现网 58 MiB 且有流量）。取证：空载 23.1 MiB；8 次串行登录 +19 MiB 后持平；12 次并发登录 92→303 MiB；空闲 90s 不回落；`smaps` 显示 5–12 个 19 MiB 匿名映射，与 `argon2id m=19456KiB`（Go 侧同参数，格式红线不能改小）吻合；重查询 5 轮只 +12 MiB 且平台化，不是主因。根因：19 MiB 瞬时块落在哪个 tokio worker 上，glibc 就把该线程的 arena 撑到 19 MiB 且释放后不还给内核；并发登录分散到多个 worker → arena 数 × 19 MiB。修法：① `mallopt(M_MMAP_THRESHOLD, 16 MiB)`、`M_TRIM_THRESHOLD 128 KiB`、`M_ARENA_MAX 2`（仅 linux-gnu，`main` 在 runtime 启动前调用）；② KDF 移入 `spawn_blocking`，并用 `Semaphore(2)` 给 19 MiB × 并发封顶（也避免 KDF 占住 async worker 影响转发）。复测（同镜像、不设任何环境变量）：空载 23.1 → 串行 8 次 23.3 → 并发 12 次 24.8 → 5 轮重查询 44.5（平台）→ 空闲 90s 37.6 MiB；:3006 浏览器登录并翻三页后 28.1 MiB。功能：并发 12/12 登录成功、错误密码 6/6 返回 401、单次登录 21 ms。说明：本机是 arm64 跑 amd64（Rosetta），`maps` 里 128 MB 的 `rwxp` 是 Rosetta 翻译代码缓存（约 8 MiB 驻留），生产原生 amd64 无此项 |
| 本轮测试与门禁（布局 + 内存两轮合计） | 通过 | `cargo test` 159 通过 / 1 忽略（新增 `async_kdf_roundtrip`、`concurrent_verifies_all_complete` 两例）；rustfmt 限定三文件与 HEAD 同 hunk 数（session 9 / main 4 / user 11，均为既有漂移，新增代码 0 漂移）；clippy 6 个报错全在未改文件（`priority.rs`、`relay/playground.rs`、`routes/admin/channel.rs`、`auth/session.rs` 的既有 `collapsible_if`/complex-type），HEAD 同样存在；`bun run typecheck` 722 文件 0 错、`bun run test` 103 文件 1044 例、oxfmt 限定改动文件除 `sidebar-config.test.tsx` 既有漂移外干净；`copyright:check` 的 11 个 offender 全为既有文件 |
| 发现并修复的两个缺陷（本轮实测） | 已修复 | ① 前端：概览页四张恒空卡（见上条）。② 内存：并发登录按 19 MiB × arena 抬升（见上条）；两处都有改前/改后实测数字，非推断 |
| 生图链路冒烟（sensenova-u1.5-lite，真实上游，2026-11-07 用户授权） | 通过 | 渠道 #3/#4（type 1，`base_url=https://token.sensenova.cn`）模型列表含 `sensenova-u1.5-lite`，`/api/user/models` 可见；经 :3006 `POST /v1/images/generations`（`{"model":"sensenova-u1.5-lite","prompt":"…","size":"2048x2048","watermark":false}`）→ HTTP 200、22.3 s，返回 1 张 2048×2048 PNG（`b64_json` 6.8 MB，解码 4.9 MB），上游 usage `input 1,916 / output 4,096 / total 6,012`、`images_count=1`、`output_image_type=HD`。响应体超过 4 MiB 上限，走流式透传、不缓冲；日志 #134214 记下模型/令牌/耗时 22 s，prompt/completion 为 0（大响应不解析 usage，已知口径）。容器 RSS 28 → 39.7 → 34.9 MiB（空闲 30 s 后），未随请求量上涨；临时令牌用后已删（11 → 10）。注意该模型是画图专用，只走 `/v1/images/generations` 与 `/v1/images/edits`，打到 chat completions 会 400（Qoder ImageGen 的报错即此原因）；`/v1/images/edits` 本轮未跑（需参考图） |

- 注意：容器运行期间不要用宿主机 `sqlite3`/`python` 改 `/tmp/solo-deploy-data/one-api.db`。宿主与 VM 共用同一 WAL `-shm` 映射会触发 `SQLITE_IOERR_WRITE (522)`，日志写失败是测试手法造成的假故障；改库要先 `docker stop`，查日志改用 `/api/log`。
- 本轮 Docker 登录用的是写在 `/tmp/solo-deploy-data` scratch 副本里的一次性 argon2id 口令（仓库快照 `data/one-api-consistent.db` 与线上库未改动），渠道 `base_url` 也只在这份副本里改指向 mock；验收后整个目录连同容器一起删除。要自己复核就重跑 `bash deploy/run-local.sh`，它会从快照重新播种，届时用真实口令登录。

## 5. 线上现状（用于验收）

- 数据库：`one-api.db`（约 231 MB，WAL），本地验收使用 SQLite 在线备份 API 生成的一致快照 `data/one-api-consistent.db`（不入 Git）。
  - 直接热拷贝主库 + WAL/SHM 会得到不一致副本（`PRAGMA integrity_check` 失败），不能用于兼容性验收。
  - 线上 `quick_check` 为 ok；本地热拷贝的损坏是拷贝方式造成的，不是源库问题。
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
7. 性能指标（`/api/perf-metrics{,/summary}`）：Go 由 relay 增量写 `perf_metrics` 表（含 ttft、成功/失败计数），本项目不新增表、不改表现有结构，`/api/perf-metrics*` 目前返回空，看板的性能健康面板因此显示空状态。**未实现，排在本地 Docker + 浏览器验收之后**。口径决策：`logs` 能诚实派生的只有平均耗时、吞吐（completion_tokens/use_time）与请求数（走 `idx_created_at_id`/`idx_logs_model_name` 聚合）；成功率算不出来——Rust 侧 `src/logging.rs:82` 的 INSERT 把 type 固定写 2、`other` 固定 `{}`、`content` 为 NULL，失败请求与零 token 请求在库里不可区分，ttft 也从未记录。要补成功率，需先在 relay 侧给失败路径落可判别标记（新 type 取值或 `other` 里的错误字段），再按 logs 聚合；面板不展示无法诚实计算的指标。
8. 管理端错误本地化覆盖：Go 用 `common.ApiErrorT(c, "<英文原文>", params)` 同时下发 `message`（渲染后的英文）与 `message_key`（英文原文，即前端 locale 的 key），前端 `getServerMessage()` 只翻 `message_key`/`code`。Rust 的 `ApiError` 现在支持可选 `message_key`，但本轮只给三条密钥错误标注；其余既有管理端报文（`Token name is too long`、渠道/模型/日志各模块的英文 message）仍无 `message_key`，中文界面会直接显示英文原文。补齐方式是逐个把 message 改成 `ApiError::new(...).with_message_key(...)` 并核对 locale 里已有该 key——纯机械工作，排在功能验收之后；顺带要决定是否统一收进一个 `ApiErrorT` 风格构造函数，避免各处复制英文串。

## 7. 里程碑

1. M1 基础骨架 + 数据层 + 认证（可读现有库、Token 可认证）— 已完成
2. M2 转发核心 + 渠道选择/重试（chat/completions、messages、responses 可用，跨协议转换）— 已完成
3. M3 管理 API + 前端裁剪（可登录并管理 Token/渠道/日志/看板/模型）— 已完成
4. M4 全套端点 + 兼容性/并发测试 + 本地数据验收 — 已完成
5. M5 本地交叉编译、上传切换、回滚演练 — 编译/镜像/回滚演练完成；现网切换已按要求回退到 Go 版本，验收改在本地 Docker 进行，上线需用户明确确认

## 8. 相关任务

任务清单由协作任务系统维护（项目脚手架、数据层、渠道选择、认证重试、转发核心、协议转换、全套端点、管理认证、管理 API、前端裁剪、日志统计、兼容测试、性能验证、部署切换、文档）。
