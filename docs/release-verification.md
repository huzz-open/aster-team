# Verify a release package

[简体中文](zh-CN/release-verification.md) · [Documentation](README.md)

An Aster Team Release should contain the platform packages and `SHA256SUMS`. Obtain the expected SHA-256 from a trusted channel independent of the downloaded package. A checksum file beside the package detects accidental corruption, but cannot establish initial trust by itself.

## Linux

Verify all files listed in the checksum manifest:

```bash
sha256sum --check --strict SHA256SUMS
```

Or verify one package against an independently obtained digest:

```bash
archive=aster-team-<version>-linux-amd64.tar.gz
printf '%s  %s\n' '<trusted-sha-256>' "$archive" | sha256sum --check --strict -
```

## Windows PowerShell

```powershell
$archive = 'aster-team-<version>-windows-amd64.tar.gz'
(Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
```

The output must exactly match the 64-character lowercase digest published for that filename. Do not run the installer if the name, size, or digest differs.

Linux and Windows archives are not interchangeable. It is fine to download a Linux archive on Windows, but verify the actual Linux filename and transfer it without modification.

After extraction, `init.sh` on Linux or `init.ps1` on Windows also verifies the signed release tree. The outer checksum protects the downloaded archive; the inner signature validates the release contents. Keep both checks.

Report mismatches through an [installation issue](https://github.com/huzz-open/aster-team/issues/new?template=installation.yml), including the download source, exact filename, expected digest, and actual digest. Do not upload a license, key, database, or other sensitive file.
