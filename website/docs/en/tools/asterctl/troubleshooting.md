---
title: "asterctl troubleshooting"
description: "Troubleshoot missing commands, client versions, authentication, model catalogs, and configuration conflicts."
---

# asterctl troubleshooting

## Command not found

Run `.\asterctl.exe version` in the tool directory, or follow [installation](/en/guides/asterctl) to add it to the user PATH and reopen the terminal. Running the member tool on the server does not configure another computer.

## Authentication or connection failure

Use an Aster member key. Codex URLs include `/v1`; Claude origins do not. Verify API reachability, certificate trust, key state, and model permissions. `status` is offline and cannot prove remote authentication; use the corresponding `doctor`.

## Codex running or catalog validation failure

Close Codex before setup or removal. Check that the installed client provides the required `debug models` capability; rerun [setup codex](/en/tools/asterctl/setup-codex) after a client upgrade. Resolve managed state in the old directory before switching CODEX_HOME.

## Claude version or platform mapping changes

Run `claude update`, then [setup claude](/en/tools/asterctl/setup-claude) for the same project. After mapping changes, doctor reports settings that differ from current rules; it does not rewrite files automatically.

## Removal failures or manual edits

Codex preserves conflicting fields but may already have reverted other managed fields. Claude refuses to overwrite a file with a different digest. Save and inspect your changes and original backups; do not delete state files to bypass conflicts. See the corresponding remove command for details.

## Setup completed but the client did not start

Check whether `--launch` was supplied. If launching failed, use status to inspect completed configuration, then start the client manually. A nonzero exit code does not always mean no settings changed.

## Reporting an issue

Provide `asterctl version`, the subcommand, client version, failure time, and sanitized errors. Do not submit real keys, full settings files, or backups containing credentials.

[Complete command reference](/en/tools/asterctl/commands)
