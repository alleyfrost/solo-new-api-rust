# 上游版本对齐与同步

本项目的定位是“前端 fork + 后端重写”，上游更新不会自动进入本仓库。本文记录当前对齐点、哪些代码来自上游、以及升级时的具体步骤。

## 当前对齐版本

| 上游 | 对齐点 | 说明 |
| --- | --- | --- |
| [new-api](https://github.com/QuantumNous/new-api) | `e03439033238722c0b8d4437a052445f3aeeed83`（2026-11-06，`main`） | 提交标题：`refactor(aws): remove the unused api_key HTTP request path`；`git describe`：`relaykit/v0.3.7-4-ge03439033` |
| [Switchyard](https://github.com/NVIDIA-NeMo/Switchyard) | `c365efe832822541024704aadf6a0d5c81cc0528`（`switchyard-translation` 0.3.0） | 以 vendored 形式放在 `vendor/`，Apache-2.0 |
| 前端复制范围 | new-api `web/` 目录 | 已按单用户场景裁剪并改造，不是可直接覆盖的镜像 |
| Rust / Bun | 1.96.0 / 1.4.4 | 见 `rust-toolchain.toml` 与 `web/package.json` |

对齐点只保证**行为与页面基线**来自该版本；后端 `src/` 是独立实现，不包含 new-api 的 Go 代码。

## 代码归属

| 路径 | 来源 | 升级方式 |
| --- | --- | --- |
| `web/` | 复制自 new-api `web/`，裁剪与改造 | 按上游 diff 手工挑拣，禁止整目录覆盖 |
| `vendor/switchyard-*` | Switchyard 源码 | 整目录替换 + 固定新版本，跑 golden 转换测试 |
| `src/` | 本项目重写 | 不随上游代码变动；只跟随上游**协议/行为**变化调整 |
| `tests/` | 本项目 | 上游行为变化时补回归用例 |
| 内部设计稿与验收记录 | 开发环境私有，不在开源仓库 | 升级后更新内部副本的源码锚点 |

## 升级检查清单

以升级到 `NEW_COMMIT` 为例：

```bash
# 1. 取上游并确认目标提交
git -C /path/to/new-api fetch origin
git -C /path/to/new-api log -1 --format='%H %ad %s' --date=short origin/main
git -C /path/to/new-api describe --tags NEW_COMMIT

# 2. 只看前端差异，按模块评估
git -C /path/to/new-api diff --stat OLD_COMMIT..NEW_COMMIT -- web/
git -C /path/to/new-api diff OLD_COMMIT..NEW_COMMIT -- web/src/features/<area>

# 3. 关注影响后端的协议/行为改动（不复制 Go 代码）
git -C /path/to/new-api diff --stat OLD_COMMIT..NEW_COMMIT -- relay/ controller/ model/ setting/
```

逐项处理：

1. **前端**：按 feature 目录挑拣上游修复与新增能力；保留本项目的裁剪结果与改造点（见下节）。上游新增的 i18n key 需同步进 7 个 locale 文件，删除的页面不要重新引入。
2. **后端行为**：若上游调整了端点字段、错误形状、usage 口径、重试策略或 options 语义，更新 `src/relay/*`、`src/routes/*` 与对应集成测试；不要引入 Redis、任务系统、计费引擎等被明确排除的依赖。
3. **协议转换库**：如升级 Switchyard，替换 `vendor/switchyard-protocol` 与 `vendor/switchyard-translation`，更新 `Cargo.lock`，重跑 `tests/translation_probe.rs` 与跨协议用例。
4. **数据库**：任何情况下都不改现有表结构；上游 schema 变化只允许在读取层兼容（忽略未知列、保留未知 JSON 字段）。
5. **门禁**：见下节，全部通过后再更新本文的对齐表与内部设计稿的“源码锚点”。

## 同步时必须保留的本项目改造

直接套用上游文件会覆盖以下定制，合并时逐项确认：

- 侧边栏：「聊天」组置顶游乐场；「模型调用分析」改名「数据看板」；概览页删除公告/FAQ/API 信息/运行时间四张恒空卡。
- 品牌：蓝紫新月 + 星芒的 `favicon.ico`/`logo.png` 与顶栏徽标。
- 系统设置：只保留站点信息、日志维护、生图模型三项，8 个白名单键。
- Playground：统一聊天界面；生图结果内联渲染，本地历史用 `[[image-omitted]]` 占位，不保存 base64。
- 管理端：金额/定价列改为用量口径；渠道的 Ollama/上游同步/多 Key 管理面板等已裁剪入口不要恢复。
- 认证：注册、找回密码、OAuth、2FA/Passkey 入口保持删除；令牌新建支持自定义密钥、编辑不改密钥。
- 前端工程：`bun` + `rsbuild` 构建、单用户路由（`/keys`、`/channels`、`/models`、`/usage-logs`、`/dashboard`、`/playground`、`/profile`、`/settings`）。

## 升级后的门禁

```bash
cargo test
REAL_DB=/path/to/one-api.db cargo test -- --ignored   # 可选：真实库兼容性

cd web
bun install
bun run typecheck
bun run test
bunx oxfmt --check <改动文件>
bun run copyright:check

cd ..
bash deploy/build.sh
bash deploy/run-local.sh   # :3006，本地 Docker 验收
```

`copyright:check` 目前有 11 个既有 offender，升级时不要新增；本地 Docker 验收至少覆盖登录、Token 转发、流式、日志落库、设置保存与游乐场。

## 版本记录

| 日期 | new-api | Switchyard | 说明 |
| --- | --- | --- | --- |
| 2026-11-06 | `e03439033238722c0b8d4437a052445f3aeeed83` | `c365efe832822541024704aadf6a0d5c81cc0528`（0.3.0） | 初始基线：前端 fork、后端重写、协议转换 vendored |

升级完成后在此表追加一行，并同步更新 [VS-NEW-API.md](VS-NEW-API.md) 的“上游与维护”一节。
