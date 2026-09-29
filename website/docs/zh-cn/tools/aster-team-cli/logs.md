---
title: "aster-team-cli logs"
description: "读取 Control 或 Runner 的服务日志。"
---

# aster-team-cli logs

读取 Control 或 Runner 的服务日志。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli logs <TARGET> [--follow]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `TARGET` | 是 | — | control 或 runner。 |
| `--follow` | 否 | false | 持续查看新日志，Ctrl+C 结束。 |

使用 `aster-team-cli logs --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli logs control
sudo aster-team-cli logs runner --follow
```

## 配置与运行影响

只读；自动选择活动服务。Linux 默认显示最近 200 行，Windows 读取安装根内对应日志的最近 200 行。

## 执行结果

输出已有日志；--follow 会保持运行等待新内容。

## 失败处理

服务或日志文件不存在时检查安装角色和 status。单个请求定位使用 trace。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
