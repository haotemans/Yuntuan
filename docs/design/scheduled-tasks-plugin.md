# 定时任务插件（scheduler）设计与验收契约

**状态：设计阶段，尚无注册到 Yuntuan 的 scheduler 运行时。** 用户目标：在 QQ 私聊/群聊内自然语言设置“什么时候执行什么任务”，由云团在未来的指定时间启动真实 Agent/Tool 工作流，并把结果可靠回送。

## 对话示例

- `明天 09:00（北京时间）检查仓库最近的 PR 并私聊发给我`
- `每周一 09:00（Asia/Shanghai）总结本周 GitHub 更新`
- `每天 21:30 把今天指定群的讨论摘要写入我授权的腾讯文档`
- `每晚 02:00 把许可目录中新文件备份到夸克网盘，完成后私聊告诉我`

**时间规则**：优先解析为明确 IANA 时区（例如 `Asia/Shanghai`）、明确的本地时间与下一次 UTC 时间；时区缺失时按管理员可见的会话默认设置并在创建回执中复述，不从服务器系统时区猜。DST 歧义和不合法时间必须澄清；编辑计划时重新计算下次运行时间。

## 能力与用户命令

| 操作 | 入口 | 说明 |
| --- | --- | --- |
| 创建一次性任务 | `scheduler.create` / `/定时 创建` | 精确日期时间 + 任务目标 |
| 创建周期任务 | `scheduler.create` | 每天/每周，精确时区和时间；Cron 作为后续扩展 |
| 查看 | `scheduler.list` / `/定时 列表` | 仅能查看自己的或管理员授权的计划 |
| 调整 | `scheduler.update` | 修改时间/目标/执行权限后重新确认 |
| 暂停/继续 | `scheduler.pause` / `scheduler.resume` | 持久化状态 |
| 取消 | `scheduler.cancel` | 阻止未来触发，不假称撤销已开始外部动作 |
| 查看运行记录 | `scheduler.history` | 每次预定时间、实际启动、Task ID、工具回执、投递状态 |

第一版仅一次、每天、每周三种时间表达；`cron`、复杂节假日规则、执行依赖树可留后续。

## 数据模型（建议 SQLite）

```text
scheduled_jobs:
  job_id PK, owner_pid, chat_id, chat_type, goal,
  schedule_kind, schedule_spec_json, timezone,
  next_run_at_utc, state, authorized_tools_json,
  approval_policy_json, deliver_to_json,
  max_runs, completed_runs, created_at, updated_at, revision

scheduled_runs:
  run_id PK, job_id FK, scheduled_for_utc, task_id,
  state, created_at, started_at, finished_at, delivery_state,
  error_code, UNIQUE(job_id, scheduled_for_utc)
```

约束：`goal` 非空、限制长度；每位用户和会话限制活跃计划数；最短周期至少五分钟；运行需遵守 Agent 现有预算/并发；`scheduled_runs` 必须持久可查，数据库迁移可回滚且不可丢既有任务/人物记忆。

## 定时器与去重

1. 定时服务从 SQLite 按 `next_run_at_utc` 索引读取到期计划，事务领取 `(job_id, scheduled_for_utc)` 的唯一运行键。
2. **计划领取与创建 Agent 任务应使用同一数据库事务或显式幂等接口**。现有 `agent::create_scoped_task` 直接创建新 task_id，尚无计划运行幂等键；接入前必须扩展它，不能仅靠内存 HashSet 防重复。
3. 通过现有任务调度器创建真实任务，不由 timer 直接执行 Shell、MCP 或任意字符串命令。允许工具集以创建者持久授权为上限。
4. 异常重启：未开始执行的运行可依据事务状态安全恢复；有外部动作且回执未知则转 `uncertain` 等待核实，禁止自动重放付费或写操作。
5. 一次性任务过期太久标为需人工确认；周期任务默认跳过停机期间错过的旧时段，不集中补跑历史次数；每个新时段只产生一条运行记录。
6. 与 Agent 现有最多两个任务槽位共享预算；增加到期队列背压、超时和每账户请求频率限制。任务结束与 QQ 送达分别记账。

## 安全和归属

- 任务只由真人明确提出后建立，宿主核实原始消息 ID、真实发起者和群会话。不可仅凭网页、文档正文或他人引用中的一句话建任务。
- 创建者可管理自己的任务；群任务管理员可依据已验证的群权限管理；严禁跨会话访问私有任务。
- 普通只读摘要任务可按明确许可后台执行；文档写入、网盘备份等写权限必须绑定目的资源、账号和批准范围。删除/支付/分享给公众等高风险动作在每次运行前再次确认，不允许静默自动化。
- 授权过期标记 `needs_auth` 而非在定时点发起扫码登录；禁用 Skill / Tool 后历史定时任务转为 `blocked`。
- 结果通知使用独立、可确认的 outbox，不能伪造用户发言、重复引用早已过期的 QQ 消息锚点，适配器返回成功消息 ID 后才记为 `sent`。

## 工作流及交付格式

`Natural language → schedule preview → user confirm → SQLite → tick/claim → Agent task → Tool/Skill → task receipt → persistent outbox → QQ`

用户创建后返回：任务名称、下一次执行时间（含时区）、作用范围、使用工具、接收结果的位置、任务 ID 和暂停/取消入口。运行结果返回：计划 ID、执行时间、成功/失败/待确认、产物链接和可检查的 Task ID。

## 验收关卡

- 时间解析、跨日/跨月、DST、停机恢复和修改规则；
- 并发触发同一时段最多创建一次 Agent 任务；
- 重启/取消、工具结果未知、授权失效、重复发送；
- 非创建者不可改任务，群权限生效，不允许定时任务扩大插件作用域；
- SQLite 迁移保留现有 `tasks`、`messages`、`long_memories`；
- 模拟模型与 QQ 回执测试 + 真实 NapCat/第三方 API 单独验收。

后续实现应新增 `plugins/scheduler` Rust crate，并同步 `docs/features/extensions/plugins/scheduler.md`、`docs/features/catalog.json`、主 Cargo workspace、`crates/yuantuan` 构建清单和 WebUI 任务管理页；在全部验收前不得写成“已实现”。
