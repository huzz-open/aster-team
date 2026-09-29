---
title: "asterctl remove claude"
description: "Remove managed Claude settings for a project and restore the previous file when backed up."
---

# asterctl remove claude

Remove managed Claude settings for a project and restore the previous file when backed up.

## Before running

Use the original project path and retain the setup state and any settings backup.

## Syntax

```text
asterctl remove claude [--project <PATH>]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--project <PATH>` | No | . | Project directory; relative paths resolve from the current working directory. |

Use `asterctl remove claude --help` to inspect help for the installed version.

## Examples

```powershell
asterctl remove claude --project "project-demo"
```

## Configuration and runtime effects

Restore the backup from the latest setup when settings existed beforehand; otherwise remove the generated settings. Remove managed state and the consumed backup on success. Claude itself is not uninstalled.

## Result

Print the removal result and exit 0 on success; conflicts or file-operation failures exit 1.

## Troubleshooting

Refuse to overwrite settings whose digest differs from the recorded one. A missing backup prevents restoration. Save and inspect local files before altering state records.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
