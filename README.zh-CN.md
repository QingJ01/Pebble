<p align="center">
  <img src="src/assets/app-icon.png" alt="Pebble logo" width="120">
</p>

<h1 align="center">Pebble</h1>

<p align="center">
  一个本地优先的桌面邮件客户端，让收件箱更安静、更清晰，也更可控。
</p>

<p align="center">
  <a href="README.md">English</a>
  ·
  <a href="https://github.com/QingJ01/Pebble/releases">发布版本</a>
  ·
  <a href="LICENSE">许可证</a>
</p>

<p align="center">
  <a href="https://github.com/QingJ01/Pebble/releases"><img src="https://img.shields.io/github/v/release/QingJ01/Pebble?style=flat-square&color=d4714e" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue?style=flat-square" alt="License"></a>
  <a href="https://github.com/QingJ01/Pebble/actions"><img src="https://img.shields.io/github/actions/workflow/status/QingJ01/Pebble/ci.yml?style=flat-square&label=build" alt="Build"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=flat-square" alt="Platform">
</p>

## 项目简介

Pebble 是一个使用 Rust、Tauri 和 React 构建的桌面邮件客户端。它默认把邮件数据、搜索索引、附件、规则和应用设置保存在本机。

Pebble 的设计目标很直接：

- 邮箱应该清晰、快速、安静。
- 邮件工作流应该本地优先，而不是被云端仪表盘绑住。
- 隐私控制应该明确可见，并且可以按单封邮件临时放宽。
- 搜索、稍后提醒、规则和看板应该协同工作，而不是散落在不同工具里。

Pebble 目前支持 Gmail、IMAP，以及实验性的 Outlook 账户。

## 主要特性

### 本地优先与隐私

- 使用本地 SQLite 数据库存储邮件、文件夹、标签、规则和设置。
- 使用本地 Tantivy 全文索引提供快速搜索。
- 附件保存在应用数据目录下。
- OAuth token 和账号凭据会使用设备本地密钥加密。
- 不包含遥测。
- 网络请求只发生在你启用的功能中：邮件同步、翻译、可选的 WebDAV 设置备份。

### 邮件处理

- 多账户聚合收件箱。
- 支持 Gmail、IMAP 和实验性的 Outlook。
- 支持线程视图和普通邮件列表视图。
- 支持归档、删除、星标、标记已读、批量操作和恢复。
- 支持邮件稍后提醒。
- 支持全文搜索和高级过滤。
- 支持规则引擎，自动整理邮件。

### 效率工具

- 看板视图，包含 Todo、Waiting、Done 三列。
- 命令面板和键盘优先导航。
- 内置翻译能力，支持双语阅读。
- 深色和浅色主题。
- 内置英文和中文界面。
- 可选的 WebDAV 备份，用于同步设置、规则、看板卡片和看板备注。

## 截图

<table>
  <tr>
    <td><img src="site/screenshots/inbox.png" alt="收件箱"><br><b>收件箱</b></td>
    <td><img src="site/screenshots/kanban.png" alt="看板"><br><b>看板</b></td>
  </tr>
  <tr>
    <td><img src="site/screenshots/dark.png" alt="深色模式"><br><b>深色模式</b></td>
    <td><img src="site/screenshots/settings.png" alt="设置"><br><b>设置</b></td>
  </tr>
</table>

## 技术栈

| 层级 | 技术 |
| --- | --- |
| 桌面框架 | Tauri 2 |
| 后端 | Rust |
| 前端 | React 19、TypeScript |
| 状态管理 | Zustand、TanStack Query |
| 数据库 | SQLite / rusqlite |
| 搜索 | Tantivy |
| 样式 | Tailwind CSS 和应用 CSS |
| 国际化 | i18next |

## 开始使用

### 环境要求

- Rust stable
- Node.js 18 或更新版本
- pnpm 8 或更新版本
- 当前平台所需的 Tauri 系统依赖

### 开发环境

```bash
git clone https://github.com/QingJ01/Pebble.git
cd Pebble

pnpm install
cp .env.example .env

pnpm dev
```

`pnpm dev` 会启动 Vite 前端开发服务，并运行 Tauri 桌面应用。

### 构建

```bash
pnpm build
pnpm build:windows
pnpm build:macos
pnpm build:linux
```

桌面端构建产物会输出到 `target/release/` 和 `target/release/bundle/`。
macOS 构建产物默认不签名，除非你自行配置签名流程。
将未签名的 macOS 构建复制到 `/Applications` 后，请先执行下面的命令再打开应用：

```bash
sudo xattr -cr /Applications/Pebble.app
```

## OAuth 配置

Pebble 可以通过 OAuth 连接 Gmail 和 Outlook。IMAP 账户使用应用内配置的 IMAP/SMTP 凭据。

复制 `.env.example` 为 `.env`，然后填写你需要的提供商配置。

| 变量 | 说明 |
| --- | --- |
| `GOOGLE_CLIENT_ID` | Google OAuth 客户端 ID，推荐使用 Desktop app 类型。 |
| `GOOGLE_CLIENT_SECRET` | 可选。若 Google 登录时报 `client_secret is missing`，再填写该项。 |
| `MICROSOFT_CLIENT_ID` | Microsoft public/native app 客户端 ID。 |
| `MICROSOFT_CLIENT_SECRET` | 可选。public/native Microsoft 应用通常应留空。 |

## 常用命令

| 命令 | 用途 |
| --- | --- |
| `pnpm dev` | 以开发模式运行 Tauri 桌面应用。 |
| `pnpm dev:frontend` | 只启动 Vite 前端开发服务。 |
| `pnpm test` | 使用 Vitest 运行前端测试。 |
| `pnpm build:frontend` | 类型检查并构建前端。 |
| `pnpm build` | 为当前平台构建桌面应用。 |
| `pnpm build:windows` | 构建 Windows NSIS 安装包。 |
| `pnpm build:macos` | 构建未签名的 macOS `.app` 和 `.dmg` 包。 |
| `pnpm build:linux` | 构建 Linux `.AppImage` 包。 |
| `cargo test -p pebble-mail` | 运行邮件模块测试。 |
| `cargo check` | 检查 Rust 工作区。 |

## 项目结构

```text
Pebble/
|-- src/                    React 前端
|   |-- components/         通用 UI 组件
|   |-- features/           收件箱、写信、搜索、看板、设置等功能
|   |-- hooks/              React hooks 和查询工具
|   |-- lib/                IPC API、i18n、通用工具
|   `-- stores/             Zustand 状态管理
|-- src-tauri/              Tauri 应用和 IPC 命令
|-- crates/                 Rust 工作区
|   |-- pebble-core/        共享类型和错误定义
|   |-- pebble-store/       SQLite 持久化
|   |-- pebble-mail/        邮件提供商和同步逻辑
|   |-- pebble-search/      Tantivy 搜索索引
|   |-- pebble-crypto/      凭据加密
|   |-- pebble-oauth/       OAuth 2.0 和 PKCE
|   |-- pebble-rules/       规则引擎
|   |-- pebble-translate/   翻译提供商
|   `-- pebble-privacy/     HTML 清理和追踪保护
|-- tests/                  前端测试
`-- site/                   静态项目站点和截图
```

## 快捷键

| 快捷键 | 操作 |
| --- | --- |
| `J` / `K` | 在邮件列表中上下移动 |
| `Enter` | 打开选中的邮件 |
| `E` | 归档 |
| `S` | 切换星标 |
| `R` | 回复 |
| `A` | 回复全部 |
| `F` | 转发 |
| `C` | 写新邮件 |
| `/` | 聚焦搜索 |
| `Esc` | 关闭、取消或返回 |

快捷键可以在设置中查看和自定义。

## 当前状态

Pebble 正在持续开发中。它可以用于日常测试，但邮件客户端会处理敏感数据，不同邮件服务商的行为也存在差异。测试新版本时，请为重要邮件保留备份，并在服务商网页端核对关键操作。

## 参与贡献

欢迎提交 issue 和 pull request。

代码改动请尽量保持聚焦；涉及行为变化时，请补充相应测试。提交前建议运行相关检查：

```bash
pnpm test
pnpm build:frontend
cargo check
```

## 许可证

Pebble 使用 [GNU Affero General Public License v3.0](LICENSE) 许可证。

---

<p align="center">
  由 <a href="https://github.com/QingJ01">QingJ</a> 构建。
  <br>
  友情链接：<a href="https://linux.do">LINUX DO</a>
</p>
