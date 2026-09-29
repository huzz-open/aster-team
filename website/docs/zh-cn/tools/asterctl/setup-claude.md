---
title: "asterctl setup claude"
description: "为指定项目配置 Claude Code 的 Aster 地址、成员 Key 和模型映射。"
---

# asterctl setup claude

为指定项目配置 Claude Code 的 Aster 地址、成员 Key 和模型映射。

## 执行前

先安装 Claude Code；当前实现要求版本不低于 2.1.255。版本过低时执行 claude update。

## 语法

```text
asterctl setup claude --base-url <URL> --set-key [--project <PATH>] [--launch]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--base-url <URL>` | 是 | — | Aster API 根地址，不带 /v1。 |
| `--set-key` | 是 | false | 运行时必须提供，Key 由隐藏提示读取。 |
| `--project <PATH>` | 否 | . | 项目目录；相对路径以当前终端目录为基准。 |
| `--launch` | 否 | false | 完成后在项目目录启动 Claude。 |

使用 `asterctl setup claude --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl setup claude --base-url "https://aster.example.com" --project "project-demo" --set-key --launch
```

## 配置与运行影响

必要时创建项目目录。写入 .claude/settings.local.json 和 .claude/.asterctl-state.json；已有设置需交互确认覆盖，并在写入前备份。设置文件及备份可能包含 Key，不应提交到版本控制。再次 setup 会以本次写入前的文件作为恢复基准。

## 执行结果

输出项目、设置文件位置及模型别名映射；可随后执行 doctor claude --project。同意覆盖前取消会成功退出，不修改原设置。

## 失败处理

缺少 --set-key、Key 为空、版本不支持或模型映射不可用时会报错。平台映射改变后，对同一项目重新初始化。启动失败时先用 status claude 检查已写入的设置。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
