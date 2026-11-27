# 模型角色与运行参数

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（外部 Provider 需自配）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 三角色路由、Provider 管理、调用测试、配置热更新。

## 实际行为

面板提交 Provider 配置时，新凭据同步登记到[运行日志](../runtime/console.md)的脱敏器后再进入校验与保存流程，日志读取继续使用管理员会话鉴权。

QQ 官方机器人配置独立于模型服务，主配置写入时会校验 AppID、AppSecret 及跨通道记忆关联，并返回重启要求。更换 AppID 时须移除旧应用的 OpenID 关联。字段与运行状态见 [QQ 接入](../adapter-qq/qq.md)。

- `LlmGateway` 对 `decision`、`bot_chat`、`agent_exec` 分别绑定 Provider/Model；角色职责独立，但可使用同一实际模型。
- 模型服务页支持列出/测试可用服务，运行参数页维护配置；API 写回对 Provider、Prefilter、回复参数、上下文预算和表情开关等执行按节热应用。
- 某些配置（如监听端口、数据目录）需要重启，API 会返回 `requires_restart`；不能概括为所有设置都实时生效。

BotChat 视觉输入统一携带 PNG 像素：定向消息先在应用侧下载并解码，图库审核使用本地缩略图。外部图片请求不携带 Provider 凭据，下载最长8秒、最大8MB，解码限制4096×4096及32MB分配预算，模型输入最长边1024像素、最大2MB（超限再缩至640像素），小图保留原尺寸、不放大。失效链接或无法解码的内容直接失败，不把远程 URL 留给兼容服务猜测。视觉调用复用同一角色的成本闸、并发和用量记录；不支持多模态时记录失败。后台图库审核还受独立每日预算限制。

运行参数页在夜间记忆归纳卡片中提供北京时间、每晚画像人数上限与画像间隔；保存前校验不合法的时刻和限额，保存后取消旧调度并热应用。默认画像人数10、间隔60秒，另有独立会话批次预算默认20，完整执行规则参见[记忆功能](../core/memory.md)。

## 边界

- 未绑定角色或缺少密钥时模型路径不可用；健康检查和 QQ 连接状态不足以证明 AI 回复成功。
- `providers.toml` 中的密钥宜通过环境变量提供；管理接口掩码，不应回显真实密钥。

## 对应实现（本页负责的主源码）

- [`crates/core/src/llm.rs`](../../../crates/core/src/llm.rs)
- [`crates/webui/src/config_api.rs`](../../../crates/webui/src/config_api.rs)
- [`crates/webui/src/llm_test.rs`](../../../crates/webui/src/llm_test.rs)
- [`webui-frontend/src/views/Models.vue`](../../../webui-frontend/src/views/Models.vue)
- [`webui-frontend/src/views/Params.vue`](../../../webui-frontend/src/views/Params.vue)

## 验证入口

- [`crates/webui/tests/config_hot.rs`](../../../crates/webui/tests/config_hot.rs)
- [`crates/webui/tests/llm_test.rs`](../../../crates/webui/tests/llm_test.rs)
- [`webui-frontend/tests/models-flow.spec.js`](../../../webui-frontend/tests/models-flow.spec.js)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

LLM用量记录可空缓存命中/未命中数；支持prompt_cache_hit_tokens、prompt_cache_miss_tokens及prompt_tokens_details.cached_tokens。缺失表示未知，不记为零或宣称命中。模型请求增加300k保守文本预算和8192输出预留；尚未接入供应商模型上下文能力目录。

运行参数从 `/api/decisions/status?limit=0`读取实际普通群聊间隔、即时入口、批次并发与消息容量，展示为只读值；即时队列容量继续使用原配置，保存时不把只读策略伪装成可热调字段。相关记忆字段改为与当前召回行为一致的名称。浏览器验证：[`webui-frontend/tests/trace-flow.spec.js`](../../../webui-frontend/tests/trace-flow.spec.js)。

运行参数只读展示累计失败3次上限与待处理10分钟时限。LlmGateway识别HTTP400/401/403/404/405/410/422为确定性请求/配置错误，暂停后续Decision上游请求；HTTP429及5xx等仍走有限重试。保存模型配置创建新网关，清除暂停状态；旧failed消息不自动重放。不会为修复错误而替换用户选择的模型。

视觉下载与PNG规范化拆为可复用入口，定向识图先查询同会话的精确像素缓存，再决定是否调用bot_chat；观察用auto细节，采集分类继续low细节，共享原成本闸、并发与用量统计。三角色配置不变，Decision消费结构化图片文字。兼容服务返回纯文字时的有界封装见[记忆功能](../core/memory.md)。

定向识图输出预算单独设为4096，普通Chat/Decision与图库分类原预算保持；视觉辅助上下文有界裁剪，角色、Provider、思考强度与共享调用闸不变。下载原字节可供宿主校验公开登记图片，命中时不再调用视觉模型；新回归核对请求预算，具体行为见[记忆功能](../core/memory.md)。


LLM网关的`LLM请求计时`日志区分成本闸/并发等待`queue_ms`和上游请求/响应读取`request_ms`，附角色、HTTP状态（或传输失败）及输入字节数，不记录密钥、提示正文、服务地址或响应正文。它用于判断回复慢在哪一步，不改变三角色配置、并发与调用预算；模型实测与真实QQ端到端时延需分别记录。


Agent模型绑定追加tool_protocol，可选json（缺省，兼容旧配置）和native（Chat Completions tool_calls）。模型页只在agent_exec卡片显示选择器，保存/回读/测试签名均核对协议；Decision与Chat不能开启native。原生请求声明当前可见工具、tool_choice=auto与parallel_tool_calls=false，不设置json_object；JSON兼容请求、Decision和Chat继续tool_choice=none。两种Agent路径共用原HTTP、成本闸、并发、思考强度和用量统计，不暗中切换模型、协议或权限。已开始任务沿用启动时获取的网关，后续任务使用热加载后的配置。

“保存并测试”仍是无工具的最短连通请求，返回实际生效协议元数据以核对配置；成功不证明Provider支持原生工具执行，页面明确提示此边界。完整执行需独立任务验证，见[统一执行接口](../core/tasks.md#v18-统一工具执行接口)。额外配置API回归见[`pages_api.rs`](../../../crates/webui/tests/pages_api.rs)，浏览器覆盖协议保存、刷新和回读不一致时拒绝测试。


模型热更新同时供夜间整理使用：[`config_api.rs`](../../../crates/webui/src/config_api.rs)重建归纳定时器时传递共享`llm_slot`，不再固定当时的网关；会话下一批/人物下一次读取当前Decision绑定，同批格式修复固定模型。该修复不增加模型角色或修改用户配置；测试入口见[记忆功能](../core/memory.md#v19近期增量历史补齐与整理诊断)。

统一配置 API 对 `security.super_admin_qq` 的变更返回 `requires_restart`：服务账号权限在启动时装配给插件，不属于模型 Provider 的热应用范围。原值未变化则不重复提示；回归见 `crates/webui/tests/pages_api.rs` 的 connector_probe_reports_account_owner_changes_as_requiring_restart。此规则不改变角色路由/模型配置已有的热更新。

运行参数页新增 Bash 配置卡片，保存继续合并整份现有配置并提示 sandbox 需重启，不改写 Provider 或模型角色。沙箱状态和预检使用当前进程配置，实际执行规则见[任务工具](../core/tasks.md#管理员定时入口与沙箱状态)。每日 Agent 计划读取现有 agent_exec 热更新槽，缺少模型绑定时界面明确提示、禁止创建；行为见[调度功能](../core/scheduler.md#每日云团-agent-任务v23)。表单与状态回归见[浏览器测试](../../../webui-frontend/tests/agent-schedules.spec.js)。

## 管理入口复用配置保存

机器人管理保存主人 ID 复用配置 API 的解析、校验、密钥保留和需重启响应，不替换整份模板，也不修改 Provider。已有配置重启接口在显式守护模式通过管理服务请求优雅重启；容器不具备该模式时继续拒绝。管理服务不存在时保留原非容器启动兼容路径。具体权限与状态见[机器人管理](../runtime/management.md)。

[`config_api.rs`](../../../crates/webui/src/config_api.rs) 不再硬编码具体通道槽位，而是调用 `Extras.config_hooks`（由启动装配注册），使配置 API 保持通道无关。
