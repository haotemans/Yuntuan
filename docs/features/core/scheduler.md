# 定时任务与每日只读工具调度

[功能索引](README.md) · [夸克网盘插件](../extensions/plugins/quark-drive.md) · [数据库](storage.md)

**当前状态：固定工具每日调度已有实现，新增每日 Agent 计划。** 基础逻辑与 mock/浏览器验证不替代真实模型、夸克账号和生产部署验收。

## 实际行为

- 云团核心维护 `scheduled_tools`，而不是夸克插件自己重复维护 Cron。每条任务记录工具名、资源标识、JSON 参数、每日本地时刻、固定时区偏移、启停状态、下一次运行时刻、最近运行时间、状态与摘要。
- 工作台「定时任务」 `/schedules` 可选择已授权工具、按北京时间填写每日时刻、改时间、启用或暂停、立即执行、删除及查看最近结果。列表展示关键词或分享地址，分享地址省略查询参数。支持夸克文件搜索、分享状态和更新检查，以及腾讯文档连接检查。任务由工具直接执行，无需模型产生操作计划。
- 原「分享守护」插件页继续使用同一批夸克分享任务，原入口创建任务默认每日 **北京时间 09:00**。新版页面允许调整既有任务的执行时间。
- 只允许已经在 ToolRegistry 注册、明确声明 `ToolEffect::ReadOnly` 的工具执行自动任务。执行前复核工具可用性、插件会话范围和 SQLite 能力开关、JSON Schema；调用仍经过受保护的 HostedTool 和插件调用统计。
- 新任务安排在所选时刻下一次到来时运行。重启后根据 SQLite 的下一运行时刻恢复扫描；到期任务经 `claimed_at` 原子领取，过期租约可再次领取。调用成功记 `accessible`，外部错误记 `check_failed`，不根据一次失败猜测违规。执行中的任务可暂停未来调度，待本次执行结束后再更改时刻或删除。
- 保存最近一次结果的有限摘要；执行后排到下一次指定时刻。数据留在现有 SQLite，云团服务运行时执行后台调度。恢复已错过的任务时领取一次，再按下一次指定时刻执行。

## 接口与权限

现有管理员 session 保护下新增：

- `GET/POST /api/schedules`：查询任务和可选工具、创建 `{tool_name,daily_time,input?,title?}`。相同工具与参数复用单条任务约束，修改执行时间走单独操作。
- `POST /api/schedules/{id}/time`：设置 `{daily_time: "HH:MM"}` 并重新排期。
- `POST /api/schedules/{id}`：`{enabled:boolean}` 启用或暂停。
- `POST /api/schedules/{id}/run`：执行一次已启用任务并保存结果。
- `DELETE /api/schedules/{id}`：删除调度记录。

- `GET /api/plugins/quark/watches`：查询最多 200 个已登记监控任务（登记时也限制200条）。
- `POST /api/plugins/quark/watches`：登记 `{url,title}`；URL 必须是 `https://pan.quark.cn/s/...`，要求对应插件、Tool 和权限可用。
- `POST /api/plugins/quark/watches/{id}/check`：执行一次立即检查，返回明确成功/失败。
- `POST /api/plugins/quark/watches/{id}`：`{enabled:boolean}` 启用/暂停。
- `DELETE /api/plugins/quark/watches/{id}`：仅删除监控记录，不删除云盘文件。

调度身份使用 `private:scheduled`。插件限制 `allowed_chats` 时应包含 `scheduled`，工具使用类型化的 `scopes` 时应包含 `private:scheduled`。创建与实际执行都会核验。

## 已知边界

- **只读检测，不自动续期、更换 URL 或删除分享。** 官方 `get-share-update-list` 只说明已转存分享有内容更新，不能用于判断本人分享是否存活。
- 第三方官方 `share-detail` 请求若返回错误，不保证能够准确区分网络错误、认证失败、到期或违规封禁；UI 使用「检查失败」而非直接标为封禁。
- 分享状态检查需要完整分享链接；结果可在定时任务页查看。QQ 自动通知需后续接入。
- 首期仅支持每日时刻规则；更复杂 Cron、写任务审批、消息通知和重新分享属于后续功能。
- 现有 Plugin 账号凭据存储只掩码，尚未添加独立加密 Vault；不得声称凭据已端到端加密。

## 对应实现

- [`crates/core/src/scheduler.rs`](../../../crates/core/src/scheduler.rs)
- [`crates/webui/src/schedule_api.rs`](../../../crates/webui/src/schedule_api.rs)
- [`crates/core/src/db/scheduler_v20.sql`](../../../crates/core/src/db/scheduler_v20.sql)
- [`webui-frontend/src/views/Schedules.vue`](../../../webui-frontend/src/views/Schedules.vue)

相关测试：
- [`crates/core/tests/sqlx_storage.rs`](../../../crates/core/tests/sqlx_storage.rs)
- [`webui-frontend/tests/plugins-flow.spec.js`](../../../webui-frontend/tests/plugins-flow.spec.js)
- [`crates/webui/tests/pages_api.rs`](../../../crates/webui/tests/pages_api.rs)
- [`webui-frontend/tests/tasks-flow.spec.js`](../../../webui-frontend/tests/tasks-flow.spec.js)

该功能复用插件专属页面 [`webui-frontend/src/views/PluginSettings.vue`](../../../webui-frontend/src/views/PluginSettings.vue)，页面本身的主源码归属见[扩展宿主](../extensions/host.md)，不重复归档。


`scheduler.rs` 内新增真实 SQLite + fake Tool 回归，覆盖重复登记、成功/失败区分、持久停用/范围、未过期与过期租约、重新打开数据库后的恢复及暂停/删除。该回归不访问夸克账号。`check_now` 完成表示检查结果已记入库，检查失败仍以 `check_failed` 呈现，不能等同于可访问。

## 每日云团 Agent 任务（V23）

「云团任务」允许管理员填写自然语言要求、每日北京时间、工具集合（1–16项）、轮数（1–10）与时限（30–900秒）。「固定工具」保留原有只读调度，两者互不覆盖。

计划使用 agent_schedules，触发记录使用 agent_schedule_runs。每30秒扫描，创建现有 tasks/task_runtime、任务说明、触发记录与下次时间在同一 SQLite 写事务提交，事件仅唤醒原 TaskRunner。重启补领一次错过的时间；已有运行、等待补充、等待子任务或暂停的任务阻止同计划重叠。排队计入任务时限，重新扫描不重置预算。

工具仅允许 ReadOnly 与 WorkspaceWrite（含 Bash），需要明确选择且满足 private:scheduled 会话范围。触发和实际调用复核权限，外部写入/效果未知工具不开放无人值守执行。宿主保存当前管理员 p_<super_admin_qq> 为执行人，未配置时为 scheduler；账号插件继续按自己的所有者规则授权。旧计划保留原执行人，修改管理员后需删除重建相关计划。不伪造用户原消息或重放群聊。

最近30次触发可查看日期、排队/执行/等待/完成/失败状态、结果或失败原因，并跳到对应任务的完整回放。模型缺失或工具权限撤销时记录未启动原因，排到下次时刻；修复后可手动触发。未知副作用遵循 Runner 的停止/核实规则，不自动重跑。暂停/删除计划只影响后续触发，当前任务需在任务回放中另行取消；已创建任务和记录保留。

本版结果保存在后台，**不自动推送 QQ，不新增单次、每周或 Cron**。进入队列与完成分别展示。mock/浏览器测试不替代真实模型、账号与生产验收。

管理员接口：

- GET/POST /api/agent-schedules：列表/创建，字段 title、goal、daily_time、allowed_tools、max_calls、timeout_secs。
- POST /api/agent-schedules/{id}、/{id}/time、/{id}/run：启停、改时刻、手动触发；触发返回 task_id，不等待执行完成。
- GET /api/agent-schedules/{id}/history、DELETE /api/agent-schedules/{id}：触发历史/删除计划。
- GET /api/runtime/agent：模型绑定、Bash 注册及定时权限、候选工具。已配置/已注册不证明实时调用成功。
- POST /api/runtime/bash/check：串行 Docker daemon/镜像预检，不执行命令、不安装依赖、不改配置；详情写入脱敏运行日志。

实现：[Agent 调度](../../../crates/core/src/scheduler/agents.rs)、[管理接口](../../../crates/webui/src/agent_api.rs)、[界面](../../../webui-frontend/src/components/AgentSchedules.vue)、[V23](../../../crates/core/src/db/scheduled_agents_v23.sql)。验证：[事务与权限回归](../../../crates/core/src/scheduler/agents.rs)、[mock 模型 Runner 链路](../../../crates/core/tests/agent_loop.rs)、[浏览器回归](../../../webui-frontend/tests/agent-schedules.spec.js)。
