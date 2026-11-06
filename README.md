# solo-new-api-rust

new-api 的个人自用 Rust 重写：单用户、多 Token、兼容现有 SQLite 数据，可无缝替换线上服务。

## 状态

设计阶段。范围、兼容性边界与功能取舍见 [docs/REWRITE_SCOPE.md](docs/REWRITE_SCOPE.md)。

## 目标

- 直接复用线上 `one-api.db`，不破坏原表结构。
- 保留多 Token 管理与 OpenAI/Anthropic 兼容转发。
- 复用 new-api 前端并裁剪为单用户所需页面。
- 面向高并发：SSE 零缓冲转发、进程内缓存/轮询、异步批量日志。
- 本地交叉编译 linux/amd64，上传服务器替换现有 Docker 服务。

## 开发

```bash
# 数据（不提交）放在 data/ 下
cargo build
```

技术选型与部署方式在设计与计划确认后补充。

## License

本仓库代码为个人重写；复制自 new-api 的前端代码遵循其 AGPL-3.0 许可。
