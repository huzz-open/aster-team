# How to connect an authorized AI account and synchronize models in Aster Team

[简体中文](../zh-CN/guides/connect-ai-account-and-sync-models.md) · [Deployment guides](README.md) · [Administrator guide](../admin-guide.md)

This guide connects a customer-authorized AI account or subscription to Aster Team, synchronizes its available models, and exposes selected models through independent member API keys.

## Prerequisites

- The organization controls or is authorized to use the account and subscription.
- The Aster product license includes the required account and Runner capabilities.
- At least one compatible Runner is online.
- The Runner can reach the provider's authentication and model endpoints through an organization-approved network path.
- The administrator understands the provider's usage limits, charges, and terms.

For a ChatGPT subscription connection, the Runner needs HTTPS access to `auth.openai.com` for authorization refresh and `chatgpt.com` for model synchronization and requests.

## Connect the account

1. Sign in to Admin and open **Subscriptions & accounts**.
2. Select **Add account** and start the OAuth authorization flow.
3. Sign in with the customer-authorized provider account and approve the requested access.
4. Return to Aster and confirm that the account is healthy.
5. Give the logical account a recognizable administrative name; do not use a password or secret as its name.

The connected upstream credential remains under administrator control. Developers receive Aster member keys rather than the provider password, browser session, access token, or refresh token.

## Synchronize and enable models

1. Select the connected account and run **Synchronize models**.
2. Review the returned model catalog and account health.
3. Enable only the models the team is allowed to consume.
4. Create or enable a member and grant quota.
5. Ask the member to create a personal API key and query `GET /v1/models`.

The public model list contains only administrator-enabled models. A synchronized upstream model is not automatically available to every member until the administrator enables it.

## Add capacity without sharing credentials

A logical account may contain multiple independently authorized credential instances. Each instance must come from a separate completed authorization flow. Copying one refresh token does not create independent capacity and is rejected as a duplicate credential.

Healthy Runners form a shared routing pool. Credentials are not permanently bound to one Runner; Aster selects an eligible execution path for each request.

## Troubleshooting

- **OAuth session invalid or expired:** start a new authorization flow instead of reusing the callback.
- **Model synchronization fails:** confirm Runner health and outbound DNS/TLS access, then retry synchronization on the existing account.
- **No models appear to members:** confirm the models were explicitly enabled and query `/v1/models` with an active member key.
- **`MODEL_ACCOUNT_NOT_READY` (`33002`):** the model exists, but no usable credential instance is ready.
- **`RUNNER_NOT_READY` (`33003`):** no Runner currently satisfies health, compatibility, heartbeat, and capacity checks.

See [Service boundaries](../service-boundaries.md) for account, network, entitlement, and provider responsibilities.
