# Installation and first run

[简体中文](zh-CN/getting-started.md) · [Documentation](README.md)

This guide takes an administrator from a verified release package to the first successful model request.

For platform-specific steps, use [Install Aster Team on Linux](guides/install-aster-team-linux.md) or [Install Aster Team on Windows](guides/install-aster-team-windows.md).

## 1. Prerequisites

Prepare:

- a package matching the target platform and architecture;
- the package SHA-256 obtained from a trusted channel;
- `sudo` access on Linux or an Administrator PowerShell on Windows;
- a third-party AI account or subscription the organization is authorized to use;
- network access to the Model API, Member UI, and Admin UI;
- lawful, organization-approved outbound HTTPS access to the required provider endpoints, including `auth.openai.com` and `chatgpt.com` when connecting a ChatGPT subscription account.

Aster does not supply the third-party account, subscription, provider credits, VPN, or network egress. Review [Service boundaries and customer responsibilities](service-boundaries.md) before deployment.

Package names use `aster-team-<version>-<platform>-<arch>.tar.gz`. Supported packages are listed on each [Release](https://github.com/huzz-open/aster-team/releases); do not infer platform support from files outside Releases.

## 2. Verify the download

Follow [Release verification](release-verification.md) before running anything from the archive. Aster also verifies the signed release tree during initialization; the outer checksum and inner signature serve different purposes and both should be retained.

## 3. Install on Linux

Replace the placeholders with the downloaded filename and independently obtained digest:

```bash
archive=aster-team-<version>-linux-amd64.tar.gz
printf '%s  %s\n' '<trusted-sha-256>' "$archive" | sha256sum --check --strict -
tar -xzf "$archive"
cd "${archive%.tar.gz}"
sudo ./init.sh
sudo aster-team-cli install
```

The default installation root is `/opt/aster-team`. Choose a different root only during the first initialization:

```bash
sudo ./init.sh --install-root /data/aster-team
sudo aster-team-cli install
```

## 4. Install on Windows

Run in an Administrator PowerShell:

```powershell
$archive = 'aster-team-<version>-windows-amd64.tar.gz'
$actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne '<trusted-sha-256>') { throw 'SHA-256 mismatch' }
tar.exe -xzf $archive
Set-Location 'aster-team-<version>-windows-amd64'
.\init.ps1
```

Continue with the absolute Control command printed by `init.ps1`. The default root is `C:\ProgramData\Aster Team`; choose a different root only during the first initialization:

```powershell
.\init.ps1 --install-root 'D:\Aster Team'
```

Do not keep invoking the CLI from the extracted download directory after initialization. Use the installed absolute path printed by the script.

## 5. Open the interfaces

The default IP-and-HTTP endpoints are:

| Interface | Default URL |
| --- | --- |
| Model API | `http://SERVER_IP:11080` |
| Member UI | `http://SERVER_IP:11081` |
| Admin UI | `http://SERVER_IP:11082` |

Plain HTTP is appropriate only on a trusted LAN. Configure HTTPS before exposing any interface to a public or untrusted network.

The installer prints the initial owner credentials and stores a copy at `<install-root>/config/control/initial-owner-credentials`. Sign in, change the password immediately, store the new password safely, and delete the initial credential file.

## 6. License, Runner, and account

1. In **Product license**, confirm that the bundled free license or an imported signed license is valid. Current limits are displayed in that page.
2. In **Runners**, confirm at least one Runner is online. A same-host Runner can be selected during interactive installation.
3. In **Subscriptions & accounts**, start the ChatGPT OAuth flow for an account your organization is authorized to use, then save it.
4. Synchronize models and enable the models members may use.
5. Create a member, grant quota, and let that member create an API key in the Member UI.

## 7. First API request

Use a model ID returned by `/v1/models`:

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/responses' \
  -H 'Authorization: Bearer <member-api-key>' \
  -H 'Content-Type: application/json' \
  -H 'X-Request-ID: quickstart-001' \
  --data-binary '{"model":"<enabled-model-id>","input":"Hello","stream":false}'
```

Continue with the [API reference](api-reference.md), [Codex integration](codex.md), or [Claude Code integration](claude-code.md).
