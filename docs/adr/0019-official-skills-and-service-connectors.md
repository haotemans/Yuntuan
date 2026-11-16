# ADR-0019: 官方 Skill 与服务连接统一扩展设计

## 状态

已确认架构，分阶段落地。第三方真实授权与长时调度尚未完成。

## 背景

Yuntuan 现有 ToolRegistry、SkillRegistry、原生 Plugin 宿主以及 stdio MCP。用户希望腾讯文档和夸克独立配置、同时支持显式命令和自然语言，并复用现有任务、权限和日志。

## 方案

- 将一个 Plugin 视为提供 Tool + Skill + Connector 的能力包。
- 命令和自然语言共用同一个受宿主管控的执行能力，不建立重复执行路径。
- 腾讯文档使用官方远程 HTTP MCP、实时 `tools/list` 与个人授权 Token。远程连接只能面向固定官方 endpoint。
- 夸克使用官方 CLI 与官方 OAuth 授权，具体命令通过固定 argv 白名单包装，不允许模型执行任意 Shell。
- 每个插件通过独立 WebUI 路由 `/plugins/:name` 配置，但表单、能力开关、日志及会话范围使用通用组件/API。
- 保留编译期 Rust 插件接口；`SKILL.md` 是用法与安全约束，不能因为复制 Markdown 就绕开工具注册/授权。
- 定时任务由核心公共持久调度提供，插件只能注册操作，不允许每个插件自己维护一个独立可靠性未知的 Cron。
- 默认只读，写操作单独授权。违规/封禁分享不得自动换链规避。

## 取舍

首期以可验证的连接、命令白名单和独立页面为主；CLI 和 HTTP MCP 都不是全平台零安装。当前宿主的 secret 字段仅做回显掩码，尚需统一加密凭据服务。未获得真实账号授权时不宣称通过端到端验收。

## 后续工作

OAuth 面板自动回调/续期、官方 CLI 来源版本锁定、加密凭据及真实账号联调仍需单独完成；每日只读调度与连接探针的已实现边界见后文。


## V20 补充：共享持久调度已新增

统一 `scheduled_tools` 在云团核心通过已注册只读 Tool 运行每日检查；夸克只提供 `quark-drive.inspect_share`，WebUI 只登记任务与呈现结果。不增加夸克插件私有 Cron。当前不自动换链、不发布 QQ 提醒，详见 [调度主文档](../features/core/scheduler.md)。

Tencent Docs 的配置额外支持 `token_env`：优先从服务端环境变量读取官方 Token，不必写入现有未加密的插件 settings.json；未填写 token_env 时可使用旧 secret 字段；指定变量后缺失则失败，不回退到旧账号。

Skill 和 Tool 复用授权门控：宿主 `HostServices::authorize_tool_for_skill` 在 Skill 委托 Connector 前验证工具实际注册、开关、会话权限与参数，并不在插件 Gate 里面递归调用工具（否则单并发导致死锁）。
