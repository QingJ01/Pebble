# Pebble 严格全栈审计报告

日期：2026-04-05  
范围：`d:\project\Pebble` 全仓  
方式：主代理复核 + 子代理并行审查（前端、后端/Tauri/Rust、规格闭环）  

> 说明：本报告只保留已经在当前代码中复核过的问题；不会沿用旧审计文档里已经失效或已被修复的结论。

## 总体判断

当前项目更接近 **Alpha / 高保真 Demo**，还不能视为可稳定交付的 **Beta**。

核心原因不是样式或边角问题，而是几条主业务链路仍未闭环：

1. 多 Provider 账号接入链路没有打通，OAuth / Gmail / Outlook 仍停在半实现状态。
2. 用户界面已经暴露了部分功能入口，但后端命令、远端回写、事件闭环并没有完成。
3. 安全与数据边界设计明显偏弱，尤其是翻译配置、Cloud Sync、HTML 邮件渲染。

## 验证基线

已实际执行并通过：

- `pnpm test`
- `pnpm build:frontend`
- `cargo check -p pebble --bin pebble -j 1`
- `cargo test -p pebble-store -j 1`
- `cargo test -p pebble-mail -j 1`

额外观察：

- 前端构建虽然通过，但主 chunk 仍超过 1 MB，存在明显体积债务。
- 当前前端测试只覆盖 `tests/stores/*.test.ts` 这 4 个 store 测试文件，没有组件、交互、IPC 或端到端测试。

## P0：未完成的主链路

### 1. OAuth / Gmail / Outlook 多 Provider 链路没有闭环

证据：

- OAuth 入口在前端被直接注释隐藏：`src/features/settings/AccountsTab.tsx:175-176`
- OAuth Client ID 仍是占位符：`src-tauri/src/commands/oauth.rs:9`、`src-tauri/src/commands/oauth.rs:24`
- `start_oauth_flow` 明确写着没有保存 PKCE state：`src-tauri/src/commands/oauth.rs:68-77`
- `complete_oauth_flow` 会重新启动一次新的 auth flow，而不是消费 `start_oauth_flow` 产生的 state：`src-tauri/src/commands/oauth.rs:95-111`
- 前端虽然导出了 OAuth API，但仓库中没有任何前端调用点：`src/lib/api.ts:368-377`
- 同步链路始终把账号配置反序列化为 `ImapConfig`，并固定实例化 `ImapMailProvider`：`src-tauri/src/commands/sync_cmd.rs:55-80`
- 发信链路固定读取 `smtp` 配置：`src-tauri/src/commands/compose.rs:24-41`
- Gmail provider 的 `fetch_messages` 直接返回空消息列表：`crates/pebble-mail/src/provider/gmail.rs:298-306`
- 手动加账号时即使账号实体写成 `Gmail/Outlook`，同步元数据也仍被硬编码成 `"imap"`：`src-tauri/src/commands/accounts.rs:29-33`、`src-tauri/src/commands/accounts.rs:69-75`

结论：

- 代码里已经有 Provider 抽象和 OAuth 脚手架，但真正的收信、发信、同步主路径仍然只支持 IMAP/SMTP。
- Gmail / Outlook 当前不是“待完善的小缺口”，而是“核心链路未完成”。

### 2. 规则引擎只有 CRUD，没有进入生产链路

证据：

- Tauri 侧只提供规则的增删改查：`src-tauri/src/commands/rules.rs:5-48`
- `RuleEngine` 只在规则库内部和测试中出现：`crates/pebble-rules/src/lib.rs:8-49`
- 全仓检索 `RuleEngine::new` / `pebble_rules::`，没有任何同步、收件、命令或 UI 链路接入点

结论：

- 用户可以创建规则，但当前代码无法证明这些规则会在新邮件到达、同步完成或手动操作时被执行。
- 这意味着“规则系统”目前更像数据模型和管理界面，不是已落地功能。

## P1：明确 bug / 关键未完成功能

### 3. 归档 / 删除仍是占位，后端移动能力也没完成

证据：

- 消息详情页的 Archive / Delete 按钮仍是 `disabled`，处理函数为空：`src/components/MessageDetail.tsx:298-308`
- IMAP provider 的 `move_message` 直接返回 `IMAP move not yet implemented`：`crates/pebble-mail/src/provider/imap_provider.rs:88-103`
- 当前 Tauri command 列表中也没有 `move_to_folder` / `delete_message` 类命令：`src-tauri/src/lib.rs:82-125`

影响：

- 归档、删除这类最基础的邮件处理能力尚未真正可用。

### 4. Sync 事件模型与前端状态模型脱节，UI 会把“已发起”错当“已完成”

证据：

- `start_sync` 在后台任务启动后就立即返回：`src-tauri/src/commands/sync_cmd.rs:11-18`
- 后端确实会发 `mail:sync-progress`、`mail:sync-complete`、`mail:new`：`src-tauri/src/commands/sync_cmd.rs:101-119`、`src-tauri/src/commands/sync_cmd.rs:197-214`
- 前端状态栏只监听 `mail:error`：`src/components/StatusBar.tsx:27-37`
- 前端在 `startSync()` 返回后立刻把状态切回 `idle`：`src/components/StatusBar.tsx:48-60`
- React Query 也在 mutation success 时立即刷新 folders/messages/threads：`src/hooks/mutations/useSyncMutation.ts:1-12`

影响：

- 用户看到的“同步完成”与真实后台同步生命周期不一致。
- 停止同步、观察同步中状态、等待后台索引完成等行为都可能发生漂移。

### 5. Snooze 的 `return_to` 是死字段，恢复逻辑没有实现

证据：

- 前端传入 `activeView` 作为 `returnTo`：`src/features/inbox/SnoozePopover.tsx:55-60`
- 后端确实把 `return_to` 存进 `SnoozedMessage`：`src-tauri/src/commands/snooze.rs:5-18`
- 到期处理只做 `unsnooze_message` 和系统通知：`src-tauri/src/snooze_watcher.rs:35-57`
- 仓库中找不到任何前端对 `mail:unsnoozed` 的监听：`src-tauri/src/events.rs:5`、`src-tauri/src/snooze_watcher.rs:39`

影响：

- Snooze 到期后不会回到指定位置，`return_to` 当前没有业务意义。

### 6. 通知开关只改前端 `localStorage`，并不控制后端通知

证据：

- 设置页通知开关只写 `localStorage`：`src/features/settings/AppearanceTab.tsx:26-35`
- 状态栏也只是读取这个本地键值：`src/components/StatusBar.tsx:70-71`
- Snooze watcher 在后台无条件发送系统通知：`src-tauri/src/snooze_watcher.rs:41-57`

影响：

- “启用桌面通知”当前只是 UI 偏好，不是真正的系统通知总开关。

### 7. 已读 / 星标更新只改本地库，不回写远端；前端还绕过了已有 mutation hook

证据：

- `update_message_flags` 只调用 store 更新：`src-tauri/src/commands/messages.rs:63-73`
- 前端已经存在带 optimistic update + invalidate 的 `useUpdateFlagsMutation`，但没有任何调用方：`src/hooks/mutations/useUpdateFlagsMutation.ts:11-58`
- 仓库里实际广泛直接调用裸 `updateMessageFlags(...)`：`src/components/MessageDetail.tsx:77-79`、`src/components/MessageDetail.tsx:292-295`、`src/components/MessageItem.tsx:134`、`src/features/command-palette/commands.ts:47-74`、`src/hooks/useKeyboard.ts:81`

影响：

- 本地状态和服务端状态会分叉。
- 列表缓存和详情缓存也会分叉，导致“详情已读 / 已星标，但列表仍是旧状态”的可见 bug。

### 8. Reply-all / 发件账号初始化依赖异步账号数据，存在错误收件人与空账号问题

证据：

- `fromAccountId` 只在初次渲染时从 `activeAccountId` 初始化：`src/features/compose/ComposeView.tsx:29-34`
- reply-all 的 `to` / `cc` 也只在初次渲染时用 `myEmail` 过滤：`src/features/compose/ComposeView.tsx:38-53`
- `myEmail` 依赖 `useAccountsQuery()` 的异步结果：`src/features/compose/ComposeView.tsx:30-34`
- 后续没有 effect 在账号数据到达后回填或重算

影响：

- reply-all 可能把自己的地址留在收件人里。
- 单账号场景下也可能出现发件账号初始为空的问题。

### 9. 附件“已下载路径”显示错了，展示的是缓存源路径，不是用户保存路径

证据：

- 前端计算了 `savePath`，实际下载也写到了该路径：`src/components/AttachmentList.tsx:58-68`
- 但下载完成后，前端又调用 `getAttachmentPath()` 回读路径并展示：`src/components/AttachmentList.tsx:67-70`
- `get_attachment_path()` 返回的是 attachment 记录里的 `local_path`：`src-tauri/src/commands/attachments.rs:43-50`
- `download_attachment()` 只是把缓存文件复制到 `save_to`，并不会回写 `local_path`：`src-tauri/src/commands/attachments.rs:52-70`

影响：

- 用户下载成功后，UI 提示的并不是用户真正保存到磁盘的位置。

### 10. 删除当前激活账号后，前端可能继续持有失效的 `activeAccountId`

证据：

- 删除账号后只做 `deleteAccount + invalidateQueries(accounts)`：`src/features/settings/AccountsTab.tsx:15-23`
- `Sidebar` 只会在 `!activeAccountId` 时自动选择第一个账号：`src/components/Sidebar.tsx:62-67`
- `setActiveAccountId` 才会主动清空 folder/message/thread 状态：`src/stores/mail.store.ts:62-72`

影响：

- 如果当前激活账号被删掉，前端仍可能保留一个已经失效的账号 ID，后续查询和视图状态会不一致。

### 11. Cloud Restore 成功后，前端没有任何刷新 / 失效逻辑

证据：

- `restoreFromWebdav` 成功后只是显示成功提示：`src/features/settings/CloudSyncTab.tsx:98-115`
- 没有 query invalidation、store refresh 或 reload

影响：

- 恢复出来的账号、规则、看板和设置不会立即反映到当前 UI。

## P2：功能不足 / 设计不足 / 质量债务

### 12. 翻译配置与 API Key 明文落库，而且会被 Cloud Sync 一起导出

证据：

- 前端把 `config: string` 原样传给后端：`src/lib/api.ts:336-341`
- 后端原样保存 `TranslateConfig.config`：`src-tauri/src/commands/translate.rs:38-55`
- store 层也是明文存取：`crates/pebble-store/src/translate_config.rs:6-42`
- Cloud Sync 备份结构直接包含 `translate_config`：`crates/pebble-store/src/cloud_sync.rs:8-15`
- `export_settings()` 会把 `translate_config` 一并导出：`crates/pebble-store/src/cloud_sync.rs:121-149`
- `import_settings()` 恢复时也会原样写回：`crates/pebble-store/src/cloud_sync.rs:186-189`

影响：

- 翻译服务的 endpoint、token、API key 这类敏感信息既没有本地加密，也没有从云备份中隔离。

### 13. 富文本邮件渲染隔离偏弱；回复/转发还会把原始 HTML 直接注入编辑器

证据：

- 消息详情页直接把渲染后的 HTML 注入主 DOM，只做 DOMPurify 清洗，没有 iframe/sandbox 隔离：`src/components/MessageDetail.tsx:379-381`
- 回复 / 转发时直接拼入 `body_html_raw`：`src/features/compose/ComposeView.tsx:83-94`

影响：

- 当前实现过度依赖 sanitizer 的正确性。
- 原始远端 HTML 进入编辑器上下文，本身就是额外的安全与渲染一致性风险。

### 14. 手动加账号界面并不能创建真正的 Gmail / Outlook Provider 账号

证据：

- 表单默认 `provider: "imap"`：`src/components/AccountSetup.tsx:56-67`
- 快捷预设只改 host/port/TLS，不改 provider：`src/components/AccountSetup.tsx:72-76`
- 整个界面没有 provider 选择器，也没有 OAuth 入口：`src/components/AccountSetup.tsx:150-408`
- 设置页里的 OAuth 区块仍处于隐藏状态：`src/features/settings/AccountsTab.tsx:175-176`

影响：

- 当前 UI 提供的 Gmail / Outlook 更像 IMAP/SMTP 参数模板，不是 provider-aware 账号接入。

### 15. Provider 能力抽象存在，但应用表面没有对应命令层

证据：

- 核心类型里已经定义了 `ProviderCapabilities` 和相关 trait：`crates/pebble-core/src/traits.rs:84`、`crates/pebble-core/src/types.rs:185`
- Gmail / Outlook / IMAP provider 也都实现了 `capabilities()`：`crates/pebble-mail/src/provider/gmail.rs:407-414`、`crates/pebble-mail/src/provider/imap_provider.rs:85-92`
- 但当前 Tauri command 列表中不存在 `get_provider_capabilities` / `execute_command`：`src-tauri/src/lib.rs:82-125`
- 命令面板只是本地静态数组：`src/features/command-palette/commands.ts:8-92`

影响：

- Provider 抽象停留在底层，没有真正驱动前端 UI 能力裁剪或命令执行。

### 16. `backup-to-cloud` 快捷键名不副实，只是跳到设置页

证据：

- 键盘处理里 `backup-to-cloud` 只执行 `setActiveView("settings")`：`src/hooks/useKeyboard.ts:150-152`
- 真正备份逻辑只存在于设置页按钮：`src/features/settings/CloudSyncTab.tsx:78-95`

影响：

- 用户触发“备份到云端”快捷键时，并不会真的发起备份。

### 17. Trusted sender 的 `trust_type` 基本失效

证据：

- store 层确实区分 `images` 与 `all`：`crates/pebble-store/src/trusted_senders.rs:6-17`
- 前端当前固定只写 `"all"`：`src/components/MessageDetail.tsx:129-137`
- `get_rendered_html` 只要发现 sender 被信任，就统一提升到 `PrivacyMode::TrustSender(...)`，并不区分 trust_type：`src-tauri/src/commands/messages.rs:35-47`

影响：

- `images` / `all` 这两个级别在当前产品里没有真实行为差异。

### 18. 双语翻译只翻 `body_text`，HTML-only 邮件会被送空字符串

证据：

- 双语模式直接调用 `translateText(message.body_text || "", "auto", uiLang)`：`src/components/MessageDetail.tsx:149-150`

影响：

- 对于只含 HTML 正文的邮件，双语视图会退化为翻译空文本。

### 19. 命令面板和部分 UI 文案存在本地化与编码质量问题

证据：

- 命令面板名称与分类全部硬编码英文：`src/features/command-palette/commands.ts:8-92`
- 翻译弹层默认目标语言硬编码为 `zh`，语言名也写死英文：`src/features/translate/TranslatePopover.tsx:15-28`、`src/features/translate/TranslatePopover.tsx:56-78`
- 设置页中文标签已乱码：`src/features/settings/AppearanceTab.tsx:14-17`
- 新增账号 loading 文案已乱码：`src/components/AccountSetup.tsx:407`
- Kanban 卡片移除按钮显示为无意义字符 `脳`：`src/features/kanban/KanbanCard.tsx:61-63`

影响：

- 这已经不是单纯的“国际化不足”，而是用户可见的文案错误和编码污染。

## 结论

当前 Pebble 的主要问题不是“有一些 bug”，而是：

1. 账号接入、同步、发信、规则执行、归档删除这些主路径并没有全部闭环。
2. 前后端之间存在多处“接口存在，但状态模型没接上”的断裂。
3. 安全与数据边界设计仍然偏松，离生产级桌面邮件客户端有明显差距。

如果只按修复优先级排序，我建议这样收口：

1. 先修多 Provider 主链路：OAuth、Provider 分发、Gmail/Outlook 收发同步、账号元数据一致性。
2. 再补核心动作闭环：规则执行、归档/删除、Snooze return_to、同步事件驱动刷新。
3. 再处理数据一致性与安全债务：远端 flag 回写、翻译配置加密、Cloud Sync 边界、HTML 渲染隔离。
4. 最后清理体验债务：本地化、编码乱码、快捷键语义、构建体积、测试覆盖。
