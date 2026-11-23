# 定时任务与每日只读工具调度

[功能索引](README.md) · [夸克网盘插件](../extensions/plugins/quark-drive.md) · [数据库](storage.md)

**当前状态：代码已新增，真实夸克账号/OAuth 联调与用户实际部署尚未完成。** 源码单元测试和 PR CI 用于验证基础逻辑；部署前不可把「自动检查已启动」告知用户。

## 实际行为

- 云团核心维护 `scheduled_tools`，而不是夸克插件自己重复维护 Cron。每条任务记录工具名、资源标识、JSON 参数、每日本地时刻、固定时区偏移、启停状态、下一次运行时刻、最近运行时间、状态与摘要。
- 工作台「定时任务」 `/schedules` 可选择已授权工具、按北京时间填写每日时刻、改时间、启用或暂停、立即执行、删除及查看最近结果。支持夸克文件搜索、分享状态和更新检查，以及腾讯文档连接检查。任务由工具直接执行，无需模型产生操作计划。
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
