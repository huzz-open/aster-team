---
title: "aster-team-cli runner upgrade"
description: "升级独立 Runner 主机的发布版本。"
---

# aster-team-cli runner upgrade

升级独立 Runner 主机的发布版本。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 执行前

验证新包并初始化到原安装根，再使用稳定 CLI。

## 语法

```text
aster-team-cli runner upgrade
```

## 参数

无专用参数。

使用 `aster-team-cli runner upgrade --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli runner upgrade
```

## 配置与运行影响

执行独立 Runner 升级；服务可能中断，需安排合适时间。

## 执行结果

完成后核对 version、runner status 和 Control 中的在线状态。

## 失败处理

角色或所选发布不匹配时停止；Control 主机升级使用 upgrade。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
