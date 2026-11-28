# Smart notification evaluation

Synthetic data: 78 triage cases, 38 deadline cases, 24 awaiting-reply cases (zh and en). Mode: Smart.

## Notification triage

Expected is what a careful person would want. **Missed** = deserved a notification and didn't get one; **Noisy** = notified although it could wait; **Lost** = mail that should reach you went silent (must be 0). 41 of 78 messages are ones the rules can't place and would go to the model (app defaults: newsletters and automated mail stay with the rules).

| | Accuracy | Notify precision | Notify recall | F1 | Missed | Noisy | Lost |
|---|---|---|---|---|---|---|---|
| Rules only (shipped default) | 65% (51/78) | 67% | 46% | 0.54 | 19 | 8 | 0 |

### By category

| Category | Cases | Rules correct |
|---|---|---|
| bulk_noise | 12 | 12/12 |
| bulk_urgent | 8 | 0/8 |
| cc | 5 | 3/5 |
| injection | 3 | 3/3 |
| known_ask | 8 | 8/8 |
| known_fyi | 8 | 0/8 |
| rule_interrupt | 8 | 8/8 |
| rule_silent | 8 | 8/8 |
| unknown_ask | 9 | 0/9 |
| unknown_cold | 9 | 9/9 |

### Disagreements with the expected tier

| Case | Category | Mail | Expected | Rules |
|---|---|---|---|---|
| n029 | bulk_urgent | Account Security: New sign-in from an unknown device | interrupt | digest |
| n030 | bulk_urgent | 支付安全中心: 异地登录提醒 | interrupt | digest |
| n031 | bulk_urgent | Billing: Payment failed: service suspends tonight | interrupt | digest |
| n032 | bulk_urgent | 监控告警: 【严重】生产数据库不可用 | interrupt | digest |
| n033 | bulk_urgent | Air Example: Gate change: AE 123 now departs from B42 | interrupt | digest |
| n034 | bulk_urgent | 快递: 需本人签收：快递员 15 分钟后到达 | interrupt | digest |
| n035 | bulk_urgent | Login: Your sign-in code | interrupt | digest |
| n036 | bulk_urgent | 学校通知: 今天下午停课通知 | interrupt | digest |
| n045 | known_fyi | Priya: Thanks! | digest | interrupt |
| n046 | known_fyi | 李雷: 会议纪要 | digest | interrupt |
| n047 | known_fyi | Tom: Article you might like | digest | interrupt |
| n048 | known_fyi | 赵敏: 收到 | digest | interrupt |
| n049 | known_fyi | Kai: Out next week | digest | interrupt |
| n050 | known_fyi | 王总: 节日快乐 | digest | interrupt |
| n051 | known_fyi | Jo: Photos from the trip | digest | interrupt |
| n052 | known_fyi | 孙经理: 月度报告 | digest | interrupt |
| n053 | unknown_ask | Lena (BigCo): Interview tomorrow at 10? | interrupt | digest |
| n054 | unknown_ask | 新公司 HR: 面试时间确认 | interrupt | digest |
| n055 | unknown_ask | Building Management: Water shut-off in your unit at 2pm today | interrupt | digest |
| n056 | unknown_ask | 物业服务中心: 今天下午停水通知 | interrupt | digest |
| n057 | unknown_ask | City Clinic: Your test results | interrupt | digest |
| n058 | unknown_ask | 班主任: 孩子发烧了 | interrupt | digest |
| n059 | unknown_ask | Rita, Acme Ops: Production issue with your integration | interrupt | digest |
| n060 | unknown_ask | 合作方法务: 合同今天截止签署 | interrupt | digest |
| n062 | unknown_ask | 会议组委会: 您的演讲时间调整 | interrupt | digest |
| n073 | cc | Kai: Release plan | interrupt | digest |
| n074 | cc | 赵敏: 上线安排 | interrupt | digest |

## Deadlines in mail

Mail received 2026-12-02 10:00, from a colleague you write to (so it would reach the model with AI help on). A deadline on the right day counts as found; the app reminds ahead of it. The model's deadlines are filtered as in the app, and used only when the rules find none ("Rules, then model").

| | Exact | Same day | Wrong day | Missed | False positive | Correct none | Day recall | Day precision |
|---|---|---|---|---|---|---|---|---|
| Rules (shipped) | 18 | 0 | 0 | 9 | 0 | 11 | 67% | 100% |

| Case | Text | Expected | Rules |
|---|---|---|---|
| d003 | Numbers Need this EOD please. | 12-02 18:00 | — (missed) |
| d009 | Workshop Registration closes on Dec 8. | 12-08 18:00 | — (missed) |
| d010 | Can you get back to me within 48 hours? | 12-04 10:00 | — (missed) |
| d015 | Please confirm no later than 5pm today. | 12-02 17:00 | — (missed) |
| d018 | Could you review it before our meeting on Thursd… | 12-03 14:00 | — (missed) |
| d030 | 活动 报名将于12月6日结束 | 12-06 18:00 | — (missed) |
| d031 | 48小时内回复有效 | 12-04 10:00 | — (missed) |
| d033 | 这周内给我答复就行 | 12-04 18:00 | — (missed) |
| d034 | 合同 下午三点前把合同发我 | 12-02 15:00 | — (missed) |

## Waiting for your reply (rules)

Precision 86% (12 of 14 nudges wanted), recall 92% (12 of 13 found).

| Case | Text | Expected | Score | Note |
|---|---|---|---|---|
| w007 | Contract Please sign and return the contract. | nudge | 0.00 | explicit request |
| w015 | Team update Team, any questions let me know. | no nudge | 0.67 | cc, generic |
| w019 | Survey Would you take 5 minutes for our survey? | no nudge | 0.59 | survey |
