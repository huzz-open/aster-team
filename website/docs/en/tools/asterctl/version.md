---
title: "asterctl version"
description: "Show the asterctl version and build commit."
---

# asterctl version

Show the asterctl version and build commit.

## Syntax

```text
asterctl version
```

## Parameters

No command-specific parameters.

Use `asterctl version --help` to inspect help for the installed version.

## Examples

```powershell
asterctl version
asterctl --version
asterctl --help
asterctl setup codex --help
```

## Configuration and runtime effects

No configuration changes or network requests. --version is the short version output; version also shows the build commit. --help is available at the root and for each subcommand.

## Result

Print version and Commit metadata to identify the tool in issue reports.

## Troubleshooting

Use the absolute executable path or check the user PATH if the command is not found.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
