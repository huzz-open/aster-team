---
manualSourceHash: 8529936de18e7512f3066b1fbf7dc618209f74ff0beaaa311ec99a1383a7e001
title: Before you install
description: Choose a supported platform and prepare the package, trusted digest, host access and network.
---

# Before you install

Linux amd64 with systemd is the recommended platform. Automated Linux installation coverage includes Ubuntu 20.04/22.04/24.04, Debian 12/13 and Rocky Linux 9. Windows amd64 is experimental: stability is not promised and fixes may take longer. macOS packages are not currently delivered; implementation or static checks do not establish platform support. Check the release manifest for available versions.

Prepare:

- The signed `aster-team-<version>-<platform>-<architecture>.tar.gz` package matching the host.
- Its SHA-256 obtained through an independent trusted channel.
- Linux sudo access, or an administrator PowerShell on Windows.
- Network access from users to the Admin, Member and Model API endpoints.
- HTTPS access from the server to `auth.openai.com` and `chatgpt.com` when connecting a ChatGPT account.

Verify the archive before executing its contents. A checksum downloaded beside an archive alone does not establish initial trust.

Current Linux and Windows packages carry `licenses/free-license.json`, a signed, unbound, non-expiring free license. Its capabilities, limits and minimum version cannot be edited. Setup verifies the release and license before using it for a new installation. Older packages without this file produce an offline license request instead.

[Continue to installation](/en/administration/installation).
