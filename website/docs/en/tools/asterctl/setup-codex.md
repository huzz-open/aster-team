---
title: "asterctl setup codex"
description: "Configure the Aster provider, member key, and model catalog for Windows Codex Desktop."
---

# asterctl setup codex

Configure the Aster provider, member key, and model catalog for Windows Codex Desktop.

## Before running

Install Codex and close the desktop application and its background processes. Run as the Windows user who uses Codex, with an Aster member key.

## Syntax

```text
asterctl setup codex [--base-url <URL>] [--set-key] [--launch]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--base-url <URL>` | Conditional | Saved URL | Aster API URL ending in /v1. Required for first setup; supply this option, --set-key, or both. |
| `--set-key` | No | false | Prompt for a member key without echo. Used alone, requires a saved Aster URL; --launch alone cannot initialize configuration. |
| `--launch` | No | false | Launch Codex after successful configuration. |

Use `asterctl setup codex --help` to inspect help for the installed version.

## Examples

```powershell
asterctl setup codex --base-url "https://aster.example.com/v1" --set-key --launch
asterctl setup codex --base-url "https://aster.example.com/v1"
asterctl setup codex --set-key
```

## Configuration and runtime effects

Merge config.toml under CODEX_HOME (default: %USERPROFILE%\.codex), generate a model catalog, and record managed changes. A new key is stored in the current-user ASTER_API_KEY environment variable. Preserve the default model and unrelated settings. Catalog generation requires the installed Codex debug models capability; server permissions still govern model access.

## Result

Print the target URL, catalog path, and model count. Without --launch, reopen Codex and create a new task. An empty key continues a URL-only update when a URL was supplied; a key-only invocation with empty input exits successfully without changes.

## Troubleshooting

If Codex is running, close it before retrying. For a missing key, run setup codex --set-key. For catalog verification failures, check the installation and debug models support. A launch failure after setup does not undo configuration; inspect it with status codex.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
