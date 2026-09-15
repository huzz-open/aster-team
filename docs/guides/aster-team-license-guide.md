# Aster Team free license, paid license, and offline activation guide

[简体中文](../zh-CN/guides/aster-team-license-guide.md) · [Deployment guides](README.md) · [Administrator guide](../admin-guide.md)

Aster Team uses one installer for free and paid deployments. Product capabilities and resource limits come from a locally verified signed license; changing to a paid license does not require reinstalling the gateway.

## Start with the bundled free license

When a release package contains a valid bundled free license, a new Control installation verifies and activates it automatically. After installation, open **Admin → Product license** or run:

```bash
sudo aster-team-cli license status
```

The current bundled free license provides the basic account connection, member collaboration, and Runner execution flow. Its signed resource limits are:

| Resource | Free limit | How capacity is released |
| --- | ---: | --- |
| Member seats | 3 | Disable a model-consuming member |
| Runners | 1 | Delete the Runner; offline or disabled Runners still occupy capacity |
| Subscriptions & accounts | 1 | Delete the logical account; disabled or invalid accounts still occupy capacity |
| Active API keys per member | 1 | Revoke the old key |

Use **Product license** as the source of truth for the installed release. The Aster license governs Aster features and limits; upstream accounts, subscriptions, credits, and provider charges remain separate.

## Complete offline activation

When no active license was installed automatically, create an offline request from a secure directory:

```bash
sudo aster-team-cli license request
```

The command creates a timestamped JSON request and QR image. Transfer one of them to the licensing contact through the agreed channel. When you receive the signed `license.json`, import it in **Admin → Product license** or run:

```bash
sudo aster-team-cli license install --source ./license.json
sudo aster-team-cli license status
```

License validation occurs on the customer host and does not require a runtime connection to an Aster licensing server.

## Upgrade from free to paid

Import the signed paid license into the existing installation. Aster retains members, accounts, models, usage records, settings, and Runner identities. Refresh the license state in Admin; no reinstall or data reset is required.

A future-dated renewal can be imported in advance. Admin displays it as the next license while the current license remains active, then activates it when its signed start time is reached and the installed version satisfies its minimum version.

## What happens when a paid license expires

Existing authorized data remains available for review and recovery actions. New model requests and operations that increase licensed resource use pause until a valid renewal is activated.

Expiration does not automatically replace the paid license with the free license. To continue with free limits, use **Product license → Switch to free**, review current member, Runner, account, and per-member key usage, reduce each resource to the signed free limit, and confirm the switch. Disabling a Runner or account does not release its entity limit; delete it when it is no longer needed.

## Common license errors

- `51001 LICENSE_MISSING`: install the bundled or issued signed license.
- `51002 LICENSE_EXPIRED`: import a renewal or switch to free after reducing resources.
- `51005 LICENSE_MACHINE_MISMATCH`: request a license for this installation identity.
- `51006 LICENSE_VERSION_INVALID`: upgrade Aster to the minimum version required by the license.
- `51008 FEATURE_NOT_LICENSED`: review the feature list in **Product license**.

See [Aster Team error codes](aster-error-codes.md) for additional installation and API errors.
