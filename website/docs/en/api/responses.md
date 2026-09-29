---
title: Responses API requests, tools, and field mapping
description: Learn Aster /v1/responses input, output, streaming, reasoning, and tool behavior; switch models to inspect field changes.
---

# Responses API

`POST /v1/responses` accepts an Aster API key and a public model ID. Basic text needs only `model` and `input`.

```json
{"model":"YOUR_PUBLIC_MODEL","input":"Summarize the key constraints"}
```

## Request fields

| Category | Field | Required | Type | Status | Purpose and notes |
| --- | --- | --- | --- | --- | --- |
| Input | `model` | Yes | string | Supported | A public model ID granted in this installation. |
| Input | `input` | Yes | string / array | Supported | Text, roles, and tool results are decoded by content item. Unknown content is rejected. |
| Input | `instructions` | No | string | Model dependent | Instruction priority must be preserved; incompatible targets are rejected. |
| Output | `max_output_tokens` | No | integer | Model dependent | Includes visible and reasoning tokens; incompatible limit semantics are rejected. |
| Output | `stream` | No | boolean | Channel dependent | `true` requests SSE with a converted terminal event. |
| Reasoning | `reasoning.effort` | No | string | Model dependent | Reasoning mode and level are distinct. Approximate mappings appear in the model matrix. |
| Tools | `tools` | No | array | Model dependent | The shared baseline uses function tools. Execute a returned call and send its result in a later request. |
| Tools | `tool_choice` | No | string / object | Model dependent | `required`, named, and `auto` choices differ across models. |
| Tools | `parallel_tool_calls` | No | boolean | Model dependent | Disabling parallel calls is a hard constraint. Aster rejects it if upstream would ignore it. |

## Response status and structured output

`completed` ends the model turn; `incomplete` means output was truncated or filtered. A streaming client must wait for a terminal event and cannot parse each fragment as a complete response.

JSON object and strict JSON Schema are separate capabilities under `text.format`. Remote response state, background tasks, and server tools are outside the initial shared subset. Aster rejects an unmet request before dispatch.

## Model matrix

<ModelFieldMatrix language="en" />

Your installation’s active capabilities and member grants determine which public models can execute.
