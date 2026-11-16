# 统一每日只读工具调度与夸克分享守护

[功能索引](README.md) · [夸克网盘插件](../extensions/plugins/quark-drive.md) · [数据库](storage.md)

**当前状态：代码已新增，真实夸克账号/OAuth 联调与用户实际部署尚未完成。** 源码单元测试和 PR CI 用于验证基础逻辑；部署前不可把「自动检查已启动」告知用户。

## 实际行为

- 云团核心维护 `scheduled_tools`，而不是夸克插件自己重复维护 Cron。每条任务记录工具名、资源标识、JSON 参数、每日本地时刻、固定时区偏移、启停状态、下一次运行时刻、最近运行时间、状态与摘要。
- 当前 WebUI 插件独立设置页 `/plugins/quark-drive?tab=links` 提供添加、删除、暂停、立即检查夸克分享链接。默认每日 **北京时间 09:00**，不需每次调用 LLM。
- 只允许已经在 ToolRegistry 注册、明确声明 `ToolEffect::ReadOnly` 的工具执行自动任务。执行前复核工具可用性、插件会话范围和 SQLite 能力开关、JSON Schema；调用仍经过受保护的 HostedTool 和插件调用统计。
- 新任务于下一个本地 09:00 调用 `quark-drive.inspect_share`，传入管理员登记的完整 URL。重启后根据 SQLite 下一运行时刻恢复扫描；通过 `claimed_at` 和单条件更新避免多次并发领取，过期租约可再次领取。调用成功记 `accessible`，外部错误记 `check_failed`，不根据一次失败猜测违规。
- 执行结果只保存有限摘要；每次检查后排下一个 09:00。数据留在现有 Yuntuan SQLite，不启动额外进程、不依赖聊天窗口存活。

## 接口与权限

现有管理员 session 保护下新增：

- `GET /api/plugins/quark/watches`：查询最多 200 个已登记监控任务（登记时也限制200条）。
- `POST /api/plugins/quark/watches`：登记 `{url,title}`；URL 必须是 `https://pan.quark.cn/s/...`，要求对应插件、Tool 和权限可用。
- `POST /api/plugins/quark/watches/{id}/check`：执行一次立即检查，返回明确成功/失败。
- `POST /api/plugins/quark/watches/{id}`：`{enabled:boolean}` 启用/暂停。
- `DELETE /api/plugins/quark/watches/{id}`：仅删除监控记录，不删除云盘文件。

当前调度身份使用 `private:scheduled`，插件限定了 allowed_chats 或能力 scopes 时必须显式允许该系统调用域，不能悄悄越过权限。

## 已知边界

- **只读检测，不自动续期、更换 URL 或删除分享。** 官方 `get-share-update-list` 只说明已转存分享有内容更新，不能用于判断本人分享是否存活。
- 第三方官方 `share-detail` 请求若返回错误，不保证能够准确区分网络错误、认证失败、到期或违规封禁；UI 使用「检查失败」而非直接标为封禁。
- 注册必须提供链接，尚不能经官方 API 自动列出个人所有分享。无 QQ 自动通知；结果可在插件配置页面查看。
- 首期仅支持每日时刻规则；更复杂 Cron、写任务审批、消息通知和重新分享属于后续功能。
- 现有 Plugin 账号凭据存储只掩码，尚未添加独立加密 Vault；不得声称凭据已端到端加密。

## 对应实现

- [`crates/core/src/scheduler.rs`](../../../crates/core/src/scheduler.rs)
- [`crates/webui/src/schedule_api.rs`](../../../crates/webui/src/schedule_api.rs)
- [`crates/core/src/db/scheduler_v20.sql`](../../../crates/core/src/db/scheduler_v20.sql)

相关测试：
- [`crates/core/tests/sqlx_storage.rs`](../../../crates/core/tests/sqlx_storage.rs)
- [`webui-frontend/tests/plugins-flow.spec.js`](../../../webui-frontend/tests/plugins-flow.spec.js)

该功能复用插件专属页面 [`webui-frontend/src/views/PluginSettings.vue`](../../../webui-frontend/src/views/PluginSettings.vue)，页面本身的主源码归属见[扩展宿主](../extensions/host.md)，不重复归档。


`scheduler.rs` 内新增真实 SQLite + fake Tool 回归，覆盖重复登记、成功/失败区分、持久停用/范围、未过期与过期租约、重新打开数据库后的恢复及暂停/删除。该回归不访问夸克账号。`check_now` 完成表示检查结果已记入库，检查失败仍以 `check_failed` 呈现，不能等同于可访问。
