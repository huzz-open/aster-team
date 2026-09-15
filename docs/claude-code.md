# Use Claude Code through Aster

[简体中文](zh-CN/claude-code.md) · [Documentation](README.md)

Aster exposes an Anthropic Messages-compatible endpoint for Claude Code. The base URL is the service root and must not include `/v1`; Claude Code appends `/v1/messages` itself.

Claude Code and the upstream model service remain third-party products. Aster provides a compatible gateway path but does not include a provider account, subscription, client license, network access, or upstream entitlement. See [Service boundaries](service-boundaries.md).

## Recommended setup

Install `asterctl` from **Member UI → API documentation → asterctl tool**, then run:

```powershell
asterctl setup claude --base-url "https://aster.example.com" --project "project-demo" --set-key --launch
```

Replace `project-demo` with an absolute path or a path relative to the current terminal. Omit `--project` to use the current directory, and omit `--launch` to configure without starting Claude Code.

`asterctl` checks the installed Claude Code version, validates the Aster endpoint and member API key, obtains the administrator-enabled model mapping, and writes `.claude/settings.local.json` in the selected project. Existing settings are shown before replacement and are backed up when you confirm an overwrite.

## Inspect or remove the configuration

```powershell
asterctl status claude --project "project-demo"
asterctl doctor claude --project "project-demo"
asterctl remove claude --project "project-demo"
```

Keep the project-level settings file out of source control when it contains environment-specific values. Never commit the member API key.

## Model mapping

Aster generates Claude Code model overrides from models currently enabled by the administrator. Members do not need to maintain a separate mapping. Rerun setup after the enabled model set changes.

If setup rejects the installed Claude Code version, update Claude Code first and rerun the command. If requests reach the wrong path, confirm that `--base-url` is the service root without `/v1`.
