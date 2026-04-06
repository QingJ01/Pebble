# Pebble 多角度团队审查报告

日期：2026-04-05  
仓库：`D:\project\Pebble`  
范围：全仓库（React/Tauri/Rust/workspace crates）

## 审查结论

这次审查把代码库当作一个“七人评审团队”来审：功能完成度、用户体验设计、可维护性/代码整洁度、安全/合规、性能优化、Bug 搜索、质疑者视角。

总体判断：

- 项目已经具备一套能运行的邮件客户端骨架，自动化测试当前也是绿的。
- 但代码里仍有几处“看起来支持、实际上没闭环”的功能，尤其是 OAuth、规则动作、Cloud Sync 的产品承诺。
- 前端交互已能完成主路径，但在可访问性、焦点管理、表单语义、草稿保护上还不够成熟。
- 架构上已经出现典型的二次演化信号：React Query、Zustand、本地组件状态三套状态源并存；少数文件过大；无效化刷新 scattered。

## 验证结果

- `pnpm test`：通过，`4` 个测试文件、`22` 个测试全部通过。
- `cargo test --workspace`：通过。主要 crates 的单元测试均通过，包括：
  - `pebble-mail`：`66` 通过
  - `pebble-store`：`38` 通过
  - `pebble-oauth`：`15` 通过
  - `pebble-privacy`：`13` 通过
  - `pebble-search`：`5` 通过
  - `pebble-rules`：`7` 通过
- 需要注意：前端测试仅覆盖 `tests/stores/*.test.ts` 四个 store 文件，未覆盖关键组件、Tauri 命令桥接、OAuth/Cloud Sync/规则 UI 主流程。

## 一票否决级红旗

- [高] 修改账号连接参数时，`imap_port` / `smtp_port` / `use_tls` 的单独变更不会被保存。证据：`src-tauri/src/commands/accounts.rs:95-120` 里确实更新了这些字段，但写回逻辑只在 `password` / `imap_host` / `smtp_host` 变化时才进入，条件写在 `src-tauri/src/commands/accounts.rs:96`。
- [高] OAuth 账号接入没有真正打通。前端建号表单把 provider 固定成 `imap`，证据：`src/components/AccountSetup.tsx:56-66`；前端虽导出了 OAuth API，证据：`src/lib/api.ts:437-446`，但仓库内没有 UI 调用；后端 OAuth 仍使用占位 client id，证据：`src-tauri/src/commands/oauth.rs:9`、`src-tauri/src/commands/oauth.rs:24`，且 `pkce_state` 明确 TODO 未持久化，见 `src-tauri/src/commands/oauth.rs:73-75`。
- [高] 规则系统里 `AddLabel` / `MoveToFolder` 是“声明支持、执行时只打日志”的 no-op。类型定义在 `crates/pebble-rules/src/types.rs:48-49`，真正执行时只记录日志，见 `src-tauri/src/commands/sync_cmd.rs:283-289`。
- [高] 附件下载路径校验过于宽松，注释声称只允许用户目录/下载目录，但实现实际只要求“父目录存在且是绝对路径”。证据：注释在 `src-tauri/src/commands/attachments.rs:6`，实际校验逻辑在 `src-tauri/src/commands/attachments.rs:7-32`，最终直接 `copy` 到调用方给出的路径，见 `src-tauri/src/commands/attachments.rs:63-69`。

## 1. 功能完成度队友

- [高] OAuth 登录路径是半成品，不是可交付功能。
  - 证据：`src/components/AccountSetup.tsx:56-66` 把 `provider` 默认且固定为 `imap`；`src/lib/api.ts:437-446` 暴露了 OAuth API；`src-tauri/src/commands/oauth.rs:9-24` 使用占位 client id；`src-tauri/src/commands/oauth.rs:73-75` 明确承认 `pkce_state` 还没落存。
  - 影响：代码表面上支持 Gmail/Outlook OAuth，实际产品入口没闭环，生产可用性也不成立。

- [高] 规则功能的“动作支持面”与实际实现不一致。
  - 证据：`crates/pebble-rules/src/types.rs:48-49` 暴露 `AddLabel` 与 `MoveToFolder`；`src/features/settings/RulesTab.tsx:72-99` 允许用户保存任意动作 JSON；但 `src-tauri/src/commands/sync_cmd.rs:283-289` 只打印日志，不真正执行。
  - 影响：用户会以为规则支持更多动作，实际上只有 `MarkRead`、`Archive`、`SetKanbanColumn` 真正落地。

- [中] “Cloud Sync” 更准确地说是“部分设置备份/恢复”，不是云同步。
  - 证据：`crates/pebble-store/src/cloud_sync.rs:17-24` 明确排除了密码和认证 secrets；`crates/pebble-store/src/cloud_sync.rs:121-149` 只导出账号基础信息、规则、看板、翻译配置；其中翻译配置还被清空，见 `crates/pebble-store/src/cloud_sync.rs:136-140`。
  - 影响：UI 名称容易让人理解成“设备间完整同步”，但恢复后并不会得到账号凭据、历史邮件或翻译密钥。

- [中] 当前同步主路径仍然是 IMAP-only，provider 枚举和 OAuth 支持还没有真正渗透到同步层。
  - 证据：`src-tauri/src/commands/sync_cmd.rs:57-82` 全程反序列化 `ImapConfig`，并在 `src-tauri/src/commands/sync_cmd.rs:82` 固定创建 `ImapMailProvider`。
  - 影响：即使账号类型被记成 Gmail/Outlook，同步实现依然是 IMAP 中心架构。

## 2. 用户体验设计队友

- [高] 线程消息气泡头部是可点击 `div`，没有 button 语义，也没有键盘交互。
  - 证据：`src/components/ThreadMessageBubble.tsx:42-50` 直接在 `div` 上挂 `onClick`。
  - 影响：键盘用户和辅助技术用户无法可靠展开/折叠线程消息，违反基础可访问性预期。

- [高] 账户设置表单的表单语义明显不足。
  - 证据：`src/components/AccountSetup.tsx:119-128` 把输入框统一设为 `outline: "none"`；`src/components/AccountSetup.tsx:245-340` 各输入有视觉 label，但没有 `htmlFor`、`name`、`autocomplete`。
  - 影响：焦点可见性差，密码管理器/浏览器自动填充支持弱，点击 label 不能把焦点送到字段，整体可访问性不足。

- [中] 正在编辑邮件时缺少“离开前保护”，很容易误丢草稿。
  - 证据：`src/features/compose/ComposeView.tsx:32-87` 把收件人、主题、正文等都保存在组件本地状态；`src/stores/ui.store.ts:46` 的 `setActiveView` 可以无条件切换页面。
  - 影响：用户在写信过程中点击侧边栏任何视图都可能直接卸载 `ComposeView` 并丢失本地编辑状态。

- [中] 设置页的图标按钮语义不完整。
  - 证据：`src/features/settings/AccountsTab.tsx:153-179` 的编辑/删除按钮只有 `title`，没有 `aria-label`。
  - 影响：屏幕阅读器会丢失清晰名称，图标按钮的可访问性不达标。

- [中] 设置页标签状态没有持久化，也没有任何可分享/可恢复的导航状态。
  - 证据：`src/features/settings/SettingsView.tsx:25` 用本地 `useState("accounts")` 保存当前 tab，`src/features/settings/SettingsView.tsx:36-39` 只做内存态切换。
  - 影响：回到设置页时总是重置到账号 tab，难以深链到具体设置项，状态恢复体验差。

- [低] 搜索页输入框默认 `autoFocus`，同时去掉原生 outline。
  - 证据：`src/features/search/SearchView.tsx:129-139`。
  - 影响：桌面上问题不大，但对焦点管理和可访问焦点可见性不够谨慎。

## 3. 可维护性 / 代码整洁度审查员

- [高] 少数核心文件已经膨胀成“多职责巨石文件”。
  - 证据：
    - `src/components/MessageDetail.tsx`：`652` 行，文件开头就同时承担 HTML 消毒、翻译、隐私模式、消息动作、附件区域等职责，见 `src/components/MessageDetail.tsx:1-35`。
    - `src/features/compose/ComposeView.tsx`：`610` 行，单文件混合收件人逻辑、回复/转发拼装、三种编辑模式、预览、发送错误处理，见 `src/features/compose/ComposeView.tsx:1-45`。
    - `src/lib/api.ts`：`386` 行，把所有 Tauri invoke API 聚在一个超大文件。
  - 影响：任何改动都容易牵一发动全身，组件测试和局部重构成本都在上升。

- [高] 邮件域状态存在明显双轨甚至三轨。
  - 证据：`src/stores/mail.store.ts:29-49` 明确保留 “Convenience cache” 和 “Legacy fetch methods”；与此同时项目已经大量使用 React Query；另外很多组件又持有自己的局部 state。
  - 影响：谁是 source of truth 不够清晰，导致大量无效化刷新、回填同步和遗留兼容逻辑。

- [中] 数据刷新逻辑散落在多个 UI 入口，维护成本高。
  - 证据：`src/app/Layout.tsx:48-53`、`src/components/StatusBar.tsx:42-57`、`src/features/settings/CloudSyncTab.tsx:109-110` 都在主动 `invalidateQueries`。
  - 影响：后续如果 query key 或刷新策略调整，很容易遗漏或造成重复刷新。

- [中] 规则配置 UI 直接暴露底层 JSON 字符串，领域边界没有被产品层吸收。
  - 证据：`src/features/settings/RulesTab.tsx:6-20` 直接把 `conditions/actions` 作为 string；`src/features/settings/RulesTab.tsx:72-82` 只做 `JSON.parse`；`src-tauri/src/commands/rules.rs:8-19` 原样存储。
  - 影响：前后端都缺少结构化约束，未来要加动作/条件校验时很难演进。

## 4. 安全 / 合规审查员

- [高] 附件导出路径约束名不副实，实际允许写入任意已存在的绝对目录。
  - 证据：`src-tauri/src/commands/attachments.rs:6-32`。
  - 影响：这不是典型路径穿越，但它把“可写目标范围”放得比注释和产品预期宽很多；一旦前端或插件侧传入任意路径，后端不会阻止。

- [高] OAuth 流程不具备生产合规成熟度。
  - 证据：占位 client id 在 `src-tauri/src/commands/oauth.rs:9`、`src-tauri/src/commands/oauth.rs:24`；`pkce_state` 未持久化在 `src-tauri/src/commands/oauth.rs:73-75`。
  - 影响：即使补 UI，也还不应被视作 production-ready 的认证接入。

- [中] WebDAV 连接没有强制 HTTPS，且直接使用 Basic Auth。
  - 证据：`crates/pebble-store/src/cloud_sync.rs:34-49` 接受任意 URL 并直接对该 URL 发 Basic Auth 请求，没有 scheme 校验。
  - 影响：如果用户输入 `http://` 地址，凭据会在链路上暴露；这对“Cloud Sync”一类功能来说是明显的安全姿态缺口。

- [中] 线程详情的 HTML 直接注入宿主 DOM，而主消息详情使用了 iframe 隔离，两者安全边界不一致。
  - 证据：`src/components/ThreadMessageBubble.tsx:70-73` 使用 `dangerouslySetInnerHTML`；对比 `src/components/MessageDetail.tsx` 主视图采用 sandboxed iframe。
  - 影响：虽然已有双重 sanitize，但线程场景仍然选择了更脆弱的渲染边界。

## 5. 性能优化审查员

- [高] 搜索结果存在明显 N+1 查询。
  - 证据：`src/features/search/SearchView.tsx:81-100` 对每个 hit 再 `getMessage(id)`，并用 `Promise.all` 并发补详情。
  - 影响：搜索一次会产生 `1 + N` 次 Tauri 调用；结果数一多，延迟和主线程压力都会上升。

- [中] 搜索引擎每次查询都重新构建 schema 和 reader。
  - 证据：`crates/pebble-search/src/lib.rs:137-145` 与 `crates/pebble-search/src/lib.rs:190-202`。
  - 影响：查询热路径上重复创建 reader/schema，会把本可缓存的工作反复做一遍。

- [中] 首次建号后依靠固定次数轮询刷 folder，属于“时间驱动”而不是“事件驱动”。
  - 证据：`src/components/AccountSetup.tsx:103-111` 每 2 秒轮询一次，共 5 次。
  - 影响：慢网下不稳定，快网下多余；而且每次都走 query invalidation。

- [中] 网络状态检测采用 30 秒健康检查轮询，即使浏览器已有 online/offline 事件。
  - 证据：`src/hooks/useNetworkStatus.ts:5`、`src/hooks/useNetworkStatus.ts:23-33`。
  - 影响：后台长期运行时会形成持续性健康探测流量和状态抖动源。

- [中] Cloud Sync restore 直接全局 invalidation。
  - 证据：`src/features/settings/CloudSyncTab.tsx:109-110`。
  - 影响：恢复后会把所有 query 一次性打失效，刷新范围过粗。

## 6. 寻找 Bug 审查员

- [高] 端口和 TLS 开关变更可能“提交成功但不生效”。
  - 证据：`src-tauri/src/commands/accounts.rs:96` 的 if 条件没有把 `imap_port`、`smtp_port`、`use_tls` 纳入触发条件，而实际字段更新代码在 `src-tauri/src/commands/accounts.rs:109-120`。
  - 复现思路：编辑一个已存在账号，只改 IMAP/SMTP 端口或 TLS 开关，不改密码和 host，保存后重新同步，连接配置不会更新。

- [中] 附件路径 API 的前后端类型签名不一致。
  - 证据：前端 `src/lib/api.ts:266-267` 认为 `getAttachmentPath()` 返回 `Promise<string>`；后端 `src-tauri/src/commands/attachments.rs:44-49` 实际返回 `Option<String>`。
  - 影响：一旦前端开始使用该 API，遇到本地路径不存在的附件就会出现空值处理错误。

- [中] 新增账号时 `provider` 会被写成 Gmail/Outlook，但同步元数据仍被硬编码成 `imap`。
  - 证据：`src-tauri/src/commands/accounts.rs:29-39` 会根据请求设置 provider；但 `src-tauri/src/commands/accounts.rs:69-70` 又把 metadata 写成固定 `"imap"`。
  - 影响：这会让账号类型和同步元数据出现分叉，属于后续 OAuth/Provider 逻辑的潜在埋雷点。

- [中] 正在写邮件时切换视图会丢失本地草稿，这是一个真实行为 bug，而不是单纯 UX 建议。
  - 证据：`src/features/compose/ComposeView.tsx:32-87` 全部内容都在组件 local state；`src/stores/ui.store.ts:46` 允许任意地方直接切换 `activeView`。
  - 复现思路：开始写邮件，不发送，直接点侧边栏进入 Search/Settings，再返回 Compose，内容已丢。

## 7. “质疑者” / Devil’s Advocate

- [高] 这个仓库现在更像“IMAP 客户端主干 + OAuth/Provider 扩展中的过渡态”，不是一个已经统一抽象完成的多 provider 架构。
  - 证据：类型层有 Gmail/Outlook provider，见 `src/lib/api.ts:6-13`、`src-tauri/src/commands/accounts.rs:29-32`；OAuth 命令也在；但同步核心仍然是 `ImapMailProvider`，见 `src-tauri/src/commands/sync_cmd.rs:57-82`。
  - 质疑点：如果产品对外宣称“支持 Gmail/Outlook”，当前实现深层逻辑并不真正支撑这个说法。

- [中] “Rules” 这项功能目前更像工程师调试入口，而不是面向普通用户的产品化能力。
  - 证据：`src/features/settings/RulesTab.tsx:72-82` 只做 JSON 语法校验；UI 直接要求用户填写原始 `conditions/actions`。
  - 质疑点：如果没有结构化编辑器、动作合法性校验和执行能力矩阵，这个功能的失败率会很高。

- [中] “Cloud Sync” 的名称对当前实现来说过重。
  - 证据：前端标签就是 `Cloud Sync`，见 `src/features/settings/SettingsView.tsx:20`；但备份 payload 明确不包含 auth secrets，见 `crates/pebble-store/src/cloud_sync.rs:17-24`。
  - 质疑点：在真实用户心智里，Cloud Sync 往往意味着账号、密钥、配置、数据的设备间可恢复；当前实现更接近“设置导出/导入”。

## 额外观察

- 自动化测试是通过的，但当前“测试绿”并不等于“产品已闭环”。最明显的漏网点就是：
  - OAuth 入口闭环
  - 规则动作真实性
  - 账号编辑连接参数写回
  - Compose 草稿丢失
  - 组件级可访问性
- 前端当前只有 `tests/stores/ui.store.test.ts`、`tests/stores/mail.store.test.ts`、`tests/stores/kanban.store.test.ts`、`tests/stores/command.store.test.ts` 四个 store 测试文件，缺少组件与流程级保障。

## 建议优先级

- P0
  - 修复 `update_account` 的条件 bug，确保端口/TLS 变更能落盘。
  - 明确 OAuth 状态：要么真正打通入口和 state/pkce 持久化，要么先去掉对外可见承诺。
  - 在规则系统里下架或禁用未实现动作，至少不要让 UI 保存后形成假功能。
  - 收紧附件保存路径策略，使实现与注释/产品预期一致。

- P1
  - 为 Compose 增加草稿离开保护或本地草稿持久化。
  - 把 `ThreadMessageBubble` 的交互从 clickable `div` 改为语义化按钮/summary 模式。
  - 给账户设置表单补 `htmlFor` / `name` / `autocomplete` / focus-visible。
  - 把 Cloud Sync 改名为“设置备份/恢复”，或扩展到真正的同步语义。

- P2
  - 把 `MessageDetail`、`ComposeView`、`api.ts` 拆分成更清晰的模块。
  - 收敛 React Query 与 Zustand 的职责边界，减少 duplicated cache。
  - 将搜索详情补全改成批量接口，避免 `1 + N` 查询。
  - 缓存 Tantivy reader/schema，减少查询热路径重复构建。

## 最终判断

Pebble 现在最像一个“主路径基本成型、边缘能力和产品化打磨尚未完成”的项目。  
如果目标是继续内部迭代，当前代码基座是可用的；如果目标是把“支持 OAuth / 规则 / Cloud Sync / 多 provider”作为正式卖点对外表达，现阶段还不够稳。
