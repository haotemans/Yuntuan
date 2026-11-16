---
name: quark-drive
description: Use when the user asks to search their Quark cloud drive, inspect sharing links, check updates to previously saved shared files, or create a new authorized share via the official Quark CLI.
---

# 夸克网盘

执行器必须是安装在云团服务器上、由管理员配置绝对路径的夸克官方 Skill CLI。官方项目：https://github.com/quark-clouddrive/quarkclouddrive_offical 。认证使用官方 OAuth，凭据由官方 CLI 管理；不要将 Cookie 或 Token 交给模型。

1. 查询网盘文件：`quark-drive.search`，关键词保留用户文件名及类型；无结果不能擅自改词重复搜。
2. 检查指定分享是否可访问：`quark-drive.inspect_share`，只能依据实际返回状态区分正常、失败、疑似到期。不要根据一次网络错误判断被封禁。
3. 查询以前转存的分享是否有新增：`quark-drive.share_updates` 和 `quark-drive.updated_files`；这些能力不等同于检查本人分享的到期情况。
4. 用户明确要分享自己的文件时才使用 `quark-drive.create_share`，必须使用真实 FID，管理员需开启创建分享，且仅限私聊。
5. 遇到官方平台限制、违规状态或异常，不自动通过换链规避；只通知用户并保留操作记录。
6. 执行结果中若存在 `notice.text`，在业务输出后原样附带官方提示；分享链接与提取码来自真实 `data`，不能编造。
7. 每次执行读写都使用官方 CLI，不允许 Agent 构造 shell 命令。官方 CLI 需要安装、登录授权、手动更新与版本验证；云团不在后台静默运行 install.sh。

官方操作规范见该项目 `skills/quarkclouddrive/SKILL.md` 及 `references/file-search.md`、`file-share.md`，当前插件只支持已显式映射的命令。动态官方更新需重新核对兼容性。
