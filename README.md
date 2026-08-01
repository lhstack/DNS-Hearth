# DNS Hearth

DNS Hearth 是一款面向 macOS 的本地 DNS 管理与转发工具，主要用于解决多个 VPN、代理软件同时运行时，系统 DNS 被重复覆盖的问题。

应用可以为每个 macOS 网络服务分别保存 DNS 地址列表，按照指定频率检查系统配置，并在 DNS 被其他程序修改后恢复对应的预设值。需要本地解析能力时，也可以启用本机 UDP DNS 服务，将系统 DNS 指向 `127.0.0.1`。

> 当前版本主要面向 macOS。应用管理界面通过 Tauri IPC 与 Rust 后端通信，不开放本地 HTTP 管理端口。

## 主要功能

- 展示 macOS 网络服务，并为每个网络服务单独配置 DNS 地址列表；
- 周期性检查系统 DNS，发现被 VPN 或代理软件覆盖后自动恢复；
- 在首页统一配置网络服务 DNS 和本地 UDP DNS 监听；
- 支持本地 DNS 记录、域名重写和域名拦截；
- 支持 UDP、DoT、DoH、DoQ、DoH3 等上游协议；
- 支持并发、最快响应、轮询和随机查询策略；
- “并发查询”同时请求所有健康上游，最快返回 `NOERROR` 时立即返回；如果没有 `NOERROR`，等待所有上游结束后返回最快的 `NXDOMAIN`；
- “最快响应”根据历史成功查询延迟选择最低延迟的健康上游，只向该上游发送查询；
- 提供 DNS 缓存管理、查询测试、查询日志和上游健康状态；
- 使用 SQLite 持久化配置与查询记录；
- 单实例运行，避免多个应用实例争用本地 DNS 端口；
- 使用 Element Plus 默认设计语言：标准蓝色主操作、白色卡片、浅灰页面背景和清晰的状态色，并统一卡片、表单、按钮与表格样式；
- 业务页面使用 Element Plus 按钮、输入框和图标组件，侧边栏折叠禁用重复过渡，避免文字残留和图标背景遮挡。

## DNS 查询策略与缓存

“并发查询”会同时请求所有健康上游，收到最快的 `NOERROR` 后立即返回；如果没有任何 `NOERROR`，则等待所有上游完成并返回最快的 `NXDOMAIN`。如果两类有效结论都没有收到，返回明确的上游失败，不伪造 DNS 结果。

“最快响应”不会并发广播查询，而是根据历史成功查询延迟选择最低延迟的健康上游，只向该上游发送本次查询并返回结果；该上游发生传输失败时，沿用单上游故障转移路径。首次启动且没有保存过查询策略时默认使用“并发查询”；已有用户保存的策略不会被覆盖。

缓存遵守上游 DNS 响应中的最小 TTL。缓存页面配置的默认 TTL 只用于没有 Answer 记录、无法从 Answer 获取 TTL 的响应，不会强制把所有域名缓存为该时长。这样可以避免 CDN 或 GSLB 地址变化后继续返回过期地址。

缓存命中后，界面查询日志会标记“缓存”为“是”，响应不会再显示上游服务器名称。不同域名、记录类型和上游 TTL 不同，命中率会有所差异；上游 TTL 较短的域名在 TTL 过期后重新查询属于正常行为。

## 技术栈

- Rust 2021、Tokio；
- Tauri 2；
- Vue 3、TypeScript、Vite；
- Element Plus；
- SQLite / SQLx；
- Hickory DNS 协议组件。

## macOS 安装

当前发布包为 Universal 2，同一个 DMG 同时支持：

- Intel Mac：`x86_64`；
- Apple Silicon：`arm64`，包括 M1、M2、M3、M4 及后续兼容机型。

构建产物默认位于：

```text
src-tauri/target/universal-apple-darwin/release/bundle/macos/DNS Hearth.app
src-tauri/target/universal-apple-darwin/release/bundle/dmg/DNS Hearth_0.0.2_universal.dmg
```

安装步骤：

1. 打开 `.dmg` 文件；
2. 将 `DNS Hearth.app` 拖入“应用程序”目录；
3. 从 `/Applications` 启动 DNS Hearth。

### 允许 macOS 信任并打开应用

当前开发包使用 ad-hoc 签名，尚未使用 Apple Developer ID 签名和 Apple 公证。macOS Gatekeeper 可能提示“无法验证开发者”或阻止首次打开。

仅在确认应用包来自可信来源后，使用以下任一方式打开。

#### 方式一：右键打开

1. 在 Finder 中打开“应用程序”；
2. 找到 `DNS Hearth.app`；
3. 按住 Control 点击应用，或直接右键；
4. 选择“打开”；
5. 在确认窗口中再次点击“打开”。

该方式通常只需在首次启动时操作一次。

#### 方式二：在系统设置中允许

如果应用已经被系统拦截：

1. 尝试正常启动一次 DNS Hearth；
2. 打开“系统设置”；
3. 进入“隐私与安全性”；
4. 在“安全性”区域找到 DNS Hearth 被阻止的提示；
5. 点击“仍要打开”；
6. 使用 Touch ID 或当前 macOS 用户密码确认；
7. 再次启动应用。

#### 方式三：移除隔离属性（仅限确认可信的本地构建）

如果前两种方式不可用，可以在终端执行：

```bash
xattr -dr com.apple.quarantine "/Applications/DNS Hearth.app"
```

然后重新启动应用。

不要通过下面的方式全局关闭 Gatekeeper：

```text
spctl --master-disable
```

全局关闭系统安全检查会影响所有应用，不是 DNS Hearth 所必需的操作。

### UDP 53 管理员授权

macOS 当前运行环境下，普通桌面进程不能直接绑定 DNS 使用的 UDP 53 端口。首次启用本地 UDP 53 监听时，DNS Hearth 会请求管理员授权，安装一个职责受限的 LaunchDaemon/helper。

helper 只负责：

```text
本机 UDP 53 → DNS Hearth 的 loopback 高位端口
```

主界面不会以 root 身份运行，也不会缓存管理员密码。helper 版本未变化时，后续关闭和重新开启 UDP 监听不应重复请求授权；应用升级导致 helper 版本变化时，可能需要再次授权更新。

系统级文件位置为：

```text
/Library/PrivilegedHelperTools/com.lhstack.dns-hearth.udp-forwarder-<UID>
/Library/LaunchDaemons/com.lhstack.dns-hearth.udp-forwarder.<UID>.plist
/var/run/com.lhstack.dns-hearth.udp-forwarder-<UID>.sock
```

其中 `<UID>` 是当前 macOS 用户的数字 UID。

## 数据库与日志位置

DNS Hearth 使用 Tauri 的应用数据目录保存可变数据，不会将数据库或日志写进 `.app` 安装包，也不依赖应用启动时的工作目录。

当前 Bundle Identifier 为：

```text
com.lhstack.dns-hearth
```

因此 macOS 应用数据目录是：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/
```

### 数据库

SQLite 主数据库文件：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/dns-hearth.db
```

应用运行期间还可能出现 SQLite WAL 文件：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/dns-hearth.db-wal
~/Library/Application\ Support/com.lhstack.dns-hearth/dns-hearth.db-shm
```

数据库中保存的内容包括网络服务 DNS 配置、本地记录、重写规则、上游服务器、缓存配置、查询策略和查询日志等。

### 运行日志

日志目录：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/logs/
```

日志文件以 `dns-proxy.log` 为基础名称，并按天轮转，例如：

```text
logs/dns-proxy.log.2026-08-01
```

运行日志用于记录应用初始化、监听器、DNS 转发和系统操作等诊断信息。页面中的“查询日志”属于业务数据，保存在 SQLite 数据库中，与上述运行日志不是同一类数据。

### 在 Finder 中打开数据目录

可以在 Finder 中选择：

```text
前往 → 前往文件夹…
```

然后输入：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/
```

也可以在终端执行：

```bash
open "$HOME/Library/Application\ Support/com.lhstack.dns-hearth"
```

### 备份与清理

备份前应先完全退出 DNS Hearth，再复制整个目录：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/
```

不要在应用运行时只复制 `dns-hearth.db`，因为尚未合并的数据可能仍位于 `-wal` 文件中。

删除 `.app` 不会自动删除数据库和日志。如果确认不再需要现有配置与历史数据，可以在完全退出应用后手动删除上述应用数据目录。该操作不可恢复，建议先备份。

## 开发环境

建议准备：

- macOS；
- Rust stable 与 Cargo；
- Node.js 与 npm；
- Xcode Command Line Tools；
- 项目本地 Tauri CLI（由根目录 `package.json` 管理）。

安装依赖：

```bash
npm install
npm --prefix frontend install
```

启动开发应用：

```bash
npx tauri dev
```

只启动前端开发服务器：

```bash
npm --prefix frontend run dev
```

## 检查与测试

Rust 编译检查：

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Rust 测试：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

前端 TypeScript/Vue 检查：

```bash
cd frontend
npx vue-tsc -b --noEmit
```

前端生产构建：

```bash
npm --prefix frontend run build
```

## 构建 macOS 应用

生成同时支持 Intel Mac 和 Apple Silicon 的 Universal 2 `.app` 与 `.dmg`：

```bash
npm exec -- tauri build --target universal-apple-darwin --bundles app dmg
```

Universal 2 产物目录：

```text
src-tauri/target/universal-apple-darwin/release/bundle/
```

该构建会分别编译 `x86_64-apple-darwin` 和 `aarch64-apple-darwin`，再合并为包含 `x86_64 arm64` 的 Universal 2 主程序。

正式向其他用户分发前，应配置 Apple Developer ID 签名并完成 notarization。ad-hoc 签名只适合本机开发、测试或可信环境中的手动安装。

## 安全说明

- DNS Hearth 的 GUI 必须以普通用户身份运行；
- 不要使用 `sudo` 启动整个应用；
- UDP 53 由最小职责的特权 helper 提供端口转发；
- 管理界面只通过本地 Tauri IPC 通信；
- 数据库和日志保存在当前用户的应用数据目录；
- 不要安装来源不明或校验结果不可信的应用包。
