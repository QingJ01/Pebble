# Pebble 团队多角度审查报告

日期：2026-04-06  
仓库：`D:\project\Pebble`  
范围：`src/`、`src-tauri/`、`crates/`、`tests/`、`docs/superpowers/specs/`

## 审查方式

本次审查按 7 个角色并行展开：

1. 功能完成度
2. 用户体验 / 可访问性
3. 可维护性 / 代码整洁度
4. 安全 / 合规
5. 性能
6. Bug 猎手
7. 质疑者 / devil's advocate

报告中的每个问题都给出当前代码的具体 `文件:行号`、影响、以及修改建议。  
本报告基于当前工作树做只读审查；本轮未把自动化测试结果作为结论前提。

## 总结

当前仓库已经具备一个可运行的 Tauri 邮件客户端骨架，但离“可对外承诺的多 provider、隐私优先、云同步、规则自动化邮件客户端”还有明显距离。最关键的问题不是单点语法错误，而是多条被 UI 或规格显式承诺的主路径仍未闭环。

最高优先级问题：

- `P0` Gmail OAuth 账号并未打通发送、连接测试、消息状态写回和附件主路径，当前只能算“部分 Gmail 收信接入”，不是完整 Gmail 支持。
- `P0` 规则设置页输出的 JSON 与 Rust 规则引擎要求的协议不一致，规则看起来可保存，运行时会被直接跳过。
- `P0` OAuth 回环回调未校验 `state`，存在本机抢占回调和账号错绑风险。
- `P1` 搜索索引与消息真实状态不一致：新邮件需要攒到 50 条或同步结束才提交，归档/删除/恢复又不更新索引。
- `P1` Compose 未保存内容保护只覆盖一部分离开路径，富文本正文也未计入 dirty 状态，存在真实丢稿风险。
- `P1` `Cloud Sync` 实际实现更接近“部分设置备份/恢复”，与 UI 命名、成功提示和规格叙事不一致。

## 详细问题

## 1. 功能完成度

### 1.1 `P0` Gmail OAuth 账号只完成了登录和部分收信，主链路未闭环

- 证据：
  - 前端直接暴露了 Google OAuth 登录按钮：`src/components/AccountSetup.tsx:255`
  - OAuth 完成后只把 token JSON 写入 `auth_data`：`src-tauri/src/commands/oauth.rs:175`
  - 发信要求 `auth_data.smtp` 存在：`src-tauri/src/commands/compose.rs:24`
  - 账号测试要求能反序列化 IMAP 配置：`src-tauri/src/commands/accounts.rs:203`
  - 消息已读/星标写回明确走 IMAP：`src-tauri/src/commands/messages.rs:123`
- 影响：
  - 用户会认为“Google 登录”代表 Gmail 已经完整可用，但实际上发送邮件、测试连接、归档/星标等操作并不适用于 token-only 账号。
  - 本地状态和远端 Gmail 状态会产生分叉。
- 修改建议：
  - 以 `account.provider` 为入口把发送、测试连接、flag/归档/删除等操作分发到 provider-specific 实现。
  - 在 Gmail API 的发送、修改标签、trash/untrash、附件抓取未补齐前，前端应显式禁用这些操作并给出说明。

### 1.2 `P0` 规则设置页保存的 JSON 与规则引擎协议不兼容

- 证据：
  - 前端条件被序列化成裸数组：`src/features/settings/RulesTab.tsx:41`
  - 前端动作被序列化成 `"MarkRead"` / `{ AddLabel: ... }` 等旧格式：`src/features/settings/RulesTab.tsx:62`
  - Rust 侧条件要求 `{"operator":"and","conditions":[...]}`：`crates/pebble-rules/src/types.rs:5`
  - Rust 侧动作要求 `#[serde(tag = "type", content = "value")]`：`crates/pebble-rules/src/types.rs:46`
  - 解析失败的规则会被直接 `skipping rule`：`crates/pebble-rules/src/lib.rs:17`
- 影响：
  - 规则“能创建、能保存、能展示”，但运行时不会触发。
  - 这是表面支持、实则无效的典型假功能。
- 修改建议：
  - 统一前后端规则 JSON 协议，以 Rust 结构体为单一真相源。
  - 前端保存时补 `operator`，动作改成 `[{ "type": "AddLabel", "value": "foo" }]`。
  - 增加端到端测试：`RulesTab 保存 -> RuleEngine 解析 -> 消息命中 -> 动作落地`。

### 1.3 `P1` Gmail label / folder / thread 语义没有真正落地

- 证据：
  - Gmail provider 会列出非隐藏 label：`crates/pebble-mail/src/provider/gmail.rs:507`
  - Gmail 初始同步只拉 `INBOX` 和 `SENT`：`crates/pebble-mail/src/gmail_sync.rs:165`
  - 拉到消息后只写入单个 `folder_id`：`crates/pebble-mail/src/gmail_sync.rs:241`
  - history 分支中新消息也直接塞到本地 Inbox：`crates/pebble-mail/src/gmail_sync.rs:360`
- 影响：
  - Gmail 自定义标签、Trash/Spam/Drafts、按标签切换的线程视图和计数并不可信。
  - 规格里“`message_folders` 多对多承载 Gmail label”的承诺没有实现。
- 修改建议：
  - 如果短期只同步有限标签，前端就只展示当前真正同步的标签。
  - 如果要保留统一 label/folder 模型，就要按 Gmail `labelIds` 真实维护 `message_folders` 多对多关系。
  - 保留 provider-native `thread_id`，不要一律覆盖成本地 `compute_thread_id`。

### 1.4 `P1` Gmail 附件能力只显示，不可下载

- 证据：
  - Gmail provider 会把附件存在性映射到 `has_attachments`：`crates/pebble-mail/src/provider/gmail.rs:301`
  - 详情页只要 `message.has_attachments` 就渲染附件区：`src/components/MessageDetail.tsx:628`
  - 附件列表依赖本地 `attachments` 表：`src/components/AttachmentList.tsx:37`
  - IMAP 同步会把附件写本地并插入记录：`crates/pebble-mail/src/sync.rs:370`
  - Gmail 同步没有对应的附件持久化路径：`crates/pebble-mail/src/gmail_sync.rs:233`
- 影响：
  - Gmail 邮件会表现为“有附件”，但用户点进去看不到任何可下载附件。
- 修改建议：
  - 为 Gmail 补 attachment part 拉取、本地文件落盘和 `Attachment` 记录插入。
  - 在此之前，不要对 Gmail 消息展示附件下载能力。

### 1.5 `P1` 搜索功能与真实邮箱状态不一致

- 证据：
  - 新邮件只 `index_message`，达到 50 条才 `commit`：`src-tauri/src/commands/sync_cmd.rs:357`
  - 剩余索引只在通道关闭时 `commit`：`src-tauri/src/commands/sync_cmd.rs:372`
  - 归档只改 store，不更新搜索索引：`src-tauri/src/commands/messages.rs:212`
  - 删除也只改 store，不更新搜索索引：`src-tauri/src/commands/messages.rs:261`
- 影响：
  - 刚到的新邮件可能暂时搜不到。
  - 已归档、已删除、已恢复的邮件仍可能作为“幽灵结果”留在搜索里。
- 修改建议：
  - 新邮件改成小批次定时 `commit`，不要依赖 50 条门槛。
  - 为归档、删除、恢复、移动补充索引删除 / 更新路径，或者在这些动作后对受影响消息做精准重建。

### 1.6 `P1` Cloud Sync 恢复后账户外观恢复，但能力没有恢复

- 证据：
  - 备份明确排除了账户 secret：`crates/pebble-store/src/cloud_sync.rs:17`
  - 导出时翻译配置会被清空：`crates/pebble-store/src/cloud_sync.rs:145`
  - 恢复时只 upsert 账户壳、规则、看板和非空翻译配置：`crates/pebble-store/src/cloud_sync.rs:166`
  - 应用启动会自动恢复所有账户同步：`src-tauri/src/lib.rs:75`
  - 同步入口并不会把这些账户标记成 `needs_reauth`：`src-tauri/src/commands/sync_cmd.rs:90`
- 影响：
  - 恢复后账户会正常显示在设置页里，但无法同步 / 发信。
  - “Restore completed successfully” 与实际可用性不匹配。
- 修改建议：
  - 恢复后把缺少秘密信息的账户明确标记为 `needs_reauth`。
  - UI 改名为“部分恢复”或“设置恢复”，并提供后续重连引导。
  - 如果产品确实要承诺跨设备恢复，就需要对敏感配置做加密备份。

## 2. 用户体验 / 可访问性

### 2.1 `P1` Compose 离开保护不完整，富文本正文也未纳入 dirty 检测

- 证据：
  - dirty 只看 `to / subject / rawSource`：`src/features/compose/ComposeView.tsx:89`
  - 富文本发送内容来自 `editor.getHTML()` / `editor.getText()`：`src/features/compose/ComposeView.tsx:196`
  - 返回按钮直接 `closeCompose()`：`src/features/compose/ComposeView.tsx:261`
  - `setActiveView()` 有 guard，但 `closeCompose()` 没有：`src/stores/ui.store.ts:52`, `src/stores/ui.store.ts:80`
  - Esc 快捷键也会直接 `closeCompose()`：`src/hooks/useKeyboard.ts:78`
  - 标题栏关闭窗口同样没有复用草稿保护：`src/components/TitleBar.tsx:49`
- 影响：
  - 富文本模式下只写正文、不改主题时，dirty 可能仍然是 `false`。
  - 点返回、按 Esc、直接关窗都可能静默丢稿。
- 修改建议：
  - 收口成统一的 “canLeaveCompose()” 逻辑，所有离开路径都走它。
  - dirty 计算至少覆盖 `to / cc / bcc / subject / fromAccountId / rich editor content`。
  - 用应用内确认弹窗替换裸 `confirm()`，并保证可本地化。

### 2.2 `P1` 核心列表项仍存在非语义交互

- 证据：
  - 搜索结果项是纯点击 `div`：`src/features/search/SearchResultItem.tsx:18`
  - 线程项也是纯点击 `div`：`src/components/ThreadItem.tsx:32`
  - 消息项虽然加了 `role="option"`，但父容器不是 `listbox`：`src/components/MessageItem.tsx:45`, `src/components/MessageList.tsx:55`
- 影响：
  - 核心收件箱 / 搜索流程对键盘用户和读屏用户不够可靠。
- 修改建议：
  - 统一改成 `button` / `li > button`，或完整实现 `listbox/option + roving tabindex`。
  - 避免“视觉上像列表，语义上是 div”。

### 2.3 `P1` 账号弹层和确认弹层不是完整模态框

- 证据：
  - 新建账号弹层只有遮罩层和容器，没有 `role="dialog"` / `aria-modal`：`src/components/AccountSetup.tsx:188`
  - 编辑账号弹层同样只是覆盖层：`src/features/settings/AccountsTab.tsx:378`
  - `ConfirmDialog` 虽然有 `role="dialog"`，但没有焦点陷阱和焦点恢复：`src/components/ConfirmDialog.tsx:26`
- 影响：
  - Tab 很容易跑到背景界面，读屏器也拿不到完整“进入模态框”的上下文。
- 修改建议：
  - 抽一个统一的 modal primitive：补 `role="dialog"`、`aria-modal`、标题关联、初始焦点、Tab trap、关闭后焦点恢复。

### 2.4 `P2` 多处输入控件显式移除了焦点样式

- 证据：
  - 搜索输入 `outline: none`：`src/features/search/SearchView.tsx:134`
  - Compose 多个输入和 HTML textarea 移除了 outline：`src/features/compose/ComposeView.tsx:325`, `src/features/compose/ComposeView.tsx:436`
  - 编辑账号输入样式同样去掉 outline：`src/features/settings/AccountsTab.tsx:361`
- 影响：
  - 键盘用户能聚焦，但很难知道当前焦点在哪。
- 修改建议：
  - 建立统一 `:focus-visible` 样式，不要在组件内继续散落 `outline: none`。

### 2.5 `P2` 表单标签关联不一致

- 证据：
  - `AccountSetup` 已经使用了 `htmlFor / id / name / autocomplete`：`src/components/AccountSetup.tsx:313`
  - 但编辑账号、Cloud Sync、Translate、搜索筛选中大量字段仍是“视觉 label + 裸 input”：`src/features/settings/AccountsTab.tsx:424`, `src/features/settings/CloudSyncTab.tsx:138`, `src/features/settings/TranslateTab.tsx:189`, `src/features/search/SearchFilters.tsx:49`
- 影响：
  - 点 label 不会聚焦字段，读屏器表单导航信息弱，密码管理器 / 浏览器自动填充也较弱。
- 修改建议：
  - 把 `AccountSetup` 的字段模式推广到其余表单。

### 2.6 `P2` 状态反馈没有 live region

- 证据：
  - Toast 容器是普通视觉容器：`src/components/ToastContainer.tsx:23`
  - Compose 错误条没有 `role="alert"`：`src/features/compose/ComposeView.tsx:293`
  - 翻译页状态消息同样只是 `div`：`src/features/settings/TranslateTab.tsx:397`
- 影响：
  - 读屏用户可能执行了“发送 / 备份 / 测试连接”，却听不到结果。
- 修改建议：
  - 错误消息用 `role="alert"`。
  - 普通成功状态用 `role="status"` 或 `aria-live="polite"`。

### 2.7 `P2` 界面中存在真实可见的乱码文本

- 证据：
  - 联系人输入芯片的移除按钮显示为 `脳`：`src/components/ContactAutocomplete.tsx:203`
  - 语言选项中文显示为 `涓枃`：`src/features/settings/AppearanceTab.tsx:13`
- 影响：
  - 这不是注释脏数据，而是实际会出现在 UI 里的文案错误。
- 修改建议：
  - 统一检查文件编码为 UTF-8。
  - 修正所有已经进入 UI 的乱码文本，并补一轮 i18n 回归检查。

## 3. 可维护性 / 代码整洁度

### 3.1 `P1` IMAP 与 Gmail 维护了两套同步管线

- 证据：
  - IMAP 同步路径：`crates/pebble-mail/src/sync.rs:159`
  - Gmail 同步路径：`crates/pebble-mail/src/gmail_sync.rs:115`
  - `pebble-core` 已定义统一 trait：`crates/pebble-core/src/traits.rs:78`
  - `provider/mod.rs` 也已有 provider 工厂：`crates/pebble-mail/src/provider/mod.rs:10`
- 影响：
  - folder upsert、thread mapping、cursor 更新、消息入库、事件回调等逻辑在两套实现里重复存在。
  - 任何同步语义改动都要改两次，并且更难测试。
- 修改建议：
  - 把 provider 负责的范围收敛为“取远端变化”，其余通过共享 pipeline 和 `MessageSink` / repository trait 注入。

### 3.2 `P1` Tauri command 已经膨胀成应用服务本体

- 证据：
  - `AppState` 直接暴露基础设施：`src-tauri/src/state.rs:15`
  - `start_sync_inner()` 同时处理账号读取、凭据解密、worker 构造、索引、规则和事件：`src-tauri/src/commands/sync_cmd.rs:45`
  - `update_message_flags()` 同时做本地状态更新和远端 IMAP 写回：`src-tauri/src/commands/messages.rs:124`
- 影响：
  - command 层边界不清，测试必须携带完整 `State<AppState>` 和多种副作用。
- 修改建议：
  - 引入 `SyncService`、`MessageService`、`AccountService`。
  - command 只负责参数解包、调用服务和错误映射。

### 3.3 `P1` 账号秘密数据结构以原始 JSON 形式散落在多个 command 中

- 证据：
  - 账号新增时把 `{imap,smtp}` 写入加密 `auth_data`：`src-tauri/src/commands/accounts.rs:73`
  - 发信、消息写回、同步、账号测试各自重新解密并解析 JSON：`src-tauri/src/commands/compose.rs:24`, `src-tauri/src/commands/messages.rs:168`, `src-tauri/src/commands/sync_cmd.rs:192`, `src-tauri/src/commands/accounts.rs:203`
- 影响：
  - 字段名、provider 扩展和回退逻辑极易漂移。
- 修改建议：
  - 抽单一 `AccountSecretsService` 或 typed loader，禁止 command 直接操作原始 JSON。

### 3.4 `P1` 前端服务端状态没有唯一真相源

- 证据：
  - `useMessageQuery` / `useSearchQuery` 已定义，但消费很少：`src/hooks/queries/useMessageQuery.ts:7`, `src/hooks/queries/useSearchQuery.ts:7`
  - 搜索页自己维护 `results` 和 `messages`：`src/features/search/SearchView.tsx:27`
  - Kanban 使用 Zustand 持有服务端卡片：`src/stores/kanban.store.ts:5`
  - 键盘层又直接遍历全局 query cache：`src/hooks/useKeyboard.ts:27`
- 影响：
  - React Query、Zustand、局部 state 和全局 queryClient 的职责发生重叠。
  - 缓存失效和 optimistic update 会越来越难统一。
- 修改建议：
  - 明确约束：服务端数据只放 React Query，Zustand 只放 UI / navigation state。

### 3.5 `P2` 消息动作在多个入口重复实现

- 证据：
  - 详情页消息动作：`src/components/MessageDetail.tsx:372`
  - 列表项归档 / 星标：`src/components/MessageItem.tsx:184`
  - 快捷键动作：`src/hooks/useKeyboard.ts:91`
  - 命令面板动作：`src/features/command-palette/commands.ts:50`
- 影响：
  - toast、回滚、缓存策略和错误处理会在多个入口上逐步漂移。
- 修改建议：
  - 抽一个统一的 `useMessageActions` 或 application command 层。

### 3.6 `P2` `MessageDetail` 已经是“功能包”，不是单纯组件

- 证据：
  - 文件长度 600+ 行：`src/components/MessageDetail.tsx:1`
  - 同时包含消息加载、隐私模式、翻译、Snooze/Kanban、HTML 清洗和正文渲染：`src/components/MessageDetail.tsx:57`, `src/components/MessageDetail.tsx:191`, `src/components/MessageDetail.tsx:372`
- 影响：
  - 小改动也容易牵动无关副作用，针对单一职责补测试成本很高。
- 修改建议：
  - 拆成 `useMessageDetailData`、`MessageActionsBar`、`MessageBody`、`TranslationController`、`PrivacyController`。

## 4. 安全 / 合规

### 4.1 `P0` OAuth 回调未校验 `state`

- 证据：
  - `start_auth()` 会生成 `csrf_token` 并放进 `PkceState`：`crates/pebble-oauth/src/lib.rs:91`
  - 回调监听只提取 `code`，完全忽略 `state`：`crates/pebble-oauth/src/redirect.rs:17`
  - Tauri 侧随后直接拿这个 `code` 去换 token：`src-tauri/src/commands/oauth.rs:143`
- 影响：
  - 本机恶意进程或恶意网页可以抢先打到固定回环端口，造成登录 DoS 或账号错绑。
- 修改建议：
  - 回调处理器返回 `(code, state)`。
  - 在换 token 前把返回的 `state` 与 `pkce_state.csrf_token.secret()` 做常量时间比较。
  - 优先使用随机回环端口，而不是固定 `8756/8757`。

### 4.2 `P1` 附件文件名清洗未覆盖 Windows 特有危险名

- 证据：
  - 同步落盘时的文件名清洗只去掉 `/`、`\\`、`..` 和控制字符：`crates/pebble-mail/src/sync.rs:24`
  - IMAP 附件直接按清洗后文件名写盘：`crates/pebble-mail/src/sync.rs:376`
  - 前端下载时也只做了弱替换：`src/components/AttachmentList.tsx:61`
- 影响：
  - Windows 上仍可能遇到 `:`、保留设备名、尾随点 / 空格等问题，导致落盘异常或写入非预期目标。
- 修改建议：
  - 把文件名规范化统一收敛到 Rust 侧一个平台感知函数。
  - 额外拒绝 `: * ? " < > |`、尾随点 / 空格、`CON/AUX/NUL/COM1/LPT1` 等保留名。
  - 不满足白名单时改用 UUID 文件名。

### 4.3 `P1` 仍允许 IMAP / SMTP 明文传输

- 证据：
  - 旧配置 `use_tls = false` 会映射成 `Plain`：`crates/pebble-mail/src/imap.rs:18`
  - SMTP `Plain` 分支使用 `builder_dangerous()`：`crates/pebble-mail/src/smtp.rs:110`
  - UI 直接暴露了 `None` 安全选项：`src/components/AccountSetup.tsx:381`, `src/features/settings/AccountsTab.tsx:453`
- 影响：
  - 用户可能在无意识下把邮箱凭据和邮件内容暴露在明文链路上。
- 修改建议：
  - 默认禁用 `Plain`。
  - 如果必须保留，只放到高级设置并给出显式风险说明。

### 4.4 `P1` WebDAV 备份未加密也未签名

- 证据：
  - 导出时直接把设置序列化成 JSON：`crates/pebble-store/src/cloud_sync.rs:131`
  - 上传时原样发送：`src-tauri/src/commands/cloud_sync.rs:20`
  - 恢复时下载后直接导入，且会先删本地规则再写远端内容：`src-tauri/src/commands/cloud_sync.rs:33`, `crates/pebble-store/src/cloud_sync.rs:185`
- 影响：
  - WebDAV 运营方、服务端入侵者或能篡改对象内容的人，可以读取或篡改备份内容。
- 修改建议：
  - 备份前做客户端加密和完整性校验。
  - 恢复前展示摘要并要求确认。

### 4.5 `P2` Google OAuth `client_secret` 被硬编码且重复出现

- 证据：
  - Gmail OAuth 配置中硬编码 `client_secret`：`src-tauri/src/commands/oauth.rs:44`
  - refresh token 路径又复制了一份：`src-tauri/src/commands/sync_cmd.rs:124`
- 影响：
  - 轮换困难，且任何拿到源码或二进制的人都能提取该 secret。
- 修改建议：
  - 改成标准公有客户端 + PKCE，不再内嵌 secret。
  - 如果必须保留，也要改为构建期 / 运行期注入并集中到单一配置点。

### 4.6 `P2` 默认 debug 日志会输出 PII

- 证据：
  - 应用启动默认多个 crate 开到 `debug`：`src-tauri/src/lib.rs:31`
  - OAuth 会打出邮箱和姓名：`src-tauri/src/commands/oauth.rs:37`
  - 同步路径会打出账户邮箱：`src-tauri/src/commands/sync_cmd.rs:37`
- 影响：
  - 本地日志、支持包、崩溃上报都会携带不必要的用户信息。
- 修改建议：
  - 发布构建默认降到 `info` / `warn`，并对 PII 做统一 redaction。

## 5. 性能

### 5.1 `P1` 搜索结果把整封邮件批量拉回，后端却逐条查询

- 证据：
  - 搜索页会对命中结果执行 `getMessagesBatch(idsToFetch)`：`src/features/search/SearchView.tsx:79`
  - Tauri command 只是循环调用 `get_message()`：`src-tauri/src/commands/messages.rs:41`
  - store 侧也是逐条查整封邮件：`crates/pebble-store/src/messages.rs:327`
  - 搜索结果项实际只用到了 `subject/from/date`：`src/features/search/SearchResultItem.tsx:13`
- 影响：
  - 搜索一次最多 50 条命中时，会产生 50 次 SQLite 读取，并跨 Tauri 传回不必要的大字段。
- 修改建议：
  - 让搜索命令直接返回轻量 DTO。
  - 或至少提供真正的批量 `WHERE id IN (...)` 查询。

### 5.2 `P1` 消息列表标签查询是典型 N+1

- 证据：
  - 每个消息项都独立 `useQuery(getMessageLabels(message.id))`：`src/components/MessageItem.tsx:38`
  - 标签命令每次都单独查表：`src-tauri/src/commands/labels.rs:7`, `crates/pebble-store/src/labels.rs:70`
- 影响：
  - 首屏和滚动过程中会形成大量 IPC/SQLite 往返。
- 修改建议：
  - 提供 `get_message_labels_batch(messageIds)`，或直接把标签合并进列表接口。

### 5.3 `P2` 列表 DTO 里仍携带并反序列化收件人 JSON

- 证据：
  - `MSG_SUMMARY_SELECT` 仍选出 `to_list/cc_list/bcc_list`：`crates/pebble-store/src/messages.rs:64`
  - `row_to_message_summary()` 会逐行反序列化这些字段：`crates/pebble-store/src/messages.rs:79`
  - 列表 UI 并不消费它们：`src/components/MessageItem.tsx:89`
- 影响：
  - 收件箱 / 星标 / 线程列表都在付出不必要的反序列化和 IPC 载荷成本。
- 修改建议：
  - 为列表单独定义最小 DTO，只保留真正展示所需字段。

### 5.4 `P2` 打开未读邮件会触发整列表重拉

- 证据：
  - 已读 mutation 只乐观更新单邮件缓存：`src/hooks/mutations/useUpdateFlagsMutation.ts:17`
  - 结束后会全量 `invalidateQueries(["messages"])`：`src/hooks/mutations/useUpdateFlagsMutation.ts:55`
- 影响：
  - 用户连续查看未读邮件时，每次打开都会导致列表重拉和重渲染。
- 修改建议：
  - 精准修补当前列表缓存。
  - 把远端已读写回改成批量或延迟提交。

### 5.5 `P2` 联系人自动补全在热路径上扫描消息表

- 证据：
  - 输入 200ms 后就调用 `searchContacts`：`src/components/ContactAutocomplete.tsx:55`
  - store 侧既查 `from_address`，又扫 `to_list` JSON：`crates/pebble-store/src/contacts.rs:18`, `crates/pebble-store/src/contacts.rs:56`
- 影响：
  - 邮箱数据量增大后，写信输入阶段会明显变卡。
- 修改建议：
  - 同步阶段维护标准化联系人表或 FTS / 前缀索引。
  - 前端增加前缀缓存，避免对相近输入重复查询。

### 5.6 `P2` 大 HTML 邮件被重复 sanitize

- 证据：
  - Rust 端已通过 `PrivacyGuard` 生成 `RenderedHtml`：`src-tauri/src/commands/messages.rs:55`
  - 前端 `ShadowDomEmail` 又再次 `DOMPurify.sanitize`：`src/components/ShadowDomEmail.tsx:34`
  - 双语模式还会在详情页额外再做 DOM 处理：`src/components/MessageDetail.tsx:203`
- 影响：
  - 大 HTML 邮件会在 Rust 和 JS 两边重复解析 / 清洗，占用主线程。
- 修改建议：
  - 明确单一信任边界。
  - 若保留前端防线，至少按 `messageId + privacyMode` 缓存结果。

### 5.7 `P2` 线程摘要查询的代价跟全库体量绑定

- 证据：
  - 线程列表子查询先对全库 `messages` 做 `GROUP BY thread_id`：`crates/pebble-store/src/messages.rs:718`
  - 当前 folder 过滤直到外层才生效：`crates/pebble-store/src/messages.rs:740`
- 影响：
  - 打开某个文件夹的线程视图时，代价仍然接近全邮箱规模。
- 修改建议：
  - 把 folder / account 约束推进子查询，或维护线程摘要表。

## 6. Bug 猎手

### 6.1 `High` 富文本正文可被静默丢失

- 文件：`src/features/compose/ComposeView.tsx:89`, `src/features/compose/ComposeView.tsx:196`, `src/stores/ui.store.ts:52`, `src/stores/ui.store.ts:80`, `src/hooks/useKeyboard.ts:78`, `src/components/TitleBar.tsx:49`
- 复现：
  - 打开 Compose。
  - 在 rich text 模式只输入正文，不改 `to/subject`。
  - 点击返回、按 Esc 或直接关闭窗口。
- 影响：
  - `composeDirty` 可能还是 `false`，正文会直接丢失。
- 修复建议：
  - 统一离开 guard，并把 rich text 内容纳入 dirty 计算。

### 6.2 `High` 规则“保存成功但永不执行”

- 文件：`src/features/settings/RulesTab.tsx:41`, `src/features/settings/RulesTab.tsx:62`, `crates/pebble-rules/src/lib.rs:17`
- 复现：
  - 在 Rules 页面创建一条 `from contains foo -> MarkRead` 规则并保存。
  - 导入一封满足条件的邮件。
- 影响：
  - 规则会显示为 enabled，但引擎解析失败后直接跳过。
- 修复建议：
  - 前后端统一协议，并增加端到端回归测试。

### 6.3 `High` Google OAuth 账号在发信 / 测试连接 / flag 写回上会失败

- 文件：`src-tauri/src/commands/oauth.rs:175`, `src-tauri/src/commands/compose.rs:24`, `src-tauri/src/commands/accounts.rs:203`, `src-tauri/src/commands/messages.rs:135`
- 复现：
  - 使用 “Sign in with Google” 创建账号。
  - 尝试发送邮件、测试账号连接、给消息加星或标记已读。
- 影响：
  - OAuth 账号的主链路不成立，操作要么失败，要么只改本地不改远端。
- 修复建议：
  - 所有操作都按 provider 分发，不允许 token-only 账号复用 IMAP/SMTP 路径。

### 6.4 `Medium` 搜索结果会出现“新邮件搜不到 / 已删邮件还在”

- 文件：`src-tauri/src/commands/sync_cmd.rs:357`, `src-tauri/src/commands/sync_cmd.rs:372`, `src-tauri/src/commands/messages.rs:212`, `src-tauri/src/commands/messages.rs:261`
- 复现：
  - 长时间同步过程中收到少量新邮件，立即搜索。
  - 删除或归档一封邮件后，再次搜索原关键词。
- 影响：
  - 搜索结果会暂时滞后于真实邮箱状态。
- 修复建议：
  - 为新增、删除、移动、恢复补精确索引更新。

### 6.5 `Medium` Cloud Sync 恢复后会留下“看起来存在、实际上失效”的账户

- 文件：`crates/pebble-store/src/cloud_sync.rs:166`, `src-tauri/src/lib.rs:75`
- 复现：
  - 备份后恢复到新环境。
  - 重启应用，等待自动恢复同步。
- 影响：
  - 账户会显示在 UI 中，但由于没有 `auth_data`，同步和发信会失败。
- 修复建议：
  - 恢复时把账户显式标记为 `needs_reauth`，禁止自动恢复同步。

## 7. 质疑者 / Devil's Advocate

### 7.1 `High` “统一多 provider 客户端”这件事还没有成立

- 证据：
  - Outlook 同步仍并入 IMAP 路径：`src-tauri/src/commands/sync_cmd.rs:190`
  - OAuth 登录写入的是 token JSON，但发送仍强依赖 `smtp`：`src-tauri/src/commands/compose.rs:24`
  - 连接测试同样只解 IMAP：`src-tauri/src/commands/accounts.rs:203`
  - 前端当前只有 Google OAuth 入口：`src/components/AccountSetup.tsx:99`
- 结论：
  - 当前更准确的说法是“IMAP-first + 部分 Gmail 同步”，不是“多 provider 已统一抽象完成”。
- 建议：
  - 对外收缩承诺，或先把 Gmail / Outlook 的主操作链路真正补齐。

### 7.2 `High` “统一 folder/label/category 模型”并没有保住 Gmail 的原生语义

- 证据：
  - 规格明确说 `message_folders` 多对多承载 Gmail label：`docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:215`
  - 当前 Gmail 同步只同步两个系统标签：`crates/pebble-mail/src/gmail_sync.rs:166`
  - 同一封 Gmail 邮件只写入一个本地 folder：`crates/pebble-mail/src/gmail_sync.rs:241`
- 结论：
  - 这更像“把 Gmail 压扁成了 IMAP 风格的文件夹镜像”。
- 建议：
  - 要么减少前端暴露范围，要么真实维护多标签关系。

### 7.3 `High` 规格里“幂等同步 / 冲突检测”依赖的 `remote_version` 当前根本没落地

- 证据：
  - 规格把 `remote_version` 定义成 Gmail `historyId` / Outlook `changeKey` / IMAP `MODSEQ`：`docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:136`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:218`
  - 现有三条主写入路径都把它写成 `None`：`crates/pebble-mail/src/sync.rs:338`, `crates/pebble-mail/src/provider/gmail.rs:340`, `crates/pebble-mail/src/provider/outlook.rs:333`
- 结论：
  - 当前同步更接近“cursor/UID 驱动抓取”，不是具备冲突判断能力的双向状态机。
- 建议：
  - 删除相关产品承诺，或真正持久化 per-message 版本戳。

### 7.4 `Medium` `Cloud Sync` 的命名明显重于实现

- 证据：
  - 规格写的是可选 WebDAV 云同步：`docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:14`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:725`
  - 前端标题直接叫 `Cloud Sync`：`src/features/settings/CloudSyncTab.tsx:135`
  - 实际导出只包含 account 壳、规则、看板和被清空的翻译配置：`crates/pebble-store/src/cloud_sync.rs:143`
- 结论：
  - 这不是 sync，而是 subset backup / restore。
- 建议：
  - 直接改名为“WebDAV 设置备份”，除非后续真的补自动导出、远端版本检查和冲突策略。

### 7.5 `Medium` “默认严格的隐私客户端”与当前实现不符

- 证据：
  - 规格写的是默认 `Strict`：`docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:377`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:398`
  - 规格还写了 iframe sandbox 渲染：`docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:708`
  - 当前设置页默认是 `relaxed`：`src/features/settings/PrivacyTab.tsx:14`
  - 消息详情默认是 `LoadOnce`：`src/components/MessageDetail.tsx:65`
  - 实际渲染用的是 sanitized shadow DOM：`src/components/ShadowDomEmail.tsx:31`
- 结论：
  - 当前更准确的叙事是“隐私增强渲染”，不是“默认最严隔离”。
- 建议：
  - 对外调整表述；如果要保留 privacy-first 叙事，就把默认切回 strict，并恢复真正的隔离模型。

### 7.6 `Medium` “双语对照 + 本地缓存翻译系统”也还不是成立的产品能力

- 证据：
  - 规格承诺按原文哈希缓存、支持双语分段：`docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:390`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:396`, `docs/superpowers/specs/2026-04-04-pebble-email-client-design.md:397`
  - 当前翻译服务更像即时转发：`crates/pebble-translate/src/lib.rs:10`
  - HTML 邮件的 bilingual 只是直接改写 DOM 文本节点：`src/components/MessageDetail.tsx:203`
- 结论：
  - 当前更像“按需翻译”，不是有缓存、有稳定 segment 模型的双语工作流。
- 建议：
  - 先收缩为“按需翻译”，再决定是否投入做缓存层和统一 `segments` 数据模型。

## 建议优先级

### 立即处理

1. 修复 OAuth 回调 `state` 校验。
2. 统一规则 JSON 协议，修复“保存成功但永不执行”。
3. 明确 Gmail OAuth 当前状态：要么补齐发送 / flag / 附件 / 连接测试，要么在 UI 中禁用并改文案。
4. 修复 Compose 草稿离开保护，避免静默丢稿。
5. 修复搜索索引一致性，至少补新消息小批量提交和删除 / 归档索引更新。

### 第二阶段

1. 统一服务端状态管理，减少 React Query / Zustand / 局部 state 的并存。
2. 批量化搜索结果详情、消息标签查询和联系人搜索。
3. 把 `Cloud Sync` 改名或补成真正的同步。
4. 完整补齐模态框、焦点和 live region 这类基础可访问性能力。

### 第三阶段

1. 拆分 `MessageDetail`、重复消息动作、command 层应用服务。
2. 决定是否继续保留“多 provider / privacy-first / bilingual workflow”的产品叙事；如果保留，就按规格把底层能力真正补上。

## 最终判断

Pebble 当前最像一个“IMAP-first 的桌面邮件客户端 + 一批本地增强功能 + 局部 Gmail 同步能力”。  
如果目标是继续内部迭代，这个基础是可用的；如果目标是把 `Gmail OAuth`、`Rules`、`Cloud Sync`、`privacy-first`、`bilingual workflow` 当作正式卖点对外承诺，当前实现还不够扎实。
