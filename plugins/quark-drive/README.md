# 夸克网盘插件

适配 https://github.com/quark-clouddrive/quarkclouddrive_offical 官方 Skill CLI。

先按官方说明在 Linux（或 Windows WSL）安装 Skill、完成 OAuth 登录并验证 Node.js。然后到云团插件中心填写 `scripts/quark-drive.cjs` 的绝对路径、启用插件。不要把账号的 Cookie、Token 填入聊天、群消息或仓库。

已映射官方命令：`get-user-info`、`search`、`share-detail`、`get-share-update-list`、`get-share-update-files`、`share`。工具均采用白名单参数，禁止任意命令执行。写操作默认关闭，且仅限私聊。调用产生的 JSONL 结果返回云团 ToolOutput，复用现有任务检查点与投递。后台不会自动更新 CLI 或取消授权。

`get-share-update-list` 检查的是已转存分享的内容更新，不代表能管理自己创建的所有分享。每日监控和自动续期需要后续接入统一持久任务调度器，并须绑定源文件 FID 和可靠的分享状态接口，不能把检测失败等同于平台封禁。

本插件尚未使用真实夸克账号验证；官方 CLI 版本与环境差异需在部署环境测试。
