# Repository instructions for AI coding agents

These instructions apply to the entire repository.

## Required verification

- Before marking a pull request ready for review or merging it, run `npm run verify:changed` from the fixed PR-integration worktree after that worktree has applied the exact current PR Head SHA on the latest `origin/main`. This deterministic command selects affected checks, upgrades sensitive Customer or Operations source changes to their complete domain suite, and upgrades repository-wide foundations or unclassified changes to `npm run verify`. An initial branch push and Draft pull request may happen before local verification solely to make that immutable Head SHA available to the integration workflow; keep the pull request Draft and do not merge it until verification passes. Every later source push invalidates the result and requires the updated Head SHA to be reapplied and reverified.
- Do not upgrade an ordinary affected or domain plan to repository-full verification merely because the change will merge to `main`. Repository-full verification belongs to repository-wide foundations, unclassified changes, scheduled validation, and releases.
- Explicit user instruction is the only local-verification exception: when the user clearly says that the current pull request must be submitted without checks, skip both pre-merge and post-merge local verification for that pull request. Do not describe the result as verified, and report the exact command or commands that were intentionally not run so the user can execute them later. If a local Git hook would run those checks, this explicit instruction authorizes bypassing that hook for the current pull request; it does not authorize weakening repository check definitions or bypassing required checks enforced by the hosting platform.
- Every required verification path must be reproducible and non-interactive through a documented one-line repository command with a meaningful exit code. Do not make AI review, manual screenshot inspection, or another subjective step a required gate.
- Agent-metadata and local-developer-tooling changes covered by the direct-to-`main` exception below do not require product verification. Run the focused syntax, type, or test command for the edited agent metadata or local developer tooling when one exists. The explicit `main, no pr` workflow override does not extend this verification exception to product-affecting changes.
- Before tagging a release, run the full `npm run verify` suite plus the applicable release-level browser, system, packaging, and platform checks. `npm run release:local -- --platform windows` (with `linux` or `all` selected instead when applicable) is the canonical one-line local Customer release lifecycle and runs full source verification by default.
- When the user explicitly asks to create a commit, the commit may be created without first running verification. By itself this exception applies only to the local commit; a push or pull request additionally requires either successful verification or the explicit user-directed no-check pull-request rule above. It never exempts a release.
- Do not claim that a change is ready for push or release while its required verification is failing.
- Except when bypassing a local Git hook under the explicit user-directed no-check pull-request rule above, never bypass checks with `--no-verify`, suppress a warning, weaken `-D warnings`, or add a lint allowance merely to make verification pass. Fix the underlying issue unless the user explicitly approves a documented exception.
- Unless the explicit user-directed no-check pull-request rule applies, if the current environment cannot complete the required verification, keep the pull request Draft and stop before marking it ready, merging it, or tagging a release; report the exact command and blocker. The verification-staging branch push and Draft pull-request creation or update remain allowed so the exact remote Head SHA can be integrated and checked. Another task's running build, test, preview, or acceptance session is not itself a verification blocker or a reason to delay that staging flow. List any narrower checks that did pass, but do not present them as equivalent to the required verification.

## Development workflow

### Main branch and task isolation

- `main` is the only long-lived integration branch and must always remain releasable. Do not create a long-lived `develop` branch or a version-wide development branch such as `2.0.1`.
- Never commit or push requirement, optimization, bug-fix, UI, documentation, or maintenance work directly to `main`. Each independently reviewable task uses one short-lived branch and one separate Git worktree.
- Explicit workflow override: when the user says `main, no pr`, `main,no pr`, “直接在 main 上改，无需 PR”, or an equivalent instruction, perform that task directly on the primary `main` checkout without creating a task branch, worktree, or pull request. This overrides the task-isolation and PR-only rules for that task, including code changes. Synchronize clean `main` with `origin/main` first and preserve unrelated changes. It does not itself request a commit or push, waive applicable verification, or authorize bypassing remote branch protection. If the user subsequently requests pushing that task, use the direct-to-`main` flow instead of the default PR delivery flow.
- Exception: repository-global agent instructions and local developer-tooling changes may be edited, committed, and pushed directly on `main` without a task worktree or pull request when they cannot affect shipped Customer or Operations source, production output, dependencies, CI, packaging, release behavior, or installed runtime behavior. This exception includes `AGENTS.md`, auxiliary agent UI metadata, the Python local development manager, and local-development launch or configuration adapters used by that manager, even when developers can also invoke those adapters manually. Tests for those local tools may accompany the direct change. If the change also affects a shipped artifact or production/runtime path, use the normal task branch and pull-request workflow.
- Keep the primary `aster-team` checkout clean and use it to synchronize and inspect `main`. Store every task worktree under the sibling directory `../aster-team_worktrees/`, with one uniquely named child directory per task. Do not place task worktrees directly beside the repository or inside the repository.
- AI-created task branches use the `codex/` prefix and an intent-based name: `codex/feat-<topic>` for requirements, `codex/fix-<topic>` for defects (including incorrect copy or broken layout), `codex/ui-<topic>` for copy, styling, layout, and visual changes that do not alter business behavior, `codex/opt-<topic>` for compatible performance or experience improvements, `codex/docs-<topic>` for documentation-only work, and `codex/chore-<topic>` for build, dependency, script, and repository maintenance.
- One branch and pull request must represent one coherent task. Do not mix unrelated requirements, optimizations, bug fixes, formatting sweeps, or refactors into the same change.
- Parallel conversations must never develop in the same working directory or reuse the same task branch. Existing uncommitted changes belong to their current owner and must not be moved, overwritten, or folded into another task.

### Start every task from the latest main

- Before inspecting code for a new implementation task, run `git fetch origin --prune` and use `origin/main` as the source of truth. Do not branch from a possibly stale local `main`.
- Create every task worktree by running `npm run worktree:new -- <type> <task-name>` from the primary checkout, for example `npm run worktree:new -- fix dashboard-responsive`. Do not reproduce the underlying Git commands manually unless the helper script is broken and the user approves a fallback.
- Read `WORKTREE_PATH` from the script output and immediately switch the coding session to that absolute path. For tool calls, set `cwd` or `workdir` to `WORKTREE_PATH` on every subsequent command. Before reading or modifying project files, verify both `git rev-parse --show-toplevel` and `git branch --show-current` from that directory. Creating a worktree without actually continuing the task inside it does not satisfy this rule.
- If the task is already running in a dedicated branch/worktree, verify its base and merge the latest `origin/main` before implementation when safe. Do not switch branches or rewrite history in a shared or dirty working tree.
- If fetching fails, report that freshness could not be verified. Do not claim the task is based on the latest `main`.

### Dependency and verification workspace locality

- Persistent downloaded dependencies, compiler outputs, and reusable build caches may exist only in the primary `aster-team` checkout and the fixed `../aster-team_worktrees/integration-test` worktree. In particular, ordinary task worktrees must not retain their own `node_modules/`, Rust `target/`, application `dist/`, package-manager dependency trees, or equivalent heavyweight generated directories.
- Treat ordinary task worktrees as source-only workspaces. Do not run `npm install`, `npm ci`, Cargo build/test/lint commands, frontend builds, or repository verification commands there when they would populate worktree-local dependency or build directories. Lightweight source inspection and checks that do not materialize those directories remain allowed.
- Run task-branch verification in the fixed integration worktree through the documented PR-integration workflow. First synchronize the task branch with the latest `origin/main`, commit it, push it, and create or update a Draft pull request. Apply that exact remote PR Head SHA to the integration worktree, install or refresh dependencies there when its applied lockfiles require it, and run the required one-line verification command from the integration worktree root.
- The fixed `codex/integration-test-local` worktree is a shared, cumulative multi-PR integration environment. Read its current selected/applied PR set, preserve the other PRs, and add or refresh the current PR on the latest `origin/main`. Do not replace the set with only your own PR. Remove a PR only when explicitly requested or when it is closed or already merged into the refreshed base; report any such selection change. Rebuilding must replay the preserved set and keep a recoverable reference to the previous integration commit.
- Serialize only the short Git mutation and integration-state update, not the lifetime of builds, tests, previews, debugging, or acceptance sessions. Coordinate an exclusive mutation window across tasks, then re-read the branch, worktree status, base, and PR selection inside that window before applying changes; release it immediately after success or rollback. Do not wait for another PR to finish acceptance, become Ready, or merge. Existing PRs and running checks are not reasons to refuse an additional conflict-free integration.
- Preserve all uncommitted files and unfinished Git operations. If the integration helper cannot safely retain them, report the exact blocking files or operation instead of resetting, stashing, or deleting another task's work. If adding or refreshing a PR causes a merge conflict, abort that integration attempt, restore the previous committed combination without discarding unrelated changes, and report the conflicting PRs and files. Resolve source conflicts in the owning task branches, never by silently dropping another PR or changing product source only in the integration branch. A clean Git merge permits integration but is not evidence of behavioral compatibility.
- Every verification run must record the integration commit, latest `origin/main` base, and complete set of PR numbers and exact Head SHAs. A passing result applies only to that unchanged combination. Integration may proceed while checks are running; if the source combination changes during a run, retain its output as diagnostic evidence but do not claim it verifies the new combination. Arrange a fresh `npm run verify:changed` for the latest stable combination, restarting affected checks or services when needed and coordinating with their owners rather than stopping unrelated processes. Any task-branch push, selected-PR change, conflict resolution, or `origin/main` advancement requires refreshing the candidate and verification. Do not use source or generated output from the integration branch as implementation input for a task branch.

### Implement and verify

- Read the current implementation after synchronizing with `origin/main`; do not rely only on context from an earlier conversation because another pull request may have changed the same area.
- Keep the patch scoped to the task and add or update proportionate tests. Preserve unrelated user changes.
- Use local commits as recoverable checkpoints. Do not force-push or rewrite shared task-branch history unless the user explicitly requests it.
- Before the verification-staging push or opening/updating the Draft pull request, fetch again and merge the latest main with `git merge --no-edit origin/main`. Resolve conflicts in the task branch, then inspect the merged behavior rather than treating a clean textual merge as sufficient.
- After the final main sync, apply the exact pushed PR Head SHA in the fixed integration worktree and run `npm run verify:changed` there. The command runs `npm run verify` itself when the changed paths require full verification. A result from a task worktree, from an earlier Head SHA, or from before the final main sync is stale.

### Pull requests and merge order

- All product changes enter `main` through a pull request. Prefer Squash Merge so each task becomes one focused commit on `main`, then delete the merged task branch and remove its worktree.
- Treat an unqualified user request to “推送” or “push” as a request to complete the pull-request delivery flow, never as permission to push directly to `main`. By default: push the task branch, create or update its Draft pull request, apply and verify its exact Head SHA in the fixed integration worktree, mark it ready and merge it into `main` after required checks pass, and then update the primary local `main` from `origin/main` with a fast-forward-only pull.
- When the user explicitly says “推送但不合并”, “push without merging”, or an equivalent instruction, push the task branch and create or update the pull request, but leave it open and do not merge it.
- After a successful pull-request merge, verify that the primary checkout is clean, fetch `origin`, and run `git pull --ff-only origin main` from the primary checkout. If the primary checkout is dirty or cannot fast-forward, do not overwrite, reset, or discard anything; report the blocker instead.
- Configure branch protection for `main` to require a pull request, passing required checks, and an up-to-date branch before merge. Do not bypass protection for routine solo development.
- When one parallel pull request merges, every other open task branch is now potentially stale. Before its merge, fetch and merge the new `origin/main`, re-read any overlapping code, resolve conflicts, and rerun `npm run verify:changed`.
- Merge overlapping changes sequentially. The later task must adapt to the already-merged result; it must not restore an older implementation simply because its branch started earlier.

### Versioning and releases

- Ordinary task branches do not bump the product version. Decide the next version only after the intended task pull requests have merged into `main`.
- Use a short-lived `release/<version>` branch only for release preparation such as version fields, lockfiles, and release notes. It is not a shared development branch and must also start from the latest `origin/main`.
- Patch releases such as `2.0.1` contain backward-compatible bug fixes and small compatible optimizations. User-visible features normally increment the minor version, such as `2.1.0`; breaking changes increment the major version.
- Merge release preparation through a pull request, rerun the required verification, and create the release tag from the resulting commit on `main`. Never tag an unmerged task or release branch.
- Do not repeat an already successful full verification when retrying a release that failed only because of an external infrastructure problem such as a registry/network interruption, Docker engine interruption, or download failure. Reuse that verification only when the Git commit, working tree, version, platform, release inputs, and signing configuration are unchanged; any source or release-input change requires verification again.
- A release retry must use a repository-supported resume or verified-retry path that binds the successful verification to the exact release inputs. Do not bypass the release command by invoking lower-level packaging or signing scripts manually. If the current release tool cannot resume without repeating verification, report that tooling limitation instead of repeatedly restarting the full release command.

## Alignment requests

- When the user says “先对齐”, “对齐一下”, or otherwise asks to align before implementation, treat the turn as analysis and decision alignment only. Do not modify product code, assets, configuration, or documentation until the user explicitly asks to proceed. Updating this repository instruction itself when explicitly requested is the only exception.

## Keep local and CI checks aligned

- GitHub Actions must call the repository scripts in `package.json` instead of duplicating formatter, lint, test, or build commands in workflow YAML.
- `tools/verification-map.json` is the canonical affected-verification policy. Pull-request and local affected checks must use `npm run verify:changed`; release workflows may invoke explicit package scripts for platform artifact validation. Do not reproduce path-selection rules in workflow YAML.
- Risk selection is deterministic. Planning failures and unclassified paths must fail closed to repository-full verification. Except for the explicit user-directed no-check pull-request rule above, AI agents and developers may upgrade a run to `npm run verify`, but must not downgrade or bypass the level selected by `npm run verify:changed`.
- Rust formatting and linting use `npm run check:rust`, which is included in the Rust affected suite and in `npm run verify`.
- The Rust version is pinned by `rust-toolchain.toml`. Keep the validation and release workflows on the same exact version. When upgrading Rust, update the toolchain file, `Cargo.toml` compatibility declaration when appropriate, and every CI toolchain reference in the same change.
- Linux-only package installation, systemd, rollback, and SQLCipher smoke tests are additional release checks. Do not claim they ran on Windows; run them in the Linux workflow or clearly report that they remain pending.

## Keep generated files out of agent context

- Do not recursively enumerate ignored or generated directories such as `node_modules/`, `target/`, `dist/`, `data/`, `.wrangler/`, `.aster-deploy/`, or `linux-diagnostics/`. In particular, do not run broad commands such as `git status --ignored --untracked-files=all`.
- Use `git status --short --untracked-files=all` for the actionable worktree state. Inspect generated logs or diagnostics only when relevant to the current failure, and then read the specific run or service path instead of the entire directory tree.
- Runtime logs, failed-smoke diagnostics, build outputs, local databases, credentials, and tool caches are not source files and must remain ignored unless a repository contract explicitly requires a fixture.

## Canonical user manual

- `docs/user-manual.md` is the canonical end-user installation and troubleshooting manual.
- When the user asks to add to or update the “用户手册”, maintain that file instead of creating another overlapping guide.

## Documentation context

- Use `docs/README.md` to select relevant documents; do not load all of `docs/internal/` for ordinary implementation tasks. Plans and dated review evidence are not current implementation facts or reusable task authorization.
- When creating, reorganizing, or publishing documentation, read `.agents/rules/documentation.md`. When changing client setup/configuration or packaged client-tool downloads, read `.agents/rules/client-tools.md`.

## Frontend coding rules

- For changes to any frontend surface or the shared UI package, read `.agents/rules/frontend/README.md` and `.agents/rules/frontend/core.md` before implementation, then read the applicable topic files listed in the index. These files are the canonical frontend rules; keep new frontend rules there and retain only this entry point in `AGENTS.md`.

## Backend coding rules

- For changes to backend APIs, persistence, or error responses, read `.agents/rules/backend/README.md` and the applicable backend topic files. Keep detailed rules in that directory and retain only this entry point here.
