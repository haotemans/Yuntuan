# 启动、监督与部署

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（部署依赖现场环境）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 程序装配、服务生命周期、容器部署和停机安全。

## 实际行为

- `yuantuan` 包仍是运行时二进制名称；攸宁是项目名称。入口依次加载配置、数据迁移、插件、QQ、任务、定时归纳和 WebUI。
- `Supervisor` 管理长期后台任务的异常与重启；数据库与外部服务的关闭顺序有明确边界。
- 发布包内嵌 Vue 构建产物；常规 Linux 测试和构建由 GitHub Actions 完成，Windows 在本地验证。Windows Actions 打包仅在 `v*` 发布标签触发时执行。

启动时先安装内置云团图包，再扫描管理员图库；注册采集监听和限速视觉审核后台任务，复用可热更新的 LLM/steal 槽位。审核失败保持待审，不阻塞消息管线；图包安装标记保留管理员后续删除和分类调整。

Linux CI 在全量工作区测试和发布构建前，先运行主理人命令、QQ审核权限与申请队列迁移的针对性回归；复用现有任务及编译缓存，不新增并行打包流水线。

运行时与管理API复用核心归纳配置类型，旧配置缺少画像限额字段时沿用默认值；配置加载和管理页写入均校验时刻、人数、间隔及会话批次预算。夜间画像与群聊归纳共用一个可取消调度句柄，具体规则见[记忆功能](../core/memory.md)。

## 边界

- 更新攸宁时必须保留 SQLite、密钥、插件配置和 NapCat 账号登录状态；**不要顺手重启/重建 NapCat**。
- 部署指南见[项目 README](../../../README.md)和[运行时设计](../../design/runtime-design.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/lib.rs`](../../../crates/core/src/lib.rs)
- [`crates/core/src/supervisor.rs`](../../../crates/core/src/supervisor.rs)
- [`crates/yuantuan/src/config.rs`](../../../crates/yuantuan/src/config.rs)
- [`crates/yuantuan/src/db.rs`](../../../crates/yuantuan/src/db.rs)
- [`crates/yuantuan/src/main.rs`](../../../crates/yuantuan/src/main.rs)
- [`crates/yuantuan/src/shutdown.rs`](../../../crates/yuantuan/src/shutdown.rs)

## 验证入口

- [`crates/core/tests/sqlx_storage.rs`](../../../crates/core/tests/sqlx_storage.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

V12启动保持现有服务装配，Agent默认2个执行槽；任务回聊由pipeline持有的后台worker处理，随pipeline取消。主进程用量sink保存供应商可用的缓存统计，未知值保持NULL。

消息恢复不再在启动阶段逐条等待模型：只修复持久处理状态，由消息管线每30秒补读。受监督管线重启时同样恢复推理状态，执行状态不明保持uncertain。

V14迁移回归同步最新版本断言，继续验证已有申请、记忆、消息和运行配置保留；消息失败边界见[会话调度](../core/conversation.md)。

核心导出working_context模块，启动迁移新增V16工作记忆表；现有画像、群记忆、任务、Provider和插件配置不被模板覆盖。Linux CI新增工作记忆和回复时效针对性回归，再运行既有全工作区测试。部署仍只切换yuantuan应用，NapCat保持原容器与登录态；设计完整性不代表已部署，详见[工作记忆](../core/memory.md)。

核心迁移增加V17任务说明表，Linux CI在全量测试前验证任务说明、版本恢复与管理API；仍保留2个任务执行槽，不改变NapCat、Provider或插件设置。迁移实现不代表已部署，服务器切换需独立请求与现场验收。


启动装配将同一个ToolRegistry同时注入聊天管线与TaskRunner，Router/Chat可了解实际底层工具，仍由Agent执行任务细节。插件热注册/注销共享实例；不额外创建一套工具实现、不改变默认开关或自动启用沙箱。该改动不新增数据库迁移，工具协议和回执继续V18，见[任务功能](../core/tasks.md#工具发现与参数修正)。


启动夜间调度时传递与Router/Chat/任务共用的`llm_slot`；保存Provider或角色配置后，后台会话下一批、人物下一次也读取新配置，避免持续使用启动时模型。凌晨3点、每晚10人、串行60秒间隔仍沿用现有配置；持久覆盖、诊断及同类失败熔断见[记忆功能](../core/memory.md#v19近期增量历史补齐与整理诊断)。这项源码修改不等于目标服务器已更新。


新增公共 `scheduled_tools` 受监督调度协程，使用与插件和 Agent 共用的 ToolRegistry；通过 SQLite 到期扫描和租约恢复，不为夸克另造 Cron。服务关闭时与其它受监督后台组件一同停止。此处只读每日检查，无自动换链；业务契约见[统一每日只读工具调度](../core/scheduler.md)。

编译清单新增默认编入的 `plugin-qzone` feature，装配层将已有适配器和security.super_admin_qq传入受限QQ空间服务；未编译该feature则无对应插件，新运行目录仍沿用插件默认未启用规则。不创建第二个QQ登录会话，应用部署不得随带重启NapCat。具体功能见[QQ空间](../extensions/plugins/qzone.md)。


服务插件默认编入 `plugin-tencent-docs` 和 `plugin-quark-drive`，运行时仍默认关闭；无 Token 或无 CLI/授权时不伪造连接成功。Dockerfile 的 connector-runtime 阶段提供 Node.js 22，CLI 及其 OAuth 配置须由管理员安装到持久 data 路径；仓库镜像不会下载账号凭据或在后台运行官方更新脚本。Docker 构建已补齐全部 workspace manifests、编译所需 SKILL.md 和实际前端构建产物。CI 对 Node 基础运行时做烟测，该烟测不代表完整应用镜像/真实账号联调；应用 Linux 构建和后端测试另行执行。只改仓库不等于更新服务器。

装配在插件加载后、开始接收事件前，将 security.super_admin_qq 写入共享账号身份槽；腾讯文档/夸克的 Connector 和工具可见性均从该槽校验。它是启动配置，变更后需要重启云团；不改变现有 QQ空间适配器的账号检查。
