---
title: "asterctl doctor codex"
description: "Check Codex configuration, the member key, Aster connectivity, and the local model catalog."
---

# asterctl doctor codex

Check Codex configuration, the member key, Aster connectivity, and the local model catalog.

## Before running

Run on Windows after setup codex.

## Syntax

```text
asterctl doctor codex
```

## Parameters

No command-specific parameters.

Use `asterctl doctor codex --help` to inspect help for the installed version.

## Examples

```powershell
asterctl doctor codex
```

## Configuration and runtime effects

Read local settings, call the models endpoint with the saved key, and invoke local Codex provider/catalog checks. No generation request or automatic repair is performed.

## Result

Exit 0 when all checks pass; exit 1 and print the reason when a check fails.

## Troubleshooting

Reset the key if missing or rejected. Check the deployment URL and certificate for URL/TLS failures; check the Codex version for catalog failures.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
