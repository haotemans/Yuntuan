# 卷耳：夸克网盘连接与分享检查

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

Skill 在执行所选工具前复核同名 Tool 的持久开关、会话范围及最终参数；不能绕过宿主。只读与写入各自声明超时和取消策略。聊天工具白名单不包含删除、自动登录、安装更新、批量修改或异常后自动换链；管理页授权是独立入口。

## 管理页授权与持久配置

`/plugins/quark-drive?tab=config` 提供“开始授权登录 → 打开夸克授权页 → 回填跳转链接或授权码 → 保存并连接”。未启用、未填写脚本路径时也能开始。登录和账号同意由管理员本人在夸克官方页面完成；授权入口不是模型 Tool，云团不索取账号密码，也不让群聊代收授权码。

首次开始时，后台从官方 CDN 下载固定的 Skill 1.0.25，验证 SHA-256 后安全解压到当前持久 data 目录的 `connectors/quark/`。镜像提供 Node.js 22；不执行远程安装 shell、不改系统包、不覆盖已有组件或登录。每包最多8MiB、解压总量32MiB/100项，拒绝路径越界、符号链接及覆盖文件。已有 `cli_script` 则直接复用，不替换自定义安装。组件根目录在 Unix 上设为0700。高级配置仍可指定应用容器内的 `quark-drive.cjs` 绝对路径。

官方 CLI `login` 生成实际 OAuth 链接；后台仅提取官方 HTTPS `quark.cn` 域链接，不编造 client_id 或 redirect_uri。无桌面容器不能接收浏览器跳转时，管理员把完整跳转链接中的唯一 `code`/`agent_auth_code`（或授权页显示的裸码）粘贴回管理页。回填链接仅在本地解析，不请求其地址；目前不实现自动公开回调端点。链接仍受官方有效期约束，本地流程最多保留10分钟，刷新可恢复同一管理会话，云团重启后须重新开始。

授权准备、回填及检查在后台执行，管理页轮询进度；每插件只允许一项操作，流程绑定当前 WebUI 登录会话。回填码使用一次，不因失败自动重试；官方 `-118` 已授权状态通过 `get-user-info` 复核后复用。成功检查后原子写入脚本路径并启用插件，保留会话范围、调用限制及分享开关。官方凭据由 CLI 保存于持久安装目录，不回显到接口、日志或模型。

之后可在高级配置点击“检查连接”，通过已注册 `quark-drive.account` 执行只读检查；此探针还受 `private:plugin-check` 与持久能力开关限制。授权向导仅核实账号并保存连接，不证明具体文件或分享已读取，也不扩大聊天调用权限。

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
- [`plugins/quark-drive/src/auth.rs`](../../../../plugins/quark-drive/src/auth.rs)：固定组件安装、OAuth 链接提取及回填；[`auth/tests.rs`](../../../../plugins/quark-drive/src/auth/tests.rs) 使用 Node fixture 验证非零退出、授权码、已有登录和解压边界。
- [`plugins/quark-drive/src/tests.rs`](../../../../plugins/quark-drive/src/tests.rs)：真实 Node fixture 验证 argv 不经 shell、会话复用、能力停用不执行、结果文件/notice/敏感字段与无效分享输入。
- [`crates/core/src/scheduler.rs`](../../../../crates/core/src/scheduler.rs)：内含真实 SQLite + fake Tool 的持久、权限、租约和失败状态回归。
- [`webui-frontend/tests/plugins-flow.spec.js`](../../../../webui-frontend/tests/plugins-flow.spec.js)：独立配置、连接检查和分享守护 API 流程。
- [`crates/webui/tests/pages_api.rs`](../../../../crates/webui/tests/pages_api.rs)：真实管理页会话、授权回填、重复提交和原配置保留（使用 Node fixture，非真实夸克账号）。

验证命令：`cargo test -p yuantuan-plugin-quark-drive`；实际账号及服务器行为另行验收，不由绿色构建代替。


账号操作人统一取 `security.super_admin_qq`：对话 Tool/Skill 只对该真实发言者开放，未配置则拒绝，范围开关不能把管理员账号授权给其他群成员。管理员页面探针及明确登记的核心只读巡检使用宿主固定身份；它们不能执行写操作。此身份由应用启动加载，修改主配置后需重启应用；不是再填一套插件账号权限。


界面展示名为“卷耳”，沿用现有《诗经》系命名，出处《周南·卷耳》。副标题保留夸克网盘和用途；内部插件 ID `quark-drive`、Tool/Skill 名、命令、配置路径与持久记录继续沿用原值，无数据迁移。
