---
title: "asterctl setup codex"
description: "配置 Windows Codex Desktop 的 Aster 提供方、成员 Key 和模型目录。"
---

# asterctl setup codex

配置 Windows Codex Desktop 的 Aster 提供方、成员 Key 和模型目录。

## 执行前

先安装 Codex，并完全退出客户端和后台进程。以使用 Codex 的 Windows 用户执行；使用 Aster 成员 Key。

## 语法

```text
asterctl setup codex [--base-url <URL>] [--set-key] [--launch]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--base-url <URL>` | 条件必填 | 已有配置 | Aster API 地址，以 /v1 结尾。首次接入必填；与 --set-key 至少提供一个。 |
| `--set-key` | 否 | false | 隐藏输入成员 Key。单独使用时需要已有 Aster 地址；只传 --launch 不足以执行初始化。 |
| `--launch` | 否 | false | 配置成功后启动 Codex。 |

使用 `asterctl setup codex --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl setup codex --base-url "https://aster.example.com/v1" --set-key --launch
asterctl setup codex --base-url "https://aster.example.com/v1"
asterctl setup codex --set-key
```

## 配置与运行影响

合并 CODEX_HOME 下的 config.toml，默认位置为 %USERPROFILE%\.codex。生成模型目录并记录变更状态；输入新 Key 时写入当前用户的 ASTER_API_KEY。保留默认模型和非托管配置。模型目录依赖已安装 Codex 的 debug models 能力，实际模型权限仍由服务端决定。

## 执行结果

输出目标地址、模型目录位置及模型数量。未设置 --launch 时需重新打开 Codex 并创建新任务。Key 输入为空且提供了地址时继续仅更新地址；仅更换 Key 时输入为空会不作修改并成功退出。

## 失败处理

客户端仍在运行：完全退出后再执行。缺少 Key：运行 setup codex --set-key。模型目录校验失败：核对 Codex 安装及 debug models 支持。配置成功后启动失败不等于配置被撤销，可用 status codex 检查。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
