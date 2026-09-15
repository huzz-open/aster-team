# How to configure Codex with an Aster API key

[简体中文](../zh-CN/guides/configure-codex-api-key.md) · [Developer guides](README.md) · [Codex reference](../codex.md)

This guide configures Codex with a custom API key, custom API base URL, and custom model provider supplied by Aster Team. It is the recommended path when an organization wants each developer to use an independent Aster member key while administrators control quotas, routing, and usage records.

## Choose the correct authentication path

These three paths are different:

| Path | Credential and endpoint | Best for |
| --- | --- | --- |
| ChatGPT sign-in | ChatGPT account at OpenAI | OpenAI-hosted subscription and workspace features |
| `codex login --with-api-key` | OpenAI Platform API key at OpenAI | Direct, usage-billed OpenAI API access |
| Aster custom provider | Aster member key at the customer Aster endpoint | Customer-operated access, per-member quota, routing, and audit |

OpenAI documents API-key authentication for local Codex workflows and notes that some ChatGPT workspace or cloud features may be limited or unavailable. Aster uses Codex's documented custom-provider configuration; it does not replace or modify the OpenAI-hosted sign-in flows. See OpenAI's [authentication guide](https://learn.chatgpt.com/docs/auth) and [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference).

## Prerequisites

- Install the official Codex app or Codex CLI.
- In Aster Member UI, create your personal API key.
- Copy the public Aster API address shown by your administrator.
- Confirm that an administrator-enabled model and a healthy Runner are available.
- Fully exit Codex before changing its configuration.

## Recommended setup with `asterctl`

Download `asterctl` from **Member UI → API documentation → asterctl tool**, then run:

```powershell
asterctl setup codex --base-url "https://aster.example.com/v1" --set-key --launch
```

Enter the member key at the hidden prompt. This avoids placing the key in shell history. `asterctl` validates the endpoint and key, stores `ASTER_API_KEY` for the current user, merges the Aster provider into the existing Codex configuration, and checks the effective provider. Omit `--launch` when you do not want Codex to start automatically.

Verify the result:

```powershell
asterctl status codex
asterctl doctor codex
```

## Manual configuration

When `asterctl` is unavailable, store the member key in the `ASTER_API_KEY` environment variable and add this top-level configuration to Codex `config.toml`:

```toml
model_provider = "aster"

[model_providers]

[model_providers.aster]
base_url = "https://aster.example.com/v1"
env_key = "ASTER_API_KEY"
name = "Aster Team"
wire_api = "responses"

[model_providers.aster.http_headers]
x-openai-actor-authorization = "aster-proxy"
```

On Windows, the default file is `%USERPROFILE%\.codex\config.toml`. When `CODEX_HOME` is set, use that directory instead. `ASTER_API_KEY` is the secret credential. The `x-openai-actor-authorization` value is a non-secret compatibility marker and does not replace API-key authentication.

## Troubleshooting

- **Codex still uses the previous provider:** fully exit every Codex process, confirm the top-level `model_provider = "aster"`, then reopen Codex.
- **The key is missing:** open a new terminal after setting the user environment variable and run `asterctl doctor codex`.
- **The model is unavailable:** ask the administrator to confirm model enablement, account readiness, and Runner health.
- **The endpoint is wrong:** Codex uses the OpenAI-compatible Aster URL ending in `/v1`.
- **You expected Codex cloud features:** an Aster custom provider is a local Codex workflow; it is not an Aster-hosted replacement for OpenAI's cloud or ChatGPT workspace features.

For image workflows, continue with [Use image generation and image editing in Codex with Aster](codex-image-generation-editing.md). For the full command reference, see [Use Codex through Aster](../codex.md).
