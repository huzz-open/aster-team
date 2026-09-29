---
title: Anthropic Messages API 兼容与参数映射
description: 使用 /v1/messages 调用统一网关，了解 system、thinking、工具内容块与流式事件的兼容边界。
---

# Anthropic Messages API

`POST /v1/messages` 接受 Anthropic Messages 形状；鉴权仍使用 Aster 成员 Key。

```json
{"model":"YOUR_PUBLIC_MODEL","max_tokens":1024,"messages":[{"role":"user","content":"你好"}]}
```

## 输入与输出

`max_tokens` 是必填项。`system` 与多轮 `messages` 保持角色语义。目标模型的输出 token 上限与 Messages 口径不一致时，请查看下方模型映射；无法保留硬性上限的组合在派发前拒绝。

## 工具与结构化内容

工具调用使用 `tool_use` 内容块，结果使用 `tool_result`。工具 ID、结果错误标记和强制选择语义必须能保留；目标不能保真时，组合在派发前拒绝。JSON object 与严格 JSON Schema 是独立能力，不能仅因目标会输出 JSON 就视为支持 Schema。

## 思考与流式

`thinking.type`、思考预算与输出等级不是同一参数。流式事件以 `message_start`、内容块事件、`message_delta`、`message_stop` 组织；用量可能是累计值，不能逐片相加。

## 模型矩阵

选择 Anthropic Messages 后，可切换模型查看支持与映射状态。

<ModelFieldMatrix language="zh" initial-protocol="anthropic_messages" />

[查看模型参数速查](/zh-cn/models/) · [Responses API](/zh-cn/api/responses)
