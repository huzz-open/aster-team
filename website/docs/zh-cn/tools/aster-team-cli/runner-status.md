---
title: "aster-team-cli runner status"
description: "检查本机 Runner 服务及身份文件是否存在。"
---

# aster-team-cli runner status

检查本机 Runner 服务及身份文件是否存在。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli runner status
```

## 参数

无专用参数。

使用 `aster-team-cli runner status --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli runner status
```

## 配置与运行影响

只读服务状态、身份文件及任务密钥文件的存在性，不重启服务。

## 执行结果

输出 Runner 状态及 identity、task keys 的 present/missing；文件存在不等于与 Control 连接正常。

## 失败处理

服务未安装会报错；连接问题结合 doctor、logs runner 和管理端在线状态判断。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
