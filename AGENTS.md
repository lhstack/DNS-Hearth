# AGENTS.md

## 项目概述

- `mac-bind-dns` 用于解决 macOS 多个 VPN 同时启用时系统 DNS 配置互相覆盖的问题。
- 项目在 FluxDNS 完整 DNS 代理能力基础上，提供可选的本地 DNS 服务、系统 DNS 预设地址管理和周期性系统 DNS 校正。
- 桌面应用采用 Tauri；本地 HTTP 管理服务承载 DNS 管理 API 与前端页面。

## 工程环境与主要工具

- Rust 2021 edition，异步运行时为 Tokio。
- DNS 代理核心来自 `/Volumes/Documents/projects/rust/rust-dns-server/backend`，包含 hickory-proto、UDP、DoT、DoH、DoQ/DoH3 上游与服务端实现。
- 管理界面为 Vue 3 + Vite + TypeScript + Element Plus。
- 桌面壳为 Tauri v2；macOS 系统 DNS 操作通过明确调用 `networksetup` 完成，不在 DNS 核心中隐式处理。
- 当前未确认 CI、发布签名和 notarization 配置。

## 目录与模块结构

- `src/application/`：应用启动装配和 HTTP Web 路由。
- `src/business/`：业务用例编排。
- `src/dns/`：DNS 报文、解析、缓存、重写、上游代理和各协议服务端。
- `src/infrastructure/`：配置、日志、数据库、监听器、系统 DNS 控制和监控。
- `frontend/`：Vue 管理界面，构建产物嵌入后端静态资源。
- `src-tauri/`：Tauri 桌面应用配置与构建入口（若与后端分离使用）。

## 分层架构与依赖方向

- 启动装配位于 `application::bootstrap`，负责组装配置、数据库、DNS 核心、监听器、Web 路由和桌面控制任务。
- DNS 请求路径由 `dns::resolver` 编排，协议服务端依赖 resolver；系统 DNS 控制只依赖配置与 macOS 命令，不参与 DNS 查询解析。
- HTTP handler 通过业务入口访问数据和 DNS 组件；新增系统 DNS 行为必须保留在其基础设施职责边界内。
- 不使用运行时数据库迁移、伪迁移或静默回退掩盖配置/数据错误。

## 构建、测试和验证方式

- Rust：在项目根目录执行 `cargo fmt --check`、`cargo test`、`cargo build`。
- 前端：在 `frontend/` 执行 `pnpm build`（需要 Node/pnpm 依赖）。
- Tauri：在配置和依赖可用时执行 `cargo tauri build`；当前签名/notarization 未确认。
- 修改后优先验证受影响模块，再进行全项目构建。

## 项目编码约定

- Rust 使用 snake_case / CamelCase 命名，异步共享状态使用 `Arc` 与 Tokio 锁。
- 配置和边界输入使用显式校验；错误应返回并记录，不用默认值掩盖无效配置。
- 入口方法表达高层流程，具体协议、系统命令和持久化细节下沉到职责明确的方法。
- 前端保持现有 Vue 单文件组件和 Element Plus 风格。

## 错误处理约定

- 初始化和不可恢复错误向上返回，不能伪造成功。
- DNS 查询失败按 DNS 协议语义生成失败响应；配置和系统 DNS 操作失败应在控制层显式暴露。
- 系统命令必须检查退出状态和输出，不能只依赖命令存在或吞掉 stderr。

## 版本控制信息

- 当前目标项目初始化前为空目录，尚未确认其 Git 提交历史、远程仓库和 CI 约定。
- 参考项目使用 Git，提交身份和历史仅作为迁移来源，不作为当前项目用户习惯的证据。

## 有证据支持的用户编码习惯

- 当前目标项目没有可用于判断用户个人编码习惯的提交记录，未确认。

## 当前无法确认的事项

- Tauri 应用图标、签名、自动更新、发布渠道和 notarization 未确认。
- macOS 支持的最低版本及 `networksetup` 服务选择策略未确认；实现默认要求用户显式配置网络服务名称，不自动猜测目标服务。
