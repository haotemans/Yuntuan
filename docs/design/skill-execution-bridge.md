# Yuntuan 外部 Skill 执行桥（提案）

**状态：设计阶段，不可宣称已集成。** 目标把 `skills/<id>/SKILL.md` 的行为约束与 Yuntuan 现有 `ToolRegistry / SkillRegistry` 连接起来，同时复用现有 Plugin / MCP 权限闸门。

## 背景与边界

- 当前 `Plugin` 是 Rust 编译期组件，可以启停、配置 Tool/Skill，但不会自动解析或安装 `SKILL.md`。
- 现有 MCP 只支持本地 stdio 子集。腾讯文档提供远程 MCP（`https://docs.qq.com/openapi/mcp`），须增加**经测试的代理 sidecar**或实现有鉴权的远端 MCP 传输适配，不能把 HTTP 地址当作可执行文件。
- 夸克官方 Skill 以 Node.js CLI 工作；应作为经过权限约束的受控外部能力运行，不在 Yuntuan 主进程中直接执行未经审查的安装代码。
- 本文不是把另一个 `yuantuan` Agent 项目合并进来。

## 数据流

`用户自然语言 → Decision（识别意图与请求者） → 已注册 Skill → 工具 schema 与权限检查 → 授权账号下的 MCP/CLI sidecar → 有界结构化结果 → Chat / 原会话回复`

对后台计划任务：`持久计划任务 → 可信调度器创建 Agent 任务 → ToolRegistry → MCP/CLI → 执行回执 → 独立出站通知`。不得伪造普通聊天消息或复用过期的 QQ 消息锚点。

## 最低实施接口

1. **SkillManifest**：固定 ID、版本、触发词、必需工具、使用说明相对路径、授权方式、支持平台、默认只读权限；仅允许可信本地清单，禁止远端指令直接改写清单。
2. **ToolAdapter**：腾讯文档为远程 MCP 桥接，夸克为 Node.js CLI sidecar；均需工具 schema、超时、输出/附件限额和返回类型化错误。
3. **CredentialVault**：按真实 QQ 用户、平台授权账号、作用域隔离；运行时凭证仅传给被批准的 sidecar，面板只显示“已授权/到期”，不回显秘密。
4. **PermissionGrant**：工具 `read`、`write`、`share`、`delete` 分级。每次写操作检查实际调用者与会话；后台执行只使用创建时持久授权、不能自行提权。
5. **Capability discovery**：只有工具在当前进程真实注册且依赖/身份可用，才对模型宣传对应能力。失败时保留明确诊断，不生成假的文档/网盘链接。
6. **审计**：记录调用来源、工具、受影响文件 ID、运行状态与回执；敏感参数与账号令牌脱敏。工具返回的网络内容属于不可信数据。
7. **测试**：测试账号的搜索/创建/读取/编辑；权限拒绝；重复执行防护；掉线/凭证过期；CLI 超时/崩溃；Windows WSL/ Linux 平台差异。

## 分阶段落地

- A. 本次：提交两份 `SKILL.md`、官方源和明确的行为契约，**不自动安装第三方软件**。
- B. 执行桥：优先实现腾讯文档只读 MCP 和夸克只读 CLI。授权方式与管理员配置在 WebUI 中可见。此阶段方可标记“只读可用”。
- C. 添加文件写入/分享的二次确认、审计日志与任务产物支持。
- D. 与 [定时任务插件](scheduled-tasks-plugin.md) 联动，执行经授权的文档更新与网盘备份。

## 依赖

腾讯文档 [官方 MCP](https://developer.cloud.tencent.com/mcp/server/11803)；
夸克 [官方 Skill](https://github.com/quark-clouddrive/quarkclouddrive_offical)。
