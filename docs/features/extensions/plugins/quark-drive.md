# 夸克网盘官方 Skill / CLI 插件

[独立插件](README.md) · [统一扩展宿主](../host.md) · [夸克插件](../../../../plugins/quark-drive/README.md)

**当前状态：Rust CLI 适配与命令白名单已新增；未实际登录夸克账号，真实 SDK / CLI 行为未验证。**

## 能力

- 只封装明确允许的官方 CLI 命令：get-user-info、search、share-detail、get-share-update-list、get-share-update-files、share。
- `quark-drive.assist` Skill 供自然语言和 `/夸克`、`/quark` 命令调用；底层六个 Tool 也提供给统一 Agent 执行。
- 调用 Node.js 的 argv 接口，不以 shell 字符串执行用户输入，限制脚本必须是 `quark-drive.cjs` 绝对路径。
- 默认不开启分享创建，管理员开启后也仅允许私聊执行。插件不提供删除、批量重命名或无条件换链。

## 安装与授权

在 Linux / Windows WSL 安装 [夸克官方 Skill](https://github.com/quark-clouddrive/quarkclouddrive_offical)，按其 OAuth 流程登录，再到插件配置页填写官方脚本绝对路径。凭据由官方 CLI 授权机制管理，不导入云团的 Token 配置。

官方 Skill 操作参考：[SKILL.md](https://github.com/quark-clouddrive/quarkclouddrive_offical/blob/main/skills/quarkclouddrive/SKILL.md)。适配器采用明确可控的命令子集，不在服务运行时静默执行其安装和自动更新脚本。需要更新时由管理员主动升级并验证兼容。

## 分享状态与每日巡检边界

`get-share-update-list` 返回的是**已转存分享的内容更新**，并不等于“本人创建的分享有效性”。目前 `share-detail` 可对提供的单个链接查询详情，但官方 CLI 未提供已验证的本人分享总列表/自动续期合同。

所以“每日维护本人创建的所有链接”尚未投入运行。后续须借助**云团公共持久调度**、源文件 FID 记录、分享状态分类以及经过验证的新链接替换能力。到期可在授权下重新分享；违规和政策限制不自动换链规避。

## 对应实现

- [`plugins/quark-drive/src/lib.rs`](../../../../plugins/quark-drive/src/lib.rs)
- [`plugins/quark-drive/SKILL.md`](../../../../plugins/quark-drive/SKILL.md)

测试：[插件源码内的单元测试](../../../../plugins/quark-drive/src/lib.rs)。

维护时同步 `docs/features/catalog.json` 和 [`plugins/quark-drive/README.md`](../../../../plugins/quark-drive/README.md)。
