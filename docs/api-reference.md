# Compatible model API

[简体中文](zh-CN/api-reference.md) · [Documentation](README.md)

Aster exposes OpenAI- and Anthropic-compatible model endpoints behind one member identity, quota, routing, and audit layer. Compatibility is endpoint-specific; it does not imply that every provider feature is available.

The API uses customer-connected provider accounts. An Aster API key does not include a provider subscription or credit, and the endpoint does not bypass provider availability, entitlement, policy, or network requirements. See [Service boundaries](service-boundaries.md).

## Base URLs and authentication

| Protocol | Base URL | Authentication |
| --- | --- | --- |
| OpenAI-compatible | `https://aster.example.com/v1` | `Authorization: Bearer <member-api-key>` |
| Anthropic-compatible | `https://aster.example.com` | Client-specific API key header or bearer authentication |

Always use the public address shown in Member UI. Never place an API key in a URL.

## Supported endpoints

- `GET /v1/models`
- `POST /v1/responses`
- `POST /v1/chat/completions`
- `POST /v1/messages`
- `POST /v1/images/generations`
- `POST /v1/images/edits`

Only administrator-enabled models are returned by `/v1/models`.

## Responses example

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/responses' \
  -H 'Authorization: Bearer <member-api-key>' \
  -H 'Content-Type: application/json' \
  -H 'X-Request-ID: example-001' \
  --data-binary '{"model":"<enabled-model-id>","input":"Explain the deployment in three bullets.","stream":false}'
```

The same endpoint works with the OpenAI SDK when its base URL is configurable. TypeScript:

```ts
import OpenAI from "openai";

const client = new OpenAI({
  apiKey: process.env.ASTER_API_KEY,
  baseURL: "https://aster.example.com/v1",
});

const response = await client.responses.create({
  model: "<enabled-model-id>",
  input: "Explain the deployment in three bullets.",
});
```

Python:

```python
import os
from openai import OpenAI

client = OpenAI(
    api_key=os.environ["ASTER_API_KEY"],
    base_url="https://aster.example.com/v1",
)

response = client.responses.create(
    model="<enabled-model-id>",
    input="Explain the deployment in three bullets.",
)
```

## Chat Completions example

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/chat/completions' \
  -H 'Authorization: Bearer <member-api-key>' \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"<enabled-model-id>","messages":[{"role":"user","content":"Hello"}],"stream":false}'
```

## Anthropic Messages example

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/messages' \
  -H 'x-api-key: <member-api-key>' \
  -H 'anthropic-version: 2023-06-01' \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"<enabled-model-id>","max_tokens":512,"messages":[{"role":"user","content":"Hello"}]}'
```

## Model variants

`/v1/models` lists base models. Clients that cannot send native speed or reasoning fields may append a strict variant suffix:

- `<base>-fast` or `<base>-standard` selects processing speed.
- `<base>-none`, `-low`, `-medium`, `-high`, `-xhigh`, or `-max` selects reasoning effort when supported.
- `<base>-fast-high` selects both, always in `base-speed-effort` order.

For example, `<base>-fast-high` routes to `<base>` with fast processing and high reasoning effort. Variant names are exact and are not added to `/v1/models`.

Native fields are preferred when the client supports them:

| Protocol | Speed | Reasoning effort |
| --- | --- | --- |
| Responses | `"service_tier":"fast"` | `"reasoning":{"effort":"high"}` |
| Chat Completions | `"service_tier":"fast"` | `"reasoning_effort":"high"` |
| Anthropic Messages | `"speed":"fast"` | `"output_config":{"effort":"high"}` |

If a variant and explicit fields disagree, Aster returns `INVALID_REQUEST` (`32001`). A provider may reject combinations unsupported by the selected model.

## Errors

Errors use a stable string code and five-digit number:

```json
{
  "error": {
    "code": "RUNNER_NOT_READY",
    "message": "No compatible runner is currently ready.",
    "number": 33003
  }
}
```

The number is also returned in `X-Aster-Error-Number`. Common public API errors include:

| Number | Code | Meaning |
| --- | --- | --- |
| `31001` | `INVALID_API_KEY` | The key is missing, malformed, revoked, or no longer valid for the member |
| `32001` | `INVALID_REQUEST` | Request fields or query parameters are invalid |
| `33001` | `MODEL_NOT_FOUND` | The model does not exist, is disabled, or has no account mapping |
| `33002` | `MODEL_ACCOUNT_NOT_READY` | No usable credential instance is ready for the model |
| `33003` | `RUNNER_NOT_READY` | No healthy compatible Runner is available; quota is not deducted |
| `34001` | `INSUFFICIENT_QUOTA` | Available quota is insufficient after in-flight reservations |
| `34005` | `DUPLICATE_REQUEST` | The request ID has already been reserved or accounted for |
| `35002` | `UPSTREAM_REQUEST_FAILED` | A Runner accepted the task but the upstream request failed |

Treat `request_id` as a trace identifier, not an error code. Preserve both values when asking for help.

## Streaming and retries

A request may move to another Runner only before it reaches the provider. After a stream begins, Aster does not replay it silently on another Runner. Applications should use explicit request IDs and retry only operations that are safe to repeat.
