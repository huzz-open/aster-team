---
title: "asterctl remove codex"
description: "Revert Codex fields and the generated model catalog managed by asterctl."
---

# asterctl remove codex

Revert Codex fields and the generated model catalog managed by asterctl.

## Before running

On Windows, close Codex completely and retain the state file created by setup.

## Syntax

```text
asterctl remove codex
```

## Parameters

No command-specific parameters.

Use `asterctl remove codex --help` to inspect help for the installed version.

## Examples

```powershell
asterctl remove codex
```

## Configuration and runtime effects

Revert only fields that still match managed values. Preserve later edits and ASTER_API_KEY. Remove an unchanged generated catalog and restore the previous catalog setting. With conflicts, some fields may already be reverted while conflict state is retained.

## Result

Print the removal result and exit 0 on success; conflicts or file-operation failures exit 1.

## Troubleshooting

Without a state record, the tool refuses to guess which fields to remove. Conflicting fields are kept; a failure exit does not mean that no fields changed.

[All commands](/en/tools/asterctl/commands) · [Tool overview](/en/guides/asterctl)
