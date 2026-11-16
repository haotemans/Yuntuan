# QQ空间：说说发布与回执

[插件索引](README.md) · [扩展宿主](../host.md) · [项目首页](../../../../README.md)

**状态：实现入口与测试已补充，真实发布需要部署后以实际 tid 验收。** 不能把插件已加载、QQ已连接或 mock 回执当作真实说说已经发布。

## 入口与配置

编译期插件 `qzone` 注册 `qzone.publish`、`qzone.status`、`qzone.history` 三组同名 Tool/Skill。独立配置页为 `/plugins/qzone`，复用公共插件表单、启停、会话范围、能力开关和调用统计；不另设模型、登录 Cookie 或私有调度器。新安装默认未启用，沿用宿主启停规则。

| 入口 | 行为 |
| --- | --- |
| `/说说 正文`、`/发说说 正文` | 使用默认可见范围发布纯文字 |
| `/说说 --公开/--好友/--私密 正文` | 指定本次可见范围 |
| `/空间状态` | 无参数查询 NapCat 连接与当前 QQ，不代表已验证空间发布 |
| `/空间记录` | 无参数查看当前账号最近10条发布结果；群中隐藏私密说说正文 |
| 自然语言 / Agent | 由已注册 Skill/Tool 调用同一个 Publisher；只有明确发布请求才可执行，草稿与润色不等于发布 |

`default_visibility` 默认 `friends`，可选 `public/friends/private`；`daily_limit` 默认10，范围1–100，按北京时间计日，待核实的发布尝试也计入额度。可见范围映射 NapCat `ugc_right=1/4/64`，不是其他 SDK 的枚举值。

Tool 参数为 `content`（最多2000字）、可选 `images`（最多9张）和可选 `visibility`。纯图片时正文可为空，但正文与图片不能同时为空。图片必须为云团 `memes/` 或 `plugins/<id>/artifacts/` 中已存在的本地 PNG/JPEG/GIF/WebP，单张最多8MiB、合计24MiB；宿主验证真实路径与文件头，在边界内转换为 base64。两个 Docker 容器无须共享本地路径，不接受任意 URL、file URL、base64 参数或配置文件。

## 权限与执行边界

仅 `security.super_admin_qq` 指定的真实发言者可操作本机器人空间；未配置则拒绝，群管理员身份不会自动获得账号级发布权。ToolCtx 的作者来自宿主，模型参数不能指定账号或操作人。发布绑定预检时的 QQ，适配器在发送前重新核对登录号。插件范围、Skill 开关及当前选中 Tool 的持久开关/类型化会话范围/参数共同生效。

Skill 委托 Publisher 前调用 `HostServices::authorize_tool_for_skill`；这只检查已注册 Tool，不递归进入同一插件的调用 Gate，故单并发也可正常运行。状态/记录 Skill 显式声明支持空参数；该声明不绕过权限。共享 API 与独立页面机制见[扩展宿主](../host.md)。

适配器复用已有 `NapcatSender`，只调用 `get_version_info`、`get_login_info` 和 `send_qzone_msg`。本次核对的服务器 NapCat bundle 包含该发布接口及上述参数；未安装该接口的其他版本不能据此认为兼容。QQ空间调用不是腾讯公开开放平台 API，也没有把第三方 Skill 冒充腾讯官方 Skill；[`SKILL.md`](../../../../plugins/qzone/SKILL.md) 是云团自己的操作契约，随编译后的 Skill 注册提供。

只有带合法 `tid` 的成功回执才记为发布成功。缺失回执、传输异常或取消后保留未确认状态，不自动重试；API 错误正文、Cookie、Token 和图片 base64 不进入返回信息。发布请求使用90秒回执等待，总插件与工具预算默认120秒。

## 持久化与去重

复用宿主隔离的 `read_data/write_data`，在 `data/plugins/qzone/kv-publications.json` 先原子写入 `sending` 占位，再执行外部发布。保留账号、请求摘要、可见范围、图片数量、时间、状态和成功回执；不复制图片或保存凭据。Tool/Skill 使用同一串行 Publisher，避免命令与 Agent 同时发送重复内容。

同一任务同一参数，或同一天相同的直接命令，成功后返回原回执；未确认的相同内容另有跨任务指纹，重启、换任务或跨日都不能直接重发。已知前置校验拒绝可修正后重试。成功但最终记录保存失败时仍返回真实 tid 并标记 `receipt_recorded=false`，原 `sending` 占位阻止重放。

最多保留200条记录，只淘汰较早日期的已完成/前置拒绝记录；不静默清除待核实记录。额度和记录容量满时拒绝新发布。记录核实目前需要管理员人工查看实际 QQ空间；没有自动读取说说列表、自动修复未知结果、删除、评论或定时发布。未来自动化必须走云团共享持久调度及单独的外部写操作授权。

## 对应实现

- [`plugins/qzone/src/lib.rs`](../../../../plugins/qzone/src/lib.rs)：插件元数据、配置和注册。
- [`plugins/qzone/src/tools.rs`](../../../../plugins/qzone/src/tools.rs)：Tool/Skill、命令解析、统一权限检查。
- [`plugins/qzone/src/publisher.rs`](../../../../plugins/qzone/src/publisher.rs)：持久回执、额度、去重与结果呈现。
- [`crates/core/src/plugins/qzone.rs`](../../../../crates/core/src/plugins/qzone.rs)：宿主能力与安全图片转换边界。
- [`crates/adapter-qq/src/qzone.rs`](../../../../crates/adapter-qq/src/qzone.rs)：账号权限、原生动作和回执。

共享类型由 [`crates/plugin-api/src/qzone.rs`](../../../../crates/plugin-api/src/qzone.rs) 提供；独立页面使用 [`PluginSettings.vue`](../../../../webui-frontend/src/views/PluginSettings.vue)。

## 验证入口

- [`plugins/qzone/src/tests.rs`](../../../../plugins/qzone/src/tests.rs)：混合入口去重、权限、额度、重启、取消、缺失回执与落盘失败。
- [`crates/core/tests/qzone_host.rs`](../../../../crates/core/tests/qzone_host.rs)：真实插件宿主、工具开关/会话范围、单并发不死锁；连接器为 fake。
- 宿主/适配器模块内测试：图片边界、容器传输、账号、可见范围与回执；[`client.rs`](../../../../crates/adapter-qq/src/client.rs) 验证发布等待预算。
- [`bot.rs`](../../../../crates/core/src/bot.rs) 的命令到发送回归包含无参数查询；[`plugins-flow.spec.js`](../../../../webui-frontend/tests/plugins-flow.spec.js) 验证独立配置页、参数元数据、权限开关及配置保存。

上述自动化不向真实 QQ空间发布内容；验收结果按当前提交的实际 CI 与部署报告分别记录。
