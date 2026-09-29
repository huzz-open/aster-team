---
title: "asterctl remove codex"
description: "撤销 asterctl 管理的 Codex 字段与生成模型目录。"
---

# asterctl remove codex

撤销 asterctl 管理的 Codex 字段与生成模型目录。

## 执行前

在 Windows 上完全退出 Codex，并保留初始化产生的状态文件。

## 语法

```text
asterctl remove codex
```

## 参数

无专用参数。

使用 `asterctl remove codex --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl remove codex
```

## 配置与运行影响

只恢复仍与工具写入值一致的字段，保留后来手动修改的字段及 ASTER_API_KEY。未修改的生成目录可被删除，原目录设置被恢复。存在冲突时可能已经撤销部分字段，同时保留冲突状态供处理。

## 执行结果

完成时打印移除结果并返回 0；冲突或文件操作失败返回 1。

## 失败处理

没有状态记录时拒绝猜测哪些字段应删除。冲突字段保留，不要把失败退出理解为所有字段都未改变。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
