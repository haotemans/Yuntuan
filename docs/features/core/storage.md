# SQLite 数据与事件追踪

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> SQLx 访问、数据迁移、持久化业务状态和事件 Trace。

## 实际行为

`RuntimeFailure` 在现有 `events` 表保存会话类型、会话 ID、原消息编号、阶段及固定错误码，供[运行自检](diagnostics.md)关联调用者自己的请求，沿用 Tracer 观测和轮转规则。

- SQLite/WAL 保存消息、记忆、任务、人格、表情、关系等状态；核心层提供迁移与访问封装。
- `EventBus` 是进程内 broadcast，Tracer 将观测事件写入 `events`；它不是任务的持久化队列或消息送达保证。
- OneBot 戳一戳作为 `QqPoked` 留在事件追踪。收到通知时无需创建聊天消息、人物档案或消息锚点，成功发出的文本按现有回复规则保存。
- 事务边界、数据库迁移和异步取消处理以代码为准；不直接把早期 V0.1 数据模型当当前表结构。

数据库 V9 增加图片来源和观察状态，保存原消息关联；旧消息没有地址时不补造。图库审核的租约、重试次数、每日预算和结论按图片 ID/MD5 写入 state_kv，不新增图库业务表；人工删除同步清理对应结论。

V10新增 `membership_requests` 持久申请队列及待审索引，保留账号、类型、群、申请人、原始flag、理由、阶段状态、评估依据、次数和时间。已有消息、画像、群聊记忆与摘要不改写；唯一键、原子领取和条件状态转换限制重复执行。事件只提供观察，持久处理状态以该表为准，详见[主理人](../extensions/plugins/moderator.md)。

V11仅追加增量游标/批次、事实、会话日记、画像版本和相关索引/触发器。迁移保留旧消息、记忆、摘要、画像及state_kv，现存画像成为可追溯基线，后续当前值与历史记录在一个SQLite语句事务内更新。新归纳结果和游标一起提交，失败可续处理；旧数据保留并标记兼容边界，不伪装成已经逐条提炼。业务细节见[记忆功能](memory.md)，数据布局见[数据模型](../../reference/data-model.md)。

## 边界

- 需要判断任务是否真正完成应读取 `tasks/task_events`，不能只看观测流。
- 参见[数据基线](../../reference/data-model.md)和[SQLx 访问](../../reference/sqlite-access.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/db.rs`](../../../crates/core/src/db.rs)
- [`crates/core/src/db/access.rs`](../../../crates/core/src/db/access.rs)
- [`crates/core/src/db/platform_v21.sql`](../../../crates/core/src/db/platform_v21.sql)
- [`crates/core/src/db/qq_memory_v22.sql`](../../../crates/core/src/db/qq_memory_v22.sql)
- [`crates/core/src/event.rs`](../../../crates/core/src/event.rs)

## 验证入口

- [`crates/core/tests/sqlx_storage.rs`](../../../crates/core/tests/sqlx_storage.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

V21 增加 `platform_messages`，按平台、账号与外部消息 ID 唯一关联消息表内的锚点。回复序号随锚点持久化；现有 OneBot 消息继续使用旧字段。平台收发规则见 [QQ 接入](../adapter-qq/qq.md)。

V22 追加 `qq_memory_links`、只读 `qq_memory_aliases` 及 `qq_memory_directory` 视图。应用启动时从明确配置的 OpenID、QQ 号原子更新关联，并使记忆缓存失效。目录只归并读取时的卡片键，原消息、归纳证据、人物和通道回执原样保留。

V11迁移可幂等重放，已有处理游标和画像版本不被重置或重复插入；数据库版本断言与当前V11保持一致。

V12采用追加迁移：memory_items、task_runtime、task_invocations、task_outbox、capability_catalog、capability_invocations；llm_usage新增可空cache_hit_tokens/cache_miss_tokens。旧数据保留，迁移与新增列均幂等。SQL定义见 crates/core/src/db/runtime_v12.sql。

材料缓存使用SQLite `memory_material_epoch` 与画像/记忆/原文更改触发器实现失效；缓存自身不写入历史或改变原文。

V13追加消息 `sender_bot`、`decision_state`、`decision_attempts`、`decision_retry_at` 及待判断索引；已有processed_at和原文不变。pending→deciding→executing→done分开记录，模型失败回pending并退避；重启的executing转uncertain而非重复执行。已有完成消息迁移为done，原始群聊/私聊记忆、凭据和配置均不清理。SQL定义见 [`crates/core/src/db/decision_v13.sql`](../../../crates/core/src/db/decision_v13.sql)，最新迁移版本为V13。

DecisionMade观察事件追加入口类型、会话类型、实际作者名、锚点与批次成员编号；不改变事件表结构，不以Trace代替messages中的持久处理状态。

V14给messages追加可空 `decision_error`，保留原始数据、已有失败次数、配置和记忆。`failed`表示停止自动判断，与执行结果未知的`uncertain`区分。队列终止可回写processed_at，但不能解释为模型已回复；状态与原因是事实来源。最新迁移与数据库回归断言为14。

V15给message_images追加可空content_hash与meme_id，以及摘要索引；image_corrections按图片原消息、图片序号、纠正原消息唯一保存描述和逐字依据，外键随对应原消息删除。保留全部原始观察、旧消息、群记忆和配置；原始表不清空、不回填未知图片。迁移重放使用现有加列检查及IF NOT EXISTS。SQL为[`vision_v15.sql`](../../../crates/core/src/db/vision_v15.sql)，当时数据库版本为15，最新版本见下文。详细行为见[记忆功能](memory.md)。

V14→V15迁移夹具使用独立消息编号，保留已有V3种子原文；重放加列后仍保留原始观察及纠正行。

V16新增版本化working_context_versions及作用域索引，SQL见[`working_v16.sql`](../../../crates/core/src/db/working_v16.sql)。只新增表，不清空原消息、人物画像、群记忆、任务与配置；迁移重放幂等。该阶段数据库版本为16，来源、所有者及版本写入由Runtime事务校验；详见[工作记忆](memory.md)。

V17新增task_brief_versions及原消息/动作唯一索引，SQL见[`brief_v17.sql`](../../../crates/core/src/db/brief_v17.sql)。任务说明与任务控制在同一写事务提交，旧消息、任务、回执、画像及群记忆均不重写；迁移可重放。该阶段数据库版本为17；V16升级和旧任务不虚构来源的回归见[任务功能](tasks.md)。


V18仅追加tool_contracts和tool_invocation_records，SQL见[`tools_v18.sql`](../../../crates/core/src/db/tools_v18.sql)。前者保存当前注册工具契约；后者关联task_invocations、任务版本和call_id，保存所用协议、当次契约快照、结果状态、错误及耗时，(task_id,revision,call_id)唯一。原task_invocations结果平铺形状不变，unknown继续标旧started以阻止恢复重放；完整输出与模型有界预览分离。保留消息、画像、群记忆、任务说明、配置和旧回执。最新数据库版本18，迁移保留/幂等及调用事务回归见[任务功能](tasks.md#v18-统一工具执行接口)。


V19追加`portrait_coverage`与`portrait_runs`，SQL见[`portrait_v19.sql`](../../../crates/core/src/db/portrait_v19.sql)。精确消息覆盖关联已有批次，运行诊断保存实际模型和安全错误类别；原消息、旧游标/baseline、人物版本、群记忆、摘要、配置均保留。旧数据不虚构覆盖记录，迁移可重放，最新数据库版本19。V18升级、保留和回滚回归见[`profile_refresh.rs`](../../../crates/core/tests/profile_refresh.rs)，行为详见[记忆功能](memory.md#v19近期增量历史补齐与整理诊断)。


V20 新增 SQLite 表 \`scheduled_tools\` 与待执行索引，记录工具、参数、每日本地时刻、下一次执行、租约和最近结果。迁移仅追加表与索引，保留既有任务、消息和插件配置。SQL见[\`scheduler_v20.sql\`](../../../crates/core/src/db/scheduler_v20.sql)，读写和恢复边界见[调度功能](scheduler.md)；最新数据库版本为20。

## V23 每日 Agent 计划

[迁移 SQL](../../../crates/core/src/db/scheduled_agents_v23.sql) 追加计划与运行关联表，当前数据库版本23。原 scheduled_tools、任务、人物/消息/记忆及插件配置不改写。删除计划仅使历史的 schedule_id 置空，任务和工具回执保留。创建任务与触发收据/下一次时间使用同一写事务，失败整体回滚。规则见[定时任务](scheduler.md)。

[V0.24 迁移 SQL](../../../crates/core/src/db/memory_identity_v24.sql) 新增平台无关的 `memory_identity_links`、`memory_aliases` 与 `memory_directory`，自动回填既有 `qq_memory_links` 并通过触发器保持向后兼容，当前数据库版本24。
