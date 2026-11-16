# 腾讯文档连接、读取与受控编辑

[插件索引](README.md) · [扩展宿主](../host.md) · [操作契约](../../../../plugins/tencent-docs/SKILL.md)

已实现官方 HTTP MCP 连接及 Tool/Skill 入口。自动化使用临时 HTTP 服务和 fake 账号接口；没有真实 Token 时，不将构建、插件加载或模拟回执当作实服验收。

## 入口与执行

- `tencent-docs.catalog`：初始化连接，查询实时工具目录与输入 Schema，属于只读工具。
- `tencent-docs.read`：仅调用目录中确认只读的搜索、读取操作，拒绝写入。
- `tencent-docs.call`：创建、编辑入口，声明 ExternalWrite；管理员开启 `allow_writes` 后仍只允许私聊。删除及破坏性操作始终拒绝，不能用只读注解覆盖删除限制。
- `tencent-docs.assist` 与 `/文档`、`/docs` 共用 Connector。`/文档 工具` 无需模型即可检查目录；普通指令选择一步工具，先检查 catalog，再检查选中的 read/call 开关和会话范围。复杂多步任务由 Agent 使用同一组工具推进。

参数 `{name,arguments}` 必须匹配实际目录；使用宿主共享的有界 Schema 子集做前置校验，完整标准仍由官方服务校验，不猜文件 ID。读取结果原文进入返回数据及有界命令摘要，不能只回复“调用完成”而丢掉文档内容。

## 配置与授权

固定端点 `https://docs.qq.com/openapi/mcp`，Token 通过 `Authorization` 请求头发送；禁止自定义域名、重定向及将凭据放在 URL。配置页 `/plugins/tencent-docs` 复用公共表单与能力权限，新增插件默认关闭。

先在[腾讯文档授权页](https://docs.qq.com/open/auth/mcp.html)获取个人 Token，再选择一种来源：

1. 在运行云团的进程/容器设置 `TENCENT_DOCS_TOKEN`，插件 `token_env` 填该变量名。必须把变量实际传入容器，宿主 shell 中存在不等于容器可读取。
2. `token_env` 留空时，可使用面板的 `token` secret 字段。沿用已有存储与掩码，不声称已引入加密 Vault。

一旦指定 `token_env`，变量缺失或为空会明确失败，不静默回退到旧 Token、误连另一个账号。凭据不发送到 QQ 或模型。配置保存并启用后，“检查连接”通过已注册 `tencent-docs.catalog` 实际取目录；成功不等于某个文档已读取或已编辑。系统检查域为 `private:plugin-check`，有限会话范围需显式允许该域。

## 传输与失败

共享 [`crates/mcp-http/src/lib.rs`](../../../../crates/mcp-http/src/lib.rs) 完成 initialize、initialized、Session、目录分页及 JSON/SSE 返回。SSE 等完整事件并匹配 JSON-RPC id；响应流最多1MiB，目录最多200项/8页，业务返回最多48KB。无无限流读取、自动重连写入或重试。

写入后的传输错误、缺失回执或超大回执返回 `ResultUnknown`，需核对原文档；不当作“未执行”继续 loop。服务端错误只返回错误码，不直接回显可能含凭据的错误正文。此客户端只支持有界单次结果，不实现长期订阅或反向服务端工具请求；会话失效时重新应用插件连接配置。

官方契约见[腾讯文档团队 MCP 指南](https://developer.cloud.tencent.com.cn/mcp/server/11803)，实际工具和完整参数以实时 `tools/list` 为准。

## 源码与验证

- [`plugins/tencent-docs/src/lib.rs`](../../../../plugins/tencent-docs/src/lib.rs)：配置、动态目录、读写分路、命令/聊天/Agent 共享入口。
- [`plugins/tencent-docs/src/tests.rs`](../../../../plugins/tencent-docs/src/tests.rs)：假接口校验参数、写权限、删除限制、Skill 工具门控、未知结果不重试及原文输出。
- [`crates/mcp-http/src/tests.rs`](../../../../crates/mcp-http/src/tests.rs)：真实 loopback HTTP 握手、Session、分页、SSE、ID 关联和凭据错误脱敏；不连接真实腾讯账号。
- [`webui-frontend/tests/plugins-flow.spec.js`](../../../../webui-frontend/tests/plugins-flow.spec.js)：连接配置、secret 保留、先保存后检查。
- [`crates/webui/tests/pages_api.rs`](../../../../crates/webui/tests/pages_api.rs)：认证、只读探针、真实注册表及持久工具开关。

验证命令：`cargo test -p yuantuan-plugin-tencent-docs -p yuantuan-mcp-http`。当前提交的 CI 和实服结果分别报告；源码、功能映射及此文档须同步维护。
