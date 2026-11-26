# QQ 接入与消息采集

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已接入（需真实 QQ 验收）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> OneBot 11 反向 WebSocket、QQ 官方机器人网关、平台配置与发送回执。

## 实际行为

- NapCat 作为 QQ 协议端，攸宁监听反向 WebSocket；收到消息后解析 text、at、reply、image 段，写入 SQLite 并发布事件。
- 发送者的机器人/系统判定随原消息在同一事务保存，周期补读和重启恢复继续遵守原始过滤结果。
- 图片消息保存原消息与图片来源关联，供定向视觉观察、引用复用与表情采集使用；普通聊天刷图不自动触发定向识图。
- 视觉理解由核心层按请求类型和成本限制调用 BotChat；适配层只提供消息原始关联，保留真实 @ 对象，不能把截图内的昵称当协议身份。
- WebUI 的「平台接入」页按接入通道并列显示 OneBot 11 与 QQ 官方机器人的运行状态和配置。两条通道可独立启用，状态以宿主探针为准。

好友与入群 `request` 事件校验登录账号、申请人及类型后先去重落库，再发布观察事件；不混入聊天消息、画像或普通对话提示。握手期按事件自身账号暂存，后续审核仍绑定真实登录QQ；申请处理和权限约束见[主理人](../extensions/plugins/moderator.md)。

OneBot 在登录握手后接收 NapCat 的 `notice/notify/poke`。事件账号和被戳对象都须匹配已登录的机器人账号，发起者须是其他用户。群聊使用群号和发起人，私聊使用对端 QQ，并核对可用的 `sender_id`。群聊和私聊回应规则见[会话调度](../core/conversation.md)。QQ 官方机器人公开的事件订阅列表未列出戳一戳。参见[官方事件订阅文档](https://bot.q.qq.com/wiki/develop/api-v2/dev-prepare/interface-framework/event-emit.html#事件订阅intents)。

戳一戳回归使用模拟 NapCat WebSocket 和发送回执，覆盖群聊、私聊、其他目标、自身发起和重复通知。真实 QQ 客户端收发仍需验收。

## 边界

- 只声明当前 QQ 接入，桌面、多平台适配器仍属规划；群管理权限检查另见[群管理插件](../extensions/plugins/moderator.md)。
- 普通文件附件仍需 NapCat 能读取路径，库存表情使用 base64；图片结合服务器返回的消息 ID、序号、目标、发送者和媒体段核验，回执不明确时保持待确认，不盲目重复发送。
- 相关实现规则见[信息流参考](../../reference/message-flow-rules.md)和[ADR-0001](../../adr/0001-napcat-as-qq-protocol-endpoint.md)。

## QQ 官方机器人

在 [QQ 开放平台](https://q.qq.com/)创建机器人，并开通单聊及群 @ 事件。将 AppID、AppSecret 填入「平台接入 → QQ 官方机器人」，启用后保存并重启云团。开放平台要求 IP 白名单时，填入运行云团的服务器公网出口 IP。未发布的机器人需使用配置的沙箱成员和群聊。

配置存于主配置文件的 `[qq_official]`，默认关闭。管理面板只显示密钥是否已保存，留空保留原值。云团从 `api.bot.qq.com` 获取令牌和网关地址，主动连接安全 WebSocket，支持新旧官方网关域名。收到 READY 或 RESUMED 后显示已连接；断线时退避并尝试续接。协议见 [腾讯 WebSocket 文档](https://bot.q.qq.com/wiki/develop/api-v2/dev-prepare/event-emit/websocket.html)。

接收 `C2C_MESSAGE_CREATE`、`GROUP_AT_MESSAGE_CREATE`，以及群侧授权后的 `GROUP_MESSAGE_CREATE`。全量事件由官方 `mentions.is_you` 标记判定是否 @ 当前机器人，普通消息按现有群聊策略参与决策和记忆。可识别的图片交给现有视觉链路。回复通过官方 API 被动发送文本，两种场景均在 60 分钟内，默认群聊每条最多 5 次、单聊最多 4 次，暂按标准群额度执行。回复使用原通道、原会话的锚点。不同事件类型重复派发按账号和原始消息 ID 去重，发送序号先持久化，回执不明时保持待确认。引用、主动推送、频道、出站富媒体与群管理留待扩展。详见 [全量群消息](https://bot.q.qq.com/wiki/develop/api-v2/autogen/event/group_message_create.html) 和 [发送规则](https://bot.q.qq.com/wiki/develop/api-v2/server-inter/message/overview.html)。

两条通道使用同一个记忆库。管理员可在「平台接入 → QQ 官方机器人 → 跨通道记忆关联」每行填写 `OpenID = OneBot QQ 号`，保存后重启。配置文件等价字段为 `[qq_official.memory_links]`，QQ 号需用字符串。管理员可从记忆人物列表 API `/api/memories/people` 的 `id` 或 `aliases` 中读取 `qq-official_user_<AppID>_<OpenID>` 对应的 OpenID。确认属于同一人的身份合并展示为一张卡，原人物、会话、回执和权限保留各自的标识。人物记忆的读取规则详见[记忆库](../core/memory.md)。

验证覆盖模拟官方网关及重复事件、SQLite 与模拟回复接口、跨通道历史记忆的范围限制，以及面板两套配置的独立提交。真实账号权限、网络和 QQ 客户端收发仍需在目标环境验收。

## 对应实现（本页负责的主源码）

- [`crates/adapter-qq/src/client.rs`](../../../crates/adapter-qq/src/client.rs)
- [`crates/adapter-qq/src/delivery.rs`](../../../crates/adapter-qq/src/delivery.rs)
- [`crates/adapter-qq/src/ingest.rs`](../../../crates/adapter-qq/src/ingest.rs)
- [`crates/adapter-qq/src/lib.rs`](../../../crates/adapter-qq/src/lib.rs)
- [`crates/adapter-qq/src/official.rs`](../../../crates/adapter-qq/src/official.rs)
- [`crates/adapter-qq/src/official/gateway.rs`](../../../crates/adapter-qq/src/official/gateway.rs)
- [`crates/adapter-qq/src/official/ingest.rs`](../../../crates/adapter-qq/src/official/ingest.rs)
- [`crates/adapter-qq/src/server.rs`](../../../crates/adapter-qq/src/server.rs)
- [`crates/core/src/napcat_slot.rs`](../../../crates/core/src/napcat_slot.rs)
- [`crates/core/src/platform.rs`](../../../crates/core/src/platform.rs)
- [`webui-frontend/src/views/Platform.vue`](../../../webui-frontend/src/views/Platform.vue)

## 验证入口

- [`crates/adapter-qq/tests/ingest.rs`](../../../crates/adapter-qq/tests/ingest.rs)
- [`crates/adapter-qq/tests/reply.rs`](../../../crates/adapter-qq/tests/reply.rs)
- [`crates/adapter-qq/tests/delayed_delivery.rs`](../../../crates/adapter-qq/tests/delayed_delivery.rs)
- [`crates/adapter-qq/src/official/tests.rs`](../../../crates/adapter-qq/src/official/tests.rs)
- [`crates/webui/tests/config_hot.rs`](../../../crates/webui/tests/config_hot.rs)
- [`webui-frontend/tests/platforms-flow.spec.js`](../../../webui-frontend/tests/platforms-flow.spec.js)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。


QQ空间通过装配层注入受限服务并复用现有NapcatSender，不导出QQ登录凭据。`send_qzone_msg` 使用90秒回执等待，兼容图片数组为字符串的原生接口；普通非媒体动作预算不变。账号、图片和发布回执规则见[QQ空间](../extensions/plugins/qzone.md)，不将QQ连接成功当作说说已经发布。

OneBot 传输层通过 [`onebot_gateway`](../../../crates/adapter-qq/src/client.rs) 实现 `ProtocolGateway` 并注册到 `ServiceRegistry`，供平台插件通过宿主统一协议网关调用 OneBot 动作。
