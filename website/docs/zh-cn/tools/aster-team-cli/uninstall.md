---
title: "aster-team-cli uninstall"
description: "卸载服务与程序，默认保留业务数据。"
---

# aster-team-cli uninstall

卸载服务与程序，默认保留业务数据。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli uninstall [--purge]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--purge` | 否 | false | 删除本机配置和数据，必须在交互终端准确输入 PURGE 确认。 |

使用 `aster-team-cli uninstall --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli uninstall
```

## 配置与运行影响

普通卸载移除服务和程序，保留配置、身份、密钥和业务数据供恢复。--purge 删除安装根内的数据；外部数据库的数据生命周期需单独管理。Windows 在 CLI 退出后清理被占用的程序文件。

## 执行结果

卸载后服务不可用；保留数据的重装使用初始化后的 install --recover-preserved。

## 失败处理

非交互终端不能执行 purge，确认文字不匹配会拒绝；卸载前需保存需要保留的备份。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
