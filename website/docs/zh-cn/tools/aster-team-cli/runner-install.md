---
title: "aster-team-cli runner install"
description: "安装独立 Runner 主机。"
---

# aster-team-cli runner install

安装独立 Runner 主机。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 执行前

先初始化已验证的 Runner 发布包，并在独立主机执行。

## 语法

```text
aster-team-cli runner install
```

## 参数

无专用参数。

使用 `aster-team-cli runner install --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli runner install
```

## 配置与运行影响

安装 Runner 的程序、配置和服务。加入 Control 使用 runner enroll；不要用此命令替代 Control 安装。

## 执行结果

完成安装后使用注册令牌进行 enroll，再查看 runner status。

## 失败处理

未选中发布包、平台不支持或已有角色冲突时停止，核对发布包及目标主机。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
