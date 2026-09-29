---
title: "asterctl remove claude"
description: "撤销指定项目的 Claude 设置，并在有备份时恢复原文件。"
---

# asterctl remove claude

撤销指定项目的 Claude 设置，并在有备份时恢复原文件。

## 执行前

使用初始化时的项目路径，并保留状态文件及原设置备份。

## 语法

```text
asterctl remove claude [--project <PATH>]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--project <PATH>` | 否 | . | 项目目录；相对路径以当前终端目录为基准。 |

使用 `asterctl remove claude --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl remove claude --project "project-demo"
```

## 配置与运行影响

若原来有设置文件，恢复本次 setup 的备份；否则删除工具创建的设置。成功后移除托管状态及已使用的备份，不卸载 Claude。

## 执行结果

完成时打印移除结果并返回 0；冲突或文件操作失败返回 1。

## 失败处理

设置摘要与记录不一致时拒绝覆盖；原备份丢失时无法恢复。先保存并核对本地文件，不要直接删除状态记录。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
