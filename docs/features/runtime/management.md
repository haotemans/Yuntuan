# 机器人自管理与权限分层

[运行时索引](README.md) · [插件宿主](../extensions/host.md) · [Agent](../core/tasks.md)

## 实际行为

新增「机器人管理」页 `/management`，统一查询插件加载状态、授权运维管理员、按版本启停/重载、查看带日期的管理记录，并在已配置进程守护时请求应用重启。网页会话仍是现有管理员密码认证，不是多账号网页 RBAC；运维授权仅约束 QQ 聊天工具。

- 主人沿用启动配置 `security.super_admin_qq`。网页可保存主人 QQ，重启生效；未配置时拒绝所有聊天管理。群主、群管理员、昵称、QQ 官方 OpenID 或记忆关联均不获得主人权限。
- 聊天管理仅在 OneBot QQ 私聊开放，主体使用适配器注入的 `p_<QQ>`。主人可维护全部内置插件；运维管理员只可查看授权插件，并分别授予常规配置、启停、重载权限。普通用户无法看到或调用管理工具。
- 管理员授权存入现有 `state_kv` 的 `management.grants.v1`，保存后立即影响下一次管理调用，重启恢复。不增加数据库版本。授权不改变普通插件使用权限、能力会话范围或账号连接所有者。
- `management.plugins_status` 返回实际加载、启停、应用中、待重载、需重启和是否出错，并给出不透明 revision。`management.plugins_apply` 要求先读 revision，再指定 configure/enable/disable/reload；实际执行器仍检查调用身份与任务归属。
- 聊天配置仅开放插件声明为非密钥的 boolean/integer/choice 字段；凭据、地址与文件路径使用原插件配置页。不返回密钥、地址、数据目录或原始供应商错误；错误详情留在插件管理页。修改通过宿主字段补丁保留旧密钥、允许会话和限额。
- 所有插件设置入口共用更新锁和 revision。状态在读取后发生变更时拒绝旧写入；重载显式重建实例，即使配置未变也执行停止/启动。等待既有调用时返回 applying，再读状态确认；不能将提交成功当作生效。异步审计视图按同版本最终状态解析，后续修改不冒充前次成功。

管理审计存入现有 `events`，kind 为 ManagementAction，记录时间、发起主体、动作、插件 ID、修改字段名和状态，不记录配置值/凭据。网页显示最近100条。任务调用另外保留现有工具回执。

## 应用重启

仅当运行时显式配置 `YUANTUAN_RESTART_MODE=supervised` 时启用管理重启。发布的 Docker runtime 设置此标记，部署方必须同时配置 Docker restart policy（现有 Compose 使用 unless-stopped）；二进制由 systemd 等守护时可采用同一契约。未配置守护不宣称可以自动恢复。

网页 `POST /api/config/restart` 返回202后，通过内存信号通知主进程，留出回复时间，再关闭消息处理、插件、发送队列、MCP 和数据库，正常退出，由守护重启。同一进程只接受一次请求。不会执行任意 Shell、操作宿主 Docker API 或重启 NapCat。

聊天重启要求主人私聊明确发送 `/重启云团`，经过同一核心授权。技能核对宿主注入的原消息与显式命令标记；普通自然语言、网页资料中的重启指令及运维管理员不触发重启。返回“重启已受理”只表示请求收到，实际恢复需看进程健康；网页 Session 在应用重启后失效，需要重新登录。

## 接口

全部接口复用现有管理员 Session，未认证返回401：

- `GET /api/management`：当前/已保存主人、权限、插件投影、审计与运行维护支持情况。
- `POST /api/management/owner {qq}`：通过原配置写回接口只修改 security.super_admin_qq，返回需重启列表。
- `POST /api/management/permissions {grants}`：最多64名管理员，每条含 qq、plugins、configure、toggle、reload；拒绝重复 QQ、主人重复授权、未知插件。
- `POST /api/management/plugins {plugin,operation,revision,patch?}`：和 Agent 共用的版本化插件维护入口。

新插件安装和插件代码更新仍需发布应用，管理页明确显示当前不支持在线安装/更新，不提供假成功按钮。本功能不迁移部署方式，不把 Bash 沙箱变成宿主配置编辑器，也不赋予后台定时任务无人值守管理写权限。

## 实现与验证

- [核心边界](../../../crates/core/src/management.rs)、[WebUI API](../../../crates/webui/src/management_api.rs)、[管理页](../../../webui-frontend/src/views/Management.vue)。
- [权限/重载/密钥保留/重启信号回归](../../../crates/core/tests/management.rs)、[插件生命周期](../../../crates/core/tests/plugin_host.rs)、[认证接口](../../../crates/webui/tests/pages_api.rs)、[浏览器流程](../../../webui-frontend/tests/management-flow.spec.js)。

以上测试是可追踪入口，真实模型选择正确管理工具、实际 QQ 消息交互、Docker 自动重启与部署健康需要分别现场验收，不能由静态设计或 mock 测试替代。
