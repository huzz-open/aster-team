---
title: "asterctl status claude"
description: "检查指定项目的 Claude 托管状态与文件完整性，不发送网络请求。"
---

# asterctl status claude

检查指定项目的 Claude 托管状态与文件完整性，不发送网络请求。

## 语法

```text
asterctl status claude [--project <PATH>]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--project <PATH>` | 否 | . | 项目目录；相对路径以当前终端目录为基准。 |

使用 `asterctl status claude --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl status claude --project "project-demo"
```

## 配置与运行影响

读取项目设置和 .asterctl-state.json，比对初始化时记录的摘要，不修改文件。

## 执行结果

成功时显示托管配置未改变、项目路径和初始化时的 Claude 版本。

## 失败处理

项目不存在、没有初始化记录或设置后来被修改时会失败。确认 --project 指向同一项目，保留自己的修改后再决定是否重新初始化。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
