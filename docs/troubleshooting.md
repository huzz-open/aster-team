# Troubleshooting

[简体中文](zh-CN/troubleshooting.md) · [Documentation](README.md)

Start with:

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor --verbose
```

On Windows, use the installed `aster-team-cli.exe` absolute path and omit `sudo`.

## The UI does not open although the service is running

Confirm that `status` reports the real server address rather than `127.0.0.1`, then check listeners, host firewall, cloud security groups, routing, and DNS:

```bash
sudo aster-team-cli status
ip -4 route get 1.1.1.1
sudo ss -lntp | grep -E ':(11080|11081|11082)\b'
```

Domain deployments also require correct DNS or client host mapping and valid TLS configuration.

## `RUNNER_NOT_READY` / `33003`

The router found no Runner that was simultaneously online, enabled, protocol-compatible, fresh, and below capacity. Use **Admin → Runners → Test connectivity**, then inspect both sides:

```bash
sudo aster-team-cli logs runner
sudo aster-team-cli logs control
```

Also verify Runner DNS, TLS, proxy, firewall, and outbound access to the configured provider domains. A `33003` request is not charged.

Aster does not provide network egress or make an unavailable provider reachable. Use only organization-approved connectivity and confirm the account and service are available for the deployment environment. See [Service boundaries](service-boundaries.md).

## Missing `GLIBC_2.xx`

The package is an obsolete dynamically linked build. Current Linux release packages are expected to be static builds. Download the latest signed package from Releases, verify it again, and do not continue from the old extracted directory.

## License file not found

`license install` imports an already issued license; it does not create one. Confirm the path first:

```bash
pwd
ls -l ./license.json
sudo aster-team-cli license install --source ./license.json
```

If you do not yet have a signed license, use the bundled free license or follow the offline request process shown by the CLI and Admin UI.

## Codex still uses another provider

Fully exit Codex, then run:

```powershell
asterctl status codex
asterctl doctor codex
```

Confirm that the configured Base URL ends in `/v1`, `ASTER_API_KEY` exists for the same operating-system user, and the API key is active. See [Codex integration](codex.md).

## Claude Code calls the wrong path

Its Base URL must be the service root without `/v1`. Rerun setup with `https://aster.example.com`, not `https://aster.example.com/v1`. See [Claude Code integration](claude-code.md).

## Chinese JSON fails from Windows Git Bash

The Member curl examples target standard curl behavior. If English succeeds but Chinese JSON returns `INVALID_REQUEST`, check which executable Git Bash resolves:

```bash
type -a curl
curl --version
```

Retry with Windows' built-in curl while keeping the same JSON and headers:

```bash
/c/Windows/System32/curl.exe --fail-with-body -sS 'https://aster.example.com/v1/responses' \
  -H "Authorization: Bearer ${ASTER_API_KEY}" \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"<enabled-model-id>","input":"你好","stream":false}'
```

## Collect a useful report

For a model request failure, copy `X-Aster-Request-ID` or the request ID in the error response, then run on Control:

```bash
sudo aster-team-cli trace
```

Provide the exact time and time zone, affected interface or command, model, five-digit error number and string code, redacted `status` and `doctor` output, and whether the deployment uses HTTP, HTTPS, a domain, proxy, or internal CA.

Never share passwords, cookies, API keys, access or refresh tokens, license files, private keys, databases, complete conversations, or unredacted diagnostic bundles. Open an [installation issue](https://github.com/huzz-open/aster-team/issues/new?template=installation.yml) or [bug report](https://github.com/huzz-open/aster-team/issues/new?template=bug.yml) after redaction.
