---
title: "使用 asterctl 快速接入"
description: "使用成员 Key 将 Codex Desktop 或 Claude Code 接入 Aster，并检查配置结果。"
---

# 使用 asterctl 快速接入

先完成 [工具安装](/zh-cn/guides/asterctl)，并取得 Aster API 地址与成员 Key。以下命令假设 asterctl 已加入 PATH。

## Codex Desktop

在 Windows 上安装并完全退出 Codex，包括后台进程。API 地址以 `/v1` 结尾：

```powershell
asterctl setup codex --base-url "https://aster.example.com/v1" --set-key --launch
asterctl doctor codex
```

按隐藏提示输入成员 Key。配置完成后创建新任务。省略 `--launch` 可只配置，稍后手动启动。更新地址、Key 和目录的详细行为见 [setup codex](/zh-cn/tools/asterctl/setup-codex)。

## Claude Code

当前实现要求 Claude Code 不低于 `2.1.255`；必要时执行 `claude update`。API 根地址不带 `/v1`：

```powershell
asterctl setup claude --base-url "https://aster.example.com" --project "project-demo" --set-key --launch
asterctl doctor claude --project "project-demo"
```

已在项目目录时可以省略 `--project`。工具写入项目配置；已有配置需要确认覆盖并备份。配置及备份可能包含 Key，不要提交到版本控制。具体参数见 [setup claude](/zh-cn/tools/asterctl/setup-claude)。

## 接入后

- 只检查本地状态： [status codex](/zh-cn/tools/asterctl/status-codex) / [status claude](/zh-cn/tools/asterctl/status-claude)。
- 移除托管配置： [remove codex](/zh-cn/tools/asterctl/remove-codex) / [remove claude](/zh-cn/tools/asterctl/remove-claude)。
- 检查失败时查看 [故障排查](/zh-cn/tools/asterctl/troubleshooting)。
