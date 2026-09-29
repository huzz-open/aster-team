---
manualSourceHash: 5ed882f86707b5195404b5dd4a35a8c2b5a177d8ca19533693a953919c825936
title: Daily checks
description: Check services, installation health and logs without confusing process status with working model calls.
---

# Daily checks

Linux examples:

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor
sudo aster-team-cli doctor --verbose
sudo aster-team-cli logs control
sudo aster-team-cli logs runner
```

On Windows, use the absolute `<installation-root>\bin\aster-team-cli.exe` printed by `init.ps1` in administrator PowerShell, omitting `sudo`; the subcommands are the same.

`status` reports service state and access addresses. `doctor` checks installation identity, database, authorization, public endpoints and service preflight. A systemd service being `active` only proves that its process is running, not that an upstream model call will succeed.

Follow current logs with:

```bash
sudo aster-team-cli logs control --follow
sudo aster-team-cli logs runner --follow
```

The CLI selects the active Control slot; you do not need to choose the blue/green systemd unit manually.

[Status reference](/en/tools/aster-team-cli/status) · [Doctor reference](/en/tools/aster-team-cli/doctor) · [Logs reference](/en/tools/aster-team-cli/logs) · [Troubleshooting](/en/administration/troubleshooting).
