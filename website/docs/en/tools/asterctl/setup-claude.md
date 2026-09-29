---
title: "asterctl setup claude"
description: "Configure the Aster URL, member key, and model mappings for a Claude Code project."
---

# asterctl setup claude

Configure the Aster URL, member key, and model mappings for a Claude Code project.

## Before running

Install Claude Code first. The current implementation requires version 2.1.255 or later; run claude update when needed.

## Syntax

```text
asterctl setup claude --base-url <URL> --set-key [--project <PATH>] [--launch]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--base-url <URL>` | Yes | — | Aster API origin without /v1. |
| `--set-key` | Yes | false | Required at runtime; the key is read through a hidden prompt. |
| `--project <PATH>` | No | . | Project directory; relative paths resolve from the current working directory. |
| `--launch` | No | false | Launch Claude in the project after setup. |

Use `asterctl setup claude --help` to inspect help for the installed version.

## Examples

```powershell
asterctl setup claude --base-url "https://aster.example.com" --project "project-demo" --set-key --launch
```

## Configuration and runtime effects

Create the project directory if needed. Write .claude/settings.local.json and .claude/.asterctl-state.json. Existing settings require interactive overwrite confirmation and are backed up before writing. Settings and backups can contain a key and must stay out of version control. Repeated setup uses the file immediately before that setup as the restore baseline.

## Result

Print the project, settings path, and model alias mappings. Follow with doctor claude --project. Declining overwrite exits successfully without changing existing settings.

## Troubleshooting

Missing --set-key, an empty key, an unsupported version, or unavailable model mappings cause errors. Rerun setup for the same project after mapping changes. If launch fails, inspect the written settings with status claude.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
