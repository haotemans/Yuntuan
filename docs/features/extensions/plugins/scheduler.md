# 知时：定时 Agent 任务（Scheduler）

[返回插件索引](README.md) · [设计说明](../../../design/scheduled-tasks-plugin.md) · [运行时](../../runtime/runtime.md)

**当前状态：编译期插件代码已提交，尚未在 Rust 环境编译和完成真实 QQ/LLM 集成验收。** 默认关闭，管理员需在插件页启用。`plugins/scheduler` 不是独立运行进程；当前第一方实现位于 core，用构造器注入现有 Agent EventBus 与 QQ 发送接口。

## 当前实现范围

- 注册六个 Tool：`scheduler.create`、`scheduler.list`、`scheduler.pause`、`scheduler.resume`、`scheduler.cancel`、`scheduler.history`。通过当前 `ToolRegistry` 供 Agent 使用，**不含独立 /定时 命令及 WebUI 专属任务表页面**。
- 创建任务需要明确带时区的 `when`，格式例如 `2026-11-17T09:00:00+08:00`；支持一次性、固定 24h 与固定 7d UTC 周期。**固定秒数周期不是夏令时自动调整的当地时钟规则**。
- 持久化 `scheduled_jobs`、`scheduled_runs` 到原 SQLite（表在插件启动时以 `CREATE TABLE IF NOT EXISTS` 创建），不改现有聊天/记忆数据表。
- 后台 2 秒扫描到期任务，先持久领取，再调用 `agent::create_scoped_task` 将目标放进现有 Agent 执行队列；仍共享两个 Agent 执行槽。
- 执行不绑定过期 QQ 源消息，而是独立轮询 Agent 任务结果；结束、失败、需补充时，通过单独 QQ 私聊发送给真实创建者。仅检测到发送回执 `message_id` 后记为 `sent`；回执未知时标记 `unknown`，禁止自动重发。
- 只在当前已识别的 QQ 用户会话可操作；列表/修改仅限本人（群聊场景还限制原群）。结果默认私聊创建者，**不会在公共群广播**。
- 默认不授权任何 Tool；`use_search=true` 才允许 `anysearch.search` 和 `anysearch.extract` 两个只读搜索工具。其他插件（腾讯文档、夸克网盘、群管理、生图、Docker）**不能通过本版定时任务自动执行**。

## 失败与恢复边界

- 采用「先数据库领取，再创建 Agent 任务」的保守**至多一次启动**策略；进程恰好在两步间崩溃会留下 `uncertain`，不自动重跑以防重复副作用。因此不承诺任务必定恰好执行一次。
- 插件停用后定时器停止，之后启动保留 SQLite 记录；停机漏过的周期只触发最新到期点，不追补多次。
- 单次任务恢复过去时间时顺延至一分钟后；暂停/取消不能撤销已创建的 Agent 任务。失去 QQ 私聊能力可能使消息回执成为 `unknown`。
- 目前仅针对创建者拥有的轻量只读任务设计；自动跨账号文档写入、夸克备份、复杂 Cron、每次写入审批、故障重试、日历时区 DST 处理均需后续桥接。
- 模型必须把用户口头时间转换成明确 RFC3339 并展示确认；若来源时区不确定则应询问，不得静默使用服务器时区。

## 对应实现（本页负责的主源码）

- [`crates/core/src/scheduler.rs`](../../../../crates/core/src/scheduler.rs)

## 验证入口

- `scheduler.rs` 内的单元测试覆盖时区转换等价、非法日期拒绝和 QQ 身份校验。
- 尚待运行 `cargo test --locked -p yuantuan-core scheduler`、`cargo test --locked --workspace` 和真实 NapCat/LLM 测试；代码提交并非编译成功。
