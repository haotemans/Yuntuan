# Yuntuan Skills（接入契约）

本目录存放可复用的 `SKILL.md` 工作流规范，**不是已经启用的插件**。当前 Yuntuan 的 `SkillRegistry` 注册的是编译期 Rust `Skill` 实例，尚不会解析这些 Markdown 并自动调用外部 CLI 或远程 MCP；不要把目录存在理解为可在 QQ 中直接使用。

| Skill | 数据源与官方接口 | 用途 | 当前落地状态 |
| --- | --- | --- | --- |
| [tencent-docs](tencent-docs/SKILL.md) | 腾讯文档官方 MCP | 读取、搜索、创建、编辑云文档、智能表格 | 行为与权限契约已编写；远程 MCP 桥接/登录/端到端待实现 |
| [quark-drive](quark-drive/SKILL.md) | 夸克网盘官方 Skill/CLI | 文件搜索、上传、备份、整理、分享 | 行为与权限契约已编写；受控 CLI 执行器/登录/端到端待实现 |

## 运行时接入条件

1. 宿主实现 [Skill 执行桥](../docs/design/skill-execution-bridge.md)：扫描与校验、依赖检测、权限检查、工具动态映射以及独立进程隔离。
2. 腾讯文档使用官方授权 Token；夸克使用官方登录授权。凭证**只放宿主安全存储**，不在 Git、Skill 内容、聊天记录与 Agent prompt 中透传。
3. 设置每个账号/会话的可读写范围。默认只读；共享、删除、覆盖、授权变更等操作要求确认。计划任务不得自动扩大权限。
4. 用真实账号在测试文档/文件夹验证后，才能把集成标为“已实现”。

## 范围

本阶段只提交 Skill 工作流定义与架构契约，没有修改现有四个编译期插件，也没有通过 QQ 访问任何用户云文档或网盘。项目 [Yuntuan](../README.md) 与独立仓库 `yuantuan` 无关。
