# Plugin、Skill 与 Tool

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：编译期插件已实现（动态安装未实现）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 注册表、编译期插件装配、热配置、作用域与调用限制。

附件发送失败同时记录发送结果与 `RuntimeFailure` 诊断事件。附件回归分别核对两类事件的内容，详见[运行自检](../core/diagnostics.md)。

## 实际行为

管理员提交插件配置时，敏感配置值同步登记到[运行日志](../runtime/console.md)的脱敏器，再交给现有配置应用流程。

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


工具调用易用性补充：基础Schema检查移到[`schema.rs`](../../../crates/plugin-api/src/schema.rs)，错误包含JSON Pointer字段路径和期望类型/范围，不回显用户实参。支持type（含类型数组）、required、enum、properties、additionalProperties、items、min/maxItems、min/maxLength及minimum/maximum；递归检查限32层，$ref/组合Schema等完整标准仍不支持，业务校验继续由Tool负责。回归见[`tool_usability.rs`](../../../crates/core/tests/tool_usability.rs)。

插件详情页「Skills / Tools」显示 Tool 独立开关、持久调用计数和可展开的输入 Schema。同名 Skill 和 Tool 以 kind 区分，启停操作保留原会话范围。浏览器回归 [`plugins-flow.spec.js`](../../../webui-frontend/tests/plugins-flow.spec.js) 覆盖开关与参数查看，纳入 Linux 前端 CI。

插件详情页「Skills / Tools」可编辑每个能力的会话范围。选择「仅指定会话」后逐行输入带类型的会话 ID，例如 `group:123456` 或 `private:dm_123456`。明确选择「全部会话」会清空这一层的限制，实际调用仍同时检查插件范围、能力启停和业务权限。Skill 与同名 Tool 独立保存。保存失败会保留编辑内容。

面板使用已认证的 `PATCH /api/capabilities` 分别提交 `enabled` 或 `scopes`，SQLite 原子更新提供的字段，因此修改范围和开关相互独立。旧 `POST` 请求格式保留。回归见 [`plugins-flow.spec.js`](../../../webui-frontend/tests/plugins-flow.spec.js) 和 [`pages_api.rs`](../../../crates/webui/tests/pages_api.rs)。


## 独立配置页与共享执行权限

插件中心导航到 `/plugins/:name`，通用 [`PluginSettings.vue`](../../../webui-frontend/src/views/PluginSettings.vue) 承载概览、配置、Skills/Tools和运行状态；不为每个插件复制表单、启停或授权存储。保留工具参数契约、各类型独立开关、原会话范围及调用数；加载状态不等于账号连通或写入成功。页面来自同仓库独立插件页公共实现，新增插件由真实编译目录提供元数据，没有安装的服务不伪造为可用。

插件列表和详情复用通用页面样式以及同一套启用、应用中、待处理、需重启状态。配置页提供账号授权、检查连接和设置操作；保存期间展示实际加载状态，异步回读以最新请求为准。未保存的草稿保留，当前页启停与连接检查需要先处理草稿。

[`PluginPreflight.vue`](../../../webui-frontend/src/components/PluginPreflight.vue) 只读预检指定群聊或私聊的已注册能力。逐项根据实际配置查看插件范围、能力开关、能力范围及宿主声明的 OneBot 和群聊要求。使用 OneBot 的插件还读取实时连接状态。预检确认的是会话配置，角色、账号和参数仍由正常执行路径核验。

[`PluginHistory.vue`](../../../webui-frontend/src/components/PluginHistory.vue) 分开显示在详情页的持久历史，由单独的本次启动计数作为补充。认证后的 `GET /api/plugins/{name}/history` 按插件前缀和可选时间范围读取最多 50 条已登记 Skill 与 Agent Tool 调用，包括会话、来源消息、任务 ID、状态和耗时。参数、响应正文和密钥不出现在历史响应中。面板以浏览器所在时区按日期筛选，结果待核实的调用单独标记。历史覆盖已持久化的调用入口，连接探针等进程内操作继续计入运行状态。

`HostServices::authorize_tool_for_skill` 核验本插件已注册 Tool、可用性、SQLite 持久开关、类型化会话范围和最终参数，随后 Skill 委托共享业务连接器；不递归进入同插件 Tool Gate，以免单并发死锁。QQ空间在每个实际操作前采用此入口。它是可复用的委托接口，不是加载任意SKILL.md就能获得执行权。

`Skill::accepts_empty_command` 默认false；状态/列表类 Skill 可显式声明接受无参命令，宿主包装透传此标记。它只解除公共空参数提示，Skill/Tool权限与业务校验仍执行。SDK共享QQ空间动作见 [`qzone.rs`](../../../crates/plugin-api/src/qzone.rs)；仅qzone插件获得对应宿主服务，账号发布边界由适配器核验，详见[QQ空间](plugins/qzone.md)。

回归见 [`qzone_host.rs`](../../../crates/core/tests/qzone_host.rs)、[`plugins-flow.spec.js`](../../../webui-frontend/tests/plugins-flow.spec.js) 与会话命令测试。独立页面的只读运行统计继续沿用原接口，不声明为完整的历史审计库；QQ空间发布的持久回执由该插件复用宿主数据服务保存。


## 服务连接检查与共享 Schema

腾讯文档、夸克与QQ空间复用上述页面及授权入口，分别见[腾讯文档](plugins/tencent-docs.md)、[夸克](plugins/quark-drive.md)和[QQ空间](plugins/qzone.md)。通用操作契约是源码内注册的 Skill，外部 SKILL.md 不会自行变成执行器。

新增认证接口 `POST /api/plugins/connection/check {name}` 只允许腾讯文档 catalog、夸克 account 两个已注册只读探针。它通过原 ToolRegistry、HostedTool、SQLite 能力开关及参数验证后调用连接器，使用明确系统域 `private:plugin-check`；没有伪装成群成员绕开会话范围，也不把加载成功当作账号连接成功。页面有未保存更改时不能检查旧配置。验证见 [`pages_api.rs`](../../../crates/webui/tests/pages_api.rs) 及上述浏览器测试。

公共 Schema 校验器已移入 `crates/plugin-api/src/schema.rs`，通过 `validate_tool_arguments` 提供给宿主和连接器；原 `yuantuan_core::capabilities::validate` 继续重导出，保留旧调用接口。仍只支持本页列出的有界子集，完整远端 Schema 由服务端校验，不另加一个不一致的实现。


账号连接权限复用主配置 `security.super_admin_qq`，由启动装配传入 PluginManager 的共享原子槽，插件重载保持同一身份；不新增每插件 owner 表单。`HostServices::account_owner_qq` 和 [`crates/plugin-api/src/account.rs`](../../../crates/plugin-api/src/account.rs) 的 `require_account_access` 同时约束服务 Tool 可见性、Skill 与实际 Connector，普通群成员/私聊用户不能借用管理员账号读写。未配置管理员则拒绝对话操作。`private:plugin-check`/webui 与 `private:scheduled`/scheduler 仅作为无 task_id 的宿主只读身份开放，绝不放行写入，且仍检查原会话范围/能力开关；模型不能从参数指定这些身份。身份配置随应用启动加载，修改后需重启应用。账号策略单元测试与两插件的非本人零调用测试分别验证此边界。


服务插件的展示名采用与采薇、采苓、敬之、琼华一致的两字雅名：子衿（腾讯文档）、卷耳（夸克网盘）、鹿鸣（QQ空间）。卡片和详情页均读取宿主 display_name，副标题说明平台及用途；内部注册 ID、配置和权限不随显示名更换。现有插件页面回归沿真实显示名检查跳转、配置与调用入口。


## 账号连接向导

[`ConnectorAuth.vue`](../../../webui-frontend/src/components/ConnectorAuth.vue) 和 [`connector_auth.rs`](../../../crates/webui/src/connector_auth.rs) 提供独立于聊天工具的管理员连接向导：`GET/POST /api/plugins/auth/{name}` 查询/发起，`POST .../complete` 私密回填。仅接受 `quark-drive`、`tencent-docs` 且必须已编译注册，沿用全部 API 的 Bearer 会话检查；无公共授权回调、任意命令或任意远端地址入口。

每插件仅一项有界后台操作，等待流程绑定发起会话并在10分钟后失效。状态仅返回阶段、经过限制的官方授权链接和脱敏结果；不返回凭据、CLI 原始输出或授权码。配置变更使用 PluginManager 同一更新锁内的字段补丁：不根据掩码目录重建设置，保留旧密钥、会话范围和限额，拒绝未知配置键，授权成功才启用。该入口本身不发 QQ 消息、不扩大群成员权限；现有聊天/Agent 执行仍检查超级管理员和能力范围。

卷耳的组件与 OAuth 细节见[夸克文档](plugins/quark-drive.md)，子衿的个人 Token 细节见[腾讯文档](plugins/tencent-docs.md)。自动化覆盖 Node fixture、真实 WebUI 认证/回填/防重复及前端模拟流程；没有真实账号同意时不声称实服连接成功。


卷耳增加独立工具运行层传输，仍复用现有 Tool/Skill 权限，管理页 `unsupported` 阶段区分官方运行环境拒绝与 OAuth 失败。直连和运行服务不能混用登录目录；服务存在时所有操作共用同一执行入口。真实 OAuth 链接验证、工作服务与票据边界详见[卷耳](plugins/quark-drive.md#linux-独立工具服务)。
