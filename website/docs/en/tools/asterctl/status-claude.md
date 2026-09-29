---
title: "asterctl status claude"
description: "Inspect the managed state and settings integrity of a Claude project without network requests."
---

# asterctl status claude

Inspect the managed state and settings integrity of a Claude project without network requests.

## Syntax

```text
asterctl status claude [--project <PATH>]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--project <PATH>` | No | . | Project directory; relative paths resolve from the current working directory. |

Use `asterctl status claude --help` to inspect help for the installed version.

## Examples

```powershell
asterctl status claude --project "project-demo"
```

## Configuration and runtime effects

Read project settings and .asterctl-state.json and compare the recorded setup digest, without modifying files.

## Result

On success, report unchanged managed settings, the project path, and the Claude version recorded at setup.

## Troubleshooting

A missing project, missing setup record, or edited settings cause failure. Check --project and preserve your edits before deciding whether to rerun setup.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
