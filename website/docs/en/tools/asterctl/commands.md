---
title: "asterctl command reference"
description: "Index of all public asterctl subcommands, parameters, and use cases."
---

# asterctl command reference

Choose a command by task; open its page for parameters, examples, and runtime effects.

## Setup

- [`setup codex`](/en/tools/asterctl/setup-codex): Configure the Aster provider, member key, and model catalog for Windows Codex Desktop.
- [`setup claude`](/en/tools/asterctl/setup-claude): Configure the Aster URL, member key, and model mappings for a Claude Code project.

## Status and diagnostics

- [`status codex`](/en/tools/asterctl/status-codex): Inspect local Codex configuration completeness without network requests.
- [`status claude`](/en/tools/asterctl/status-claude): Inspect the managed state and settings integrity of a Claude project without network requests.
- [`doctor codex`](/en/tools/asterctl/doctor-codex): Check Codex configuration, the member key, Aster connectivity, and the local model catalog.
- [`doctor claude`](/en/tools/asterctl/doctor-claude): Check Claude project configuration, authentication, client version, and platform model mappings.

## Removal and version

- [`remove codex`](/en/tools/asterctl/remove-codex): Revert Codex fields and the generated model catalog managed by asterctl.
- [`remove claude`](/en/tools/asterctl/remove-claude): Remove managed Claude settings for a project and restore the previous file when backed up.
- [`version`](/en/tools/asterctl/version): Show the asterctl version and build commit.

## Help and exit codes

```text
asterctl --help
asterctl setup codex --help
asterctl --version
```

Help and version queries return 0 on success; command execution failures return 1 and argument parsing failures normally return 2. Read the reported state as well: declining to overwrite Claude settings exits successfully without updating configuration.
