# New API Desktop（Tauri 2）

无需 Docker 的本地桌面版：同一套 Rust 后端与 React 前端，单进程运行，可隐藏到系统托盘后台常驻。

## 依赖

- Rust 1.96（见仓库根 `rust-toolchain.toml`）
- Bun 1.4+
- macOS：Xcode Command Line Tools
- Windows：WebView2 Runtime + MSVC 生成工具
- Linux：webkit2gtk-4.1、libappindicator 等 Tauri 2 依赖

## 开发运行

```bash
cd desktop
bun install
bun run dev      # 构建 web/dist 并启动 tauri dev；窗口加载 http://127.0.0.1:3000
```

首次启动自动创建数据库与管理员，数据目录：

- macOS：`~/Library/Application Support/com.solo.newapi.desktop/`
- Windows：`%APPDATA%\com.solo.newapi.desktop\`
- Linux：`~/.local/share/com.solo.newapi.desktop/`

默认账号 `root / 123456`，登录后请修改密码。目录中已有 `one-api.db` 时只校验结构、不迁移；结构不匹配会弹窗报错并退出。

### 免登录

桌面版启动时自动进入管理界面，无需输入账号密码：

- 桌面进程设置 `DESKTOP_DISABLE_LOGIN=1`，服务端把它解析为桌面会话（管理员身份来自本地库）。
- 该开关**只在本机回环监听（127.0.0.1 / ::1）时生效**；Docker、公网或任何非回环绑定即使误设环境变量也会被忽略并打印警告，仍走正常登录。
- `/v1/*` 的客户端 Token 认证不受影响；Web 前端代码没有免登录分支，远程部署不存在绕过入口。
- 本地任意进程只要拿到回环端口即可访问管理 API，这是桌面单用户模式的预期取舍。

## 构建安装包

```bash
cd desktop
bun run build    # 构建 web/dist + tauri build；产物在 target/release/bundle/
```

- macOS：`.app` / `.dmg`（未签名，首次打开需右键「打开」）
- Windows：`.msi` / NSIS `setup.exe`
- Linux：`.AppImage` / `.deb`

跨平台需在对应系统上构建；当前在 macOS arm64 实测。

## 托盘与生命周期

- 关闭窗口：隐藏到系统托盘，服务继续运行，`/v1` 仍可访问。
- 托盘图标左键：切换窗口显示/隐藏；菜单：显示主窗口 / 隐藏到托盘 / 退出。
- ⌘Q（macOS）或托盘「退出」：优雅关停服务、日志落盘后退出。
- 二次启动只聚焦已有窗口（单实例）。
- 端口：`PORT` 优先，否则尝试 3000–3009，仍被占用则系统分配；仅绑定 `127.0.0.1`。
- 外部链接（文档、公告等）交给系统浏览器打开。

## 图标

```bash
bun run icons    # 从 desktop/app-icon.png（1024×1024，内容四周留透明边距）重新生成
```

`app-icon.png` 已按 macOS 图标网格留白（内容约占画布 80%），避免 Dock 中显得比其它应用偏大；更换品牌图时替换该文件后重新运行即可。

## 测试

```bash
cargo test -p solo-new-api-desktop
```
