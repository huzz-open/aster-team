# How to configure Claude Code with an Aster API key

[简体中文](../zh-CN/guides/configure-claude-code-api-key.md) · [Developer guides](README.md) · [Claude Code reference](../claude-code.md)

Aster exposes an Anthropic Messages-compatible endpoint for Claude Code. Each developer can use a separate Aster member key while administrators manage model access, quotas, routing, and usage records.

## Prerequisites

- Install Claude Code.
- Create a personal API key in Aster Member UI.
- Copy the public Aster service address.
- Confirm that an administrator-enabled model and healthy Runner are available.

For Claude Code, the Aster base URL is the service root and must not end in `/v1`; Claude Code appends `/v1/messages`.

## Recommended setup

Download `asterctl` from **Member UI → API documentation → asterctl tool**, then run:

```powershell
asterctl setup claude --base-url "https://aster.example.com" --project "project-demo" --set-key --launch
```

Replace `project-demo` with an absolute path or a path relative to the current terminal. Omit `--project` to configure the current directory, and omit `--launch` to avoid starting Claude Code.

`asterctl` validates the endpoint and hidden key input, obtains the administrator-enabled model mapping, and writes project-local settings to `.claude/settings.local.json`. It displays existing settings and creates a backup when you confirm replacement.

Inspect, diagnose, or remove the setup:

```powershell
asterctl status claude --project "project-demo"
asterctl doctor claude --project "project-demo"
asterctl remove claude --project "project-demo"
```

## Keep local settings out of Git

Add environment-specific Claude Code settings and `asterctl` state to `.gitignore`:

```gitignore
**/.claude/settings.local.json
**/.claude/.asterctl-state.json
**/.claude/settings.local.json.asterctl-*.bak
```

Never commit a member key. Use a different member or automation identity for each person, machine, or CI boundary that needs separate quota and audit attribution.

## Troubleshooting

- **Requests reach the wrong path:** remove `/v1` from `--base-url`.
- **The installed client is rejected:** update Claude Code and rerun setup.
- **A model is missing:** rerun setup after the administrator changes enabled models.
- **Authentication fails:** create a valid member key and run `asterctl doctor claude`.
- **The wrong project is configured:** pass an absolute `--project` path and inspect `.claude/settings.local.json` there.

For the complete behavior and removal semantics, see [Use Claude Code through Aster](../claude-code.md).
