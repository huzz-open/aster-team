---
title: Chat Completions API 与模型兼容
description: 使用 Aster /v1/chat/completions 调用不同厂商模型，并查询推理、工具和 JSON 参数映射。
---

# Chat Completions API

`POST /v1/chat/completions` 使用公开模型 ID 与消息数组。

将 `ASTER_BASE_URL` 设为[快速开始](/zh-cn/start/quickstart)中显示的当前实例 API 地址，并将 `YOUR_PUBLIC_MODEL` 换成[当前授权的模型](/zh-cn/models/) ID。

<InstanceCurlExample language="zh" kind="chat" />

通用模板：

```bash
curl --fail-with-body "$ASTER_BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer $ASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"YOUR_PUBLIC_MODEL","messages":[{"role":"user","content":"你好"}]}'
```

```json
{"model":"YOUR_PUBLIC_MODEL","messages":[{"role":"user","content":"你好"}]}
```

## 常用字段

| 任务 | 参数 | 必填 | 注意 |
| --- | --- | --- | --- |
| 组织对话 | `messages`、`model` | 是 | 工具调用与结果需保留关联 ID。 |
| 限制输出 | `max_completion_tokens` | 否 | 思考模型的上限可能包含推理 token。 |
| 接收流 | `stream`、`stream_options.include_usage` | 否 | 空 `choices` 的用量片段不是空回答。 |
| 思考 | `reasoning_effort` | 否 | 等级按具体模型映射；不是所有模型都能关闭思考。 |
| 工具 | `tools`、`tool_choice`、`parallel_tool_calls` | 否 | 强制选择与禁止并行分别检查。 |
| JSON | `response_format` | 否 | JSON object 与严格 Schema 分开判断。 |

## 模型矩阵

下方默认选择 Chat Completions；切换模型可以比较字段支持和映射。对于旧 Chat `max_tokens`，需要检查思考 token 的上限语义，不能无条件等同新字段。旧 `POST /v1/completions` 不属于公开网关接口。

<ModelFieldMatrix language="zh" initial-protocol="chat_completions" />

[查看模型参数速查](/zh-cn/models/) · [Responses API](/zh-cn/api/responses)
