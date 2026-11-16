# 夸克网盘连接与分享检查

[插件索引](README.md) · [扩展宿主](../host.md) · [操作契约](../../../../plugins/quark-drive/SKILL.md)

已实现官方 CLI 的受限适配。自动化实际执行 Node fixture、NDJSON/JSONL 与 SQLite 调度，但未登录真实夸克账号；不将模拟结果当作 OAuth 或真实网盘验收。

## 能力与双入口

`quark-drive.assist`、`/夸克`、`/quark` 和 Agent 共用 Connector，支持六个 Tool：

| Tool | 官方命令 | 效果 |
| --- | --- | --- |
| `quark-drive.account` | `get-user-info` | 只读账号查询/连接检查 |
| `quark-drive.search` | `search` | 搜索文件，关键词1–50字 |
| `quark-drive.inspect_share` | `share-detail` | 检查已提供的分享链接 |
| `quark-drive.share_updates` | `get-share-update-list` | 已转存分享的更新列表 |
| `quark-drive.updated_files` | `get-share-update-files` | 指定已转存分享的更新文件 |
| `quark-drive.create_share` | `share` | ExternalWrite，需开启创建分享且仅限私聊 |

Skill 在执行所选工具前复核同名 Tool 的持久开关、会话范围及最终参数；不能绕过宿主。只读与写入各自声明超时和取消策略。白名单不包含删除、自动登录、安装更新、批量修改或异常后自动换链。

## Docker 安装与授权

配置页 `/plugins/quark-drive` 使用公共表单，`cli_script` 必须是**应用容器内**已存在的 `quark-drive.cjs` 绝对路径；仅填写 Windows/宿主机路径不能使容器访问该文件。新插件默认关闭。

仓库 Docker 运行时提供 Node.js 22。管理员依[官方 Skill](https://github.com/quark-clouddrive/quarkclouddrive_offical)安装并授权，建议把完整官方 Skill 目录放到已持久挂载的 `/app/data/connectors/quark/`，配置示例：

```text
/app/data/connectors/quark/scripts/quark-drive.cjs
```

安装、登录须在云团容器的实际运行用户下操作，保证 CLI、其 `config.json`/OAuth 文件及查询输出目录可写并随 data 持久保存；不可仅在宿主机或另一用户下登录后假定容器已授权。浏览器回调无法到达容器时，依[官方授权说明](https://github.com/quark-clouddrive/quarkclouddrive_offical/blob/main/skills/quarkclouddrive/references/auth.md)完成管理员授权，不把授权码或 Token 发到群聊。插件不在聊天请求中静默执行安装脚本或自动登录。

保存、启用后点“检查连接”，通过已注册 `quark-drive.account` 实际调用 CLI；未安装 Node、路径错误或未授权会失败。检查身份为 `private:plugin-check`，受限会话范围需明确允许。Node 运行时烟测只证明镜像内可以执行 Node，不证明官方 CLI 已安装或账号可用。

## 调用、输出和状态

使用 `Command` 的 argv 启动 Node，无 shell 拼接，stdin 关闭，工作目录固定在官方 Skill 根目录。共享 Schema 拒绝额外字段和无效参数；对话调用保留原始 `session_input`，按会话类型及 ID 复用会话标识。非对话巡检可无原始提问。

stdout/stderr 分别有界（256KB/16KB），总执行100秒，超时/输出过大结束子进程。解析最终 result，同时保留 list/file 行；搜索返回 artifact 时，只读取官方安装根目录内的真实 JSONL，最多50条/24KB预览，明确标记截断，不宣称已经展示全部文件。不会读取其配置文件作为查询结果，返回数据移除 Token/Cookie 类字段。命令摘要包含实际结果，并附本次实际 `notice.text`；官方 notice 超出支持上限则明确失败，不偷偷删减。

分享创建必须取得实际 `share_url`；超时、非正常退出或缺失链接均视为待核实结果，不自动重试。公开/私密链接和过期类型使用官方 argv；原始 stderr、凭据不会直接进入模型或日志。管理员授权开关不是新增一次审批弹窗，仍应根据用户明确请求及已有真实 FID 执行。

## 每日只读检查

[核心持久调度](../../core/scheduler.md)负责 SQLite `scheduled_tools`，插件不自建 Cron。配置页“分享守护”登记 URL，默认每天北京时间09:00检查，支持立即检查、暂停和删除；最多200条记录，重启按租约恢复，每次复核工具权限。有限会话范围需显式允许 `private:scheduled`。

`get-share-update-list` 是**已转存分享内容的更新**，不是本人所有分享的有效性列表。巡检只检查登记链接并保存结果；不自动续期、重新分享、绕过平台限制或发 QQ 通知。一次失败不能区分网络、认证、到期或平台限制。

本次按官方 Skill 1.0.25 的接口文档核对受限子集，未来升级由管理员主动进行并重新检查兼容性；不宣称完全执行官方通用 Agent Skill 的自动安装、登录和所有操作流程。

## 源码与验证

- [`plugins/quark-drive/src/lib.rs`](../../../../plugins/quark-drive/src/lib.rs)：配置、工具白名单、Schema、共享 Skill/Tool 权限和参数构造。
- [`plugins/quark-drive/src/process.rs`](../../../../plugins/quark-drive/src/process.rs)：有界进程、NDJSON 和受限 JSONL 读取。
- [`plugins/quark-drive/src/tests.rs`](../../../../plugins/quark-drive/src/tests.rs)：真实 Node fixture 验证 argv 不经 shell、会话复用、能力停用不执行、结果文件/notice/敏感字段与无效分享输入。
- [`crates/core/src/scheduler.rs`](../../../../crates/core/src/scheduler.rs)：内含真实 SQLite + fake Tool 的持久、权限、租约和失败状态回归。
- [`webui-frontend/tests/plugins-flow.spec.js`](../../../../webui-frontend/tests/plugins-flow.spec.js)：独立配置、连接检查和分享守护 API 流程。

验证命令：`cargo test -p yuantuan-plugin-quark-drive`；实际账号及服务器行为另行验收，不由绿色构建代替。


账号操作人统一取 `security.super_admin_qq`：对话 Tool/Skill 只对该真实发言者开放，未配置则拒绝，范围开关不能把管理员账号授权给其他群成员。管理员页面探针及明确登记的核心只读巡检使用宿主固定身份；它们不能执行写操作。此身份由应用启动加载，修改主配置后需重启应用；不是再填一套插件账号权限。
