# How to use image generation and image editing in Codex with Aster

[简体中文](../zh-CN/guides/codex-image-generation-editing.md) · [Developer guides](README.md) · [Compatible API](../api-reference.md)

Aster provides a supported local Codex path for text requests, image generation, and image editing through a customer-operated gateway. API-key and custom-provider setups can expose a different feature set from ChatGPT sign-in, so this guide describes the Aster path that can be tested and audited directly.

OpenAI states that API-key authentication supports local Codex workflows while some ChatGPT workspace or cloud features may be limited or unavailable. That is not a universal statement that API-key authentication cannot generate images. Capability depends on the client, provider configuration, enabled model, and endpoint support. See OpenAI's [authentication guide](https://learn.chatgpt.com/docs/auth).

## How the Aster path works

Codex sends Responses API traffic to the configured Aster provider. The provider configuration includes:

- the customer Aster `/v1` base URL;
- the developer's `ASTER_API_KEY`;
- the `responses` wire API;
- the non-secret `x-openai-actor-authorization = "aster-proxy"` compatibility marker.

Aster authenticates the member, checks quota and policy, selects a healthy Runner and authorized account, and records usage. Image operations are supported through Aster's image-generation and image-editing routes when the installed Aster version, enabled capability, selected model, Runner, and connected provider account all support them.

## Prerequisites

1. Complete [Codex setup with an Aster API key](configure-codex-api-key.md).
2. Run `asterctl doctor codex` successfully.
3. Confirm that your administrator has enabled an image-capable model.
4. Confirm that a compatible Runner and customer-authorized account are ready.
5. Fully restart Codex after configuration changes.

## Test image generation

Start a new local Codex task and use an explicit request such as:

> Generate a 1024 × 1024 product illustration of a private AI gateway, with a transparent-looking technical diagram style. Save the result in the current workspace.

Confirm that Codex creates the image artifact and that the request appears in Aster usage records under your member identity.

## Test image editing

Attach a PNG or JPEG to a new local Codex task and request a bounded edit, for example:

> Replace only the background with a soft blue gradient. Preserve the product, composition, dimensions, and all foreground text.

Confirm both the edited output and its corresponding Aster usage record. Clear constraints make it easier to distinguish a successful edit from a newly generated replacement.

## Direct API endpoints

Applications can also call the compatible endpoints directly:

- `POST /v1/images/generations`
- `POST /v1/images/edits`

The generation endpoint accepts JSON. The edit endpoint accepts multipart form data, including one or more `image[]` files. Use only model IDs currently shown in Member UI or returned by the Aster model catalog. See the [compatible model API](../api-reference.md) for authentication and error semantics.

## Troubleshooting

- **No image tool appears:** verify that Codex is using the Aster provider, the static compatibility header exists, and `ASTER_API_KEY` is visible to the newly started process.
- **`31001 INVALID_API_KEY`:** create or copy a valid member key and rerun setup.
- **`33003 RUNNER_NOT_READY`:** no healthy compatible Runner is available.
- **`34001 INSUFFICIENT_QUOTA`:** request additional member quota from the administrator.
- **`35002 UPSTREAM_REQUEST_FAILED`:** the Runner accepted the task but the connected provider failed; preserve the request ID for diagnosis.
- **Text works but images do not:** verify the Aster version, administrator-enabled capability, selected image model, connected account entitlement, and provider availability.

This guide describes Aster-supported local workflows. It does not promise every Codex cloud feature, every upstream image model, or permanent availability of a third-party capability.
