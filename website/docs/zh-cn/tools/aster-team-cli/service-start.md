---
title: "aster-team-cli service start"
description: "启动指定的 Aster 服务。"
---

# aster-team-cli service start

启动指定的 Aster 服务。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli service start <TARGET>
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `TARGET` | 是 | — | control、runner 或 all；具体服务由当前安装解析。 |

使用 `aster-team-cli service start --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli service start all
```

## 配置与运行影响

启动已安装的服务。命令使用维护锁协调操作，按服务依赖处理顺序，不新增服务配置。

## 执行结果

输出所操作服务的状态；随后使用 status 核对依赖。

## 失败处理

服务不存在、平台服务命令失败或维护锁被占用时按报错处理，不应通过重复执行掩盖原因。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
