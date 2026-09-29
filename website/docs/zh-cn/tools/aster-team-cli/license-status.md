---
title: "aster-team-cli license status"
description: "查看当前 Control 许可证状态。"
---

# aster-team-cli license status

查看当前 Control 许可证状态。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli license status
```

## 参数

无专用参数。

使用 `aster-team-cli license status --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli license status
```

## 配置与运行影响

读取许可证状态，不改变授权。独立 Runner 不独立安装许可证。

## 执行结果

显示当前许可证信息；missing 表示未安装，需申请或导入。

## 失败处理

不是 Control 主机或许可证读取校验失败时按错误处理；状态文字需与退出结果一起阅读。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
