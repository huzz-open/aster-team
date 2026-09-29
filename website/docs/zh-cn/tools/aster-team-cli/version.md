---
title: "aster-team-cli version"
description: "显示 CLI 构建信息和已安装产品版本。"
---

# aster-team-cli version

显示 CLI 构建信息和已安装产品版本。

## 语法

```text
aster-team-cli version
```

## 参数

无专用参数。

使用 `aster-team-cli version --help` 查看当前安装版本的帮助。

## 示例

```bash
aster-team-cli version
aster-team-cli --version
aster-team-cli --help
```

## 配置与运行影响

只读，不更改安装。version 显示 CLI 版本、提交、构建时间，并尝试读取安装版本；--version 仅显示简短 CLI 版本。

## 执行结果

未安装时显示未安装状态；此命令不要求管理员权限，但文件访问权限仍适用。

## 失败处理

安装版本文件无法读取时打印文件错误；优先确认执行的是当前安装根内的 CLI。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
