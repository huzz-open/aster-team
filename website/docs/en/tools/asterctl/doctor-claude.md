---
title: "asterctl doctor claude"
description: "Check Claude project configuration, authentication, client version, and platform model mappings."
---

# asterctl doctor claude

Check Claude project configuration, authentication, client version, and platform model mappings.

## Before running

Run against a project initialized with setup claude.

## Syntax

```text
asterctl doctor claude [--project <PATH>]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--project <PATH>` | No | . | Project directory; relative paths resolve from the current working directory. |

Use `asterctl doctor claude --help` to inspect help for the installed version.

## Examples

```powershell
asterctl doctor claude --project "project-demo"
```

## Configuration and runtime effects

Check local state, read the current Claude version, request platform settings, and compare with the project file. No generation request or automatic settings rewrite is performed.

## Result

Exit 0 when all checks pass; exit 1 and print the reason when a check fails.

## Troubleshooting

Rerun setup claude if the current Claude version or platform mappings differ from saved settings. Review manual file edits first.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
