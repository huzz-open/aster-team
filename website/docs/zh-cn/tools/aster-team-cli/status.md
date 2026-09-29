---
title: "aster-team-cli status"
description: "查看服务、许可证与数据库运行状态。"
---

# aster-team-cli status

查看服务、许可证与数据库运行状态。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli status
```

## 参数

无专用参数。

使用 `aster-team-cli status --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli status
```

## 配置与运行影响

检查当前安装角色的运行状态，不启动或修复服务。

## 执行结果

Control 会检查相关服务与运行依赖；独立 Runner 检查 Runner。关键检查失败会以失败退出。

## 失败处理

异常项需要结合 doctor 与 logs 排查；服务未运行不等于需要重新安装。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
