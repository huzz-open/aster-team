---
title: "aster-team-cli backup restore"
description: "恢复Control 本地数据库安装备份。"
---

# aster-team-cli backup restore

恢复Control 本地数据库安装备份。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 执行前

仅适用于本地 SQLCipher；外部数据库需要数据库原生快照和匹配的安装配置、身份与密钥，不支持此命令的完整备份恢复。

## 语法

```text
aster-team-cli backup restore --source <PATH> --confirm
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--source <PATH>` | 是 | — | 受保护的备份归档，需与主机角色和安装身份匹配。 |
| `--confirm` | 是 | false | 确认替换当前数据；缺少此参数会拒绝恢复。 |

使用 `aster-team-cli backup restore --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli backup restore --source /root/aster-backup.tar.gz --confirm
```

## 配置与运行影响

恢复会替换当前数据并影响服务；使用安装中的恢复引擎校验归档，不能把其他实例的归档当作普通数据导入。Windows 自定义实例还需匹配根目录及实例配置。

## 执行结果

完成后执行 status 和 doctor 检查实例；独立 Runner 同时检查 runner status。

## 失败处理

角色不匹配、归档权限不正确、输出已存在或恢复确认缺失时按具体错误处理，不要跳过校验。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
