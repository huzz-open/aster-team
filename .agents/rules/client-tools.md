# Client configuration tools

Read when changing `asterctl`, shared client configuration crates, or member client-tool downloads.

- Derive downloadable tools from files actually present and verified in the installed signed Release. Do not invent platform availability, patch a binary with a user URL, or create a separate unsigned download registry.
- Keep the service URL deployment-specific, derived from the configured public API base URL. Public examples and member instructions must agree with the current CLI and signed artifact list.
- Never pass API keys as command-line values or store them in logs, transaction metadata or diagnostics. Use the explicit hidden-input flow. URL-only changes must not read/send an existing key; validate a newly entered key before overwriting it.
- Configuration generation belongs to `aster-codex-config` / `aster-claude-config`; Control, CLI and frontend consumers must not implement independent fragments. Edit parsed configuration while preserving unrelated fields and formatting.
- For Codex setup/remove, require the client to exit and use an interprocess lock. Persist field ownership and a recoverable pending/committed transaction; preserve original permissions and use atomic file replacement.
- Removal restores a managed field only if its current value still equals the installed value. Preserve user changes and report conflicts; missing ownership records are not permission to restore a whole old file. Do not claim that a transaction record can recover an old plaintext key.
- Keep status read-only and diagnostics bounded. Model discovery does not prove a successful paid model call; distinguish missing configuration, authentication failure and unavailable capabilities.
- The Fast model resource is embedded from `customer/backend/asterctl/assets/codex-models-aster-fast.json`. Merge native, preexisting custom and Aster entries without losing native metadata; regenerate from the original sources, not just the previously generated file. Verify the candidate catalog through the installed client before committing it with config/key state, and protect manual edits on update/remove.
- CLI UX and troubleshooting belong in [the user manual](../../docs/user-manual.md) and bilingual [command references](../../website/docs/zh-cn/tools/asterctl/commands.md). Do not retain a separate setup-design tutorial after its behavior is implemented.
