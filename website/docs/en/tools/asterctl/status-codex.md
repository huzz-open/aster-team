---
title: "asterctl status codex"
description: "Inspect local Codex configuration completeness without network requests."
---

# asterctl status codex

Inspect local Codex configuration completeness without network requests.

## Syntax

```text
asterctl status codex
```

## Parameters

No command-specific parameters.

Use `asterctl status codex --help` to inspect help for the installed version.

## Examples

```powershell
asterctl status codex
```

## Configuration and runtime effects

Read configuration, the current-user key, and model catalog without changing them. Show only a masked key.

## Result

Print configuration path, provider, URL, key state, and catalog. Complete configuration reports ready; incomplete configuration reports incomplete and exits with failure.

## Troubleshooting

Run setup codex for incomplete configuration. ready only describes local completeness; use doctor codex to check remote authentication.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
