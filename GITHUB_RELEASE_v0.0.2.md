# GitHub Release 发布信息

## Release 配置

**Tag：**

```text
v0.0.2
```

**Target：**

```text
0.0.2
```

**Release title：**

```text
DNS Hearth v0.0.2 — Universal 2 后台运行与 DNS 管理
```

**安装包建议名称：**

```text
DNS-Hearth_0.0.2_macos_universal2.dmg
```

建议将该版本标记为 **Pre-release**。

---

# 以下内容复制到 GitHub Release 描述

DNS Hearth 是一款面向 macOS 的本地 DNS 管理与转发工具，主要用于解决多个 VPN、代理软件同时运行时，系统 DNS 配置被反复覆盖的问题。

`v0.0.2` 是 DNS Hearth 的第二个预览版本，重点改进 Universal 2 发布包、应用后台运行、Dock/托盘窗口恢复、系统 DNS 生命周期管理、首页自动应用和 DoH3 诊断。

## 本版本更新

### 后台运行与托盘

- 点击窗口关闭按钮后，应用隐藏到后台，不会停止 DNS 校正或本地 DNS 服务；
- 增加 macOS 菜单栏托盘图标；
- 可通过托盘图标重新打开 DNS Hearth；
- 关闭窗口后，点击 macOS Dock 图标也可以恢复并聚焦主窗口；
- 增加“退出并清空 DNS”菜单项；
- 应用图形界面不会以 root 身份运行。

### 系统 DNS 生命周期管理

- 应用真正退出前，会清空已配置网络服务的系统 DNS；
- 清理过程与后台 DNS 校正互斥，避免退出时发生并发覆盖；
- 任一网络服务清理失败时，不会伪造退出成功；
- 首页关闭某个网络服务的“监听”后，可单独点击“清空 DNS”；
- 单独清空 DNS 不会删除该网络服务保存的 DNS 预设；
- 清空失败会显示明确错误信息。

### 首页交互

- 移除“保存并立即应用”按钮；
- 修改 UDP 设置、检测频率、网络服务监听状态或 DNS 预设后自动应用；
- 自动应用使用短暂防抖，避免输入 DNS 地址时重复提交；
- 应用过程中显示当前阶段、超时和错误状态；
- 支持取消等待中的自动应用操作。

### 稳定性、权限与 DoH3

- 修复前端接收退出清理失败通知时缺少 `core:event` ACL 权限的问题；
- 增加 Tauri 事件监听和取消监听权限；
- 修复阿里云 DoH3 默认地址使用 IP 作为 TLS SNI 导致 QUIC 连接超时的问题；
- 阿里云 DoH3 默认入口改为 `https://dns.alidns.com/dns-query`；
- 启动时仅修复已知的旧版阿里云 DoH3 默认记录，不改写用户自行配置的其他 DoH3 上游；
- DoH3 连接失败信息现在包含上游名称、SNI、目标地址和 QUIC 阶段；
- 对比参考 DNS 服务实现并增加真实 UDP 443/QUIC 连接诊断；
- 保留前端错误透明传播，不使用默认值掩盖系统 DNS 或上游协议操作失败。

## 主要功能

- 展示 macOS 网络服务，并为每个网络服务单独配置 DNS 地址列表；
- 周期性检查系统 DNS，并在配置被 VPN 或代理软件覆盖后自动恢复；
- 支持本地 UDP DNS 转发和 `127.0.0.1` 系统 DNS 模式；
- 支持本地 DNS 记录、域名重写与域名拦截；
- 支持 UDP、DoT、DoH、DoQ 和 DoH3 上游协议；
- 支持并发、最快响应、轮询和随机查询策略；
- 提供 DNS 缓存管理、查询测试、查询日志和上游健康检查；
- 单实例运行，避免多个应用实例争用本地 DNS 端口；
- 所有管理操作通过本地 Tauri IPC 完成，不开放 HTTP 管理端口。

## 系统兼容性

本版本提供 Universal 2 安装包，同时支持：

- Intel Mac：`x86_64`；
- Apple Silicon：`arm64`，包括 M1、M2、M3、M4 及后续兼容的 Apple Silicon 处理器。

安装包：

```text
DNS-Hearth_0.0.2_macos_universal2.dmg
```

## 安装方式

1. 下载并打开 DMG；
2. 将 `DNS Hearth.app` 拖入 `Applications`；
3. 从“应用程序”目录启动 DNS Hearth。

## 首次打开

当前版本使用 ad-hoc 签名，尚未完成 Apple Developer ID 签名和 Apple 公证。

如果 macOS 提示“无法验证开发者”：

1. 在 Finder 中打开“应用程序”；
2. 右键点击 `DNS Hearth.app`；
3. 选择“打开”；
4. 在确认窗口中再次点击“打开”。

也可以前往：

```text
系统设置 → 隐私与安全性
```

然后点击“仍要打开”。

仅在确认安装包来自本项目 GitHub Release 后，也可以执行：

```bash
xattr -dr com.apple.quarantine "/Applications/DNS Hearth.app"
```

请勿全局关闭 Gatekeeper。

## UDP 53 管理员授权

首次启用本地 UDP 53 监听时，应用会请求一次 macOS 管理员授权，用于安装职责受限的 DNS 端口转发服务。

- DNS Hearth 图形界面不会以 root 身份运行；
- 应用不会保存管理员密码；
- helper 只负责将本机 UDP 53 转发到应用内部监听端口；
- helper 版本未变化时，后续关闭和重新启用 UDP 不应重复授权；
- 应用升级并更新 helper 时，可能需要再次授权。

## 数据位置

应用数据目录：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/
```

SQLite 数据库：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/dns-hearth.db
```

运行日志：

```text
~/Library/Application\ Support/com.lhstack.dns-hearth/logs/
```

删除应用不会自动删除数据库和日志。

## 文件校验

当前重新构建的 Universal 2 DMG：

```text
DNS Hearth_0.0.2_universal.dmg
```

建议上传 GitHub Release 时重命名为：

```text
DNS-Hearth_0.0.2_macos_universal2.dmg
```

构建信息：

```text
版本：0.0.2
DMG 大小：21922965 bytes
DMG SHA-256：8c53e381713ffdc1e8a9162bc6fab4d225feb62d636760b0609666e161143933
应用主二进制 SHA-256：316ac5a775187fc423b074ede26ed1e723cfed18b24e6f61d6388a0f007c06a5
应用主二进制为 Universal 2：x86_64 arm64
```

校验命令：

```bash
shasum -a 256 DNS-Hearth_0.0.2_macos_universal2.dmg
```

重命名不会改变文件内容，因此 SHA-256 不会改变。

## DoH3 配置与网络说明

DoH3 使用 QUIC + TLS，地址中的主机名同时用于 TLS SNI 和 HTTP/3 请求主机。对于要求域名 SNI 的服务端，应使用服务商提供的域名地址，不要直接把 IP 写成 HTTPS URL。

本版本已将默认阿里云 DoH3 地址修正为：

```text
https://dns.alidns.com/dns-query
```

如果此前已经使用旧版默认地址，应用启动时会仅针对“阿里云H3”的旧 IP 地址记录执行一次明确修复。用户自己添加或修改的其他 DoH3 上游不会被改写。

DoH3 依赖 UDP 443/QUIC。若当前网络经过 VPN、TUN 或代理，且路由走 `utun` 虚拟接口，或代理仅转发 TCP/HTTP 而不转发 UDP 443，DoH3 会在 QUIC 握手阶段超时。这不是 DNS 报文解析失败，也不是自动降级为 DoH 的理由。

排查时请确认：

- `dns.alidns.com` 可以解析到地址；
- 路由没有被仅支持 TCP 的代理接管；
- VPN/代理允许 UDP 443 和 QUIC；
- 使用参考项目或独立 QUIC 工具测试时，同一网络也能建立 UDP 443 连接。

本次对比了参考 DNS 服务项目的 DoH3 客户端实现和依赖版本，确认应用与参考项目的 QUIC、TLS、ALPN `h3`、HTTP/3 请求流程一致。若两者在同一网络下都超时，应优先检查 VPN、TUN 或代理的 UDP 443 能力。

## 问题反馈

这是 `0.0.2` 预览版本。使用过程中如遇到问题，请在 GitHub Issues 中提供：

- macOS 版本；
- Mac 处理器类型；
- DNS Hearth 版本；
- 上游名称、地址和协议；
- 具体错误信息；
- 问题复现步骤；
- 必要的应用日志。
