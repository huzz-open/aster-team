---
title: "Quick setup with asterctl"
description: "Connect Codex Desktop or Claude Code to Aster with a member key and verify the configuration."
---

# Quick setup with asterctl

Complete [tool installation](/en/guides/asterctl), and obtain the Aster API URL and a member key. These commands assume asterctl is on PATH.

## Codex Desktop

On Windows, install Codex and close it completely, including background processes. Use an API URL ending in `/v1`:

```powershell
asterctl setup codex --base-url "https://aster.example.com/v1" --set-key --launch
asterctl doctor codex
```

Enter the member key at the hidden prompt. Create a new task after setup. Omit `--launch` to configure now and start the client manually later. For URL, key, and catalog updates, see [setup codex](/en/tools/asterctl/setup-codex).

## Claude Code

The current implementation requires Claude Code `2.1.255` or later; run `claude update` if needed. Use the API origin without `/v1`:

```powershell
asterctl setup claude --base-url "https://aster.example.com" --project "project-demo" --set-key --launch
asterctl doctor claude --project "project-demo"
```

Omit `--project` when already in the project directory. Existing project settings require overwrite confirmation and are backed up. Settings and backups may contain a key; keep them out of version control. See [setup claude](/en/tools/asterctl/setup-claude).

## After setup

- Inspect local state: [status codex](/en/tools/asterctl/status-codex) / [status claude](/en/tools/asterctl/status-claude).
- Remove managed configuration: [remove codex](/en/tools/asterctl/remove-codex) / [remove claude](/en/tools/asterctl/remove-claude).
- If checks fail, see [troubleshooting](/en/tools/asterctl/troubleshooting).
