# How Aster isolates developer access, quotas, and usage

[简体中文](../zh-CN/guides/developer-access-isolation.md) · [Developer guides](README.md) · [Architecture](../architecture.md)

Aster separates developer access at the gateway identity and governance layer. It is designed to prevent teams from distributing an upstream provider credential while still giving each person or automation identity a separately revocable path.

## Isolation layers

| Layer | Aster behavior | Customer responsibility |
| --- | --- | --- |
| Identity and key | Each member or automation identity receives a separate Aster API key | Do not reuse keys across people; rotate or revoke exposed keys |
| Quota | Grants, reservations, and available balance are evaluated for the authenticated member | Define allocation policy and monitor exceptional use |
| Usage and audit | Requests are attributed to the member path and recorded for operational review | Set retention and access policy for usage records and logs |
| Upstream secret | Members do not need the connected account credential | Restrict Control and Runner administration and protect backups |
| Local Codex state | Can be isolated with separate devices, OS users, or `CODEX_HOME` | Do not share the same local profile when history separation is required |
| Prompt and response bodies | Aster's business database is not intended as a prompt/response archive | Runner operators can inspect handled traffic; configure host and log security accordingly |

## Boundaries that remain

- The third-party provider processes requests under the connected customer-authorized account.
- Administrators with access to a Runner host can inspect prompts and responses handled there.
- Two clients using the same OS account, process environment, and `CODEX_HOME` do not have a strong local-history boundary.
- Customer DNS, TLS, proxies, observability systems, and host logs may contain operational or request data according to their configuration.
- Aster separation does not create separate upstream provider identities, subscriptions, credits, or contractual entitlements.

## Recommended deployment controls

1. Create one member or automation identity per accountable actor.
2. Give every identity its own API key and quota grant.
3. Use separate OS users or managed devices when local Codex history must be isolated.
4. Run Runners only on customer-controlled hosts with restricted administrator access.
5. Treat member keys, upstream credentials, logs, backups, and diagnostics as sensitive.
6. Review usage by member and revoke keys immediately when a person or workload leaves scope.

For the end-to-end pattern, see [Run multiple independent Codex clients](multiple-independent-codex-clients.md). For product and customer responsibilities, see [Service boundaries](../service-boundaries.md).
