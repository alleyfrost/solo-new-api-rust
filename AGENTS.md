# 项目规则（solo-new-api-rust）

## 协作方式

- 具体实现方案（库选型、模块边界、算法与参数、前端裁剪细节）由 agent 自行评估、自行决定并自审，不再逐项询问用户。
- 范围/里程碑级决策、不可逆或影响线上数据的操作，仍按约定确认。
- 自审 = 对照本文件、`docs/REWRITE_SCOPE.md` 与设计 spec 的检查清单逐项核验，不是跳过评审。
- 中文沟通。验证通过的改动及时 git commit，不等用户开口。

## 兼容性红线

- 直接使用现有 `/data/one-api.db`，不修改表结构、不重命名列、不做自动迁移。
- 未知列与 JSON 字段（`channels.other`、`settings`、`setting`、`param_override`、`header_override`、`channel_info`）读写时原样保留。
- 对外接口行为与数据可读性优先；不照搬 Go/Redis 内部机制。
- 不引入 Redis、多节点、任务系统、计费引擎。

## 流式与内存红线（new-api 上游已知缺陷，禁止复刻）

- 禁止为统计 completion tokens 缓冲完整流式响应。用量必须增量提取：优先上游 `usage`，缺失时用有界状态的增量估算。
- 客户端断连必须立即取消上游读取；禁止启动后台 detached 任务继续读流。日志只记录已获得的用量/状态。
- 所有队列有界并带背压；请求体、非流式响应、翻译中间态都设硬上限，超限返回明确错误。
- 大输入/大批量/长流场景必须做内存压测，验收标准是处理结束后 RSS 回落且不随请求量持续上涨。

## 技术栈

- HTTP：tokio + axum/hyper；上游：reqwest 连接池；SSE 零缓冲转发。
- 数据：SQLite + sqlx（WAL、busy_timeout、读写分离），日志异步批量写。
- 协议转换：复用 `switchyard-translation` 0.3.0（Apache-2.0），vendored 并固定版本；同格式请求直接透传，避免无谓转换。
- 前端：复用 new-api React（锁定源码提交），裁剪后 bun + rsbuild 构建；未实现的页面与入口一并删除。
- 手工实现仅限：认证、选路/重试、管理 API、日志统计、静态服务。

## 测试与交付

- 必须覆盖：现有库启动与读写兼容、Token 认证、渠道选择/重试、黄金协议转换、断连取消、并发流式内存。
- 真实上游冒烟测试需用户明确授权后再跑。
- 部署：本地交叉编译 linux/amd64，上传产物替换 Docker 服务；保留旧镜像与数据库备份，演练回滚。
- 每个里程碑提交 git，并在 `docs/REWRITE_SCOPE.md`/spec 中更新状态。
