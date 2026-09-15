# 兼容模型 API

[English](../api-reference.md) · [中文文档](README.md)

Aster 在统一的成员身份、额度、路由与审计层之后提供兼容 OpenAI 和 Anthropic 的模型接口。兼容范围以具体接口为准，并不表示支持上游的所有功能。

接口使用客户接入的服务商账号。Aster API Key 不包含上游订阅或额度，接口也不会绕过服务商可用性、权益、策略或网络要求，详见[服务边界](service-boundaries.md)。

## Base URL 与鉴权

| 协议 | Base URL | 鉴权 |
| --- | --- | --- |
| OpenAI 兼容 | `https://aster.example.com/v1` | `Authorization: Bearer <成员API-Key>` |
| Anthropic 兼容 | `https://aster.example.com` | 客户端使用的 API Key 请求头或 Bearer 鉴权 |

实际地址以 Member UI 显示为准。不要把 API Key 放进 URL。

## 支持的接口

- `GET /v1/models`
- `POST /v1/responses`
- `POST /v1/chat/completions`
- `POST /v1/messages`
- `POST /v1/images/generations`
- `POST /v1/images/edits`

`/v1/models` 只返回管理员已经开放的模型。

## Responses 示例

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/responses' \
  -H 'Authorization: Bearer <成员API-Key>' \
  -H 'Content-Type: application/json' \
  -H 'X-Request-ID: example-001' \
  --data-binary '{"model":"<已开放模型ID>","input":"用三点解释当前部署。","stream":false}'
```

支持配置 Base URL 的 OpenAI SDK 可以直接调用同一接口。TypeScript：

```ts
import OpenAI from "openai";

const client = new OpenAI({
  apiKey: process.env.ASTER_API_KEY,
  baseURL: "https://aster.example.com/v1",
});

const response = await client.responses.create({
  model: "<已开放模型ID>",
  input: "用三点解释当前部署。",
});
```

Python：

```python
import os
from openai import OpenAI

client = OpenAI(
    api_key=os.environ["ASTER_API_KEY"],
    base_url="https://aster.example.com/v1",
)

response = client.responses.create(
    model="<已开放模型ID>",
    input="用三点解释当前部署。",
)
```

## Chat Completions 示例

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/chat/completions' \
  -H 'Authorization: Bearer <成员API-Key>' \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"<已开放模型ID>","messages":[{"role":"user","content":"你好"}],"stream":false}'
```

## Anthropic Messages 示例

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/messages' \
  -H 'x-api-key: <成员API-Key>' \
  -H 'anthropic-version: 2023-06-01' \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"<已开放模型ID>","max_tokens":512,"messages":[{"role":"user","content":"你好"}]}'
```

## 图片生成示例

使用管理员当前启用且支持图片的模型：

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/images/generations' \
  -H 'Authorization: Bearer <member-api-key>' \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"<enabled-image-model-id>","prompt":"A clean technical illustration of a private AI gateway","size":"1024x1024","response_format":"b64_json"}'
```

当 `response_format` 为 `b64_json` 时，响应包含 Base64 编码的图片数据。

## 图片编辑示例

图片编辑使用 multipart 表单。所选模型支持多张输入图片时，可以重复 `image[]` 字段：

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/images/edits' \
  -H 'Authorization: Bearer <member-api-key>' \
  -F 'model=<enabled-image-model-id>' \
  -F 'prompt=Replace only the background with a soft blue gradient' \
  -F 'image[]=@input.png'
```

实际可用性取决于已安装的 Aster 版本、管理员启用能力、所选模型、已连接账号权益、Runner 就绪状态和上游可用性。参见 [Codex 生图与图片编辑指南](guides/codex-image-generation-editing.md)。

## 模型变体

`/v1/models` 只列出基础模型。不支持原生速度或推理字段的客户端，可以追加严格的模型变体后缀：

- `<基础模型>-fast` 或 `<基础模型>-standard`：选择处理速度。
- `<基础模型>-none`、`-low`、`-medium`、`-high`、`-xhigh` 或 `-max`：选择模型支持的推理强度。
- `<基础模型>-fast-high`：同时选择两项，固定使用“基础模型-速度-推理强度”顺序。

例如 `<基础模型>-fast-high` 会路由到 `<基础模型>`，并使用快速处理和 high 推理强度。变体名要求精确匹配，也不会出现在 `/v1/models` 中。

客户端支持时，优先使用协议原生字段：

| 协议 | 速度 | 推理强度 |
| --- | --- | --- |
| Responses | `"service_tier":"fast"` | `"reasoning":{"effort":"high"}` |
| Chat Completions | `"service_tier":"fast"` | `"reasoning_effort":"high"` |
| Anthropic Messages | `"speed":"fast"` | `"output_config":{"effort":"high"}` |

变体与显式字段冲突时，Aster 返回 `INVALID_REQUEST`（`32001`）。具体模型不支持某个组合时，上游也可能拒绝请求。

## 错误

错误包含稳定的字符串码和五位数字编号：

```json
{
  "error": {
    "code": "RUNNER_NOT_READY",
    "message": "当前没有满足条件的 Runner。",
    "number": 33003
  }
}
```

响应头 `X-Aster-Error-Number` 也会包含该编号。常见公开 API 错误包括：

| 编号 | 字符串码 | 含义 |
| --- | --- | --- |
| `31001` | `INVALID_API_KEY` | Key 缺失、格式错误、已撤销或成员状态失效 |
| `32001` | `INVALID_REQUEST` | 请求字段或查询参数无效 |
| `33001` | `MODEL_NOT_FOUND` | 模型不存在、未开放或没有账号映射 |
| `33002` | `MODEL_ACCOUNT_NOT_READY` | 当前没有可用的模型凭据实例 |
| `33003` | `RUNNER_NOT_READY` | 没有健康且兼容的 Runner；不会扣除额度 |
| `34001` | `INSUFFICIENT_QUOTA` | 扣除在途预留后可用额度不足 |
| `34005` | `DUPLICATE_REQUEST` | 请求 ID 已经用于预留或记账 |
| `35002` | `UPSTREAM_REQUEST_FAILED` | Runner 已接单，但上游请求失败 |

`request_id` 是请求追踪标识，不是错误码。寻求帮助时应同时保留两者。

## 流式请求与重试

请求只可能在发送到上游之前切换 Runner。流式输出开始后，Aster 不会在其他 Runner 上静默重放。应用应显式设置请求 ID，并且只重试可以安全重复的操作。
