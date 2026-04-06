# Pebble 邮件客户端 -- 全栈代码审计报告

> 审计日期: 2026-04-05  
> 审计范围: 全部 Rust 后端 crate、Tauri 命令层、React/TypeScript 前端、构建配置  
> 审计方式: 6 个并行子代理逐文件逐行审查

---

## 目录

- [一、总体评估](#一总体评估)
- [二、严重问题汇总 (P0 -- 必须立即修复)](#二严重问题汇总-p0----必须立即修复)
- [三、重要问题汇总 (P1 -- 应尽快修复)](#三重要问题汇总-p1----应尽快修复)
- [四、设计缺陷 (P2 -- 需要改进)](#四设计缺陷-p2----需要改进)
- [五、代码质量问题 (P3 -- 建议优化)](#五代码质量问题-p3----建议优化)
- [六、未完成功能清单](#六未完成功能清单)
- [七、按模块详细报告](#七按模块详细报告)
- [八、优先修复路线图](#八优先修复路线图)

---

## 一、总体评估

| 维度 | 评分 | 说明 |
|------|------|------|
| 架构设计 | ★★★☆☆ | Crate 划分合理,但前端双重状态管理是重大技术债 |
| 安全性 | ★★☆☆☆ | 存在路径穿越漏洞、XSS 风险、OAuth 流程断裂等多个安全问题 |
| 功能完整度 | ★★☆☆☆ | OAuth 不可用、多个 trait 方法为桩实现、Archive/Delete 未实现 |
| 代码质量 | ★★★☆☆ | SQL 参数化做得好,但 unwrap/expect 泛滥,错误处理不一致 |
| 测试覆盖 | ★★☆☆☆ | 8/9 个后端 crate 无 dev-dependencies,前端仅 4 个 store 测试 |
| 国际化 | ★★☆☆☆ | en/zh JSON 键完整,但大量组件存在硬编码英文字符串 |

### 问题统计

| 严重级别 | 数量 |
|----------|------|
| **P0 严重** (安全漏洞/功能完全不可用) | **22** |
| **P1 重要** (Bug/重要功能缺陷) | **54** |
| **P2 设计缺陷** | **27** |
| **P3 代码质量/建议** | **43** |

---

## 二、严重问题汇总 (P0 -- 必须立即修复)

### 安全漏洞

| # | 模块 | 文件 | 问题描述 |
|---|------|------|----------|
| S1 | pebble-mail | `sync.rs:299` | **路径穿越漏洞**: 附件文件名未清理,恶意文件名如 `../../etc/passwd` 可导致任意文件写入 |
| S2 | src-tauri | `attachments.rs:33` | **路径穿越漏洞**: `download_attachment` 的 `save_to` 参数直接用于 `std::fs::copy`,无路径校验 |
| S3 | 前端 | `MessageDetail.tsx:358`, `ThreadMessageBubble.tsx:69` | **XSS 风险**: `dangerouslySetInnerHTML` 渲染邮件 HTML,无前端 sanitization 防御层 |
| S4 | 前端 | `ComposeView.tsx:80-83` | **XSS 风险**: reply/forward 引用内容直接拼接 HTML,`from_name` 可包含脚本 |
| S5 | 前端 | `AttachmentList.tsx:60` | **路径穿越风险**: 附件文件名未经 sanitization 直接拼接为保存路径 |

### 功能完全不可用

| # | 模块 | 文件 | 问题描述 |
|---|------|------|----------|
| F1 | src-tauri | `oauth.rs:9,24` | **OAuth 使用占位符 Client ID**: `GOOGLE_CLIENT_ID_PLACEHOLDER` / `MICROSOFT_CLIENT_ID_PLACEHOLDER`,OAuth 登录完全不可用 |
| F2 | src-tauri | `oauth.rs:68,96` | **PKCE 状态丢弃**: `start_oauth_flow` 丢弃 PKCE state,`complete_oauth_flow` 重新生成不匹配的 PKCE,安全流程断裂 |
| F3 | pebble-translate | `generic.rs:19-23` | **GenericApi 翻译完全不可用**: `serde_json::json!` 宏不展开变量值作为键名,生成的 JSON 字段名全部错误 |
| F4 | pebble-mail | `imap_provider.rs:50-65` | **`fetch_messages` 桩实现**: 调用底层方法但丢弃结果,返回空 vec,违反 trait 契约 |
| F5 | pebble-mail | `imap_provider.rs:72-83` | **`sync_changes` 桩实现**: 总是返回空 `ChangeSet`,增量同步不工作 |
| F6 | pebble-mail | `imap_provider.rs:102-107` | **`move_message` 未实现**: 直接返回错误 "IMAP move not yet implemented" |
| F7 | 前端 | `MessageDetail.tsx:278` | **Archive/Trash 按钮无功能**: action 是空函数 `async () => {}`,UI 展示但不工作 |

### 系统稳定性

| # | 模块 | 文件 | 问题描述 |
|---|------|------|----------|
| C1 | pebble-mail | `imap.rs:126` | **IMAP greeting 读取不完整**: 单次 `read` 不保证读取完整数据,TCP 分片时导致异常 |
| C2 | pebble-mail | `imap.rs:145-158` | **ID 命令无超时**: 响应读取循环无超时机制,服务器不响应时永久阻塞 |
| C3 | pebble-mail | `imap.rs:313,329,386,596` | **UID 回退到序列号**: `unwrap_or(fetch.message)` 将序列号当 UID 使用,操作错误消息 |
| C4 | src-tauri | `lib.rs:12-65` | **启动阶段 6 处 `expect()`**: 初始化失败直接 panic,无用户友好错误 |
| C5 | pebble-search | `lib.rs:36-41` | **索引操作 `expect()`/`unwrap()`**: 索引损坏时 panic 崩溃应用 |
| C6 | pebble-store | `migrations.rs` | **无 schema 版本跟踪**: 无 `user_version` 或版本表,V2 迁移列检测方式不可靠,未来 schema 变更无法管理 |
| C7 | 前端 | `kanban.store.ts:50-67` | **reorderInColumn 竞态回滚**: 多个独立请求的 `.catch()` 各自回滚整个数组,部分成功时状态不一致 |
| C8 | 前端 | `useKeyboard.ts:70-76` | **toggle-star 状态不同步**: 直接调用 API 但不更新 UI 状态,用户操作无视觉反馈 |
| C9 | 前端 | `commands.ts` | **命令面板不支持 i18n**: 所有命令 name/category 在 `buildCommands()` 中静态构建,切换语言后不更新 |
| C10 | 前端 | 多个组件 | **大量硬编码英文字符串**: RulesTab、TranslateTab、PrivacyBanner、CommandPalette、KanbanView 等整个组件无 i18n |

---

## 三、重要问题汇总 (P1 -- 应尽快修复)

### 后端 -- pebble-mail

| # | 文件 | 问题 |
|---|------|------|
| 1 | `smtp.rs` | SMTP 使用同步 `SmtpTransport` 阻塞 Tokio 运行时,应改用 `AsyncSmtpTransport` |
| 2 | `smtp.rs` | 每次发送新建 SMTP 连接,无复用 |
| 3 | `imap.rs:358-362` | 大量 UID 拼为单一 IMAP 命令,可能超服务器长度限制,需分批 |
| 4 | `imap.rs` | 每次操作重新 SELECT 邮箱,高频调用下产生大量冗余 IMAP 命令 |
| 5 | `sync.rs:295,300` | 异步上下文中使用同步文件 I/O (`std::fs`),阻塞运行时 |
| 6 | `sync.rs:529` | 连接失败直接 return,无重试机制,网络短暂中断导致同步永久停止 |
| 7 | `sync.rs:145` | `insert_folder` 错误被 `let _ =` 静默丢弃,包括数据库损坏等严重错误 |
| 8 | `thread.rs` | `normalize_subject` 函数是死代码,声明但从未被调用 |
| 9 | `gmail.rs:152,156` | `RwLock::unwrap()` 在 poison 后级联 panic |
| 10 | `gmail.rs:281-306` | `fetch_messages` 只获取 ID 列表,返回空 messages vec |
| 11 | `gmail.rs:524-532` | `list_drafts` 返回空 vec,功能未完成 |
| 12 | `gmail.rs:607-655` | `build_raw_message` 缺少 From/Date/MIME-Version header,Subject 无 RFC 2047 编码 |
| 13 | `outlook.rs:204,208` | 同 Gmail 的 `RwLock::unwrap()` poison 风险 |
| 14 | `outlook.rs:435-478` | `sync_changes` delta 查询不处理分页,大变更集只取第一页 |
| 15 | `outlook.rs:798-826` | 手写 ISO 8601 解析器不处理时区偏移,不验证非法日期 |

### 后端 -- pebble-store

| # | 文件 | 问题 |
|---|------|------|
| 16 | `messages.rs:73-138` | 手动事务管理: COMMIT 失败不 ROLLBACK,连接状态不一致 |
| 17 | `messages.rs:302-360` | `bulk_update_flags`/`bulk_soft_delete` 循环中失败不 ROLLBACK |
| 18 | `messages.rs:407-462` | `list_threads_by_folder`: `MAX(subject)` 取字母序而非最新消息主题 |
| 19 | `cloud_sync.rs:153-192` | `import_settings` 非事务性,中途失败导致数据不一致 |
| 20 | `contacts.rs:19` | LIKE 通配符注入: `%` 和 `_` 未转义 |
| 21 | `folders.rs:49-55` | `.ok()` 吞掉所有数据库错误,不仅仅是 "未找到" |
| 22 | `auth_data.rs` | `set_auth_data` UPDATE 在账户不存在时静默成功(影响 0 行) |
| 23 | `accounts.rs` | `get_account_sync_state` 与 `get_sync_cursor` 错误行为不一致 |
| 24 | `migrations.rs` | 缺少 `(account_id, remote_id)` 和 `unsnoozed_at` 关键索引 |
| 25 | `migrations.rs` | `labels`/`message_labels` 表已建但无代码使用(死表) |

### 后端 -- pebble-privacy

| # | 文件 | 问题 |
|---|------|------|
| 26 | `sanitizer.rs:134-197` | 手写 HTML img 解析器极脆弱,注释中 `>`、属性名前缀匹配等边界情况均会失败 |
| 27 | `sanitizer.rs:303-328` | `extract_attr_value` 错误匹配 `data-src` 中的 `src` |
| 28 | `tracker.rs:21` | `"facebook.com/tr"` 包含路径但 `domain.contains()` 只匹配域名,永不生效 |

### 后端 -- pebble-search

| # | 文件 | 问题 |
|---|------|------|
| 29 | `lib.rs` | `build_schema()` 每次调用重建,应缓存到结构体 |
| 30 | `lib.rs` | `index_message` 不去重,同一消息多次索引产生重复文档 |
| 31 | `lib.rs` | 缺少 `delete_message` 方法,无法增量更新索引 |

### 后端 -- pebble-oauth

| # | 文件 | 问题 |
|---|------|------|
| 32 | `redirect.rs:17-63` | `wait_for_redirect` 无超时,用户不完成授权时永久阻塞 |
| 33 | `lib.rs` | CSRF token (`state`) 在 `complete_auth` 中未验证,CSRF 保护形同虚设 |

### Tauri 命令层

| # | 文件 | 问题 |
|---|------|------|
| 34 | `events.rs` | 4/5 事件常量未使用: `MAIL_SYNC_PROGRESS`/`MAIL_NEW` 等从未发出,前端无法获知同步状态 |
| 35 | `snooze_watcher.rs` | 在异步上下文使用 `std::sync::mpsc::Receiver`,应用 `tokio::sync` |
| 36 | `snooze_watcher.rs:34` | `unsnooze` 失败被 `let _ =` 静默忽略,无日志 |
| 37 | `snooze_watcher.rs:35` | 事件名硬编码 `"mail:unsnoozed"` 而非使用常量 |
| 38 | `accounts.rs` | `AddAccountRequest` 完全无输入验证(邮箱格式、端口范围、空值) |
| 39 | `accounts.rs:88-102` | `delete_account` 未清理关联数据(文件夹、消息、索引、附件、凭证) |
| 40 | `sync_cmd.rs` | 同步任务结束后 `SyncHandle` 不从 map 中移除,导致无法重启同步 |
| 41 | `sync_cmd.rs:60` | IMAP 配置 fallback: 无 `"imap"` 键时将整个 JSON 反序列化为 `ImapConfig` |
| 42 | `cloud_sync.rs` | WebDAV 凭证每次通过 IPC 明文传输 |
| 43 | `compose.rs` | 缺少收件人、邮箱格式等输入验证 |
| 44 | `kanban.rs:16` | position 默认 0,多卡片同默认值导致排序不确定 |
| 45 | `rules.rs` | `conditions`/`actions` 为原始 JSON 字符串,无格式校验 |
| 46 | `attachments.rs` | 同步 `std::fs::copy` 阻塞异步运行时 |
| 47 | `capabilities/default.json` | 缺少 `shell:open`、`dialog`、`fs` 等权限声明 |

### 前端

| # | 文件 | 问题 |
|---|------|------|
| 48 | `AccountSetup.tsx:347-371` | TLS checkbox 点击事件双重触发(冒泡 + onChange) |
| 49 | `AccountSetup.tsx:289` | `parseInt` 未处理 NaN,非数字输入导致 NaN 写入 state |
| 50 | `MessageDetail.tsx:55` | `updateMessageFlags` 标记已读的错误被 `.catch(() => {})` 吞掉 |
| 51 | `MessageDetail.tsx:127` | `localStorage.getItem` 与 UIStore 状态可能不一致 |
| 52 | `StatusBar.tsx:70` | `retryQueue.pendingCount` 非响应式,显示可能过时 |
| 53 | `KanbanView.tsx:32-49` | `setMessages` 回调内触发异步操作再 setState,反模式 |
| 54 | `SearchView.tsx:77,103` | 禁用 `exhaustive-deps` 规则,存在陈旧闭包风险 |

---

## 四、设计缺陷 (P2 -- 需要改进)

### 架构层面

| # | 问题 | 影响 |
|---|------|------|
| 1 | **前端双重状态管理**: Zustand store 的 "Legacy fetch methods" 与 React Query hooks 并存,同一数据在两套缓存中独立运行 | 数据不一致,维护成本高 |
| 2 | **mail.store.ts 混合关注点**: 既含 UI 状态又含数据缓存和获取逻辑 | 违反单一职责 |
| 3 | **kanban.store.ts 未用 React Query**: 与 messages/folders 的数据管理模式不一致 | 缺失自动重验证/乐观更新 |
| 4 | **TLS 后端冲突**: `async-native-tls`(IMAP) + `rustls`(SMTP/HTTP) 混用 | 二进制膨胀,安全审计面扩大 |
| 5 | **pebble-store 中包含 reqwest**: 存储层直接发起 HTTP 请求(cloud_sync),违反分层 | 关注点混乱 |
| 6 | **PebbleError 全用 String**: 丢失原始错误类型信息,无 `From` 实现,样板代码多 | 错误处理体验差 |

### API/接口设计

| # | 问题 | 影响 |
|---|------|------|
| 7 | `FetchQuery::folder_id` 语义不一致: IMAP 当邮箱名,Gmail/Outlook 当 folder ID | 实现者易误用 |
| 8 | `compose` 命令 9 个参数,应重构为 struct | 可读性/可维护性差 |
| 9 | `command.store.ts` 的 `registerCommands` 替换而非合并 | 多组件注册互相覆盖 |
| 10 | `useUpdateFlagsMutation` 乐观更新不完整: 只更新单条缓存不更新列表 | 列表 UI 延迟刷新 |
| 11 | `useSyncMutation` 未更新 `syncStatus` | 同步中 StatusBar 无状态指示 |
| 12 | `useSendEmailMutation` 无 `onError` | 发送失败无用户反馈 |

### 缺失设计

| # | 问题 | 影响 |
|---|------|------|
| 13 | 无全局 React Query 错误处理 | query 失败静默消化 |
| 14 | 所有 query 使用相同 `staleTime`,未按场景差异化 | Accounts 等稳定数据频繁重请求 |
| 15 | `useMessagesQuery`/`useThreadsQuery` 无分页/无限滚动 | 翻页丢失之前数据 |
| 16 | 前端缺少响应式设计: 所有布局固定像素值 | 窗口缩放时布局异常 |
| 17 | 命令式 hover 效果 (`onMouseEnter`/`onMouseLeave` 操作 style) | 无法处理键盘 focus,代码重复 |
| 18 | CryptoService DEK 内存明文,无 `zeroize` | 内存 dump 可提取密钥 |
| 19 | DEK 仅存于 OS 凭据库,无备份/恢复机制 | 重装系统后数据永久不可解密 |
| 20 | `searchQueryKey` 不在 sync mutation 中失效 | 同步后搜索缓存仍是旧数据 |

---

## 五、代码质量问题 (P3 -- 建议优化)

### 死代码

| 文件 | 描述 |
|------|------|
| `src/hooks/useFolders.ts` | 无任何组件导入,遗留 hook |
| `src/hooks/useMessages.ts` | 无任何组件导入,遗留 hook |
| `src/lib/api.ts` `withRetry` | 导出但无调用者 |
| `mail.store.ts` Legacy fetch methods | 标记 "Legacy" 但迁移未完成 |
| `pebble-mail/thread.rs` `normalize_subject` | 声明+测试但无调用者 |
| `pebble-store/migrations.rs` `labels`/`message_labels` 表 | 已建表但无代码使用 |
| `events.rs` 4/5 事件常量 | `#[allow(dead_code)]` 标注 |

### 测试覆盖

| 缺失测试 | 重要性 |
|-----------|--------|
| 8/9 个后端 crate 无 `[dev-dependencies]` | 高 -- 加密/邮件解析/搜索等关键模块 |
| `shortcut.store.ts` | 中 |
| `hooks/mutations/*` | 中 |
| `hooks/queries/*` | 中 |
| `useKeyboard.ts` | 中 |
| `lib/retry-queue.ts` | 低 |
| `lib/api.ts` | 中 |
| `tsconfig.json` 不包含 tests 目录 | 测试文件不受 TS 严格检查 |

### 构建配置

| 问题 | 影响 |
|------|------|
| 缺少 `[profile.release]` 优化配置 (lto/strip/codegen-units) | Release 产物远大于必要 |
| `tokio` 使用 `features = ["full"]`,过于宽泛 | 编译时间和体积增大 |
| `async-trait` 未纳入 workspace 管理 | 版本不一致风险 |
| `rand = "0.8"` 过旧 (当前 0.9.x),用于加密场景 | 安全风险 |
| `keyring` 仅配置 `windows-native` feature | 不支持跨平台 |
| `lettre` 在 src-tauri 中可能是无用依赖 | 编译时间浪费 |
| 安全依赖 (aes-gcm, ammonia, oauth2) 版本约束过宽 | 可能引入未审查变更 |
| 缺少 ESLint/Prettier 配置和 lint 脚本 | 前端代码风格无保障 |

---

## 六、未完成功能清单

| # | 功能 | 当前状态 | 影响 |
|---|------|----------|------|
| 1 | **OAuth 登录 (Google/Microsoft)** | Client ID 为占位符,PKCE 流程断裂 | 无法使用 OAuth 方式添加账户 |
| 2 | **GenericApi 翻译** | JSON 键名构建 Bug,完全不工作 | 自定义翻译引擎无法使用 |
| 3 | **IMAP move_message** | 返回 "not yet implemented" 错误 | 邮件移动功能不可用 |
| 4 | **IMAP sync_changes** | 桩实现返回空 ChangeSet | 增量同步不工作,只能全量同步 |
| 5 | **Archive/Trash 操作** | 前端按钮绑定空函数 | 用户可见但不可用 |
| 6 | **Gmail 草稿** | `list_drafts` 返回空 vec | 草稿功能不可用 |
| 7 | **邮件标签系统** | 数据库表已建但无代码 | labels/message_labels 为死表 |
| 8 | **同步进度事件** | 事件常量定义但从未发出 | 前端无法显示同步进度 |
| 9 | **新邮件通知事件** | 事件常量定义但从未发出 | 无新邮件实时提醒 |
| 10 | **离线重试队列** | 基础设施就绪但未接入业务代码 | `withRetry` 无调用者 |
| 11 | **附件上传** | ComposeView 无附件功能 | 无法发送带附件邮件 |
| 12 | **草稿自动保存** | ComposeView 无保存逻辑 | 意外关闭丢失编辑内容 |
| 13 | **取消信任发件人** | 只有 `trust_sender`,无 untrust/list | 无法管理已信任发件人 |
| 14 | **规则启用/禁用** | 有 `is_enabled` 字段但无快捷命令 | 只能通过完整 update 修改 |
| 15 | **连接重试** | 同步连接失败直接停止 | 网络波动后需手动重启同步 |

---

## 七、按模块详细报告

### 7.1 pebble-core

| 文件 | 级别 | 问题 |
|------|------|------|
| `types.rs:265` | P0 | `now_timestamp()` 中 `unwrap()`,系统时钟异常时 panic |
| `traits.rs` | P1 | `FetchQuery::folder_id` 语义在不同 provider 间不一致 |
| `error.rs` | P2 | PebbleError 全用 String,无 `From` 实现,样板代码多 |
| `types.rs` | P3 | `Message` 21 字段无 builder;`EmailAddress` 无格式验证 |
| `lib.rs` | P3 | `pub use types::*` 通配符重导出 |
| `traits.rs` | P3 | 多个结构体缺少 `Debug` derive |

### 7.2 pebble-crypto

| 文件 | 级别 | 问题 |
|------|------|------|
| `keystore.rs:31` | P2 | DEK 仅存于 OS 凭据库,无备份/恢复,重装系统后数据不可解密 |
| `lib.rs` | P2 | DEK 内存明文无 zeroize |
| `aes.rs` | P3 | 加密错误用 `PebbleError::Auth` 语义不精确 |
| `keystore.rs` | P3 | `SERVICE_NAME` 固定,多实例共享 DEK |

### 7.3 pebble-mail

| 文件 | 级别 | 问题 |
|------|------|------|
| `sync.rs:299` | **P0** | **路径穿越漏洞**: 附件文件名未清理 |
| `imap.rs:126` | P0 | IMAP greeting 单次 read 不完整 |
| `imap.rs:145-158` | P0 | ID 命令响应读取无超时 |
| `imap.rs:313,329,386,596` | P0 | UID 回退到序列号,语义错误 |
| `imap_provider.rs:50-83` | P0 | `fetch_messages`/`sync_changes` 桩实现 |
| `imap_provider.rs:102-107` | P0 | `move_message` 未实现 |
| `smtp.rs` | P1 | 同步阻塞;每次新建连接 |
| `imap.rs:358` | P1 | 大量 UID 拼单一命令可能超长度限制 |
| `sync.rs:295,300` | P1 | 异步中用同步文件 I/O |
| `sync.rs:529` | P1 | 连接失败无重试 |
| `gmail.rs:152,156` | P1 | RwLock poison 级联 panic |
| `gmail.rs:607-655` | P1 | `build_raw_message` 缺少 From/Date/MIME-Version header |
| `outlook.rs:435-478` | P1 | delta 查询不处理分页 |
| `outlook.rs:798-826` | P1 | 手写日期解析器不处理时区 |
| `thread.rs` | P1 | `normalize_subject` 是死代码 |

### 7.4 pebble-oauth

| 文件 | 级别 | 问题 |
|------|------|------|
| `redirect.rs:17-63` | P1 | `wait_for_redirect` 无超时 |
| `lib.rs` | P1 | CSRF state 不验证 |
| `lib.rs` | P3 | 每次新建 reqwest::Client |
| `lib.rs` | P3 | `OAuthConfig` 与 core 中的重复定义 |

### 7.5 pebble-store

| 文件 | 级别 | 问题 |
|------|------|------|
| `migrations.rs` | P0 | 无 schema 版本跟踪 |
| `messages.rs:73-138` | P1 | 事务 COMMIT 失败不 ROLLBACK |
| `messages.rs:407-462` | P1 | `MAX(subject)` 取字母序而非最新 |
| `cloud_sync.rs:153-192` | P1 | `import_settings` 非事务性 |
| `contacts.rs:19` | P1 | LIKE 通配符注入 |
| `folders.rs:49-55` | P1 | `.ok()` 吞掉所有错误 |
| `auth_data.rs` | P1 | UPDATE 0 行静默成功 |
| `migrations.rs` | P1 | 缺少关键索引 |
| `attachments.rs` | P2 | 无物理文件清理机制 |
| `trusted_senders.rs` | P3 | 邮箱比较无大小写规范化 |
| `cloud_sync.rs` | P3 | WebDAV 密码明文 String |

### 7.6 pebble-privacy

| 文件 | 级别 | 问题 |
|------|------|------|
| `sanitizer.rs:134-197` | P1 | 手写 HTML 解析器极脆弱 |
| `sanitizer.rs:303` | P1 | `data-src` 误匹配 |
| `tracker.rs:21` | P1 | `facebook.com/tr` 包含路径永不匹配 |
| `sanitizer.rs:245-248` | P3 | `TrustSender(_)` 未使用内部值 |

### 7.7 pebble-translate

| 文件 | 级别 | 问题 |
|------|------|------|
| `generic.rs:19-23` | **P0** | JSON 键名构建 Bug,功能完全不可用 |
| `deepl.rs`/`deeplx.rs`/`generic.rs`/`llm.rs` | P3 | 每次新建 reqwest::Client;无超时 |
| `deeplx.rs:49-59` | P3 | `build_segments` zip 静默丢弃不等长数据 |

### 7.8 pebble-search

| 文件 | 级别 | 问题 |
|------|------|------|
| `lib.rs:36-41` | P0 | expect/unwrap 导致 panic |
| `lib.rs` | P1 | schema 每次重建;无去重;无单条删除 |

### 7.9 pebble-rules

| 文件 | 级别 | 问题 |
|------|------|------|
| `matcher.rs:8-13` | P1 | 空 `to_list` 对 `NotContains` 总返回 true |
| `matcher.rs` | P3 | 不支持 CC/BCC 匹配 |

### 7.10 src-tauri 命令层

| 文件 | 级别 | 问题 |
|------|------|------|
| `attachments.rs:33` | **P0** | 路径穿越漏洞 |
| `oauth.rs` | **P0** | OAuth 完全不可用(占位符 + PKCE 断裂) |
| `lib.rs:12-65` | P0 | 6 处 expect() panic |
| `accounts.rs` | P1 | 无输入验证;delete 不清理关联数据 |
| `sync_cmd.rs` | P1 | SyncHandle 泄漏;IMAP 配置 fallback 宽松 |
| `events.rs` | P1 | 4/5 事件未使用 |
| `snooze_watcher.rs` | P1 | std::sync::mpsc 在异步上下文;错误静默 |
| `compose.rs` | P1 | 无输入验证;9 参数需重构 |
| `cloud_sync.rs` | P1 | 凭证明文传输;恢复无确认 |

### 7.11 前端组件

| 文件 | 级别 | 问题 |
|------|------|------|
| `MessageDetail.tsx:358` | P0 | XSS: dangerouslySetInnerHTML 无前端防御 |
| `ComposeView.tsx:80` | P0 | XSS: 引用内容直接拼接 HTML |
| `MessageDetail.tsx:278` | P0 | Archive/Trash 按钮空函数 |
| 多文件 | P0 | 大量硬编码英文字符串未 i18n |
| `AccountSetup.tsx:347` | P1 | TLS checkbox 双重触发 |
| `AccountSetup.tsx:289` | P1 | parseInt NaN 未处理 |
| `AccountSetup.tsx:104` | P1 | pollFolders 无取消机制,组件卸载后继续执行 |
| `MessageDetail.tsx:55` | P1 | 标记已读错误被吞掉 |
| `ThreadMessageBubble.tsx:25` | P1 | HTML 渲染错误被 `.catch(() => {})` 吞掉 |
| `InboxView.tsx:81` | P1 | SearchBar props 回调形同虚设 |
| `AccountsTab.tsx:15` | P1 | 删除账户无确认对话框 |
| `CloudSyncTab.tsx` | P1 | 恢复操作无确认;凭据不持久化 |
| `RulesTab.tsx` | P1 | JSON textarea 编辑条件/动作,用户体验极差 |
| `TranslatePopover.tsx` | P1 | 切换语言无防抖,快速切换大量请求 |
| `TitleBar.tsx:4` | P1 | `getCurrentWindow()` 每次渲染调用 |
| `Layout.tsx:20` | P1 | `useUIStore()` 解构导致任何 state 变化都重渲染 |

### 7.12 前端基础设施

| 文件 | 级别 | 问题 |
|------|------|------|
| `kanban.store.ts:50-67` | P0 | reorderInColumn 竞态回滚 |
| `useKeyboard.ts:70-76` | P0 | toggle-star 不更新 UI |
| stores + queries | P0 | Zustand/React Query 双重数据源冲突 |
| `useFolders.ts`/`useMessages.ts` | P1 | 死代码 -- 无导入者 |
| `retry-queue.ts` | P1 | 基础设施就绪但无业务接入 |
| `useUpdateFlagsMutation` | P1 | 乐观更新不完整 |
| `useSyncMutation` | P1 | 不更新 syncStatus |
| `useSendEmailMutation` | P1 | 无 onError |
| `command.store.ts:53` | P1 | registerCommands 替换而非合并 |
| `query-client.ts` | P2 | 无全局错误处理 |
| CSS `fade-in-item` | P3 | 动画延迟只到第 10 子元素 |

### 7.13 构建配置

| 问题 | 级别 |
|------|------|
| TLS 后端冲突 (native-tls + rustls) | P1 |
| 缺少 `[profile.release]` 优化 | P1 |
| `tokio features = ["full"]` 过宽 | P2 |
| `rand = "0.8"` 过旧,用于加密 | P1 |
| `lettre` 在 src-tauri 可能无用 | P2 |
| `async-trait` 未纳入 workspace | P3 |
| `keyring` 仅 windows-native | P3 |
| 安全依赖版本约束过宽 | P3 |

---

## 八、优先修复路线图

### Phase 1: 紧急安全修复 (立即)

1. **修复路径穿越漏洞** (`sync.rs:299`, `attachments.rs:33`, `AttachmentList.tsx:60`)
   - 附件文件名: 移除路径分隔符 (`/`, `\`, `..`),限制长度
   - `download_attachment`: 校验 `save_to` 在用户下载目录内
2. **添加前端 HTML sanitization** -- 在 `dangerouslySetInnerHTML` 前用 DOMPurify 处理
3. **修复 ComposeView XSS** -- 对 `from_name` 等用户数据做 HTML 转义

### Phase 2: 功能修复 (1-2 周)

4. **修复 GenericApi 翻译** (`generic.rs:19-23`) -- 改用 `serde_json::Map` 手动构建
5. **修复 IMAP UID 回退** -- 跳过无 UID 的消息而非使用序列号
6. **修复前端状态管理** -- 统一到 React Query,移除 Zustand Legacy fetch
7. **修复 toggle-star** (`useKeyboard.ts`) -- 使用 mutation hook 或更新 query 缓存
8. **修复 kanban reorderInColumn** -- 批量 API 或 Promise.all 后统一回滚
9. **修复事务管理** (`messages.rs`) -- COMMIT 失败后 ROLLBACK
10. **添加 schema 版本跟踪** (`migrations.rs`) -- 使用 `PRAGMA user_version`

### Phase 3: 完善功能 (2-4 周)

11. 实现 Archive/Trash 功能或移除按钮
12. 实现 `move_message` / `sync_changes`
13. 补全事件发射 (sync progress, new mail)
14. 完成所有组件 i18n 国际化
15. 实现连接重试 (指数退避)
16. 为破坏性操作添加确认对话框

### Phase 4: 质量提升 (持续)

17. 补充后端单元测试 (尤其是 crypto, mail parser, search)
18. 统一 TLS 后端
19. 添加 `[profile.release]` 优化
20. 将内联样式迁移到 CSS 类
21. 添加 ESLint/Prettier 配置
22. 清理所有死代码

---

> 本报告由 6 个并行审查代理对项目全部源代码逐行审查后生成。
> 每个问题均标注了具体文件和行号,可直接定位修复。
