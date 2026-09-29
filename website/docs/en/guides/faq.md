---
title: FAQ
description: Common questions about Aster model IDs, protocols, grants, and parameter compatibility.
---

# FAQ

## Why does the model ID differ from the provider name?

Aster uses a deployment-defined public model ID. It can differ from the provider’s internal model name. Always copy it from the member model page.

## Why can’t I call a model that an administrator enabled?

The model must be open in the deployment and granted to the current member. The member model page shows the models available to you.

## Which text protocol should I choose?

Prefer Responses for new integrations, Chat Completions for common OpenAI Chat SDKs, and Messages for Anthropic clients. Field names change across protocols.

## What does “Converted” mean?

Aster converts the field name or a supported value. If conversion cannot preserve the requested semantics, the request is rejected before provider dispatch.
