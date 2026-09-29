---
title: "aster-team-cli backup create"
description: "创建Control 本地数据库安装备份。"
---

# aster-team-cli backup create

创建Control 本地数据库安装备份。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 执行前

仅适用于本地 SQLCipher；外部数据库需要数据库原生快照和匹配的安装配置、身份与密钥，不支持此命令的完整备份恢复。

## 语法

```text
aster-team-cli backup create [--output <PATH>]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--output <PATH>` | 否 | 安装根 backups 内时间戳归档 | 必须是尚不存在的新归档路径。 |

使用 `aster-team-cli backup create --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli backup create
```

## 配置与运行影响

备份会暂时停止相关活动服务，创建受保护归档后恢复原先运行的服务。归档包含配置和密钥，需妥善保管。

## 执行结果

输出最终备份路径；停止、归档或恢复服务失败均可能导致命令失败，需检查服务状态。

## 失败处理

角色不匹配、归档权限不正确、输出已存在或恢复确认缺失时按具体错误处理，不要跳过校验。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
