---
title: Anthropic Messages API compatibility and mapping
description: Use /v1/messages through Aster and understand system messages, thinking, tool blocks, and streaming event limits.
---

# Anthropic Messages API

`POST /v1/messages` accepts an Anthropic Messages shaped request. Authentication still uses an Aster member key.

```json
{"model":"YOUR_PUBLIC_MODEL","max_tokens":1024,"messages":[{"role":"user","content":"Hello"}]}
```

## Input and output

`max_tokens` is required. `system` and conversation messages retain their roles. Check the model mapping below when a target counts output and reasoning tokens differently. Aster rejects a hard output limit that the target cannot preserve before dispatch.

## Tools and structured content

Tool calls use `tool_use` blocks and results use `tool_result`. Aster rejects combinations when a target cannot preserve IDs, result error markers, or a forced tool choice. JSON object output and strict JSON Schema are separate capabilities.

## Reasoning and streaming

`thinking.type`, a thinking budget, and an output effort level are different controls. Streaming uses `message_start`, content block events, `message_delta`, and `message_stop`; cumulative usage must not be added once per fragment.

## Model matrix

Select Anthropic Messages below and switch models to compare supported fields and mappings.

<ModelFieldMatrix language="en" initial-protocol="anthropic_messages" />

[Model quick reference](/en/models/) · [Responses API](/en/api/responses)
