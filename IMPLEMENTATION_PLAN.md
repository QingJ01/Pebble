# Pebble 审查修复计��

**基于:** `docs/2026-04-17-comprehensive-seven-angle-review.md`
**创建:** 2026-04-17
**总问题数:** ~180 → 去重合并后 **72 个独立修复项**，分 8 个阶段

> 已完成：~~MAINT-09~~ `require()` 循环依赖 → 已改为 ESM import（2026-04-17）

---

## Stage 1: 紧急安全修复
**Goal**: 消除凭证泄露和数据��露风险
**Success Criteria**: 无明文凭证、无 panic 点、IPC 输入有边界
**Tests**: 手动验证 `.env` 无 secret、`cargo test`、前端 `pnpm test`
**Status**: Not Started

| # | 原始编号 | 问题 | 文件 | 工作量 |
|---|----------|------|------|--------|
| 1.2 | DA-02 / SEC-09 | Microsoft OAuth 迁移到 PKCE 公共客户端，移除 `MICROSOFT_CLIENT_SECRET` | `oauth.rs:78-80` | 2h |
| 1.3 | SEC-03 | `poll_interval_secs` 添加 `clamp(15, 3600)` 边界 | `sync_cmd.rs:17-24` | 0.5h |
| 1.4 | SEC-04 | 解密缓冲区使用 `Zeroizing<Vec<u8>>` 包装 | `compose.rs:125`, `accounts.rs:159`, `oauth.rs:193` | 1.5h |
| 1.5 | MAINT-10 | `.expect()` → `map_err()?` （TLS 版本配置） | `imap.rs:288`, `smtp.rs:305` | 0.5h |
| 1.6 | MAINT-11 | `.unwrap()` → `?` （PROPFIND 方法创建） | `cloud_sync.rs:122` | 0.5h |
| 1.7 | SEC-05 / BUG-06 | `sanitizeHtml` 添加 `FORBID_TAGS: ["style",...]` + CSS 属性白名单 hook | `sanitizeHtml.ts:16-21` | 1h |

**小计: ~6.5h**

---

## Stage 2: 高优先级 Bug 修复
**Goal**: 修复导致数据丢失或功能失效的 bug
**Success Criteria**: 批量操作正确区分离线/失败、断路器正常工作、草稿不丢失
**Tests**: 对每个 bug 编写回归测试
**Status**: Complete

| # | 原始编号 | 问题 | 文件 | 工作量 |
|---|----------|------|------|--------|
| 2.1 | BUG-01 | `batch_delete/mark_read/star` 区分离线 vs 远程失败（添加 `connection_attempted` 标志） | `batch.rs:212-221` | 1h |
| 2.2 | BUG-02 | 断路器 `is_circuit_open()` 分支后添加 `continue` | `sync.rs:807-860` | 0.5h |
| 2.3 | BUG-03 | `empty_trash` 循环分页替代硬编码 10,000 | `lifecycle.rs:345` | 1h |
| 2.4 | BUG-04 | 键盘归档失败后恢复 `selectedMessageId` | `useKeyboard.ts:149-165` | 0.5h |
| 2.5 | BUG-05 | 富文本模式保存草稿时提取纯文本 `bodyText` | `useComposeDraft.ts:140-141` | 0.5h |
| 2.6 | BUG-07 | sync task 完成后自动移除 `sync_handles` 条目 | `sync_cmd.rs:56-136` | 1h |
| 2.7 | BUG-08 | token 刷新返回实际 `expires_in` 而非硬编码 3600 | `gmail_sync.rs:196-198` | 1h |
| 2.8 | DA-05 | 每个迁移步骤包装在 `BEGIN/COMMIT` 事务中 | `migrations.rs` | 1h |
| 2.9 | DA-04 | SQLite/Tantivy 同步：用 pending 日志替代 10% 阈值 | `lib.rs:101-132`, `pebble-search/lib.rs` | 3h |
| 2.10 | SEC-07 / DA-08.3 | 翻译配置向后兼容路径：启动时自动重加密明文配置 | `translate.rs:25-33` | 1h |

**小计: ~10.5h**

---

## Stage 3: 搜索与同步性能
**Goal**: ���决索引膨胀、同步串行化、异步阻塞问题
**Success Criteria**: 索引体积缩小 >50%、Gmail 初始同��� <30s（200 消息）、无 Tokio 线程阻塞
**Tests**: 基准测试索引大小、同步耗时
**Status**: Complete

| # | 原始编号 | 问题 | 文件 | 工作量 |
|---|----------|------|------|--------|
| 3.1 | PERF-11 | `body_text` 改用 CJK 感知分词器，仅 `subject`/`from_*` 保留 n-gram | `schema.rs` | 2h |
| 3.2 | PERF-12 | 添加 `index_messages_batch` 批量索引方法 | `pebble-search/lib.rs` | 1.5h |
| 3.3 | PERF-06 | Gmail `sync_label` 改为 `buffer_unordered(10)` 并发获取 | `gmail_sync.rs` | 2h |
| 3.4 | PERF-05 / FEAT-03 | Gmail 增量同步使用 `get_thread_mappings_for_refs` 替代全量加载 | `gmail_sync.rs` | 1.5h |
| 3.5 | PERF-09 | `persist_message_attachments` 包装在 `spawn_blocking` | `sync.rs` | 1h |
| 3.6 | PERF-10 | `Store::with_read_async/with_write_async` 方法添加 | `pebble-store/lib.rs` | 2h |
| 3.7 | PERF-08 | IMAP `reconcile_folder` 仅在 `EXISTS` 计数变化时全量获取 UID | `sync.rs`, `imap.rs` | 2h |
| 3.8 | PERF-07 / DA-06 | IMAP 双连接（分离 IDLE 与主动操作） | `imap.rs`, `imap_provider.rs`, `sync.rs` | 4h |

**小计: ~16h**

---

## Stage 4: 功能完成度
**Goal**: 补全未完成的核心功能
**Success Criteria**: Gmail 文件夹移动远程同步、规则 MoveToFolder 生效、已加星标分页
**Tests**: 手动测试 Gmail 移动 + 同步回查、规则触发验证
**Status**: Not Started

| # | 原始编号 | 问题 | 文件 | 工作量 |
|---|----------|------|------|--------|
| 4.1 | FEAT-01 | Gmail `move_to_folder` 完整标签语义（移除源标签 + 添加目标标签） | `lifecycle.rs:282` | 4h |
| 4.2 | FEAT-02 | 规则引擎 `MoveToFolder` 添加 provider-aware 远程同步 | `indexing.rs:216-218` | 3h |
| 4.3 | FEAT-04 | `StarredView` 添加无限滚动分页（匹配 `useMessagesQuery` 模式） | `StarredView.tsx:34` | 2h |
| 4.4 | FEAT-05 | IMAP 草稿删除：连接失败时仍删除本地记录 | `drafts.rs:133-137` | 1h |
| 4.5 | DA-01 | DEK 导出/恢复流程（用户密码加密导出 + 首次运行提示） | `keystore.rs`, 新增 UI | 4h |
| 4.6 | DA-07 | 离线发件箱（SQLite `outbox` 表 + 网络恢复后自动发送） | `sync.rs`, 新增 `outbox.rs` | 6h |

**小计: ~20h**

---

## Stage 5: React 渲染性能
**Goal**: ��除不必要的重渲染，提升列表流畅度
**Success Criteria**: `MessageItem` memo 有效、批量选择不触发全列表重渲染
**Tests**: React DevTools Profiler 验证渲染次数
**Status**: Not Started

| # | 原始编号 | 问题 | 文件 | 工作量 |
|---|----------|------|------|--------|
| 5.1 | PERF-01 | `MessageList` 稳定 `onClick` 回调 + `EMPTY_LABELS` 常量 | `MessageList.tsx:202-207` | 1h |
| 5.2 | PERF-02 | `MessageList` 使用 `useShallow` 订阅 Zustand | `MessageList.tsx:36-45` | 0.5h |
| 5.3 | PERF-13 | `siblingFolderIds` IIFE → `useMemo` | `InboxView.tsx:43-47` | 0.5h |
| 5.4 | PERF-16 / PERF-17 | 消除 `getRenderedHtml` 双重调用 + DOMPurify 双重消毒 | `useMessageLoader.ts:14-58`, `ShadowDomEmail.tsx:17` | 1.5h |
| 5.5 | PERF-18 | `SidebarButton` 包装 `React.memo` | `Sidebar.tsx:344-401` | 0.5h |
| 5.6 | PERF-14 | `useSearchQuery` 添加 `staleTime: 30_000` | `useSearchQuery.ts:7-13` | 0.5h |
| 5.7 | PERF-15 | `useThreadsQuery` 转换为 `useInfiniteQuery` | `useThreadsQuery.ts:10-21` | 2h |

**小计: ~6.5h**

---

## Stage 6: 无障碍 + i18n + 用户反馈
**Goal**: 满足基本 a11y 标准，补全 i18n，增强用户操作反馈
**Success Criteria**: 所有模态框有焦点陷阱���所有交互元素有 ARIA 标签、零英文硬编码
**Tests**: 键盘导航手动��试、屏幕阅读器测试
**Status**: Not Started

| # | 原始编号 | 问题 | 文件 | 工作量 |
|---|----------|------|------|--------|
| 6.1 | UX-01 | `EditAccountModal` ��加焦点陷阱（复用 `AccountSetup` ��式） | `AccountsTab.tsx:341-360` | 1h |
| 6.2 | UX-02/03/04 | `RulesTab` 输入、`MessageItem` 复选框、`SnoozePopover` 添加 ARIA | 多文件 | 2h |
| 6.3 | UX-06~17 | 批量添加 `aria-hidden`/`aria-pressed`/`role`/`htmlFor` | 12 个文件 | 3h |
| 6.4 | FEAT-06 | 补充 14 个缺失 i18n 键（`en.json` + `zh.json`） | `locales/en.json`, `locales/zh.json` | 1h |
| 6.5 | UX-23/24/25 | 发送无主题警告、无效邮箱提示、规则空值验证 | `ComposeView`, `ContactAutocomplete`, `RulesTab` | 2h |
| 6.6 | UX-26/27/28 | 添加账户成功 toast、删除模板确认、看板移除撤销 | `AccountSetup`, `ComposeView`, `KanbanCard` | 2h |
| 6.7 | UX-29 | 生产环境隐藏错误堆栈跟踪（仅 DEV 模式显���） | `Layout.tsx:162-169` | 0.5h |
| 6.8 | UX-18/19/20 | 附件错误展示、规则保存 `role="alert"`、延后失败提示改进 | `AttachmentList`, `RulesTab`, `SnoozePopover` | 1h |

**小计: ~12.5h**

---

## Stage 7: 代码质量与重构
**Goal**: 消除重复、拆分大文件、增强类型安全
**Success Criteria**: 无 >500 行文件、无重复 `formatDate`、翻译配置��型化
**Tests**: 编译通过 + 现有测试通过 + 无功能回退
**Status**: Not Started

| # | 原始编号 | 问题 | 文件 | 工作量 |
|---|----------|------|------|--------|
| 7.1 | MAINT-01 | 提取 `src/lib/formatDate.ts` 共享工具 | 5 个文件 | 1h |
| 7.2 | MAINT-02 | 提取 `<ModalOverlay>` 可复用组件 | 4 个文件 | 1h |
| 7.3 | MAINT-03 | 提取 `invalidateMailCaches()` 工具函数 | `MessageItem`, `MessageActionToolbar` | 1h |
| 7.4 | MAINT-04 | `RulesTab.tsx` 拆分为 4 个文件 | `RulesTab.tsx` (733 行) | 2h |
| 7.5 | MAINT-05 | `ComposeView.tsx` 提取 `TemplateDropdown`/`SaveTemplateBar`/`LeaveConfirmDialog` | `ComposeView.tsx` (694 行) | 2h |
| 7.6 | MAINT-06 | `AccountSetup.tsx` 提取 `ImapSmtpFields` + `useFocusTrap` hook | `AccountSetup.tsx` (655 行) | 2h |
| 7.7 | MAINT-07 | `batch.rs` 提取通用 `batch_apply` 函数 | `batch.rs` (399 行) | 2h |
| 7.8 | MAINT-08 | 翻译配置改为类型化 IPC（TS 判别联合 + Rust enum） | `TranslateTab.tsx`, `translate.rs` | 3h |
| 7.9 | DA-08.1 | DEK 生成改用 `OsRng` 替代 `thread_rng()` | `aes.rs` | 0.5h |

**小计: ~14.5h**

---

## Stage 8: 测试补全 + 数据库优化
**Goal**: 覆盖安全关键路径测试、添加性能关键索引
**Success Criteria**: OAuth/批量操作有单元测试、线程查询有覆盖索引
**Tests**: `cargo test` + `pnpm test` 新增测试全部通过
**Status**: Not Started

| # | 原始编号 | 问题 | 文件 | 工作量 |
|---|----------|------|------|--------|
| 8.1 | MAINT-13 | OAuth PKCE ��态匹配、token 刷新/过期逻辑单元测试 | `oauth.rs` | 3h |
| 8.2 | MAINT-14 | `batch_delete/archive/mark_read/star` 离线回退逻辑测试 | `batch.rs` | 2h |
| 8.3 | MAINT-16 | `useComposeEditor.switchMode` 6 个分支测试 | `useComposeEditor.ts` | 1.5h |
| 8.4 | MAINT-17 | 翻译配置 JSON round-trip 测试（TS → IPC → Rust） | `TranslateTab`, `translate.rs` | 1h |
| 8.5 | MAINT-15 | `StarredView`/`SnoozedView` 改用 React Query + 测试 | `StarredView`, `SnoozedView` | 2h |
| 8.6 | PERF-03 | 添加 `idx_messages_deleted_thread` 覆盖索引（V6 迁移） | `migrations.rs` | 1h |
| 8.7 | PERF-04 | 添加 `idx_messages_account_deleted_date` 索引 | `migrations.rs` | 0.5h |
| 8.8 | PERF-03 补充 | 添加 `idx_messages_unread_account` 索引 | `migrations.rs` | 0.5h |

**小计: ~11.5h**

---

## 不纳入计划的项目

以下问题经评估后暂不实施，记录理由：

| 原始编号 | 问题 | 不实施理由 |
|----------|------|------------|
| SEC-02 | SQLCipher 加密数据库 | 破坏性变更，需要数据迁移策略，单独立项 |
| DA-03 | 翻译功能每次确认 | 影响用户体验流畅度，改为在设置中增加风险提示 |
| DA-06 | 10+ 账户写入队列 | 当前用户量不需要，等反馈再决定 |
| DA-09 | WebView 版本检查 | Tauri 团队负责，非应用层问题 |
| DA-12 | Cloud Sync 重命名 | 仅 UI 文案修改，优先级极低 |
| SEC-06 | 禁止 `http://` URL | 部分合法邮件含 http 链接，改为浏览器打开前警告 |

---

## 总览

| 阶段 | 主题 | 修复项 | 预估工时 | 依赖 |
|------|------|--------|----------|------|
| 1 | 紧急安全 | 7 | 6.5h | 无 |
| 2 | 高优 Bug | 10 | 10.5h | 无 |
| 3 | 搜索同步性能 | 8 | 16h | 无 |
| 4 | 功能完成 | 6 | 20h | Stage 2 (迁移事务) |
| 5 | React 渲染 | 7 | 6.5h | 无 |
| 6 | a11y + i18n | 8 | 12.5h | 无 |
| 7 | 代码重构 | 9 | 14.5h | Stage 5 (重构��先优化) |
| 8 | 测试 + 索引 | 8 | 11.5h | Stage 2, 7 |
| **合计** | | **63** | **~98h** | |

Stage 1-2 可并行（安全 + Bug），Stage 3/5/6 可并行（性能 + a11y），Stage 7-8 在前序完成后执行。
