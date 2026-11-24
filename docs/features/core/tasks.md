# Agent 任务与工具执行

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现持久执行与结果回聊（需真实环境验收）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 持久化任务队列、有限轮工具调用、失败终态和任务时间轴。

## 实际行为

任务文本结果按原会话的接入通道交付。QQ 官方机器人的字符串回执可确认发送，实际回复仍受官方被动回复有效期约束。详见 [QQ 接入](../adapter-qq/qq.md)。

- Decision 的 `start_task` 将目标持久化为 `tasks`，事件只负责唤醒 Runner；Runner 也扫描数据库等待任务。
- `agent_exec` 默认使用 JSON 动作协议，可显式选择原生 tool_calls；两者进入同一执行器调度工具；任务与事件流水在 WebUI「任务回放」查看。
- 当前默认同时最多 2 个任务、Router未指定时每根任务最多 10 轮、单工具超时 120 秒；解析与工具错误有终止阈值。
- 可选 Docker 开发沙箱默认关闭，仅对特定工具提供隔离，不代表所有插件和 MCP 在沙箱里执行。

## 边界

- 根任务完成/等待补充经Decision与Chat进入持久outbox；中间 `reply` 动作仍只写入任务事件。
- 重启后已开始但中断的可能有副作用任务不盲目重跑；无法宣称恰好一次执行。
- 参考[Agent 沙箱](../../reference/agent-sandbox.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/agent.rs`](../../../crates/core/src/agent.rs)
- [`crates/core/src/tools.rs`](../../../crates/core/src/tools.rs)
- [`crates/core/src/tools/sandbox.rs`](../../../crates/core/src/tools/sandbox.rs)
- [`crates/webui/src/tasks_api.rs`](../../../crates/webui/src/tasks_api.rs)
- [`webui-frontend/src/views/Tasks.vue`](../../../webui-frontend/src/views/Tasks.vue)

## 验证入口

- [`crates/core/tests/agent_loop.rs`](../../../crates/core/tests/agent_loop.rs)
- [`crates/core/tests/sandbox_docker.rs`](../../../crates/core/tests/sandbox_docker.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

## V12 可恢复执行

默认2个执行槽，父子任务共享。task_runtime 保存原始会话类型、发起人、锚点、版本、检查点、等待原因、累计预算和回复状态。Decision 可提供 TaskPlan；缺省10轮、300000保守token额度、900秒；运行时硬上限64轮、4000000累计额度、3600秒。额度按请求UTF-8字节加8192输出预留保守扣减，不冒充实际供应商token用量；子任务向根任务原子扣账，恢复不重置。

Agent支持 tool_call/reply/done/delegate/wait_input。每次拆分最多2个子目标、根任务最多8个子任务，不允许递归拆分；等待子任务不占槽，全部结束后唤醒父任务汇总。队列按发起人/会话分组轮转，组间优先早建任务。工具调用先落回执，成功结果和检查点同事务提交；同根任务跨版本/子任务的相同工具参数不重复调用，未知外部结果不自动重放。连续3轮无进展停止，原有解析失败/工具失败及超时限制仍有效。

pause/cancel/resume/amend 必须匹配原发起人、会话编号和类型；WebUI管理员入口单独受session认证。任务版本使迟到结果失效，修改/暂停会取消子任务，已经开始的外部动作无法保证撤销。已开始但无完成回执的工具在重启后终止并提示核实，安全检查不交给另一个模型循环。

根任务结果和等待补充事件经Decision再由Chat组织，生成正文先入task_outbox，实际发送状态与任务完成分开记录。ready可恢复，sending/unknown不自动重发；需要后续人工核实。仅收到适配层message_id记sent，不证明群成员已阅读。任务页显示阶段、原始调用回执状态、等待原因及暂停/继续/取消。该链路回传正文和当前版本成功回执的至多4个附件，图片用base64，视频沿用已有共享路径方式；总附件限30MiB，预检失败停止发送并标failed，不丢弃原产物。普通插件媒体投递继续走已有链路。

新增实现 [`runtime.rs`](../../../crates/core/src/agent/runtime.rs)、[`delivery.rs`](../../../crates/core/src/agent/delivery.rs)、[`runtime_api.rs`](../../../crates/webui/src/runtime_api.rs)，验证 [`context_runtime.rs`](../../../crates/core/tests/context_runtime.rs)。以上为实现契约及mock验证目标，不能代替真实QQ/模型验收。

长任务历史超过48KB且多于10项时进行确定性的阶段压缩：保留原目标、工具回执索引和最近8项记录，完整结果仍在SQLite；压缩本身不增加模型调用或算作任务进展。

持久outbox入发送队列前先标queued，避免忙碌会话重复积压；重启只恢复未开始发送的queued正文，sending保持待核实。模型等待与工具调用均受根任务剩余时间限制。任务UI操作与错误回显见 [`tasks-flow.spec.js`](../../../webui-frontend/tests/tasks-flow.spec.js)，纳入Linux浏览器CI。

任务已取消/完成/停止时不再展示继续控制按钮。新增存储与Agent回归先于全workspace测试运行，完整lib测试仍由workspace检查覆盖。

暂停/修改后恢复也会读取根任务已有回执，不能通过提升版本或改用子任务绕过外部动作去重；已完成结果先核对后复用。


## 后续设计边界

目标设计补充见 [ADR-0018](../../adr/0018-lightweight-agent-runtime.md)：Agent仅执行任务事件、Router交接任务说明、Chat对接用户，并统一工具回执、等待和恢复。该ADR为待实现目标，不把当前JSON循环或已有检查点等同于完整新架构。

本阶段Agent任务创建快照额外包含Router选中的working_context、dialogue、visual_context和execution_results；继续使用现有任务版本和原话授权检查。工作记忆本身不启动任务、不授予工具权限，也不证明某步已完成。V17进一步保存任务说明并承接运行中的补充，详见下文；ADR-0018的完整事件loop、统一文件/浏览器工具与任务档案尚未实现。

## V17 任务说明与当前任务语境

新增 [`crates/core/src/agent/brief.rs`](../../../crates/core/src/agent/brief.rs) 和task_brief_versions。每次建任务及控制操作与任务版本同事务保存说明；源消息核对作者、会话、原文及时间，任务创建核对原锚点。Router选中的工作事项从数据库重读，不接受调用方伪造条件。说明包含目标、原始要求、当前事项条件和逐条补充；原话超过4000字显式截取并保留原消息编号，每版说明限64KB，超限拒绝更新并回滚，不丢弃旧条件。已有task_runtime但没有说明的旧任务标legacy_task并保留目标/历史，不编造来源；更早、缺少会话类型的任务仍保留在管理列表，不自动带入聊天。

用户任务控制以可见revision检查并对同一原消息/动作去重；补充后新执行轮读取新说明，已成功或不确定的工具不因变更被重放，累计调用/预算/截止时间不重置。工作事项明确修改目标时同步tasks.goal，其他补充保留原目标。子任务继承父任务说明及版本；父任务控制后子任务版本失效。历史压缩不再承担保存有效要求的职责。wait_input同时保存提问、历史与等待事件，再释放执行槽。

Agent请求按task_brief在前、execution_history在后组织JSON；用户补充、工具回执和校验失败保持不同记录类型。工具目录每轮按实际注册、启用状态、会话范围与任务允许集合过滤并稳定排序；执行前仍校验参数与范围。参数无效不开始外部动作，以现有连续3轮无进展和根预算限制修正，不能无限重试。此处仍使用兼容JSON动作协议，未实现原生tool_call协议适配。

Router可用focus_task_id选择同一作者/同一会话的运行中或近30天已结束任务；Chat仅接收所选任务说明、状态和有界回执预览。旧版本回执标明matches_current_revision，不把已完成当作已交付。任务回聊不重放旧原消息的画像、工作记忆或图片纠正副作用。运行API返回brief及最近10版元数据，任务页显示条件、来源和补充，保持独立的执行/投递状态。管理页只读取当前说明，未增加任意SQL或扩大工具权限。

回归：[`crates/core/tests/task_brief.rs`](../../../crates/core/tests/task_brief.rs)覆盖来源/隔离、版本更新、压缩后恢复、预算保留、已结束任务检索、V16升级、真实mock Runner参数修正与工具可见集合；[`crates/webui/tests/pages_api.rs`](../../../crates/webui/tests/pages_api.rs)验证认证及API，现有tasks-flow验证当前要求与控制回显。mock不代表真实模型、浏览器或QQ文件闭环验收。

任务页刷新同时更新当前详情与版本，控制成功后复用同一刷新路径，避免列表更新而条件仍停留在旧版。

外部动作尚未确认时不推进amend；确认旧回执后，新的明确amend可应用先前已保存的同一事项目标，仍不重置预算或重放旧动作。

Router明确选中事项但材料缺失时，任务创建整体回滚，失败交给Chat说明，不能按缩水条件悄悄启动。


## V18 统一工具执行接口

[`executor.rs`](../../../crates/core/src/tools/executor.rs) 为已注册本地/插件/MCP Tool 提供一个任务执行边界；[`protocol.rs`](../../../crates/core/src/agent/protocol.rs) 将兼容JSON动作和原生Chat Completions函数调用归一成同一调用。Skill保留原入口和共享ToolOutput，不因此向Agent开放全部技能。

工具契约包含稳定名称、输入Schema、可选输出data Schema、效果类别、等待上限、取消语义和附件保留规则。调用编号由宿主关联任务、版本、原始发起人和会话；模型不能换任务目录或伪造来源。目录和实际执行均检查实现就绪、任务白名单、SQLite启停/类型化范围及插件权限。注册更新描述而保留已有开关和范围。

统一结果包含call_id、name、task_id、revision、receipt_id、status、output、error.kind/message、retryable、external_action_started和elapsed_ms。状态分为succeeded、rejected、failed、unknown、cancelled；参数或权限预检拒绝为rejected，未执行且不创建外部动作回执，但保留检查点/拒绝事件。工具明确返回的前置错误也可标rejected。实际执行先写started及V18关联信息，结果、事件和检查点同事务提交；旧回执仍保存平铺summary/data/artifacts供既有成果投递读取。

没有声明效果的旧Tool/MCP采用unknown，执行失败、超时、panic或输出无效后不能证明没有副作用，故记录unknown，停止任务并保留旧started状态供恢复检查。显式声明read_only时，确定返回的执行失败标failed，取消标cancelled；这不自动重试。retryable仅表示参数可以在有界新轮次修正。根任务相同工具/参数仍不重复调用，成功、失败、未知均不能靠换协议、版本或子任务绕过。

默认AfterCall在任务暂停/修改时等待调用完成或超时，再归档旧版本结果而不推进新版本；显式DropFuture每秒检查取消并停止等待，不承诺撤回远端动作。工具最大120秒且受根任务剩余时间限制；输出存储限2MB、30个附件，模型投影data超过16KB转有截断标记的预览，summary限2000字，完整有效输出仍在原回执。基础Schema验证子集不等于完整JSON Schema实现，复杂约束与业务权限仍由工具负责。

原生模式只接受本轮声明函数，一轮最多一个调用；缺编号、未声明工具、批量调用或无效JSON未执行。保留模型原始call_id，将完整已存调用/结果配对重建为assistant.tool_calls与tool.tool_call_id；历史压缩按整对保留，不留下悬空编号。独立任务说明作为稳定首条输入，执行历史按原始顺序追加；修正反馈保留在对应结果之后，前缀不因把所有工具结果移到末尾而反复变化。带冒号/点的MCP名称通过稳定别名映射，不能由模型改写真实注册名。yuantuan_control仅表达宿主done/reply/wait_input/delegate，不直接发QQ。格式错误最多修正两轮；原生拒绝、截断和传输错误停止，不自动换模型或协议。

两种协议共享根预算、2个任务槽、去重、版本屏障与回聊链路；parallel_safe是契约元数据，本阶段不增加单任务内并行工具。WebUI工具执行记录显示协议、调用编号、任务版本、状态、耗时和错误；任务完成、工具成功与QQ送达继续分开。模型原生函数格式参见[官方说明](https://developers.openai.com/api/docs/guides/function-calling?api-mode=chat)。

验证入口：[`tool_protocol.rs`](../../../crates/core/tests/tool_protocol.rs) 用本地mock运行同一任务的双协议、调用配对、权限/来源、去重、错误分类及旧 V17 数据升级到当前架构的保留回归；[`tasks-flow.spec.js`](../../../webui-frontend/tests/tasks-flow.spec.js) 与[`pages_api.rs`](../../../crates/webui/tests/pages_api.rs) 覆盖前后端状态。真实Provider原生执行、Docker依赖、浏览器和通用文件收发仍需后续验证，不能以这些mock或CI代替。


## 工具发现与参数修正

内置 `diagnostics.recent` 同时注册为 Tool 与 Skill，目录和 Agent 执行均沿用现有能力开关及类型化会话范围。记录来源与参数见[运行自检](diagnostics.md)。

插件页可单独设置 Tool 的会话范围，Agent 目录和调用继续使用同一 SQLite 能力策略。面板更新字段及交互见[扩展宿主](../extensions/host.md)。

[`discovery.rs`](../../../crates/core/src/tools/discovery.rs) 复用执行器的当前就绪、SQLite启停及类型化范围过滤，为Router和Chat提供工具名、用途、参数名/类型及必填项概览。它不执行工具、不授予权限，也不要求用户知道工具ID或填写JSON；Skill可直接完成时仍走原入口，只有底层Tool或组合工作才交Agent。目录最多48项、条目约3600字符；超预算先缩工具提示，保留total/omitted标记，不把省略或读取失败冒充没有能力。完整Schema仍由Agent按实际任务范围读取。

Agent对invalid_arguments依据字段路径和Schema修正，无法从任务说明或已有结果获得的业务条件才wait_input；不自动编造目标、默认时长或凭据。预检拒绝不启动工具，修正继续原任务的有界loop，不重启Router或重置预算。副作用未知时仍停止，不把调用方便理解为无限重试。

验证：[`tool_usability.rs`](../../../crates/core/tests/tool_usability.rs) 覆盖无Skill但有Tool时的Chat能力、停用/范围/未就绪过滤和错误路径；[`dispatch_tests.rs`](../../../crates/core/src/bot/dispatch_tests.rs) 用本地mock跑Router→Agent→错误参数拒绝→修正→成功回执，仅实际执行一次，并检查周期入口读取当前开关。mock验证接口与宿主行为，不证明真实模型一定选对工具。
