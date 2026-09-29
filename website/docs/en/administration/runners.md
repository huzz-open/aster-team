---
manualSourceHash: 4e875ff8ca1ea69a2d7b097d9bbdcdaf4b530ee77362d63534fc7237155ff058
title: Runner nodes
description: Install a dedicated Runner and check the network and routing conditions.
---

# Runner nodes

In Admin's Runner nodes page, create a node, select its platform, copy the complete generated installation command and run it on the target host. The installer asks once for an installation root, defaulting beneath the current directory: for example `D:\software\Aster Team` or `/data/aster-team`. Accept the displayed default or supply another existing absolute root. An interrupted setup can resume its own unfinished Runner directory when the same command is run again. Completed installations and directories containing unrelated files are not overwritten. Upgrades and restores reuse the original root.

The generated flow installs only the Runner, CLI and service launch files, then enrolls with Control; it does not install Control, Caddy, frontends or a database. A one-time token is kept in a temporary file readable only by root (0600) on Linux or Administrators/SYSTEM on Windows, and removed after enrollment. Installation logs are at `<installation-root>/logs/install-runner.log`; Windows runtime logs are at `logs/runner.log`, while Linux uses `journalctl -u aster-runner.service`.

The package content comes from Control's currently installed signed Release for that platform. It does not fetch missing Runner binaries from GitHub or another public server. Preserve the full command's address, trust and enrollment options; do not construct a substitute from a different installation.

Trusted-LAN HTTP must be explicitly allowed by the generated command; use HTTPS in production. For internal CA, retain the generated trust configuration rather than disabling certificate checks.

Runners are not permanently bound to upstream accounts. Eligible routing candidates must be online, enabled, protocol-compatible, have a fresh heartbeat and available capacity. Routing prefers the account's last successful Runner, then another eligible node with lower recent load.

[Runner installation](/en/tools/aster-team-cli/runner-install) · [Enrollment](/en/tools/aster-team-cli/runner-enroll) · [Upgrade](/en/tools/aster-team-cli/runner-upgrade) · [Connection failures](/en/administration/troubleshooting).
