# Web 管理后台与观测

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（界面按后端能力受限）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 登录、概览、决策 Trace、事件、WebSocket 与页面路由。

## 实际行为

- Rust Axum 后端提供受保护的 `/api/*`、`/ws` 和嵌入式静态前端；登录与状态接口例外处理。
- Vue 管理页面提供总览、决策追踪、任务、配置等入口；WebSocket 推送观测事件，用于 UI 展示。
- WebUI 的 HTTP 路由可使用已绑定的 TCP 监听器启动，记忆库接口回归直接复用该入口以避免并行测试抢占临时端口。
- 插件能力的会话范围与启停通过管理员会话保护的 `PATCH /api/capabilities` 更新，字段及权限规则见[扩展宿主](../extensions/host.md)。
- 管理员接口 `GET /api/platforms` 返回各通道的运行状态与非密钥账号标识。官方机器人配置经现有配置 API 验证，读回时掩去 AppSecret，配置变更提示重启。通道行为见 [QQ 接入](../adapter-qq/qq.md)。
- 决策 Trace 和任务数据库是不同事实来源：Trace 记录过程、任务终态以持久化任务状态为准。

图库列表额外返回采集图的识别状态、用途与待审原因；关系节点详情单独按需读取画像、观察和互动依据。接口沿用管理员认证，不返回图片来源地址或 provider 密钥。

记忆库提供统一人物、参与会话、按需出处、原始发言、人物活跃统计、画像版本历史和显式整理画像接口，均注册在现有 session 保护层内；页面详情见[上下文与记忆](../core/memory.md)，不重复维护业务规则。

## 边界

- 面板页面存在不代表对应业务闭环已经实现，尤其[知识库](knowledge.md)仅为占位。
- 管理面板不应直接裸露公网；启用管理员密码和受控访问。

## 对应实现（本页负责的主源码）

- [`crates/webui/src/auth.rs`](../../../crates/webui/src/auth.rs)
- [`crates/webui/src/dashboard.rs`](../../../crates/webui/src/dashboard.rs)
- [`crates/webui/src/events_api.rs`](../../../crates/webui/src/events_api.rs)
- [`crates/webui/src/lib.rs`](../../../crates/webui/src/lib.rs)
- [`crates/webui/src/overview.rs`](../../../crates/webui/src/overview.rs)
- [`crates/webui/src/state.rs`](../../../crates/webui/src/state.rs)
- [`crates/webui/src/static_files.rs`](../../../crates/webui/src/static_files.rs)
- [`crates/webui/src/ws.rs`](../../../crates/webui/src/ws.rs)
- [`webui-frontend/src/views/Dashboard.vue`](../../../webui-frontend/src/views/Dashboard.vue)
- [`webui-frontend/src/views/Login.vue`](../../../webui-frontend/src/views/Login.vue)
- [`webui-frontend/src/views/Trace.vue`](../../../webui-frontend/src/views/Trace.vue)

## 验证入口

- [`crates/webui/tests/auth_status.rs`](../../../crates/webui/tests/auth_status.rs)
- [`crates/webui/tests/pages_api.rs`](../../../crates/webui/tests/pages_api.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

V12在现有session保护层注册 `/api/capabilities`、`/api/tasks/{id}/runtime` 与 `/api/tasks/{id}/control`；能力目录、会话范围与任务操作继续由后端核实。

决策追踪增加真实消息处理统计与待判断、已领取、执行中、待核实筛选，读取受认证保护的 `/api/decisions/status`。返回数据来自消息持久状态，支持会话编号、状态和0–100条限额；机器人和self不进入统计。批次入口、消息数、实际作者和锚点随新Trace事件展示，旧事件缺少字段时不补造。页面每10秒在可见时刷新状态，异常显示错误而非假造空队列；待核实只有查看入口，不提供盲目重试。

浏览器回归：[`webui-frontend/tests/trace-flow.spec.js`](../../../webui-frontend/tests/trace-flow.spec.js)，覆盖桌面/窄屏布局、批次/即时归属、失败不冒充忽略、队列筛选和只读状态。`pages_api.rs`验证认证、来源姓名、过滤、统计与只读未知执行状态。

处理状态新增failed筛选和“已停止重试”统计，返回并展示decision_error；累计次数标为失败数，不再误标为每条消息都必须重试。永久Decision错误显示网关暂停原因，状态读取不触发重试。浏览器与API回归覆盖旧11次记录的停止原因、认证及只读行为。

新Decision Trace显示本轮群聊热度、近2分钟消息/参与人数、实际采用的参与间隔及“自然接话”模式，沿用现有标签与布局，不增加额外解释文案或写配置请求。旧事件没有热度字段时隐藏此区，不补造数据。trace-flow浏览器回归覆盖新字段与桌面/390像素窄屏展示；事件数据仍是判断记录，不能当作实际发送回执。

Trace的policy版本5可展示“计划表情”的类别及云团专属/其他图库/优先云团范围；它是Decision的已校验计划，实际图片发送以BubbleSent回执为准。旧事件没有计划字段时不补造，trace-flow浏览器回归同时覆盖新计划标签与既有热度/失败状态。


V20 复用管理员 Session 保护层注册夸克链接守护的五个 GET/POST/DELETE 接口；API 实际实现见 [\`crates/webui/src/schedule_api.rs\`](../../../crates/webui/src/schedule_api.rs) ，业务规则由[统一每日只读工具调度](../core/scheduler.md)维护。按钮不直接执行外部 Shell，使用核心 ToolRegistry 及 SQLite 状态。此源码修改尚未经过真实夸克账号联调。

插件中心使用共享 `/plugins/:name` 独立配置页，复用已认证插件与能力API；设置草稿、secret保留、应用等待、会话范围和工具参数查看保持。QQ空间使用该模板配置可见范围与额度，没有单独Cookie表单；插件已加载不等于已完成真实发布。详见[宿主](../extensions/host.md)和[QQ空间](../extensions/plugins/qzone.md)。


服务连接配置页新增“检查连接”，只检查已保存且已启用的腾讯文档或夸克插件，调用公共认证探针 API，明确呈现返回结果或错误；不保存 Token 到前端状态回显。Token 环境变量名、secret 和创建分享/文档写入开关仍由通用元数据表单提供。回归包括真实 API 注册表/认证/持久开关与浏览器配置保存后检查，QQ空间原有参数契约与启停保持。

连接结果按当前插件标识关联，切换详情页后不把旧请求成功显示成新插件已连接。账号操作者统一使用主配置 security.super_admin_qq，插件页不复制 owner 设置。

配置 API 保存更改后的 security.super_admin_qq 会明确返回 requires_restart，原值未变时不重复提示；对应 pages_api 回归验证这不是已经热应用。


账号连接向导在 AppState 保存有界、会话绑定的临时进度，所有授权接口位于现有 API 会话中间件之后；重启清除待授权流程，插件设置和官方 CLI 已保存的登录沿用持久目录。前端向导与配置补丁行为见[扩展宿主](../extensions/host.md#账号连接向导)，不新增公开回调路由。
