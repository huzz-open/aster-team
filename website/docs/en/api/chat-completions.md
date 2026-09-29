---
title: Chat Completions API and model compatibility
description: Call models from different providers through Aster /v1/chat/completions and inspect reasoning, tools, and JSON field mappings.
---

# Chat Completions API

`POST /v1/chat/completions` uses a public model ID and a message array.

Set `ASTER_BASE_URL` to the API URL shown in the [quickstart](/en/start/quickstart) and replace `YOUR_PUBLIC_MODEL` with a [granted model](/en/models/) ID.

<InstanceCurlExample language="en" kind="chat" />

Generic template:

```bash
curl --fail-with-body "$ASTER_BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer $ASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"YOUR_PUBLIC_MODEL","messages":[{"role":"user","content":"Hello"}]}'
```

```json
{"model":"YOUR_PUBLIC_MODEL","messages":[{"role":"user","content":"Hello"}]}
```

## Common fields

| Task | Fields | Required | Watch for |
| --- | --- | --- | --- |
| Conversation | `messages`, `model` | Yes | Keep tool call and result IDs linked. |
| Output limit | `max_completion_tokens` | No | Reasoning models may count thinking tokens toward the limit. |
| Streaming | `stream`, `stream_options.include_usage` | No | A usage chunk with empty `choices` is not an empty answer. |
| Reasoning | `reasoning_effort` | No | Levels vary by model; not every model can disable thinking. |
| Tools | `tools`, `tool_choice`, `parallel_tool_calls` | No | Forced selection and parallel limits are checked separately. |
| JSON | `response_format` | No | JSON object and strict Schema are distinct capabilities. |

## Model matrix

Select Chat Completions below and switch models to compare supported fields and mappings. Legacy Chat `max_tokens` cannot always preserve the semantics of a newer limit that includes reasoning tokens. The legacy `POST /v1/completions` endpoint is outside the public gateway.

<ModelFieldMatrix language="en" initial-protocol="chat_completions" />

[Model quick reference](/en/models/) · [Responses API](/en/api/responses)
