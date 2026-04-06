# Pebble 多视角团队审查综合报告

**日期：** 2026-04-05  
**审查方式：** 7 位专项审查员并行独立审查，最终汇总  

---

## 综合评分总览

| 审查维度 | 审查员 | 评分 | 一句话总结 |
|----------|--------|------|-----------|
| 功能完成度 | Feature Reviewer | **62%** | UI 完成度高于功能闭环完成度，多处"有前端无后端"空壳 |
| 用户体验 | UX Reviewer | **5.8/10** | 基础框架在，但可访问性、错误反馈、表单体验严重不足 |
| 可维护性 | Code Quality Reviewer | **7.1/10** | TypeScript 类型安全优秀，但状态管理双轨并存、测试覆盖不足 |
| 安全合规 | Security Reviewer | **6.5/10** | 凭据加密机制良好，但 OAuth state 验证缺失、CSS 注入、SQLite 明文 |
| 性能优化 | Performance Reviewer | **5.8/10** | 虚拟滚动正确，但列表传输全量 body、单锁 SQLite、N+1 同步查询 |
| Bug 数量 | Bug Hunter | **11 个 Bug** | 3 个严重、4 个高危、3 个中等、1 个低危 |
| 战略风险 | Devil's Advocate | **3 大风险** | 功能广度陷阱、SQLite 单锁架构、OAuth 外部依赖 |

---

## 第一部分：最高优先级问题（跨审查员共识）

以下问题被 **3 个或以上审查员** 同时标记为高优先级：

### 1. SQLite 单连接 Mutex 是全局瓶颈

- **性能审查**：所有读写共享一把锁，同步写入时 UI 读取必须等待
- **Bug 猎人**：snooze_watcher 在 async 上下文执行同步阻塞 SQLite，阻塞 Tokio 线程
- **质疑者**：100 封邮件时无感，10000 封时严重卡顿，且发布后改动成本极高
- **可维护性审查**：确认架构问题，建议 r2d2-sqlite 连接池或读写分离

**共识修复方案**：在任何公开发布前，将 `Mutex<Connection>` 改为读写分离连接。

### 2. OAuth/Provider 主链路完全未打通

- **功能审查**：Gmail 完成度 20%，Outlook 10%，SyncWorker 硬编码只用 IMAP
- **安全审查**：OAuth state 参数未验证（CSRF 风险），PKCE verifier 不匹配
- **可维护性审查**：start_oauth_flow 和 complete_oauth_flow 之间 PKCE 状态丢失
- **质疑者**：OAuth Client ID 是占位符，Google/Microsoft 审核需 2-6 周

**共识修复方案**：要么立即让 Provider 工厂接入同步链路，要么明确标注当前只支持 IMAP 并从 UI 移除 OAuth 入口。

### 3. 列表查询传输完整邮件正文

- **性能审查**：MSG_SELECT 包含 body_text 和 body_html_raw，50 封邮件可能一次传输数十 MB
- **质疑者**：body_html_raw 无大小限制，Newsletter 邮件单封可达 500KB-2MB
- **安全审查**：body_html_raw 通过 IPC 明文传输到前端

**共识修复方案**：创建 MessageSummary 结构体，列表接口只返回摘要字段，正文按需加载。

### 4. HTML 邮件渲染安全不足

- **安全审查**：style 属性允许任意 CSS（CSS 追踪/注入），双语模式跳过 sanitize
- **功能审查**：规格要求 iframe sandbox 但未实现
- **UX 审查**：ErrorBoundary 暴露原始堆栈信息

**共识修复方案**：HTML 邮件必须在 `<iframe sandbox>` 中渲染，style 属性需 CSS 值过滤。

---

## 第二部分：各审查员独立发现的关键问题

### 功能完成度审查 — 空壳清单

| 功能 | 完成度 | 空壳表现 |
|------|--------|----------|
| Gmail/Outlook 同步 | 20%/10% | Provider 代码存在但 SyncWorker 不调用 |
| 规则 AddLabel/MoveToFolder | 有 UI | 执行时只打日志，不实际操作 |
| Snooze return_to | 有字段 | 唤醒事件不导航到目标位置 |
| 邮件发送附件 | 无 UI 无后端 | ComposeView 无文件选择，API 不接受附件 |
| 归档/删除 | 按钮 disabled | 无 move_to_folder Tauri command |

### UX 审查 — 最严重的体验问题

| 编号 | 严重度 | 问题 |
|------|--------|------|
| C-05 | Critical | CommandPalette 无 ARIA 角色、无焦点陷阱，完全不可访问 |
| C-02 | Critical | ErrorBoundary 暴露 error.stack 给用户 |
| M-14 | Major | RulesTab 要求手写 JSON 作为规则条件和动作 |
| M-09 | Major | MessageItem 悬停操作按钮对键盘用户不可达 |
| M-15 | Major | ComposeView 无发送确认、无草稿保存、无离开提示 |
| M-10 | Major | Compose 作为全屏视图替换主内容，无法同时参考邮件 |

### 安全审查 — Critical/High 问题

| 编号 | 严重度 | 问题 |
|------|--------|------|
| C-1 | Critical | OAuth 回调未验证 state 参数（CSRF） |
| C-2 | Critical | style 属性允许任意 CSS（跟踪像素/数据泄露） |
| C-3 | Critical | 双语翻译 HTML 未经 sanitize 注入 iframe |
| C-4 | Critical | SQLite 数据库无加密，邮件明文存储 |
| H-1 | High | SMTP `builder_dangerous` 允许无 TLS 明文连接 |
| H-2 | High | test_translate_connection 可作为 SSRF 代理 |
| H-5 | High | 附件路径遍历检查不完整 |

### 性能审查 — 最高影响问题

| 编号 | 影响 | 问题 |
|------|------|------|
| P0-1 | 最高 | 列表查询加载完整邮件正文（body_text + body_html_raw） |
| P0-2 | 严重 | 同步逐条检查消息是否存在（N+1 查询，200 封 = 400 次 DB） |
| P0-3 | 严重 | reconcile_folder 对每个文件夹全量 UID SEARCH ALL |
| P0-4 | 严重 | Store 使用单一 Mutex（全局写锁） |
| P1-5 | 中等 | list_threads_by_folder 子查询全表扫描，缺索引 |
| P1-6 | 中等 | MessageItem 每次渲染遍历 kanban.cards.some() |

**缺失索引**：
```sql
CREATE INDEX idx_message_folders_folder_id ON message_folders(folder_id);
CREATE INDEX idx_messages_account_starred ON messages(account_id, is_starred) WHERE is_starred = 1;
CREATE INDEX idx_messages_thread_date ON messages(thread_id, date);
```

### Bug 猎人 — 已确认 Bug 清单

| # | 严重度 | Bug 描述 |
|---|--------|---------|
| 1 | 严重 | `normalize_subject` Unicode 字节偏移可能 panic |
| 2 | 严重 | `has_message_by_remote_id` 不过滤软删除，已删邮件无法重新同步 |
| 3 | 严重 | snooze_watcher 在 async 中同步阻塞 SQLite，阻塞 Tokio 运行时 |
| 4 | 高 | Strict 模式 + 受信任发件人 = 邮件内容空白（React 批处理竞态） |
| 5 | 高 | `update_account` 无条件用 email 覆盖 IMAP/SMTP username |
| 6 | 高 | `empty_trash` purge 时间戳边界条件错误，物理删除不生效 |
| 7 | 高 | 线程参与者用逗号分割，display name 含逗号时错误切割 |
| 8 | 中 | Reply-All 用错账户 email 过滤收件人 |
| 9 | 中 | privacyMode useEffect 缺少 messageId 依赖（竞态） |
| 10 | 中 | EXPUNGE 误删邮箱中所有 `\Deleted` 消息 |
| 11 | 低 | folder_ids 单元素时回退用错误的 folder_id |

### 可维护性审查 — 架构债务

| 编号 | 影响 | 问题 |
|------|------|------|
| #13 | High | mail.store.ts 双轨状态（Legacy Zustand + React Query 并存） |
| #4 | Medium | connect_imap 认证逻辑在 3 处重复 |
| #5 | Medium | Gmail/Outlook Provider 结构镜像重复（1937 行） |
| #10 | High | OAuth PKCE 状态未持久化，start/complete 之间 verifier 不匹配 |
| #11 | Medium | MessageDetail.tsx 683 行，承担过多职责 |

---

## 第三部分：质疑者的战略建议

### 三大战略风险

1. **功能广度陷阱**：同时推进 14+ 功能，每个都只完成 60-80%，无法形成可用 MVP
2. **SQLite 单锁架构**：随数据量增长性能线性恶化，发布后修改成本极高（不可逆）
3. **OAuth 外部依赖**：Google/Microsoft 审核需 2-6 周，必须提前启动

### 最短可行路径

> **目标**：让一个用户能用 Pebble 作为唯一邮件客户端完成一周日常工作

1. **IMAP 完整闭环（2 周）**：修复 Archive/Delete、已读状态回写远端、SMTP 异步化
2. **存储层稳定化（1 周）**：读写分离连接 + body_html_raw 大小限制
3. **安全基线（1 周）**：iframe sandbox 隔离 + 路径遍历修复
4. **移除虚假功能（0.5 周）**：删除所有 disabled 按钮和占位符入口

---

## 第四部分：统一修复优先级

### P0 — 立即修复（阻断核心体验或存在安全漏洞）

| # | 来源 | 问题 | 工作量 |
|---|------|------|--------|
| 1 | 安全 | OAuth state 验证缺失 + PKCE 状态持久化 | 小 |
| 2 | 安全 | 双语模式 HTML 未 sanitize | 小 |
| 3 | Bug | normalize_subject Unicode panic | 小 |
| 4 | Bug | has_message_by_remote_id 不过滤软删除 | 小 |
| 5 | Bug | empty_trash purge 边界条件 | 小 |
| 6 | Bug | EXPUNGE 误删所有 \Deleted 消息 | 小 |
| 7 | Bug | update_account 覆盖 username | 小 |
| 8 | 性能 | 列表接口引入 MessageSummary，去除 body | 中 |

### P1 — 本周修复（功能闭环或重大体验问题）

| # | 来源 | 问题 | 工作量 |
|---|------|------|--------|
| 9 | 性能 | Mutex → 读写分离连接 | 中 |
| 10 | 安全 | style 属性 CSS 值过滤 | 中 |
| 11 | 安全 | HTML 邮件 iframe sandbox 隔离 | 中 |
| 12 | 安全 | SMTP 强制 TLS / 移除 builder_dangerous | 小 |
| 13 | 功能 | 归档/删除后端命令实现 | 中 |
| 14 | 功能 | 已读/星标状态回写远端 IMAP | 小 |
| 15 | Bug | snooze_watcher 移到 spawn_blocking | 小 |
| 16 | Bug | Reply-All 账户 email 过滤修复 | 小 |
| 17 | Bug | 线程参与者逗号分割修复 | 小 |
| 18 | 维护 | mail.store.ts 双轨迁移完成 | 中 |
| 19 | UX | CommandPalette ARIA + 焦点陷阱 | 中 |
| 20 | 性能 | 添加 3 个缺失数据库索引 | 小 |

### P2 — 本月修复（体验提升和技术债务）

| # | 来源 | 问题 | 工作量 |
|---|------|------|--------|
| 21 | 性能 | 同步批量检查替代逐条 has_message | 中 |
| 22 | 性能 | SearchSchema/IndexReader 缓存复用 | 小 |
| 23 | 性能 | MessageItem React.memo + kanban Set 查找 | 小 |
| 24 | 功能 | 规则引擎接入同步链路（AddLabel/MoveToFolder） | 中 |
| 25 | 功能 | Snooze return_to 回流导航 | 小 |
| 26 | 功能 | 发送邮件附件支持 | 大 |
| 27 | UX | RulesTab 可视化规则构建器 | 大 |
| 28 | UX | window.confirm → 自定义 ConfirmDialog | 小 |
| 29 | UX | ComposeView 草稿保存 + 离开确认 | 中 |
| 30 | 维护 | connect_imap 认证逻辑提取到 AppState | 小 |
| 31 | 安全 | SQLite 加密（sqlcipher） | 大 |
| 32 | 安全 | 附件路径遍历白名单 | 小 |

### P3 — 路线图（长期改进）

- Gmail/Outlook Provider 真正接入同步（取决于 OAuth 审核）
- ComposeView 懒加载 + 代码分割
- 翻译结果本地缓存
- 看板已完成列 7 天自动清理
- 搜索结果高亮
- 侧边栏未读计数
- Compose 改为浮动面板

---

## 总结

**项目当前状态：高保真 Alpha Demo**

核心矛盾是"界面完成度 >> 功能闭环完成度"。UI 层已经暗示或暴露了大量能力，但后端和数据流没有真正闭环。7 位审查员的共识是：

1. **立即启动功能冻结**，不再添加新功能
2. **聚焦 IMAP 纵向切片**，让一条路径从头到尾完美工作
3. **在公开发布前修复存储层架构**（读写分离），这是不可逆的技术债务
4. **安全基线不可妥协**：OAuth state 验证、iframe sandbox、CSS 过滤

达到"一个用户能用 Pebble 完成一周日常工作"的目标，预计需要 4-5 周的专注开发。
