# Pebble 严格代码审计报告

日期：2026-04-05  
审计范围：`d:\project\Pebble` 全仓  
审计方式：前端审查、后端/Rust/Tauri 审查、规格闭环审查三条并行检查线；再用构建与测试结果交叉验证

## 总体判断

当前项目更接近 **Alpha / 高保真 Demo**，而不是可稳定交付的 Beta。

原因很直接：

1. 规格中最核心的多 Provider 路径没有真正打通，Gmail/Outlook/OAuth 仍停留在半实现状态。
2. 多个面向用户宣传的功能只有数据模型或局部 UI，没有完整闭环，例如规则引擎、Snooze 回流、命令面板操作、Cloud Sync 自动化。
3. 安全设计与规格存在明显背离，尤其是翻译配置/API Key 明文存储与导出、HTML 邮件未做 iframe sandbox 隔离。

## 高优先级问题

### P0 未完成：OAuth / Gmail / Outlook 账户链路仍不可用

- 规格明确要求多 Provider 架构与 OAuth2 PKCE，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:12`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:256`, `docs/superpowers/plans/2026-04-04-pebble-phase6-oauth2-providers.md:5`.
- 现状 1：OAuth client id 仍是占位符，见 `src-tauri/src/commands/oauth.rs:7-24`。
- 现状 2：`start_oauth_flow()` 只返回 URL，不保存 PKCE state，代码自己也写了 TODO，见 `src-tauri/src/commands/oauth.rs:62-77`。
- 现状 3：`complete_oauth_flow()` 会重新 `start_auth()` 生成新的 PKCE state，再等待新的 redirect，但并没有把新的 auth URL 交给前端，也没有复用 `start_oauth_flow()` 产生的 state，见 `src-tauri/src/commands/oauth.rs:86-111`，以及 `crates/pebble-oauth/src/lib.rs:72-109`。
- 现状 4：设置页把 OAuth 入口直接隐藏了，见 `src/features/settings/AccountsTab.tsx:171-176`。
- 现状 5：同步层仍只会构造 `ImapMailProvider`，不会按账户 provider 分发，见 `src-tauri/src/commands/sync_cmd.rs:55-80`；而工厂函数虽然存在，却没有被同步路径使用，见 `crates/pebble-mail/src/provider/mod.rs:10-45`。
- 影响：Gmail/Outlook API 路径在当前代码下没有真实可用入口，也没有真实同步闭环。

### P0 未完成：规则引擎只有 CRUD 和库实现，没有进入生产链路

- 规格把“智能分类/标签”列为 MVP 功能，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:22`。
- `RuleEngine` 已实现，见 `crates/pebble-rules/src/lib.rs:8-48`。
- `src-tauri/src/commands/rules.rs:6-48` 只提供规则的增删改查。
- 全仓搜索 `RuleEngine::new` / `pebble_rules::`，生产代码没有命中；当前命中只出现在 `crates/pebble-rules/src/lib.rs` 内部和测试里。
- 影响：用户可以创建规则，但新邮件同步时不会执行规则，不会自动打标、归档、标已读或进看板。

### P0 设计/实现断裂：Provider 抽象已经定义，但同步核心仍被 IMAP 细节穿透

- 规格要求统一 Provider 能力与工厂分发，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:256-301`。
- `MailProvider` / `DraftProvider` / `CategoryProvider` 等 trait 已定义，见 `crates/pebble-core/src/traits.rs:78-135`。
- 但 `SyncWorker` 字段类型仍是 `Arc<ImapMailProvider>`，不是 trait object，见 `crates/pebble-mail/src/sync.rs:82-101`。
- `start_sync_inner()` 也强行把 `auth_data` 反序列化为 `ImapConfig`，并直接 new `ImapMailProvider`，见 `src-tauri/src/commands/sync_cmd.rs:55-80`。
- 影响：架构名义上支持三种 provider，实际最关键的后台同步路径仍然只支持 IMAP。

## 明确 Bug

### P1 Bug：Snooze 的 `return_to` 实际没有生效

- 规格要求 Snooze 到时“回到 `return_to` 指定位置”，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:193`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:646-655`。
- 前端调用时传的是当前视图名，例如 `inbox` / `search` / `kanban`，见 `src/features/inbox/SnoozePopover.tsx:55-60`。这已经不是规格要求的语义位置格式，例如 `kanban:todo`。
- 后台 watcher 到点后只 `unsnooze_message()` 并发出 `mail:unsnoozed` 事件，完全没有读取或使用 `return_to`，见 `src-tauri/src/snooze_watcher.rs:32-57`。
- 前端也没有任何地方监听 `mail:unsnoozed` 事件；全仓搜索只在 watcher 和常量定义里出现。
- 影响：Snooze 到期只会把记录删掉，不会按目标位置恢复；`return_to` 当前是死字段。

### P1 Bug：附件下载后的“已下载路径”显示的是缓存源路径，不是用户保存路径

- 前端下载成功后调用 `getAttachmentPath()` 并把返回值当作下载路径展示，见 `src/components/AttachmentList.tsx:67-70`, `src/components/AttachmentList.tsx:144-161`。
- 但 `get_attachment_path()` 返回的是附件记录里的 `local_path`，即缓存源文件，见 `src-tauri/src/commands/attachments.rs:14-19`。
- 真正的下载行为只是把源文件复制到 `save_to`，并不会更新数据库里的 `local_path`，见 `src-tauri/src/commands/attachments.rs:23-37`。
- 影响：用户下载到下载目录后，UI 显示/提示的仍是应用缓存路径，信息错误。

### P1 Bug：回复全部时不会稳定排除自己的邮箱地址

- `ComposeView` 在组件首次渲染时用 `myEmail` 初始化 `to` / `cc`，见 `src/features/compose/ComposeView.tsx:32-53`。
- `myEmail` 依赖 `useAccountsQuery()` 返回的账户数据，见 `src/features/compose/ComposeView.tsx:29-35`。
- 但这些 state 只在第一次渲染时计算，后续账户数据加载完成后不会重算，也没有 `useEffect` 修正。
- 影响：在 `reply-all` 场景里，如果账户数据尚未就绪，自己的地址会被保留在收件人列表中。

### P1 Bug：本地标记已读/星标不会回写远端

- `update_message_flags()` 只更新本地 store，见 `src-tauri/src/commands/messages.rs:63-73`。
- IMAP 层其实已经有 `set_flags()` 能力，见 `crates/pebble-mail/src/imap.rs:411-474`，但没有任何命令调用它。
- 影响：用户看到的已读/星标只是本地状态；下次同步时很容易被远端状态冲回去。

## 未完成功能

### P1 未完成：命令面板只覆盖了非常小的一部分能力，和规格不匹配

- 规格把“命令面板 (Ctrl+K)”定义成核心入口，并明确写了“移到看板”“Snooze”这类操作，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:21`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:623-650`。
- 当前命令表只有导航、标已读、切星、写信、通知开关，见 `src/features/command-palette/commands.ts:8-92`。
- 没有看板移动、Snooze、Cloud Backup/Restore、归档、删除等命令。
- 影响：命令面板目前更像“少量快捷操作列表”，离规格中的“全键盘主入口”差距很大。

### P1 未完成：归档/删除仍然没有后端命令，前端只能禁用按钮

- 前端计划文档明确写了 “There is no `move_to_folder` or `delete_message` Tauri command”，见 `docs/superpowers/plans/2026-04-05-frontend-bugfix-completion.md:295-297`。
- 当前 `MessageDetail` 中 Archive / Delete 按钮是显式 disabled 的“coming soon”，见 `src/components/MessageDetail.tsx:275-286`, `src/components/MessageDetail.tsx:295-316`。
- 全仓搜索 `move_to_folder|delete_message|archive_message|trash_message` 没有 Tauri command 命中。
- 影响：最基本的邮件处理能力仍然缺失，只能展示半成品 UI。

### P1 未完成：账户设置页仍然只有 IMAP/SMTP 手工配置，没有真正的 Gmail/Outlook Provider 入口

- `AccountSetup` 的初始 `provider` 被固定成 `"imap"`，见 `src/components/AccountSetup.tsx:56-67`。
- 该组件没有任何 provider 选择器，也没有 OAuth 按钮。
- 设置页的 OAuth 区块被直接隐藏，见 `src/features/settings/AccountsTab.tsx:171-176`。
- 影响：即便账户模型支持 `gmail` / `outlook`，UI 层也没有把对应入口交付给用户。

### P1 未完成：Cloud Sync 只做了手动备份/恢复，没有规格里的自动导出、启动检查和配置持久化

- 规格要求本地有 `sync/pebble-sync.json` 云同步配置，并在变更时自动导出、启动时检查远端版本，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:722-727`。
- 当前 `CloudSyncTab` 只有页面内的临时 state 和手动按钮，见 `src/features/settings/CloudSyncTab.tsx:47-59`, `src/features/settings/CloudSyncTab.tsx:78-115`。
- 全仓搜索没有任何 `pebble-sync.json` 或 WebDAV 配置持久化实现。
- 影响：当前实现只是“手工上传/下载设置快照”，不是规格定义的可选云同步。

## 功能不足

### P2 功能不足：HTML 邮件渲染没有做到规格要求的 sandbox 隔离

- 规格要求 “HTML 渲染在 iframe sandbox 中”，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:708`。
- 当前 `MessageDetail` 直接把净化后的 HTML 注入主 DOM，见 `src/components/MessageDetail.tsx:355-359`。
- 影响：现在完全依赖 sanitizer 的正确性；一旦净化规则漏掉边角，风险直接落到主界面 DOM。

### P2 功能不足：附件下载没有任何可执行文件告警

- 规格要求“附件下载前检查文件类型，警告可执行文件”，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:710`。
- 当前前端只拼接目标路径并调用 `downloadAttachment()`，见 `src/components/AttachmentList.tsx:55-76`。
- 后端 `download_attachment()` 只做文件复制，见 `src-tauri/src/commands/attachments.rs:23-37`。
- 影响：对 `.exe` / `.bat` / `.ps1` 等敏感附件没有任何提醒。

### P2 功能不足：前端产物仍然过大

- `pnpm build:frontend` 成功，但主 chunk 为 `1,061.65 kB`，Vite 明确给出大 chunk 警告。
- 影响：冷启动、更新和内存占用都会继续偏重；目前 TipTap、设置页、翻译等低频模块没有拆分出来。

## 设计不足

### P1 设计不足：翻译配置/API Key 明文落库，而且还会被 Cloud Sync 直接导出

- 规格把 OAuth Token、API Key 定义为敏感数据，需要 AES-256-GCM 加密，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:680-687`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:707`。
- `save_translate_config()` 直接把前端传入的 JSON 字符串存到 `TranslateConfig.config`，见 `src-tauri/src/commands/translate.rs:39-55`。
- `translate_config` 表的读写也是明文字符串，见 `crates/pebble-store/src/translate_config.rs:6-43`。
- `export_settings()` 还会把整个 `translate_config` 连同配置一起打包进 WebDAV 备份，见 `crates/pebble-store/src/cloud_sync.rs:8-15`, `crates/pebble-store/src/cloud_sync.rs:121-145`, `crates/pebble-store/src/cloud_sync.rs:186-188`。
- 影响：API Key 既在本地明文存储，也可能随云备份外传，和规格要求相反。

### P1 设计不足：账户元数据与真实 provider 状态不一致

- `add_account()` 会根据请求把账户实体写成 `Gmail` / `Outlook` / `Imap`，见 `src-tauri/src/commands/accounts.rs:29-44`。
- 但随后又把 `sync_state` 里的 provider 硬编码成 `"imap"`，见 `src-tauri/src/commands/accounts.rs:69-75`。
- 影响：账户实体、认证数据、同步元数据三处来源不一致，后续一旦有 provider 相关分支，极易出现行为漂移。

### P2 设计不足：Cloud Sync 导出的内容超出了规格定义的“仅元数据”

- 规格写的是“规则配置、标签定义、看板状态、UI 偏好、账户列表（不含密钥）”，见 `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:727`。
- 当前 `SettingsBackup` 直接包含 `translate_config`，见 `crates/pebble-store/src/cloud_sync.rs:8-15`。
- 对于 DeepL / LLM 配置，这个字段天然可能带 API Key。
- 影响：Cloud Sync 的数据边界设计已经越界到了敏感信息。

## 优先级建议

1. 先收口 Provider 主链路：修 OAuth、修 provider 工厂接入、修同步分发，别再让 Gmail/Outlook 停留在“代码存在但主链路不走”的状态。
2. 再补闭环型功能：规则引擎执行、Snooze `return_to`、命令面板能力、归档/删除命令。
3. 随后补安全债务：翻译配置加密、Cloud Sync 导出边界、HTML sandbox、附件下载告警。
4. 最后处理体验债务：reply-all 自邮箱过滤、构建体积、更多 i18n/可访问性清理。

## 验证结果

- `pnpm test`：通过，4 个测试文件 / 22 个测试全部通过。
- `pnpm build:frontend`：通过，但出现 `index-CD-NXkwx.js 1,061.65 kB` 的大 chunk 警告。
- `cargo check -p pebble --bin pebble -j 1`：通过。
- `cargo test -p pebble-store -j 1`：通过，38 个测试全部通过。
- `cargo test -p pebble-mail --lib -j 1`：通过，66 个测试全部通过。

## 最终结论

这不是“差一点收尾”的项目，而是“界面完成度高于功能闭环完成度”的项目。

最需要警惕的不是单点语法错误，而是：

1. 规格已经承诺多 Provider，但核心链路仍然单 Provider。
2. UI 已经暴露或暗示一些能力，但后台和数据流并没有真正闭环。
3. 安全设计在翻译配置、Cloud Sync、HTML 渲染这三处仍然明显偏软。
