# 鸿雁：QQ 平台能力与兼容扩展

[返回插件索引](README.md) · [返回功能索引](../../README.md) · [返回文档总索引](../../../README.md)

**当前状态：代码入口已实现（基于 `ProtocolGateway` 单元测试通过，真实 NapCat/QQ 环境验收待完成）**。

> 将 QQ 平台专属交互与兼容逻辑（群/成员信息、精华消息、戳一戳、群公告）收口为独立插件，通过核心 `onebot` 协议网关调用。

## 实际行为

- **纯协议网关调用**：插件通过 `HostServices::call_protocol(ctx, "onebot", ...)` 调用核心 OneBot 后端，不直接依赖 `adapter-qq` 或 `core`。
- **通道边界**：仅在 `ctx.inferred_platform() == "onebot"` 的会话中可用；QQ 官方通道或非群聊（针对群专属能力）会自动拒绝或隐藏。
- **能力清单**：
  - `qq.group_info` / `/群信息`：查询当前 QQ 群信息（`get_group_info`）。
  - `qq.member_info` / `/群成员 [QQ号]`：查询当前群成员信息（`get_group_member_info`）。
  - `qq.user_info`：查询指定 QQ 号公开资料（`get_stranger_info`）。
  - `qq.essence_list` / `/精华消息`：查询当前群精华消息（`get_essence_msg_list`）。
  - `qq.poke` / `/戳一戳 [QQ号]`：发送 QQ 戳一戳（`send_poke`），受配置项 `allow_poke` 控制。
  - `qq.group_notice`：发布群公告（`_send_group_notice`），默认关闭，须同时开启 `allow_owner_notice` 且通过 `HostServices::is_account_owner(ctx)` 超级管理员校验。

## 边界

- 所有调用继续受宿主 `PluginManager` 的启停、会话白名单（`allowed_chats`）、能力开关与参数 Schema 校验约束。
- 该插件默认未启用，需在管理面板中开启或配置后生效。

## 对应实现（本页负责的主源码）

- [`plugins/qq/src/lib.rs`](../../../../plugins/qq/src/lib.rs)
- [`plugins/qq/src/tools.rs`](../../../../plugins/qq/src/tools.rs)

## 验证入口

- [`plugins/qq/src/tests.rs`](../../../../plugins/qq/src/tests.rs)
