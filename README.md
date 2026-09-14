# Aster Team

[简体中文](README.zh-CN.md) · [Documentation](docs/README.md) · [Releases](https://github.com/huzz-open/aster-team/releases) · [Issues](https://github.com/huzz-open/aster-team/issues)

Aster Team is a privately deployed AI gateway for teams. It gives administrators one place to connect AI subscription accounts, issue independent member API keys, allocate quotas, route requests through customer-controlled runners, and audit usage without distributing upstream credentials.

## What it provides

- **Private deployment** — Control, member access, local data, and optional runners stay in infrastructure you operate.
- **Unified access** — Developers use an OpenAI- or Anthropic-compatible endpoint instead of handling shared upstream credentials.
- **Per-member control** — Each member gets independent keys, quota, status, and usage records.
- **Auditable usage** — Requests are accounted for with stable request and error semantics. Prompt and response bodies are not stored in the business database.
- **Flexible execution** — Runners connect outbound to Control over WSS, so they do not require inbound ports.
- **One package** — Free and paid deployments use the same installer. A new installation can activate the bundled free license; a signed paid license changes entitlements without reinstalling the product.

## Start here

1. Read the [architecture and trust model](docs/architecture.md).
2. Download a package from [GitHub Releases](https://github.com/huzz-open/aster-team/releases) and [verify it](docs/release-verification.md).
3. Follow the [installation and first-run guide](docs/getting-started.md).
4. Connect a subscription account and create a member API key.
5. Use the [compatible model API](docs/api-reference.md), [Codex](docs/codex.md), or [Claude Code](docs/claude-code.md).

> No public package is available until it appears in this repository's Releases page. Do not download Aster Team from third-party mirrors.

## Documentation

| Guide | Purpose |
| --- | --- |
| [Getting started](docs/getting-started.md) | Install Control, sign in, connect an account, and make a first request |
| [Architecture](docs/architecture.md) | Understand Control, Runner, trust boundaries, storage, and request flow |
| [Administrator guide](docs/admin-guide.md) | Manage licenses, accounts, models, members, keys, and quotas |
| [Member guide](docs/member-guide.md) | Create keys, find endpoints, and use the integration pages |
| [API reference](docs/api-reference.md) | Compatible endpoints, authentication, model variants, errors, and examples |
| [Codex](docs/codex.md) | Route Codex in the ChatGPT desktop app or Codex CLI through Aster |
| [Claude Code](docs/claude-code.md) | Configure Claude Code against the Anthropic-compatible endpoint |
| [Operations](docs/operations.md) | Health checks, logs, backup, upgrade, and recovery |
| [Troubleshooting](docs/troubleshooting.md) | Diagnose common installation, routing, and client problems |

The [documentation index](docs/README.md) links every English and Chinese page.

## Repository scope

This public repository contains release packages, checksums, developer and user documentation, and public issue tracking. It does **not** contain the Aster Team product source code, signing material, customer data, licenses, or internal operations documentation.

When opening an issue, never attach passwords, API keys, license files, private keys, cookies, databases, complete diagnostic bundles, or unredacted logs.

## Trademarks and compatibility

Aster Team is an independent product. OpenAI, ChatGPT, and Codex are trademarks of OpenAI. Anthropic and Claude are trademarks of Anthropic. Compatibility references describe supported client protocols and do not imply endorsement.
