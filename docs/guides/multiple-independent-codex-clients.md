# How to run multiple independent Codex clients with one authorized AI subscription

[简体中文](../zh-CN/guides/multiple-independent-codex-clients.md) · [Developer guides](README.md) · [Service boundaries](../service-boundaries.md)

Aster turns customer-authorized AI capacity into independent, governed access paths for developers. It does not distribute the provider password, session, cookie, or upstream credential. Each developer authenticates to Aster with a separate member API key.

```text
Customer-authorized account or subscription
                    │
                    ▼
      Aster Control + customer Runners
          │          │          │
          ▼          ▼          ▼
     Developer A  Developer B   CI agent
      member key   member key   member key
       own quota    own quota    own quota
       own usage    own usage    own usage
```

## Deployment workflow

1. An administrator connects an account or subscription the organization is authorized to use.
2. A customer-operated Runner becomes healthy and synchronizes administrator-enabled models.
3. The administrator creates members and grants quota according to team policy.
4. Each member signs in to Member UI and creates a personal Aster API key.
5. Each developer configures Codex on their own device or OS account with `asterctl setup codex`.
6. Administrators review usage, change quota, or revoke an individual member key without distributing the upstream credential.

## What “independent” means

- **Identity:** every developer has a separate Aster member and key.
- **Quota:** grants and available balance are evaluated per member.
- **Accounting and audit:** requests are attributed to the member path that made them.
- **Revocation:** one key can be revoked without rotating every developer's credential.
- **Local Codex state:** history and local configuration remain separate when developers use separate devices, OS users, or `CODEX_HOME` directories.

For multiple clients on one computer, separate OS users provide the clearest boundary. Advanced users can instead use distinct `CODEX_HOME` directories and process environments, but they must ensure that keys and history are not reused across sessions.

## What Aster does not isolate

- The connected provider processes requests under the customer-authorized account and its own policies.
- Administrators of a Runner host can inspect prompts and responses handled by that Runner.
- Codex sessions sharing the same OS user and `CODEX_HOME` can share local configuration and history.
- Network infrastructure and operational logs remain within the customer's security and retention responsibilities.
- Independent Aster access paths do not create additional provider accounts, subscriptions, credits, or entitlements.

This architecture is intended for authorized organizational use, not credential resale or circumvention. Review [service boundaries](../service-boundaries.md), [access isolation](developer-access-isolation.md), and the relevant provider terms before deployment.

## Frequently asked questions

### Is this the same as sharing a provider password?

No. Members receive Aster keys. The upstream credential remains under administrator control and is used only through the customer-operated gateway path.

### Does every developer receive a separate provider account?

No. Aster creates independent governance paths backed by capacity the customer already controls; it does not create upstream accounts.

### Can one developer consume another developer's quota?

Requests are charged to the authenticated member path. Protect each member key and do not reuse it across people or automation identities.
