# Plugin、Skill 与 Tool

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：编译期插件已实现（动态安装未实现）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 注册表、编译期插件装配、热配置、作用域与调用限制。

## 实际行为

- `Tool` 是结构化执行能力；`Skill` 是 Rust trait 定义的技能入口；`Plugin` 把 Tool/Skill 打包给宿主，管理归属、会话范围、超时、并发和输出限制。
- 当前插件在构建时编译进程序，面板支持列出、配置和启停已编译插件；把任意插件文件丢进目录不会自动安装。
- `SKILL.md` 是插件的参考或使用契约，不代表运行时会自动解析并安装 Markdown Skill。

好友与入群申请通过受限 `HostServices.membership` 服务提供，仅主理人插件且已声明管理能力可用；宿主传递已应用的配置和会话范围，适配层按真实申请核验权限。共享申请动作与群规则类型在 [`membership.rs`](../../../crates/plugin-api/src/membership.rs)，业务约束见[主理人](plugins/moderator.md)。

## 边界

- 编译进程序的第三方 Rust 插件是受信任本地代码，不是进程级隔离；动态插件市场、面板上传编译属于规划。
- 开发插件时参照[插件接口 README](../../../plugins/README.md)和[插件层记录](../../changes/plugins-layer-workbench.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/plugins.rs`](../../../crates/core/src/plugins.rs)
- [`crates/core/src/plugins/invocation.rs`](../../../crates/core/src/plugins/invocation.rs)
- [`crates/core/src/skills.rs`](../../../crates/core/src/skills.rs)
- [`crates/plugin-api/src/lib.rs`](../../../crates/plugin-api/src/lib.rs)
- [`crates/plugin-api/src/membership.rs`](../../../crates/plugin-api/src/membership.rs)
- [`crates/plugin-api/src/skills.rs`](../../../crates/plugin-api/src/skills.rs)
- [`crates/plugin-api/src/tools.rs`](../../../crates/plugin-api/src/tools.rs)
- [`crates/webui/src/plugins_api.rs`](../../../crates/webui/src/plugins_api.rs)
- [`webui-frontend/src/views/Plugins.vue`](../../../webui-frontend/src/views/Plugins.vue)
- [`webui-frontend/src/views/PluginSettings.vue`](../../../webui-frontend/src/views/PluginSettings.vue)

## 验证入口

- [`crates/core/tests/plugin_host.rs`](../../../crates/core/tests/plugin_host.rs)
- [`crates/core/tests/plugin_attachments.rs`](../../../crates/core/tests/plugin_attachments.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

V12增加SQLite能力目录与技能调用记录；能力启停和类型化会话范围叠加在插件自身权限之上，不能凭元数据创建可执行代码。模型/命令调用技能前登记来源消息，同一来源不重放副作用。管理页提供技能开关及持久调用数，旧插件计数仍表示本次服务启动后的统计。工具参数检查覆盖基础type/required/enum/properties/items/minimum/maximum，复杂Schema和业务权限仍由插件校验。源码 [`capabilities.rs`](../../../crates/core/src/capabilities.rs)。

Decision和Chat能力目录也读取持久启停/范围，避免把已禁用能力继续推荐给模型。

回放/测试合成消息的非正数编号不作为真实来源幂等键；真实消息编号仍确保同一技能不重复执行。


V18在plugin-api的Tool trait追加有默认实现的available_in_chat、policy和output_schema，并增加ToolDefinition、ToolCall、ToolResult、ToolError和ToolProtocol等共享类型。旧插件无需同时改写即可编译，默认效果未知、120秒、取消后等待结果；这是源码接口，不宣称动态ABI兼容。HostedTool转发可用性、契约、附件保留和audit_details；插件停止/范围拒绝/繁忙返回类型化前置错误，超时/panic保留错误种类并脱敏，不再只剩无类型字符串。宿主仍保留插件自身限流、输出/路径限制与secret redaction。

Agent任务统一执行和回执规则以[任务功能](../core/tasks.md#v18-统一工具执行接口)为准。Skill仍保留技能匹配、权限和来源幂等入口，共享ToolOutput，不因接口统一改变其调度职责；管理面板手工MCP诊断仍为已有原始stdio请求，不冒充Agent任务回执。SQLite工具契约仅描述实际注册实现，不凭一条数据库记录生成执行器。


工具调用易用性补充：基础Schema检查移到[`schema.rs`](../../../crates/core/src/capabilities/schema.rs)，错误包含JSON Pointer字段路径和期望类型/范围，不回显用户实参。支持type（含类型数组）、required、enum、properties、additionalProperties、items、min/maxItems、min/maxLength及minimum/maximum；递归检查限32层，$ref/组合Schema等完整标准仍不支持，业务校验继续由Tool负责。回归见[`tool_usability.rs`](../../../crates/core/tests/tool_usability.rs)。

插件页“调用状态→能力与存储信息”增加Tool独立开关、持久调用计数与折叠输入Schema；同名Skill和Tool按kind分别控制。切换任一能力都携带原scopes，避免开关操作清空会话范围。这里只开关已注册实现，不绕过插件总开关、业务权限或底层依赖。浏览器回归[`plugins-flow.spec.js`](../../../webui-frontend/tests/plugins-flow.spec.js)验证Tool/Skill独立、范围保留及参数查看，加入Linux前端CI。


## 独立插件详情页面与官方 Skill 配合

管理端现以 `/plugins/:name` 提供插件独立页面，包含概览、连接配置、Skills/Tools 和运行状态。页面仍复用 `/api/plugins/list`、`/api/plugins/config`、`/api/plugins/toggle` 与 `/api/capabilities`，没有新增插件专用配置数据库或上传式动态安装。

首批服务连接见 [腾讯文档](plugins/tencent-docs.md) 和 [夸克网盘](plugins/quark-drive.md)。两者只把官方能力包装成 Tool / Skill；宿主统一做会话范围、参数门控和日志。当前原生 Plugin 是编译期依赖，`SKILL.md` 被编译为使用说明而不是可在生产中从外部目录任意安装的 Skill 包。


为了避免 Skill 直接调用共享 Connector 时绕过 Tool 开关，V20 新增 `HostServices::authorize_tool_for_skill(ctx,name,args)`：宿主校验该 Tool 属于当前插件、确实在 Registry 注册、对会话开放，并复用已有持久 `capabilities::check_tool` 对启停、作用域及 JSON 参数做检查。Skill 只能在授权通过后调用对应连接器；避免重入 HostedTool 导致单并发插件死锁。此检查是预执行门控，外部调用仍由插件自身负责写权限与平台返回验证。
