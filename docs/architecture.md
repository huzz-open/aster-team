# Architecture and trust model

[简体中文](zh-CN/architecture.md) · [Documentation](README.md)

Aster Team separates policy and identity from upstream execution.

```text
Developer client
      │ member API key / HTTPS
      ▼
Control ── signed one-use task / WSS ──► Runner ── HTTPS ──► AI provider
      ▲                                  │
      └──────── streamed result ─────────┘
```

## Components

- **Control** authenticates members and API keys, applies model and quota policy, selects a healthy Runner, records usage metadata, and serves Admin, Member, and model API endpoints.
- **Runner** is an execution node operated by the customer. It opens an outbound WSS connection to Control and makes approved outbound HTTPS requests to AI providers. It requires no inbound listening port.
- **Admin UI** manages licenses, subscriptions and accounts, models, Runners, members, quota, and audit records.
- **Member UI** lets each member manage their password and API keys, view quota and usage, and obtain client-specific integration instructions.

See [Deploy a dedicated Runner](guides/deploy-dedicated-runner.md) for installation, enrollment, trust, and routing checks.

## Credential boundary

Members receive only Aster API keys. They do not receive upstream access tokens, refresh tokens, or upstream account identifiers. Control stores upstream credential instances encrypted at rest and authorizes them to a Runner for an individual task.

The operating-system administrator of a Runner can observe prompts and responses handled by that Runner. Run Runners only on customer-controlled hosts; Aster does not describe untrusted member machines as zero-trust execution nodes.

## Task integrity and routing

Each task authorization is short-lived, single-use, and bound to its Runner, task ID, command, credential revision, nonce, and payload digest. Invalid, expired, mismatched, or replayed tasks are rejected.

Eligible Runners form a shared pool. Aster prefers useful affinity, then selects a healthy low-load candidate. It may fail over before an upstream request begins. It does not migrate an in-progress stream after output has started.

If no compatible Runner is ready, the API returns `RUNNER_NOT_READY` (`33003`) and does not deduct quota.

## Data and audit boundary

Aster records the metadata needed for authentication, routing, quota accounting, model usage, and audit events in its local database. Prompt and response bodies are not stored in the business database. Application and infrastructure logs may still contain operational details, so redact them before sharing.

## Network boundary

- Clients should reach Control over HTTPS outside a trusted LAN.
- Runners reach Control over WSS and providers over HTTPS.
- A custom CA can be added for the Runner-to-Control connection without changing the provider trust store.
- Provider allowlists limit outbound destinations; they do not inspect or filter prompt content.
- The customer supplies permitted provider connectivity. Runner is not a general-purpose VPN, Internet gateway, or restriction-bypass relay.

See [Service boundaries and customer responsibilities](service-boundaries.md) for the account, billing, network, and availability boundary.
