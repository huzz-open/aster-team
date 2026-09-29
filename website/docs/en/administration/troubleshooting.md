---
manualSourceHash: 1f6549287dd736b13e070c3b5bc503c24bb56e0bbd9dc7ea5a23fdf3760b6248
title: Troubleshooting
description: Diagnose page access, Runner, license, request and quota issues, and collect safe support information.
---

# Troubleshooting

## The page is unavailable but the service is active

Check that the installer reported the actual server address rather than `127.0.0.1`:

```bash
sudo aster-team-cli status
ip -4 route get 1.1.1.1
sudo ss -lntp | grep -E ':(11080|11081|11082)\b'
```

Check firewalls, security groups and routing from the client. IP/HTTP mode must listen on the configured LAN address. Domain access also needs correct DNS or client hosts entries. A running process does not prove its public entrypoint works.

## Runner is online but model synchronization returns 33003

`33003 / RUNNER_NOT_READY` means no Runner meets all routing conditions: online, enabled, compatible protocol, fresh heartbeat and available capacity. Use the node's connectivity test in Admin, then inspect both sides:

```bash
sudo aster-team-cli logs runner
sudo aster-team-cli logs control
```

Then check Runner DNS and HTTPS access:

```bash
sudo systemctl cat aster-runner.service | grep '^ExecStart='
getent ahosts chatgpt.com
sudo -u aster-runner curl -sS -o /dev/null -w 'HTTP %{http_code}\n' https://chatgpt.com/backend-api/codex/models
```

Receiving an HTTP status demonstrates basic DNS/TLS/network reachability, not successful account authorization. Timeouts, lookup failures and certificate errors require checking proxy, firewall and CA configuration. A green connection alone does not establish task readiness.

## A Linux executable reports a missing GLIBC version

Verify that you downloaded the official package for Linux amd64. The production Linux package uses static musl executables. A GLIBC error can indicate an older, development or wrong-platform executable; preserve the exact package version and error, and obtain the matching verified release rather than replacing system libraries to accommodate an unknown binary.

## The license file cannot be found

First run `sudo aster-team-cli license status`. An active license does not need another import. If missing, generate a request with `license request` and obtain the signed file; `license install` does not create one. Relative paths use the current working directory. Check the file and import it from there:

```bash
pwd
ls -l ./license.json
sudo aster-team-cli license install --source ./license.json
```

For signature, binding, validity or history failures, follow [product authorization](/en/administration/licensing). Do not delete protected history to force activation.

## Collect useful support information

Copy the Aster request ID from the error body or `X-Aster-Request-ID`, then run on the Control host:

```bash
sudo aster-team-cli trace
```

Paste the ID at the prompt. The CLI selects the current Control service and searches the last 24 hours; `--hours` can extend the window to at most 720 hours. See [trace](/en/tools/aster-team-cli/trace) for arguments.

Each actual model HTTP execution has its own Aster request ID for reservation, settlement and tracing. A valid caller `x-client-request-id` (or fallback `x-request-id`) can be forwarded upstream and correlated, but it is not an Aster billing idempotency key. Reusing a caller ID does not prevent another request from being billed.

Provide the time and timezone, page/command, stable numeric and string error codes, `status`/`doctor` output, matching Control/Runner logs, and whether HTTP, HTTPS, a proxy, domain or internal CA is involved. Do not send passwords, cookies, API keys, access/refresh tokens, signing keys or full conversations. A request ID is a trace identifier, not an error code.

## Chinese JSON in Windows Git Bash returns 400

Prefer the generated PowerShell example on Windows. In Git Bash, check `type -a curl` and `curl --version`. If English requests work but Chinese requests return `400 / INVALID_REQUEST`, retry the same request with Windows' cURL executable instead of `/mingw64/bin/curl`:

```bash
/c/Windows/System32/curl.exe --fail-with-body -sS 'http://SERVER_IP:11080/v1/responses' \
  -H "Authorization: Bearer ${ASTER_API_KEY}" \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"YOUR_PUBLIC_MODEL","input":"你好，请介绍一下自己。","stream":false}'
```

Use your actual public model ID. Alternatively set `alias curl='/c/Windows/System32/curl.exe'` for this shell session before pasting the generated example. Keep `--fail-with-body` to retain a nonzero HTTP failure exit and the error body; `-i` also shows headers. Never paste the real key into a ticket.

## Quota remains reserved after an interrupted request

Control first recovers durable actual results and settles using the request's frozen usage and multiplier. Reservations without a durable result become eligible for periodic, batched recovery 30 minutes after request creation; active executions are protected. Database recovery and scanning can take additional time, so release is not instantaneous and no new model request is needed to trigger it.

An expired reservation with unknown usage is recorded as uncharged with an uncertain upstream outcome. Revoked keys or an expired product license do not prevent handling existing reservations; new requests remain restricted. Integrity failures retain the record for investigation. Preserve `data/settlements` under the installation root: deleting it is not a recovery procedure, and usage never durably written cannot be guaranteed recoverable. For persistent issues, collect the time, request ID and Control logs.

## Initial free authorization on Windows

Windows amd64 is experimental; Linux is recommended. A unified package with `licenses/free-license.json` verifies/imports it on a fresh Control installation without an online license claim. Verification failure stops setup. Import a privately delivered paid license into the same installation; upgrades and recovery preserve authorization. Dedicated Runners do not import the free certificate—their rights come from Control.

[Daily checks](/en/administration/daily-checks) · [API errors](/en/guides/errors).
