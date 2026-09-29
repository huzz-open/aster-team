---
title: "asterctl doctor claude"
description: "检查 Claude 项目配置、认证、客户端版本和平台模型映射。"
---

# asterctl doctor claude

检查 Claude 项目配置、认证、客户端版本和平台模型映射。

## 执行前

对已经完成 setup claude 的项目执行。

## 语法

```text
asterctl doctor claude [--project <PATH>]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--project <PATH>` | 否 | . | 项目目录；相对路径以当前终端目录为基准。 |

使用 `asterctl doctor claude --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl doctor claude --project "project-demo"
```

## 配置与运行影响

先执行本地状态检查，再读取当前 Claude 版本并请求平台设置，与项目文件比较；不发送生成请求，不自动重写设置。

## 执行结果

全部检查通过时退出码为 0；任何检查失败时退出码为 1，并打印失败原因。

## 失败处理

若当前 Claude 版本或平台映射与保存内容不同，重新执行 setup claude。文件被手动修改时先处理本地变更。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
