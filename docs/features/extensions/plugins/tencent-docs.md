# 腾讯文档官方 MCP 插件

[独立插件](README.md) · [统一扩展宿主](../host.md) · [腾讯文档插件](../../../../plugins/tencent-docs/README.md)

**当前状态：源码适配与本地单元测试入口已新增；未提供真实腾讯文档 Token，实际 API 未验证。**

## 能力

- 使用官方固定 `https://docs.qq.com/openapi/mcp` 与 `Authorization` Token，经 HTTP MCP 初始化与工具发现。
- 插件包含 `tencent-docs.catalog` 与 `tencent-docs.call` 两种统一 Tool，以及 `tencent-docs.assist` Skill。
- 用户可在 QQ 发送 `/文档 <指令>`、`/docs <指令>`，也可自然语言请求检索和操作。两类入口复用同一 Connector。
- 当前工具列表动态读取 `tools/list`；不依赖硬编码的过期服务 Schema。
- 只读工具允许默认调用；默认不允许写，管理员需开启 `allow_writes` 并在私聊调用；删除类工具始终禁止。

## 授权及安装

在 [官方授权页面](https://docs.qq.com/open/auth/mcp.html) 获取个人 Token，进入 WebUI 插件中心 → 腾讯文档 → 连接配置填写。建议在服务端环境变量设置 `TENCENT_DOCS_TOKEN` 并在插件页填写 `token_env`，不将 Token 明文写入插件配置；仍可使用宿主已有 `token` secret 字段。Token 不应该发送到 QQ 聊天、Skill 文本或日志。固定官方服务地址，防止向自定义域名发送 Token。当前沿用插件宿主的 secret 字段存储和掩码机制，并未引入新的加密凭据库。

官方参考：[腾讯文档 MCP 指南](https://developer.cloud.tencent.com/mcp/server/11803)。当参数和文档有差异时，以实际 `tools/list` 为准。

## 失败边界

- 无凭据、认证错误、网络故障和工具缺失明确失败，不允许冒充成功。
- 写权限是管理员配置开关，不等于逐条确认；复杂编辑应继续利用云团现有任务控制并审查用户意图。
- MCP SSE 当前只支持有界的一次调用结果，不是全功能长连接订阅客户端。
- 仅 mock/静态单元测试，不代表真实账号端到端成功。

## 对应实现

- [`plugins/tencent-docs/src/lib.rs`](../../../../plugins/tencent-docs/src/lib.rs)
- [`plugins/tencent-docs/SKILL.md`](../../../../plugins/tencent-docs/SKILL.md)
- [`crates/mcp-http/src/lib.rs`](../../../../crates/mcp-http/src/lib.rs)

测试：[插件源码内的单元测试](../../../../plugins/tencent-docs/src/lib.rs)。

维护时同步 `docs/features/catalog.json` 和 [`plugins/tencent-docs/README.md`](../../../../plugins/tencent-docs/README.md)。
