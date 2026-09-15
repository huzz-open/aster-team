# Aster Team

[简体中文](README.zh-CN.md) · [Documentation](docs/README.md) · [Releases](https://github.com/huzz-open/aster-team/releases) · [Issues](https://github.com/huzz-open/aster-team/issues)

Aster Team is a privately deployed AI gateway for teams. It gives administrators one place to connect customer-authorized AI accounts or subscriptions, issue independent member API keys, allocate quotas, route requests through customer-controlled runners, and audit usage without distributing upstream credentials.

**Turn customer-authorized AI capacity into independent, governed access for every developer.**

## What it provides

- **Private deployment** — Control, member access, local data, and optional runners stay in infrastructure you operate.
- **Unified access** — Developers use an OpenAI- or Anthropic-compatible endpoint instead of handling shared upstream credentials.
- **Per-member control** — Each member gets independent keys, quota, status, and usage records.
- **Auditable usage** — Requests are accounted for with stable request and error semantics. Prompt and response bodies are not stored in the business database.
- **Flexible execution** — Runners connect outbound to Control over WSS, so they do not require inbound ports.
- **One package** — Free and paid deployments use the same installer. A new installation can activate the bundled free license; a signed paid license changes entitlements without reinstalling the product.

## Independent developer access

```text
Customer-authorized AI account or subscription
                       │
                       ▼
                  Aster Team
              ┌────────┼────────┐
              ▼        ▼        ▼
        Developer A  Developer B  CI agent
         Member key   Member key   Member key
         Own quota    Own quota    Own quota
         Own usage    Own usage    Own usage
```

Aster does not create additional provider accounts. It creates independent, governed member access paths backed by capacity the customer is authorized to use. Upstream credentials remain under administrator control; each developer uses a separate Aster API key.

Popular developer guides:

- [Configure Codex with an Aster API key](docs/guides/configure-codex-api-key.md)
- [Use image generation and image editing in Codex with Aster](docs/guides/codex-image-generation-editing.md)
- [Run multiple independent Codex clients with one authorized AI subscription](docs/guides/multiple-independent-codex-clients.md)
- [Compare ChatGPT sign-in, OpenAI API keys, and Aster member keys](docs/guides/codex-authentication-comparison.md)
- [Configure Claude Code with an Aster API key](docs/guides/configure-claude-code-api-key.md)
- [Understand developer access and data isolation](docs/guides/developer-access-isolation.md)

Deployment and operations guides:

- [Install Aster Team on Linux](docs/guides/install-aster-team-linux.md)
- [Install Aster Team on Windows](docs/guides/install-aster-team-windows.md)
- [Deploy a dedicated Runner](docs/guides/deploy-dedicated-runner.md)
- [Connect an authorized AI account and synchronize models](docs/guides/connect-ai-account-and-sync-models.md)
- [Understand free, paid, and offline licenses](docs/guides/aster-team-license-guide.md)
- [Back up, restore, and upgrade Aster Team](docs/guides/backup-restore-upgrade.md)
- [Understand member quota and usage accounting](docs/guides/quota-reservation-and-usage-accounting.md)
- [Find an Aster Team error code](docs/guides/aster-error-codes.md)

## What Aster is not

Aster does not provide or resell third-party accounts, subscriptions, API credits, or model entitlements. It is not a general-purpose VPN, Internet access service, network egress service, or geographic-restriction bypass tool. Customers must supply accounts they are authorized to use, permitted connectivity to each provider, and any upstream subscription or usage fees. Read the complete [service boundaries and customer responsibilities](docs/service-boundaries.md).

## Start here

1. Read the [architecture and trust model](docs/architecture.md).
2. Download a package from [GitHub Releases](https://github.com/huzz-open/aster-team/releases) and [verify it](docs/release-verification.md).
3. Follow the [installation and first-run guide](docs/getting-started.md).
4. Connect an account or subscription your organization is authorized to use, then create a member API key.
5. Use the [compatible model API](docs/api-reference.md), [Codex](docs/codex.md), or [Claude Code](docs/claude-code.md).

> No public package is available until it appears in this repository's Releases page. Do not download Aster Team from third-party mirrors.

## Documentation

| Guide | Purpose |
| --- | --- |
| [Getting started](docs/getting-started.md) | Install Control, sign in, connect an account, and make a first request |
| [Architecture](docs/architecture.md) | Understand Control, Runner, trust boundaries, storage, and request flow |
| [Service boundaries](docs/service-boundaries.md) | Understand third-party account, network, billing, and availability responsibilities |
| [Administrator guide](docs/admin-guide.md) | Manage licenses, accounts, models, members, keys, and quotas |
| [Member guide](docs/member-guide.md) | Create keys, find endpoints, and use the integration pages |
| [API reference](docs/api-reference.md) | Compatible endpoints, authentication, model variants, errors, and examples |
| [Codex](docs/codex.md) | Route Codex in the ChatGPT desktop app or Codex CLI through Aster |
| [Claude Code](docs/claude-code.md) | Configure Claude Code against the Anthropic-compatible endpoint |
| [Task-focused guides](docs/guides/README.md) | Linux, Windows, Runner, licensing, accounts, API keys, Codex, Claude Code, and troubleshooting |
| [Operations](docs/operations.md) | Health checks, logs, backup, upgrade, and recovery |
| [Troubleshooting](docs/troubleshooting.md) | Diagnose common installation, routing, and client problems |

The [documentation index](docs/README.md) links every English and Chinese page.

## Support and issue safety

When opening an issue, never attach passwords, API keys, license files, private keys, cookies, databases, complete diagnostic bundles, or unredacted logs.

## Trademarks and compatibility

Aster Team is an independent product. OpenAI, ChatGPT, and Codex are trademarks of OpenAI. Anthropic and Claude are trademarks of Anthropic. Compatibility references describe supported client protocols and do not imply endorsement.
