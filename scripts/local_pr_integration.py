"""Local-only pull-request integration worktree support.

The integration branch is disposable and is never pushed.  Each pull request is
represented by one merge checkpoint so a changed selection can reuse the
longest unchanged prefix and replay only the affected suffix.

Checkpoints are an optimization, not a prerequisite: a clean workspace can be
rebuilt from origin/main after saving its old HEAD under a local recovery ref.
List retained commits with ``git for-each-ref refs/aster/integration-backups/``;
inspect one with ``git show <ref>`` before restoring it to a separate branch.
No dependency directories, ignored runtime data or caches are cleaned here.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from uuid import uuid4
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable, Sequence


INTEGRATION_BRANCH = "codex/integration-test-local"
STATE_FILE_NAME = "aster-pr-integration-state.json"
RECOVERY_REF_PREFIX = "refs/aster/integration-backups/"
SHA_RE = re.compile(r"^[0-9a-fA-F]{40,64}$")


class IntegrationError(RuntimeError):
    """A safe, user-facing integration operation failure."""


@dataclass(frozen=True)
class PullRequest:
    number: int
    title: str
    is_draft: bool
    head_ref_name: str
    head_sha: str
    url: str
    merge_state_status: str = "UNKNOWN"

    @classmethod
    def from_github(cls, value: object) -> "PullRequest":
        if not isinstance(value, dict):
            raise IntegrationError("GitHub 返回了无法识别的 PR 数据。")
        try:
            number = int(value["number"])
            title = str(value["title"])
            is_draft = bool(value["isDraft"])
            head_ref_name = str(value["headRefName"])
            head_sha = str(value["headRefOid"])
            url = str(value["url"])
            merge_state_status = str(value.get("mergeStateStatus", "UNKNOWN"))
        except (KeyError, TypeError, ValueError) as exc:
            raise IntegrationError("GitHub 返回的 PR 数据缺少必要字段。") from exc
        if number < 1 or not SHA_RE.fullmatch(head_sha):
            raise IntegrationError(f"PR #{number} 的 head SHA 无效。")
        return cls(number, title, is_draft, head_ref_name, head_sha.lower(), url, merge_state_status)


@dataclass(frozen=True)
class IntegrationEntry:
    number: int
    head_sha: str
    integration_commit: str
    title: str

    def identity(self) -> tuple[int, str]:
        return self.number, self.head_sha


@dataclass(frozen=True)
class IntegrationState:
    base_sha: str
    entries: tuple[IntegrationEntry, ...]

    @property
    def head_sha(self) -> str:
        return self.entries[-1].integration_commit if self.entries else self.base_sha


@dataclass(frozen=True)
class IntegrationResult:
    path: Path
    base_sha: str
    entries: tuple[IntegrationEntry, ...]
    reused: int
    replayed: int
    recovery_ref: str | None = None


CommandRunner = Callable[[Sequence[str], Path], subprocess.CompletedProcess[str]]
ProgressReporter = Callable[[str], None]


def _run(command: Sequence[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            list(command), cwd=cwd, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace", check=False,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        )
    except OSError as exc:
        raise IntegrationError(f"无法执行 {command[0]}：{exc}") from exc
    return result


def _checked(result: subprocess.CompletedProcess[str], description: str) -> str:
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or f"退出码 {result.returncode}"
        raise IntegrationError(f"{description}失败：\n{detail}")
    return result.stdout.strip()


def _git(cwd: Path, *arguments: str, runner: CommandRunner = _run) -> str:
    return _checked(runner(("git", *arguments), cwd), f"Git {' '.join(arguments)} ")


def repository_root(source: Path, *, runner: CommandRunner = _run) -> Path:
    root = _git(source, "rev-parse", "--show-toplevel", runner=runner)
    return Path(root).resolve()


def primary_worktree(source: Path, *, runner: CommandRunner = _run) -> Path:
    output = _git(source, "worktree", "list", "--porcelain", runner=runner)
    for line in output.splitlines():
        if line.startswith("worktree "):
            return Path(line.removeprefix("worktree ")).resolve()
    raise IntegrationError("无法定位主 Git worktree。")


def default_integration_path(source: Path, *, runner: CommandRunner = _run) -> Path:
    primary = primary_worktree(source, runner=runner)
    return primary.parent / f"{primary.name}_worktrees" / "integration-test"


def list_open_pull_requests(source: Path, *, runner: CommandRunner = _run) -> list[PullRequest]:
    if shutil.which("gh") is None:
        raise IntegrationError("未找到 GitHub CLI（gh），无法读取 PR。")
    result = runner((
        "gh", "pr", "list", "--state", "open", "--base", "main", "--limit", "200",
        "--json", "number,title,isDraft,headRefName,headRefOid,url,mergeStateStatus",
    ), source)
    source_json = _checked(result, "读取 GitHub PR ")
    try:
        payload = json.loads(source_json)
    except json.JSONDecodeError as exc:
        raise IntegrationError("GitHub CLI 返回了无效 JSON。") from exc
    if not isinstance(payload, list):
        raise IntegrationError("GitHub CLI 返回了无法识别的 PR 列表。")
    return sorted((PullRequest.from_github(item) for item in payload), key=lambda item: item.number)


def reconcile_selection(
    pull_requests: Iterable[PullRequest], selected: Iterable[int], seen: Iterable[int],
    previous_drafts: Iterable[int] = (),
) -> tuple[set[int], set[int], set[int]]:
    """Keep manual choices and follow PRs that change between Draft and Ready."""
    items = list(pull_requests)
    open_numbers = {item.number for item in items}
    previous_seen = set(seen)
    old_drafts = set(previous_drafts)
    next_selected = set(selected) & open_numbers
    for item in items:
        is_new = item.number not in previous_seen
        became_ready = item.number in old_drafts and not item.is_draft
        became_draft = item.number in previous_seen and item.number not in old_drafts and item.is_draft
        if (is_new and not item.is_draft) or became_ready:
            next_selected.add(item.number)
        elif became_draft:
            next_selected.discard(item.number)
    return next_selected, open_numbers, {item.number for item in items if item.is_draft}


def longest_common_prefix(current: Sequence[tuple[int, str]], desired: Sequence[tuple[int, str]]) -> int:
    length = 0
    for existing, requested in zip(current, desired):
        if existing != requested:
            break
        length += 1
    return length


class IntegrationWorkspace:
    def __init__(
        self, source: Path, target: Path, *, branch: str = INTEGRATION_BRANCH,
        runner: CommandRunner = _run, progress: ProgressReporter | None = None,
    ) -> None:
        self.source = repository_root(source, runner=runner)
        self.target = target.expanduser().resolve()
        self.branch = branch
        self.runner = runner
        self.progress = progress or (lambda _message: None)

    def _git(self, cwd: Path, *arguments: str) -> str:
        return _git(cwd, *arguments, runner=self.runner)

    def _same_repository(self, left: Path, right: Path) -> bool:
        def common(path: Path) -> Path:
            value = Path(self._git(path, "rev-parse", "--git-common-dir"))
            return (path / value).resolve() if not value.is_absolute() else value.resolve()
        return common(left) == common(right)

    def ensure(self) -> None:
        self.progress("正在连接 Git 远端并获取最新 origin/main…")
        self._git(self.source, "remote", "get-url", "origin")
        self._git(self.source, "fetch", "origin", "--prune")
        self._git(self.source, "show-ref", "--verify", "refs/remotes/origin/main")
        self.progress("已获取最新 origin/main，正在检查集成工作目录…")
        if self.target.exists():
            try:
                is_empty_directory = self.target.is_dir() and not next(self.target.iterdir(), None)
            except OSError as exc:
                raise IntegrationError(f"无法检查集成测试路径：{self.target}\n{exc}") from exc
            if is_empty_directory:
                self.target.rmdir()
            else:
                try:
                    actual = repository_root(self.target, runner=self.runner)
                except IntegrationError as exc:
                    raise IntegrationError(f"集成测试路径已经存在，但不是 Git worktree：{self.target}") from exc
                if actual != self.target:
                    raise IntegrationError(f"集成测试路径必须是 worktree 根目录：{self.target}")
                if not self._same_repository(self.source, self.target):
                    raise IntegrationError("集成测试路径属于另一个 Git 仓库。")
                active_branch = self._git(self.target, "branch", "--show-current")
                if active_branch != self.branch:
                    raise IntegrationError(
                        f"集成测试 worktree 当前位于 {active_branch or 'detached HEAD'}，预期为 {self.branch}。"
                    )
                return

        self.target.parent.mkdir(parents=True, exist_ok=True)
        branch_exists = self.runner(("git", "show-ref", "--verify", "--quiet", f"refs/heads/{self.branch}"), self.source)
        arguments = ["worktree", "add", "--no-track"]
        if branch_exists.returncode == 0:
            arguments.extend([str(self.target), self.branch])
        else:
            arguments.extend(["-b", self.branch, str(self.target), "origin/main"])
        self._git(self.source, *arguments)
        if repository_root(self.target, runner=self.runner) != self.target:
            raise IntegrationError("新建的集成测试 worktree 路径校验失败。")

    def status(self) -> str:
        if not self.target.exists():
            return ""
        # An untracked build tree is one blocker, not thousands of log entries.
        return self._git(self.target, "status", "--short", "--untracked-files=normal")

    def _state_path(self) -> Path:
        value = Path(self._git(self.target, "rev-parse", "--git-path", STATE_FILE_NAME))
        return (self.target / value).resolve() if not value.is_absolute() else value.resolve()

    def load_state(self) -> IntegrationState | None:
        if not self.target.exists():
            return None
        try:
            payload = json.loads(self._state_path().read_text(encoding="utf-8"))
            entries = tuple(
                IntegrationEntry(
                    int(item["number"]), str(item["head_sha"]),
                    str(item["integration_commit"]), str(item.get("title", "")),
                )
                for item in payload["entries"]
            )
            state = IntegrationState(str(payload["base_sha"]), entries)
        except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError):
            return None
        values = [state.base_sha, *(item.head_sha for item in entries), *(item.integration_commit for item in entries)]
        return state if all(SHA_RE.fullmatch(value) for value in values) else None

    def _save_state(self, state: IntegrationState) -> None:
        path = self._state_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "version": 1,
            "base_sha": state.base_sha,
            "entries": [
                {
                    "number": item.number,
                    "head_sha": item.head_sha,
                    "integration_commit": item.integration_commit,
                    "title": item.title,
                }
                for item in state.entries
            ],
        }
        temporary = path.with_suffix(".tmp")
        temporary.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        os.replace(temporary, path)

    def _fetch_pr_head(self, pull_request: PullRequest) -> str:
        self._git(
            self.target, "fetch", "--no-tags", "origin",
            f"refs/pull/{pull_request.number}/head",
        )
        fetched = self._git(self.target, "rev-parse", "FETCH_HEAD").lower()
        if fetched != pull_request.head_sha:
            raise IntegrationError(
                f"PR #{pull_request.number} 在刷新后又有新提交，请刷新 PR 列表后重试。"
            )
        return fetched

    def _require_idle_git_operation(self) -> None:
        for name in ("MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "rebase-merge", "rebase-apply", "sequencer"):
            value = Path(self._git(self.target, "rev-parse", "--git-path", name))
            path = value if value.is_absolute() else self.target / value
            if path.exists():
                raise IntegrationError("集成目录存在未完成的 Git 操作，已停止自动重建，请先完成或取消该操作。")

    def _valid_checkpoints(self, state: IntegrationState, base_sha: str, head_sha: str) -> bool:
        if state.base_sha != base_sha or state.head_sha != head_sha:
            return False
        previous = base_sha
        for entry in state.entries:
            if entry.integration_commit == previous:
                # A PR already included by an earlier PR produces no new merge.
                check = self.runner(("git", "merge-base", "--is-ancestor", entry.head_sha, previous), self.target)
                if check.returncode != 0:
                    return False
            else:
                result = self.runner(("git", "rev-list", "--parents", "-n", "1", entry.integration_commit), self.target)
                if result.returncode != 0 or result.stdout.split() != [entry.integration_commit, previous, entry.head_sha]:
                    return False
            previous = entry.integration_commit
        return True

    def _backup_head(self, head_sha: str) -> str:
        reference = f"{RECOVERY_REF_PREFIX}{uuid4().hex}"
        # Empty old-value makes creation fail rather than overwrite any backup.
        self._git(self.target, "update-ref", "--create-reflog", "-m", "Before local PR integration rebuild", reference, head_sha, "")
        if self._git(self.target, "rev-parse", reference) != head_sha:
            raise IntegrationError("集成备份校验失败，未开始重建。")
        self.progress(f"已保留原集成提交的本地备份：{reference}")
        return reference

    def _restore_head(self, original_head: str, recovery_ref: str) -> None:
        self.runner(("git", "merge", "--abort"), self.target)
        try:
            self._git(self.target, "reset", "--hard", original_head)
        except IntegrationError as exc:
            raise IntegrationError(f"集成更新失败，自动恢复也未完成。原提交仍保留在 {recovery_ref}，请勿清理现场。\n{exc}") from exc

    def apply(self, pull_requests: Iterable[PullRequest]) -> IntegrationResult:
        desired = sorted(pull_requests, key=lambda item: item.number)
        self.ensure()
        self._require_idle_git_operation()
        self.progress("正在检查集成工作目录状态与可复用的 PR 检查点…")
        dirty = self.status()
        if dirty:
            raise IntegrationError(
                "集成测试 worktree 存在未提交修改，已停止以避免覆盖 Debug 现场：\n" + dirty
            )

        base_sha = self._git(self.target, "rev-parse", "origin/main").lower()
        original_head = self._git(self.target, "rev-parse", "HEAD").lower()
        previous = self.load_state()
        state_is_current = previous is not None and self._valid_checkpoints(previous, base_sha, original_head)
        current_identity = [item.identity() for item in previous.entries] if state_is_current and previous else []
        desired_identity = [(item.number, item.head_sha) for item in desired]
        prefix = longest_common_prefix(current_identity, desired_identity) if state_is_current else 0

        if state_is_current and prefix == len(current_identity) == len(desired_identity):
            self.progress("所选 PR 组合没有变化，正在复用现有集成结果…")
            return IntegrationResult(self.target, base_sha, previous.entries, prefix, 0)

        # Even committed manual work is retained before replacing the disposable
        # integration branch. Do not infer safety from commit message prefixes.
        recovery_ref = self._backup_head(original_head)
        if not state_is_current:
            self.progress("检查点缺失或已失效；原提交已备份，将从最新 origin/main 重新合并所选 PR。")
        preserved = list(previous.entries[:prefix]) if state_is_current and previous else []
        checkpoint = preserved[-1].integration_commit if preserved else base_sha
        if preserved:
            self.progress(f"正在复用前 {len(preserved)} 个 PR 检查点，并重放后续变更…")
        else:
            self.progress("正在以最新 origin/main 作为集成基线…")
        entries = list(preserved)
        try:
            self._git(self.target, "reset", "--hard", checkpoint)
            remaining = desired[prefix:]
            for index, pull_request in enumerate(remaining, start=1):
                self.progress(f"正在获取 PR #{pull_request.number}：{pull_request.title}")
                head = self._fetch_pr_head(pull_request)
                self.progress(
                    f"正在合并 PR #{pull_request.number}（{index}/{len(remaining)}）：{pull_request.title}"
                )
                message = f"Integrate PR #{pull_request.number}: {pull_request.title}"
                result = self.runner((
                    "git", "-c", "user.name=Aster Integration", "-c",
                    "user.email=integration@aster.local", "merge", "--no-ff", "--no-edit",
                    "-m", message, head,
                ), self.target)
                if result.returncode != 0:
                    conflicts = self.runner(
                        ("git", "diff", "--name-only", "--diff-filter=U"), self.target,
                    ).stdout.strip()
                    detail = result.stderr.strip() or result.stdout.strip() or "未知 Git 合并错误"
                    if conflicts:
                        detail = f"冲突文件：\n{conflicts}\n\n{detail}"
                    raise IntegrationError(f"合并 PR #{pull_request.number} 失败：\n{detail}")
                integration_commit = self._git(self.target, "rev-parse", "HEAD").lower()
                entries.append(IntegrationEntry(
                    pull_request.number, pull_request.head_sha, integration_commit, pull_request.title,
                ))
        except Exception:
            self._restore_head(original_head, recovery_ref)
            raise

        state = IntegrationState(base_sha, tuple(entries))
        try:
            self.progress("PR 合并完成，正在保存集成状态…")
            self._save_state(state)
        except Exception:
            self._restore_head(original_head, recovery_ref)
            raise
        return IntegrationResult(self.target, base_sha, state.entries, prefix, len(desired) - prefix, recovery_ref)
