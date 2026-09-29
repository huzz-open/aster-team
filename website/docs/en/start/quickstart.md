---
title: Quickstart for the Aster model gateway
description: Configure an Aster API key, base URL, and public model ID, then make a Responses request.
---

# Your first request

Create an Aster API key in the member portal and copy a **public model ID** from your model list. A public ID may differ from the provider’s model name. The API URL for this installation is shown here:

<InstanceApiBase language="en" />

Set `ASTER_BASE_URL` to the API URL shown above and `ASTER_API_KEY` to your member key. When reading on the public website, get the API URL from your member portal.

<InstanceCurlExample language="en" kind="responses" />

Generic template:

```bash
curl "$ASTER_BASE_URL/v1/responses" \
  -H "Authorization: Bearer $ASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"YOUR_PUBLIC_MODEL","input":"Hello"}'
```

The key is issued by Aster, not an upstream OpenAI, DeepSeek, or GLM key.

## Switch models

For basic text, changing `model` is usually enough. Check the [model quick reference](/en/models/) before using reasoning levels, forced tools, strict JSON Schema, or images. Aster rejects a field when it cannot preserve the requested behavior.

If your client only lets you enter a model ID, append execution options to an available base model: `-fast` selects fast processing and `-high` selects high reasoning effort. Combine them in that order, for example `gpt-5.6-terra-fast-high`. Other effort suffixes are `-none`, `-low`, `-medium`, `-xhigh`, and `-max`; you can also specify `-standard` processing. Model lists show base models only, so add variants manually in your client. The upstream model must support the chosen combination.

## Next steps

- [asterctl tool](/en/guides/asterctl) to configure Codex or Claude Code and diagnose connections.
- [Responses](/en/api/responses) for text and tools.
- [Chat Completions](/en/api/chat-completions) for Chat SDKs.
- [Anthropic Messages](/en/api/messages) for Messages clients.
