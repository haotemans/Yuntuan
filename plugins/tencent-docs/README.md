# 腾讯文档插件

使用官方 Streamable HTTP MCP。安装后进入云团插件中心为「腾讯文档」输入从 https://docs.qq.com/open/auth/mcp.html 获取的个人 Token。推荐在服务器设置 `TENCENT_DOCS_TOKEN` 环境变量，并把插件的 `token_env` 设为这个变量名，避免将真实 Token 保存为插件明文配置；仍支持使用机密字段直接填写。固定官方服务地址，禁止随意替换以免泄露 Token。

官方操作指南：https://developer.cloud.tencent.com/mcp/server/11803
插件暴露 `tencent-docs.catalog`（当前工具目录）、`tencent-docs.call`（官方工具调用）和 `tencent-docs.assist`（/文档、/docs、自然语言技能）。

默认只开放官方确认的只读操作；编辑文档需在 WebUI 显式开启 allow_writes，且仅允许私聊调用；删除工具永远拒绝。所有操作复用云团的插件宿主、账号配置、能力权限和任务回执。因没有真实账号 Token，接入仅完成源码与 mock 测试，真实 API 尚需在实际部署环境验证。
