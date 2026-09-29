---
title: Authentication
description: Authenticate model requests with an Aster API key.
---

# Authentication

Create an API key in the member portal and keep it in a server-side environment variable. Do not embed it in browser code, mobile packages, or public repositories.

## OpenAI-compatible APIs

```http
Authorization: Bearer $ASTER_API_KEY
```

Use this header for Responses, Chat Completions, image generation, and image editing.

## Anthropic Messages

```http
x-api-key: $ASTER_API_KEY
anthropic-version: 2023-06-01
```

Disabled or expired keys and unauthorized models are rejected before provider dispatch.
