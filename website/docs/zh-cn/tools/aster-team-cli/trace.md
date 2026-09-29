---
title: "aster-team-cli trace"
description: "按 Aster request ID 查找活动 Control 服务中的请求日志。"
---

# aster-team-cli trace

按 Aster request ID 查找活动 Control 服务中的请求日志。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli trace [REQUEST_ID] [--hours <HOURS>]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `REQUEST_ID` | 否 | 交互输入 | 8–160 位字母、数字、点、下划线或连字符。取自错误正文或 X-Aster-Request-ID。 |
| `--hours <HOURS>` | 否 | 24 | 查询最近 1–720 小时。 |

使用 `aster-team-cli trace --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli trace
sudo aster-team-cli trace --hours 72
```

## 配置与运行影响

只读取 Control 日志；不会重发模型请求或改变额度。

## 执行结果

输出匹配日志；没有匹配记录也返回成功，并显示未找到。

## 失败处理

编号格式或小时范围错误时拒绝执行；没有记录时确认时间窗口、日志保留及是否在正确 Control 主机。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
