# M3 管理 API 与前端裁剪 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 提供与 new-api React 前端兼容的管理 API，并裁剪出登录/Token/渠道/日志/看板/模型六个页面。

**Architecture:** 管理接口沿用 `{success,message,data}` 包装；所有写操作在事务内完成并重新加载内存缓存；列表接口返回 `{items,total,page,page_size}`。前端复用 new-api 源码，删除未使用路由/侧边栏/API 模块后构建到 `web/dist`，由 Rust 静态服务托管。

**Tech Stack:** axum、sqlx SQLite、serde_json、React/Vite(new-api fork)、bun。

## Global Constraints

- 不修改数据库表结构；不使用 new-api 未使用的列。
- 未知列原样保留：渠道更新只写允许字段，其他列不动。
- Token 删除使用 `deleted_at` 软删除；渠道删除硬删除并清理 `abilities`。
- 所有 CRUD 后必须刷新 `AppState.caches`，否则转发/认证读到旧数据。
- 管理接口全部走 `admin_auth` 中间件。

---

## Task 1: 数据模型与管理写库基础

**Files:**
- Modify: `src/model.rs`、`src/state.rs`
- Create: `src/routes/admin.rs`、`src/routes/admin/token.rs`

**Interfaces:**
- `TokenRow` 增加：`created_time: i64`、`accessed_time: i64`、`used_quota: i64`、`allow_user_quota_query: bool`。
- `ChannelRow` 增加前端需要的展示字段：`created_time`、`test_time`、`response_time`、`open_ai_organization`、`test_model`、`other`、`balance`、`balance_updated_time`、`used_quota`、`status_code_mapping`、`auto_ban`、`other_info`、`tag`、`setting`、`param_override`、`remark`、`settings`。
- `AppState::reload_caches(&self) -> Result<(), sqlx::Error>`。
- 管理路由前缀：`/api/token`、`/api/channel`、`/api/log`、`/api/data`、`/api/models`。

**验收：** `cargo test` 全绿；真实库加载字段不报错。

---

## Task 2: Token 管理 API

**Files:**
- Create: `src/routes/admin/token.rs`
- Test: `tests/admin_token.rs`

**接口（与前端 keys/api.ts 对齐）：**
- `GET /api/token/?p=&size=` → 当前用户 Token 分页；`key` 用掩码，`auto_groups` 返回数组或 null。
- `GET /api/token/search?keyword=&token=&p=&size=`。
- `GET /api/token/{id}`。
- `GET /api/token/auto-groups` → `{groups: [用户分组], max_count: 1}`。
- `POST /api/token/` 新建：生成 48 位随机 Key；`auto_groups` 存 JSON；`status=1`。
- `PUT /api/token/` 更新；`?status_only=true` 只改状态。
- `DELETE /api/token/{id}/` 软删除（写 `deleted_at`）。
- `POST /api/token/batch` `{ids:[]}` 批量软删除。
- `POST /api/token/{id}/key`、`POST /api/token/batch/keys` 返回明文 Key。

**规则：**
- 名称长度 ≤ 50；不存在的 ID 返回 404；跨用户不可见（按 `user_id` 过滤）。
- 每次写后 `reload_caches`。
- 列表字段必须包含：`id,name,key,status,remain_quota,used_quota,unlimited_quota,expired_time,created_time,accessed_time,group,auto_groups,cross_group_retry,model_limits_enabled,model_limits,allow_ips,allow_user_quota_query`。

---

## Task 3: 渠道管理 API

**Files:**
- Create: `src/routes/admin/channel.rs`
- Test: `tests/admin_channel.rs`

**接口（与前端 channels/api.ts 对齐）：**
- `GET /api/channel?p=&size=&status=&type=&group=&id_sort=` → 分页，`key` 返回空串。
- `GET /api/channel/search?keyword=&group=&model=&status=&type=`。
- `GET /api/channel/{id}`。
- `POST /api/channel` body `{mode:"single"|"multi_to_single"|"batch", channel:{...}}`；新建渠道并同步 `abilities`。
- `PUT /api/channel/` 更新允许字段并重建 `abilities`。
- `POST /api/channel/{id}/status`、`POST /api/channel/status/batch`。
- `DELETE /api/channel/{id}` 硬删除渠道与 abilities。
- `POST /api/channel/batch` `{ids}` 批量删除。
- `GET /api/channel/models` → 去重模型列表；`GET /api/channel/models_enabled` → 可用模型字符串数组。
- `GET /api/channel/default_base_urls`、`GET /api/channel/priority/order` 返回空/默认结构，避免页面报错。

**abilities 重建：** `models × groups` 去重；`enabled = status==1`；`priority/weight` 取渠道值。

---

## Task 4: 日志与看板 API

**Files:**
- Create: `src/routes/admin/log.rs`、`src/routes/admin/data.rs`
- Test: `tests/admin_log.rs`

**接口：**
- `GET /api/log?p=&page_size=&type=&username=&token_name=&model_name=&start_timestamp=&end_timestamp=&channel=&group=&request_id=&upstream_request_id=` → `{items,total,page,page_size}`。
- `GET /api/log/self` 同上，固定当前用户。
- `GET /api/log/stat`、`/api/log/self/stat` → 汇总 `{quota, prompt_tokens, completion_tokens, request_count}`（字段名按前端 `LogStatistics`）。
- `GET /api/data?start_timestamp=&end_timestamp=&default_time=` → 按天聚合 `{model_name, username, created_at, quota, count, token_used}`。
- `GET /api/data/flow` → 与 `/api/data` 相同结构。
- `GET /api/data/users` → 按用户名聚合。
- `GET /api/uptime/status` → `{success:true,data:[]}`（看板不展示监控）。

**约束：** 只读 `logs`；不做 `quota_data` 双写；时间过滤用实际存在于表内的 `created_at`。

---

## Task 5: 模型管理 API（基础）

**Files:**
- Create: `src/routes/admin/models.rs`
- Test: `tests/admin_models.rs`

**接口：**
- `GET /api/models/?p=&size=&keyword=` → 读 `models` 表；为空时用 `abilities` 去重兜底，返回 `configured_channel_count`、`enable_groups`。
- `POST /api/models/`、`PUT /api/models/`、`PUT /api/models/?status_only=true`、`DELETE /api/models/{id}`。
- `GET /api/models/search` 与列表同参。
- `GET /api/models/enabled` → 启用模型名数组（若前端用到）。

**约束：** 只写 `models` 表的已知列；不触发上游同步、不维护 vendor/pricing。

---

## Task 6: 前端裁剪与构建

**Files:**
- Create: `web/`（从 new-api `web/` 复制，固定提交 `e03439033238722c0b8d4437a052445f3aeeed83`）
- Modify: 路由表、侧边栏配置、`vite.config`/base、API 模块
- Output: `web/dist/`，由 `WEB_DIST` 提供

**保留路由：** 登录、`/keys`、`/channels`、`/usage-logs`、`/dashboard`、`/models`。
**删除：** 注册/找回密码/OAuth/2FA/Passkey、钱包/充值/兑换码/订阅/邀请、Playground/聊天/定价/排行/任务插件/审计/系统设置/系统信息/用户管理。
**侧边栏：** 仅保留上述五项与登出。
**API：** 删除未使用 feature 的 API 调用；未实现端点不得在保留页面出现。

**验收：**
```bash
cd web && bun install && bun run build
```
构建成功；`web/dist/index.html` 存在；SPA 深链返回 200。

---

## Task 7: 浏览器端到端验证

- 启动 Rust 服务（临时 DB 副本 + `WEB_DIST=web/dist`）。
- 登录 → Token 列表/新建/编辑/删除 → 渠道列表/编辑 → 日志列表 → 看板 → 模型列表。
- 确认无 404/500、无未实现页面入口。
- 记录截图或控制台证据到 `docs/REWRITE_SCOPE.md` 里程碑状态。
