---
manualSourceHash: 3986da62dcca924ee03b75570a3b874ca7d344a80433704e1ede9828ee353601
title: Manage model access
description: Connect upstream accounts, enable models and let members use their own quotas and API keys.
---

# Manage model access

## Connect an upstream

1. Check product authorization and open the upstream accounts/connections page in Admin.
2. For a ChatGPT subscription, ensure a Runner is online, add the account through OAuth and synchronize models. Synchronization can be retried later from the saved account.
3. For an official API key, choose the OpenAI, DeepSeek or GLM channel, enter a name and key. OpenAI/DeepSeek can synchronize automatically; GLM or custom public names can use manual public model IDs, upstream IDs and quota type. Choose per-image accounting for image-count models.
4. If synchronization fails, the saved connection remains. Add models manually or retry after addressing the error; do not repeatedly create the connection. New public models start disabled.

The Runner's HTTPS upstream domain allowlist is an outbound-network boundary. It does not filter prompts or response content. ChatGPT discovery/calls need `chatgpt.com`; OAuth refresh needs `auth.openai.com`.

## Give members access

1. Create or enable members within the signed consuming-seat limit.
2. Enable public models in model management and select which members may use them. Existence alone does not grant access.
3. Configure member quotas. Members log in to Member and create their own Aster API keys; these are distinct from the administrator's upstream credentials.
4. Members copy a public model ID from their available models, configure a client or make a test request, then inspect usage.

Product authorization, member model access, quota and upstream availability are separate checks. Use the specific error to choose the next action instead of repeatedly submitting requests merely because the license is active.

[First API request](/en/start/quickstart) · [Client setup](/en/tools/asterctl/quickstart) · [Model reference](/en/models/).
