# Pebble 前端严格审查报告

审查范围：
- `d:\project\Pebble\src`
- `d:\project\Pebble\tests`（前端相关）
- `d:\project\Pebble\package.json`

验证结果：
- `pnpm build:frontend`：通过
- `pnpm test`：通过，4 个测试文件，22 个测试通过

## P1

- 未完成功能：OAuth 账号接入入口仍被前端直接隐藏，代码里有明确 TODO，当前界面没有可用的 OAuth 登录入口。证据：`d:\project\Pebble\src\features\settings\AccountsTab.tsx:175`、`d:\project\Pebble\src\features\settings\AccountsTab.tsx:176`
- 明确 bug：回复全部的收件人列表在 `useState` 初始化时用到了异步账号数据推导出的 `myEmail`，但后续账号数据到达时不会重算 `to/cc`，会把当前账号地址错误保留在回复对象里。证据：`d:\project\Pebble\src\features\compose\ComposeView.tsx:30`、`d:\project\Pebble\src\features\compose\ComposeView.tsx:34`、`d:\project\Pebble\src\features\compose\ComposeView.tsx:38`、`d:\project\Pebble\src\features\compose\ComposeView.tsx:48`
- 明确 bug：删除账号后，前端只刷新账号查询，不清理 store 中的 `activeAccountId`；侧边栏只会在 `!activeAccountId` 时自动补选首个账号，因此当前激活账号一旦被删，前端可能继续持有失效账号 ID。证据：`d:\project\Pebble\src\features\settings\AccountsTab.tsx:21`、`d:\project\Pebble\src\features\settings\AccountsTab.tsx:22`、`d:\project\Pebble\src\components\Sidebar.tsx:62`、`d:\project\Pebble\src\components\Sidebar.tsx:64`、`d:\project\Pebble\src\stores\mail.store.ts:62`
- 明确 bug：消息详情页会把未读邮件标记为已读，但没有同步更新列表侧状态；同文件里的星标操作也只更新详情本地 state。列表侧唯一的星标回写逻辑只存在于 `InboxView` 的列表交互里，因此从详情页进入的读状态和星标状态会与列表脱节，直到下一次刷新。证据：`d:\project\Pebble\src\components\MessageDetail.tsx:77`、`d:\project\Pebble\src\components\MessageDetail.tsx:78`、`d:\project\Pebble\src\components\MessageDetail.tsx:293`、`d:\project\Pebble\src\components\MessageDetail.tsx:294`、`d:\project\Pebble\src\features\inbox\InboxView.tsx:120`、`d:\project\Pebble\src\components\MessageItem.tsx:32`
- 未完成功能：消息详情页的归档、删除按钮仍是禁用占位，动作函数为空。证据：`d:\project\Pebble\src\components\MessageDetail.tsx:299`、`d:\project\Pebble\src\components\MessageDetail.tsx:301`、`d:\project\Pebble\src\components\MessageDetail.tsx:305`、`d:\project\Pebble\src\components\MessageDetail.tsx:307`
- 功能不足 / 契约风险：同步状态条把 `startSync` 的返回当成“同步完成”，成功后立刻把 `folders/messages/threads` 失效并把状态切回 `idle`，但前端没有消费任何同步进度或完成事件。当前前端代码只能证明它没有和异步同步流程建立闭环，容易出现“UI 显示已完成但实际仍在后台同步”的状态漂移。证据：`d:\project\Pebble\src\components\StatusBar.tsx:48`、`d:\project\Pebble\src\components\StatusBar.tsx:56`、`d:\project\Pebble\src\components\StatusBar.tsx:57`、`d:\project\Pebble\src\hooks\mutations\useSyncMutation.ts:8`、`d:\project\Pebble\src\hooks\mutations\useSyncMutation.ts:9`
- 功能不足：WebDAV 恢复成功后只显示成功提示，没有任何查询失效、store 刷新或页面重载逻辑；恢复后的前端状态不会自动和磁盘/后端恢复结果对齐。证据：`d:\project\Pebble\src\features\settings\CloudSyncTab.tsx:98`、`d:\project\Pebble\src\features\settings\CloudSyncTab.tsx:104`、`d:\project\Pebble\src\features\settings\CloudSyncTab.tsx:105`
- 功能不足 / 契约风险：附件下载成功后，前端没有使用自己刚计算出的 `savePath`，而是再次调用 `getAttachmentPath` 回读路径并写入 UI。这让前端成功态依赖一个额外的 IPC 返回契约；如果命令返回的是原缓存路径而不是用户刚选的保存路径，界面会展示错误结果。证据：`d:\project\Pebble\src\components\AttachmentList.tsx:66`、`d:\project\Pebble\src\components\AttachmentList.tsx:67`、`d:\project\Pebble\src\components\AttachmentList.tsx:68`

## P2

- 设计不足：命令面板文案全部硬编码英文，且命令只在布局首次挂载时注册一次；切换语言时，命令名称与分类不会更新。证据：`d:\project\Pebble\src\features\command-palette\commands.ts:13`、`d:\project\Pebble\src\features\command-palette\commands.ts:15`、`d:\project\Pebble\src\app\Layout.tsx:26`、`d:\project\Pebble\src\app\Layout.tsx:27`、`d:\project\Pebble\src\stores\ui.store.ts:51`、`d:\project\Pebble\src\stores\ui.store.ts:54`
- 设计不足：翻译弹层默认目标语言被硬编码成 `"zh"`，下拉选项文案也硬编码为英文；这和设置页已有语言状态没有建立任何关联。证据：`d:\project\Pebble\src\features\translate\TranslatePopover.tsx:18`、`d:\project\Pebble\src\features\translate\TranslatePopover.tsx:24`、`d:\project\Pebble\src\features\translate\TranslatePopover.tsx:77`
- 明确 bug：双语翻译入口只提交 `message.body_text`，当邮件主要内容只存在 HTML 正文时，这里会把空字符串送去翻译。证据：`d:\project\Pebble\src\components\MessageDetail.tsx:149`、`d:\project\Pebble\src\components\MessageDetail.tsx:150`
- 设计不足：富文本邮件直接使用 `dangerouslySetInnerHTML` 注入到主文档中，虽然做了 DOMPurify 清洗，但没有 iframe/sandbox 隔离；线程气泡里也是同一路径。这在邮件客户端场景下仍属于偏弱的渲染隔离设计。证据：`d:\project\Pebble\src\components\MessageDetail.tsx:379`、`d:\project\Pebble\src\components\MessageDetail.tsx:380`、`d:\project\Pebble\src\components\ThreadMessageBubble.tsx:68`、`d:\project\Pebble\src\components\ThreadMessageBubble.tsx:70`
- 可访问性问题：多个可点击列表项使用 `div` 承载点击行为，但没有按钮语义、键盘事件或焦点管理，键盘用户无法等价操作。证据：`d:\project\Pebble\src\components\MessageItem.tsx:35`、`d:\project\Pebble\src\components\ThreadItem.tsx:32`、`d:\project\Pebble\src\features\search\SearchResultItem.tsx:16`、`d:\project\Pebble\src\features\snoozed\SnoozedView.tsx:103`
- 设计不足：重试队列的待处理数直接从普通类实例读取，不是 React state，也没有订阅机制；队列长度变化本身不会驱动状态栏重渲染，待处理计数展示不可靠。证据：`d:\project\Pebble\src\components\StatusBar.tsx:70`、`d:\project\Pebble\src\components\StatusBar.tsx:126`
- 明确 bug：设置页语言列表里的中文标签已经乱码。证据：`d:\project\Pebble\src\features\settings\AppearanceTab.tsx:16`
- 明确 bug：新增账号按钮的 loading 文案 fallback 已经乱码。证据：`d:\project\Pebble\src\components\AccountSetup.tsx:407`
- 明确 bug：Kanban 卡片移除按钮显示成无意义字符 `脳`。证据：`d:\project\Pebble\src\features\kanban\KanbanCard.tsx:63`

## 残余风险 / 未覆盖点

- 当前前端测试仅覆盖 store：`d:\project\Pebble\tests\stores\ui.store.test.ts`、`d:\project\Pebble\tests\stores\mail.store.test.ts`、`d:\project\Pebble\tests\stores\kanban.store.test.ts`、`d:\project\Pebble\tests\stores\command.store.test.ts`。没有组件级、交互级、可访问性或 IPC 集成测试，所以本报告对渲染细节、事件订阅闭环和跨进程契约的判断只能基于源码路径，不能替代端到端验证。
- 本次没有深入审后端实现；凡是报告中标为“契约风险”的条目，都只基于前端当前假设和调用方式给出风险判断，没有把后端实际返回值当作既定事实。
