---
title: Responses API：请求、工具和参数映射
description: Aster /v1/responses 的输入、输出、流式、思考与工具调用说明，并可切换模型查看参数差异。
---

# Responses API

`POST /v1/responses` 接受 Aster API Key 和公开模型 ID。基础文本只需要 `model` 与 `input`。

```json
{"model":"YOUR_PUBLIC_MODEL","input":"解释这个方案的关键约束"}
```

## 请求字段

| 分类 | 参数 | 必填 | 类型 | 状态 | 用途与注意 |
| --- | --- | --- | --- | --- | --- |
| 输入 | `model` | 是 | string | 支持 | 使用当前部署授权的公开模型 ID。 |
| 输入 | `input` | 是 | string / array | 支持 | 文本、角色及工具结果按内容项解码；未知内容不会当作空文本。 |
| 输入 | `instructions` | 否 | string | 依模型 | 系统指令需要保持优先级；目标不能保真时拒绝。 |
| 输出 | `max_output_tokens` | 否 | integer | 依模型 | 包含可见输出与推理消耗；目标口径不能保留时拒绝。 |
| 输出 | `stream` | 否 | boolean | 依通道 | `true` 请求 SSE；不同上游的终态会统一转换。 |
| 思考 | `reasoning.effort` | 否 | string | 依模型 | 档位与是否启用思考分开判断；近似映射在模型矩阵中显示。 |
| 工具 | `tools` | 否 | array | 依模型 | 当前公开集合以函数工具为基础；工具调用需要由客户端执行并回传结果。 |
| 工具 | `tool_choice` | 否 | string / object | 依模型 | `required`、指定工具与 `auto` 并非所有模型等价。 |
| 工具 | `parallel_tool_calls` | 否 | boolean | 依模型 | 禁止并行是硬约束；目标会忽略它时，网关拒绝请求。 |

## 响应状态与结构化结果

`status=completed` 表示本轮响应完成；`status=incomplete` 表示被长度或内容限制截断。流式客户端应等待终态事件，不把每个片段当成完整 JSON。

`text.format` 的 JSON object 与严格 JSON Schema 是不同能力。远端状态、后台任务、内置搜索等字段不属于首版通用集合。明确要求未实现的功能时，网关在发送上游前返回字段错误。

## 模型矩阵

<ModelFieldMatrix language="zh" />

实际可执行模型仍以当前实例的能力和成员权限为准。
