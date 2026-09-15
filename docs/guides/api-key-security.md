# Aster Team API key security best practices

[简体中文](../zh-CN/guides/api-key-security.md) · [Developer guides](README.md) · [Member guide](../member-guide.md)

An Aster member API key identifies a person, device, development environment, or automation workload to the private AI gateway. Separate keys improve quota attribution, revocation, and incident response.

## Use one accountable identity per key

- Give every human and automation workload its own Aster member identity.
- Create separate keys for devices or environments when independent revocation is useful.
- Do not copy one member key across a team or embed a personal key in shared automation.
- Use a dedicated automation identity for CI and rotate it independently from human access.

## Store keys safely

Prefer the operating-system credential store, an approved secrets manager, or a protected environment variable. Never place the key in a URL, Git-tracked file, screenshot, chat message, issue, build log, or command argument that remains in shell history.

For Codex, `asterctl setup codex --set-key` reads the key through a hidden prompt and stores it as the current user's `ASTER_API_KEY`. Codex configuration refers to the variable name rather than containing the secret.

For Claude Code, `asterctl` writes project-local settings. Keep these paths out of Git:

```gitignore
**/.claude/settings.local.json
**/.claude/.asterctl-state.json
**/.claude/settings.local.json.asterctl-*.bak
```

## Rotate or revoke a key

1. Create a replacement key when the license allows another active key for that member.
2. Update the client or secrets manager and verify one request.
3. Revoke the old key in Member UI.
4. Review recent usage for unexpected models, times, endpoints, or request volume.

When the per-member active-key limit is already reached, revoke the old key before creating its replacement. Revocation releases the active-key license capacity and prevents new authentication with that key.

## Respond to possible exposure

Revoke the key immediately; do not wait for confirmation that it was used. Preserve the approximate exposure time, key name, member, affected environment, and relevant request IDs. Review Aster usage and audit records, rotate any other secret stored in the same location, and remove the value from Git history or build logs according to your organization's incident procedure.

Full API keys are shown only when created and cannot be recovered later. If a value is lost, revoke its record and create a new key.

## Share diagnostics without sharing credentials

Use the stable five-digit error code and `X-Aster-Request-ID` when requesting support. Redact authorization headers, environment dumps, local settings, cookies, provider tokens, and prompt or response bodies. See the [error-code reference](aster-error-codes.md).
