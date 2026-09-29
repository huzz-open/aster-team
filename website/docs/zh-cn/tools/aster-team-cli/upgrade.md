---
title: "aster-team-cli upgrade"
description: "使用新发布包选中的版本升级已有 Control 安装。"
---

# aster-team-cli upgrade

使用新发布包选中的版本升级已有 Control 安装。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 执行前

先备份并验证新包，在新包目录运行 init.sh；自定义安装根需传原 --install-root。Windows 使用 init.ps1 打印的稳定 CLI 路径。

## 语法

```text
aster-team-cli upgrade
```

## 参数

无专用参数。

使用 `aster-team-cli upgrade --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

## 配置与运行影响

执行当前部署支持的升级流程并保留身份、凭据、许可证与业务数据。可能停止服务；CLI 路径不能理解为一定不停服。数据库迁移只向前执行，不支持升级后手动切回旧程序。

## 执行结果

完成后执行 version、status 和 doctor，核对版本与运行状态。

## 失败处理

版本选择、发布校验或迁移失败时保留完整错误和日志。不要修改 schema_migrations 或手动替换 current 来绕过失败。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
