---
title: "aster-team-cli version"
description: "Show CLI build metadata and the installed product version."
---

# aster-team-cli version

Show CLI build metadata and the installed product version.

## Syntax

```text
aster-team-cli version
```

## Parameters

No command-specific parameters.

Use `aster-team-cli version --help` to inspect help for the installed version.

## Examples

```bash
aster-team-cli version
aster-team-cli --version
aster-team-cli --help
```

## Configuration and runtime effects

Read-only. version shows CLI version, commit, build time, and attempts to read the installed version; --version prints the short CLI version.

## Result

Report when the product is not installed. This command does not require elevation, but filesystem access permissions still apply.

## Troubleshooting

Print a file error when the installed version file cannot be read; check that the CLI belongs to the intended installation root.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
