---
title: "aster-team-cli info"
description: "查看主机角色、版本、访问入口和重要路径。"
---

# aster-team-cli info

查看主机角色、版本、访问入口和重要路径。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli info
```

## 参数

无专用参数。

使用 `aster-team-cli info --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli info
```

## 配置与运行影响

读取当前安装元数据，不修改设置；Control 与独立 Runner 的输出按角色区分。

## 执行结果

用于确认正在管理的实例及访问地址；不能替代服务健康检查。

## 失败处理

安装根、角色标记或当前版本缺失时失败；确认使用正确安装根的稳定 CLI。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
