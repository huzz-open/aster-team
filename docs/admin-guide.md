# Administrator guide

[简体中文](zh-CN/admin-guide.md) · [Documentation](README.md)

## Recommended setup order

1. **Product license** — confirm the bundled free license or import a signed license. Free and paid entitlements use the same installed product.
2. **Platform settings** — set the public Model API address and verify TLS before sharing it with members.
3. **Runners** — keep at least one compatible Runner online; add capacity or fault domains when needed.
4. **Subscriptions & accounts** — authorize a ChatGPT subscription account, then synchronize models.
5. **Models** — enable only the models the team should consume.
6. **Members & quota** — create individual member identities and grant quota according to team policy.
7. **Audit and usage** — review consumption, account health, Runner availability, and security-relevant changes.

## Licenses

A fresh package can import its bundled free license automatically. To install a separately delivered signed license from the host:

```bash
sudo aster-team-cli license install --source ./license.json
sudo aster-team-cli license status
```

The license is verified locally. Importing another valid license changes entitlements without reinstalling Aster. Always use **Product license** as the source of truth for current limits.

## Members and credentials

Give every human or automation identity its own member account and API key. Do not share one member key across a team: independent keys make revocation, quota, and incident review useful.

When resetting a member password, Aster generates a one-time temporary password and revokes existing sessions. Deliver it over a secure channel. If an administrator loses access, run on the Control host:

```bash
sudo aster-team-cli password reset-admin
```

An email can be selected non-interactively with `--email admin@example.com`; the new password itself is still entered through the hidden prompt.

## Accounts and models

Before starting ChatGPT OAuth, ensure the host can reach `auth.openai.com` and `chatgpt.com` over HTTPS and a Runner is online. After authorization, synchronize the account's models, then explicitly enable the ones exposed to members.

Account credentials are administrator-managed. Members consume enabled models through Aster API keys and never need the upstream credentials.

## Quota and usage

Quota is reserved before a request and settled from trusted usage after execution. Requests that never reach the provider do not consume quota. Use the Admin usage and audit views to investigate unusual consumption rather than relying only on provider totals.

## Runner placement

Use a same-host Runner for a simple deployment. Add dedicated Runners when execution needs separate egress, geography, host isolation, or capacity. A Runner host is trusted with the content it forwards; place it accordingly.
