---
title: "aster-team-cli password reset-admin"
description: "在 Control 主机重置管理员密码并撤销其会话。"
---

# aster-team-cli password reset-admin

在 Control 主机重置管理员密码并撤销其会话。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli password reset-admin [--email <EMAIL>]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--email <EMAIL>` | 否 | 交互输入 | 目标活动管理员邮箱；省略时按终端提示选择。 |

使用 `aster-team-cli password reset-admin --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli password reset-admin
sudo aster-team-cli password reset-admin --email admin@example.com
```

## 配置与运行影响

以隐藏输入读取并确认新密码，更新密码哈希并撤销目标管理员的所有会话。密码要求 12–1024 字节。成员密码由管理端重置。

## 执行结果

成功后使用新密码重新登录；此命令不依赖网页会话。

## 失败处理

只能在 Control 主机执行；账号不可用、两次密码不一致或数据库访问失败时不会成功。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
