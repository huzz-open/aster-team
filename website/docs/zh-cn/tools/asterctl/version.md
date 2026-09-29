---
title: "asterctl version"
description: "显示 asterctl 的版本和构建提交。"
---

# asterctl version

显示 asterctl 的版本和构建提交。

## 语法

```text
asterctl version
```

## 参数

无专用参数。

使用 `asterctl version --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl version
asterctl --version
asterctl --help
asterctl setup codex --help
```

## 配置与运行影响

不修改配置、不联网。--version 是简短版本输出；version 还显示构建提交。--help 可用于顶层及每个子命令。

## 执行结果

输出版本与 Commit；用于反馈问题时确认工具版本。

## 失败处理

命令找不到时使用工具绝对路径，或检查用户 PATH。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
