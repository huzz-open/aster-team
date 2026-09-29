---
manualSourceHash: ca254f033b2ebdc08fa74793960e8f19bfd44619d7483a2c2ce9324980d1be14
title: Product authorization
description: Check, request and import licenses; manage renewal, expiry and an explicit switch to free.
---

# Product authorization

## Check or import a license

If setup reports `license: active`, check the plan, capabilities and limits in Admin's product authorization page. Otherwise send the generated request JSON or PNG QR code to the operator through your offline delivery channel. To generate another request:

```bash
sudo aster-team-cli license request
```

Import the returned signed file through Admin, or run from its directory:

```bash
sudo aster-team-cli license install --source ./license.json
sudo aster-team-cli license status
```

On Windows, use the stable CLI path printed by `init.ps1` in administrator PowerShell instead of `sudo aster-team-cli`. CLI import may stop and restart Control; schedule for possible interruption and check service status afterward. Refreshing authorization after a successful update does not itself require another restart.

Licenses are checked locally without contacting a vendor server. The product baseline uses signed v2 capabilities and limits; earlier internal licenses/databases are not a supported migration source. Database settings or website text cannot extend signed rights.

## Free allowance

Free authorization does not expire and grants the explicitly listed gateway, member and Runner capabilities per installation. It does not provide an upstream account or upstream usage allowance.

| Resource | Limit | Counting rule |
| --- | --- | --- |
| Consuming member seats | 3 | Enabled identities that can consume models count, including consuming administrators. Disable to release; re-enable checks capacity. |
| Runners | 1 | Offline or disabled records still count; delete to release. |
| Upstream accounts | 1 | Disabled or invalid accounts still count; delete to release. |
| API keys per member | 1 | Unrevoked keys count, including during replacement; revoke to release. |

Actual rights come from the valid bundled license. A valid license does not necessarily grant every feature. ChatGPT enrollment, upstream synchronization and credential refresh require Runner execution in addition to gateway access. Creating an API-key connection or manually entering models requires gateway access; automatic synchronization also needs Runner execution. Error `51008` means a required capability is missing. With both capabilities present, a missing-ready-Runner error requires checking the node instead.

## Renewal and feature updates

A future-dated renewal can be imported early. Admin shows it as the next license; current rights remain until its signed start time. Control activates it atomically on startup, preflight or the next authorization check at/after that time. Upgrade the software first if the next license requires a newer version. A damaged pending file can be reimported without hiding the current license.

Paid subscriptions with a signed standard feature set receive new standard features through software upgrades while valid. Optional extensions require the corresponding updated grant. Free licenses only receive explicitly signed capabilities. Software authorization excludes upstream subscriptions, model usage, servers and network costs.

## Expiry and switching to free

After subscription expiry, existing authorized data remains readable/retrievable, login and recovery remain available, and users can reduce resources or revoke keys. New consumption, tasks, resource creation/re-enabling and quota issuance/redemption are restricted. Existing requests retain their existing authorization checks; no special keepalive or timed termination is added. Broken, mismatched or corrupted authorization does not qualify for these retained-access rules.

A valid renewal restores the relevant rights without removing data or configuration. Re-enabled members must log in again. Expiry does **not** automatically switch the installation to free.

To switch deliberately, open the free-switch area in product authorization:

1. Refresh the capacity check.
2. Reduce consuming seats, Runners and upstream accounts to the free limits yourself.
3. Revoke excess keys for each identity; administrators can also organize disabled members' keys. The display contains names/identifiers, not key plaintext.
4. Refresh, confirm and switch. The server rechecks capacity at commit; concurrent creation can require another cleanup/check.

The switch preserves existing data, paid-license history, audit and pending renewal. It does not delete resources. Ordinary license import or deleting history is not a substitute. A later paid subscription needs a newly issued license, not replay of an old paid file. If the area is absent, verify that the package contains an applicable signed free license; repair integrity/binding failures first.

## Interrupted import or status errors

An interrupted response does not mean saving failed. Keep the original file, check disk space, permissions and file locks, refresh status, and retry the same file if necessary. Do not delete license history or lock files. `91008 / BACKEND_LOCAL_STATE_FAILED` during concurrent authorization/resource changes calls for waiting, refreshing both authorization and resource lists, then retrying the unfinished operation.

A saved file may already be expired or not yet valid. Retry does not extend its term. Web imports preserve the original administrator and operation time; CLI imports are system actions. Audit recovery retries are durable and do not duplicate the same transaction, but a newly submitted import is a new operation. If pending audit storage reaches capacity, new imports stop while the current license remains; repair the database/disk problem instead of deleting recovery records.

[License status reference](/en/tools/aster-team-cli/license-status) · [Manage access](/en/administration/management).
