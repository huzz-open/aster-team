from __future__ import annotations

import subprocess
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts import local_pr_integration as integration


def run(command: list[str], cwd: Path) -> str:
    result = subprocess.run(
        command, cwd=cwd, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, encoding="utf-8", errors="replace", check=False,
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr or result.stdout)
    return result.stdout.strip()


def commit_file(repository: Path, branch: str, name: str, value: str) -> str:
    run(["git", "checkout", "main"], repository)
    run(["git", "checkout", "-B", branch], repository)
    (repository / name).write_text(value, encoding="utf-8")
    run(["git", "add", name], repository)
    run(["git", "commit", "-m", f"Add {name}"], repository)
    return run(["git", "rev-parse", "HEAD"], repository)


class LocalPullRequestIntegrationTest(unittest.TestCase):
    def sandbox_workspace(self):
        temporary = tempfile.TemporaryDirectory(prefix="aster-pr-recovery-")
        self.addCleanup(temporary.cleanup)
        sandbox = Path(temporary.name)
        repository, remote = sandbox / "aster-team", sandbox / "origin.git"
        repository.mkdir()
        run(["git", "init", "--initial-branch=main"], repository)
        run(["git", "config", "user.name", "Aster Test"], repository)
        run(["git", "config", "user.email", "aster@example.invalid"], repository)
        (repository / "README.md").write_text("base\n", encoding="utf-8")
        (repository / ".gitignore").write_text("/target/\n/node_modules/\n/data/\n", encoding="utf-8")
        run(["git", "add", "."], repository)
        run(["git", "commit", "-m", "Base"], repository)
        run(["git", "init", "--bare", str(remote)], repository)
        run(["git", "remote", "add", "origin", str(remote)], repository)
        run(["git", "push", "-u", "origin", "main"], repository)
        sha = commit_file(repository, "change-one", "feature.txt", "feature\n")
        run(["git", "push", "origin", f"{sha}:refs/pull/1/head"], repository)
        run(["git", "checkout", "main"], repository)
        target = sandbox / "aster-team_worktrees" / "integration-test"
        workspace = integration.IntegrationWorkspace(repository, target)
        pr = self.pull_request(1, sha)
        workspace.apply([pr])
        for directory in ("target", "node_modules", "data"):
            (target / directory).mkdir()
            (target / directory / "keep.txt").write_text(directory, encoding="utf-8")
        return repository, target, workspace, pr

    def assert_caches_preserved(self, target):
        for directory in ("target", "node_modules", "data"):
            self.assertEqual((target / directory / "keep.txt").read_text(encoding="utf-8"), directory)

    def pull_request(self, number: int, sha: str, *, draft: bool = False) -> integration.PullRequest:
        return integration.PullRequest(
            number, f"Change {number}", draft, f"change-{number}", sha,
            f"https://example.invalid/pull/{number}", "CLEAN",
        )

    def test_selection_defaults_only_new_non_draft_pull_requests(self) -> None:
        items = [
            self.pull_request(1, "1" * 40),
            self.pull_request(2, "2" * 40, draft=True),
            self.pull_request(3, "3" * 40),
        ]
        selected, seen, drafts = integration.reconcile_selection(items, {1}, {1, 2}, {2})
        self.assertEqual(selected, {1, 3})
        self.assertEqual(seen, {1, 2, 3})
        self.assertEqual(drafts, {2})

        selected, _seen, _drafts = integration.reconcile_selection(items, selected - {1}, seen, drafts)
        self.assertEqual(selected, {3}, "a manual removal must survive refresh")

        changed = [self.pull_request(1, "1" * 40, draft=True), self.pull_request(2, "2" * 40)]
        selected, _seen, _drafts = integration.reconcile_selection(changed, {1}, {1, 2}, {2})
        self.assertEqual(selected, {2}, "Draft and Ready transitions should update the default selection")

    def test_longest_common_prefix_compares_pr_number_and_head_sha(self) -> None:
        self.assertEqual(
            integration.longest_common_prefix(
                [(1, "a"), (2, "b"), (3, "c")],
                [(1, "a"), (2, "changed"), (3, "c")],
            ),
            1,
        )

    def test_removing_a_middle_pr_reuses_prefix_and_replays_only_suffix(self) -> None:
        with tempfile.TemporaryDirectory(prefix="aster-pr-integration-") as temporary:
            sandbox = Path(temporary)
            repository = sandbox / "aster-team"
            remote = sandbox / "origin.git"
            target = sandbox / "aster-team_worktrees" / "integration-test"
            repository.mkdir()
            run(["git", "init", "--initial-branch=main"], repository)
            run(["git", "config", "user.name", "Aster Test"], repository)
            run(["git", "config", "user.email", "aster@example.invalid"], repository)
            (repository / "README.md").write_text("base\n", encoding="utf-8")
            run(["git", "add", "README.md"], repository)
            run(["git", "commit", "-m", "Base"], repository)
            run(["git", "init", "--bare", str(remote)], repository)
            run(["git", "remote", "add", "origin", str(remote)], repository)
            run(["git", "push", "-u", "origin", "main"], repository)

            sha_a = commit_file(repository, "change-a", "a.txt", "a\n")
            sha_b = commit_file(repository, "change-b", "b.txt", "b\n")
            sha_c = commit_file(repository, "change-c", "c.txt", "c\n")
            run(["git", "checkout", "main"], repository)
            pull_requests = [
                self.pull_request(10, sha_a), self.pull_request(20, sha_b), self.pull_request(30, sha_c),
            ]
            (repository / "local-uncommitted.txt").write_text("must not enter integration\n", encoding="utf-8")

            progress: list[str] = []
            workspace = integration.IntegrationWorkspace(repository, target, progress=progress.append)
            with patch.object(workspace, "_fetch_pr_head", side_effect=lambda item: item.head_sha):
                first = workspace.apply(pull_requests)
                unchanged = workspace.apply(pull_requests)
                removed = workspace.apply([pull_requests[0], pull_requests[2]])

            self.assertEqual((first.reused, first.replayed), (0, 3))
            self.assertTrue(any("获取最新 origin/main" in message for message in progress))
            self.assertTrue(any("正在合并 PR #10（1/3）" in message for message in progress))
            self.assertTrue(any("PR 合并完成" in message for message in progress))
            self.assertEqual((unchanged.reused, unchanged.replayed), (3, 0))
            self.assertEqual((removed.reused, removed.replayed), (1, 1))
            self.assertTrue((target / "a.txt").is_file())
            self.assertFalse((target / "b.txt").exists())
            self.assertTrue((target / "c.txt").is_file())
            self.assertFalse((target / "local-uncommitted.txt").exists())
            self.assertEqual([item.number for item in workspace.load_state().entries], [10, 30])
            stable_head = run(["git", "rev-parse", "HEAD"], target)
            sha_conflict = commit_file(repository, "change-conflict", "a.txt", "conflict\n")
            conflicting = self.pull_request(40, sha_conflict)
            with patch.object(workspace, "_fetch_pr_head", side_effect=lambda item: item.head_sha):
                with self.assertRaisesRegex(integration.IntegrationError, "冲突文件"):
                    workspace.apply([pull_requests[0], pull_requests[2], conflicting])
            self.assertEqual(run(["git", "rev-parse", "HEAD"], target), stable_head)
            self.assertEqual(run(["git", "status", "--short"], target), "")
            (target / "manual.txt").write_text("do not discard\n", encoding="utf-8")
            run(["git", "add", "manual.txt"], target)
            run(["git", "commit", "-m", "Manual debug checkpoint"], target)
            manual_head = run(["git", "rev-parse", "HEAD"], target)
            with patch.object(workspace, "_fetch_pr_head", side_effect=lambda item: item.head_sha):
                recovered = workspace.apply([pull_requests[2]])
            self.assertEqual((recovered.reused, recovered.replayed), (0, 1))
            self.assertEqual(run(["git", "rev-parse", recovered.recovery_ref], target), manual_head)
            self.assertEqual(run(["git", "show", f"{recovered.recovery_ref}:manual.txt"], target), "do not discard")
            self.assertFalse((target / "manual.txt").exists())
            self.assertFalse((target / "a.txt").exists())
            self.assertTrue((target / "c.txt").exists())
            self.assertEqual(workspace.load_state().head_sha, run(["git", "rev-parse", "HEAD"], target))

    def test_dirty_integration_worktree_is_never_rebuilt(self) -> None:
        with tempfile.TemporaryDirectory(prefix="aster-pr-integration-dirty-") as temporary:
            sandbox = Path(temporary)
            repository = sandbox / "aster-team"
            remote = sandbox / "origin.git"
            target = sandbox / "worktrees" / "integration-test"
            repository.mkdir()
            run(["git", "init", "--initial-branch=main"], repository)
            run(["git", "config", "user.name", "Aster Test"], repository)
            run(["git", "config", "user.email", "aster@example.invalid"], repository)
            (repository / "README.md").write_text("base\n", encoding="utf-8")
            run(["git", "add", "README.md"], repository)
            run(["git", "commit", "-m", "Base"], repository)
            run(["git", "init", "--bare", str(remote)], repository)
            run(["git", "remote", "add", "origin", str(remote)], repository)
            run(["git", "push", "-u", "origin", "main"], repository)
            target.mkdir(parents=True)
            workspace = integration.IntegrationWorkspace(repository, target)
            workspace.apply([])
            (target / "debug-notes.txt").write_text("keep me\n", encoding="utf-8")

            with self.assertRaisesRegex(integration.IntegrationError, "未提交修改"):
                workspace.apply([])

            self.assertEqual((target / "debug-notes.txt").read_text(encoding="utf-8"), "keep me\n")

    def test_missing_corrupt_or_stale_checkpoint_rebuilds_with_recoverable_backup(self):
        for failure in ("missing", "corrupt", "stale"):
            with self.subTest(failure=failure):
                _repository, target, workspace, pr = self.sandbox_workspace()
                original = run(["git", "rev-parse", "HEAD"], target)
                path = workspace._state_path()
                if failure == "missing":
                    path.unlink()
                elif failure == "corrupt":
                    path.write_text("not-json", encoding="utf-8")
                else:
                    payload = json.loads(path.read_text(encoding="utf-8"))
                    payload["entries"][0]["integration_commit"] = "1" * 40
                    path.write_text(json.dumps(payload), encoding="utf-8")
                result = workspace.apply([pr])
                self.assertEqual((result.reused, result.replayed), (0, 1))
                self.assertEqual(run(["git", "rev-parse", result.recovery_ref], target), original)
                self.assertTrue((target / "feature.txt").exists())
                self.assertEqual(workspace.load_state().head_sha, run(["git", "rev-parse", "HEAD"], target))
                self.assertIsNone(workspace.apply([pr]).recovery_ref, "unchanged combinations need no backup or reset")
                self.assert_caches_preserved(target)

    def test_direct_merge_is_backed_up_and_only_selected_prs_are_rebuilt(self):
        repository, target, workspace, pr = self.sandbox_workspace()
        manual = commit_file(repository, "manual", "manual.txt", "retain this commit\n")
        run(["git", "merge", "--no-ff", "-m", "Manual merge outside manager", manual], target)
        old = run(["git", "rev-parse", "HEAD"], target)
        result = workspace.apply([pr])
        self.assertEqual(result.reused, 0)
        self.assertEqual(run(["git", "rev-parse", result.recovery_ref], target), old)
        self.assertEqual(run(["git", "show", f"{result.recovery_ref}:manual.txt"], target), "retain this commit")
        self.assertFalse((target / "manual.txt").exists())
        self.assert_caches_preserved(target)

    def test_latest_remote_main_invalidates_old_checkpoints(self):
        repository, target, workspace, pr = self.sandbox_workspace()
        (repository / "new-main.txt").write_text("latest main", encoding="utf-8")
        run(["git", "add", "new-main.txt"], repository)
        run(["git", "commit", "-m", "Advance main"], repository)
        latest = run(["git", "rev-parse", "HEAD"], repository)
        run(["git", "push", "origin", "main"], repository)
        result = workspace.apply([pr])
        self.assertEqual(result.base_sha, latest)
        self.assertEqual(result.reused, 0)
        self.assertTrue((target / "new-main.txt").exists())
        self.assertTrue((target / "feature.txt").exists())
        self.assert_caches_preserved(target)

    def test_checkpoint_must_match_actual_merge_parents_not_just_head(self):
        _repository, target, workspace, pr = self.sandbox_workspace()
        path = workspace._state_path()
        payload = json.loads(path.read_text(encoding="utf-8"))
        payload["entries"][0]["head_sha"] = payload["base_sha"]
        path.write_text(json.dumps(payload), encoding="utf-8")
        result = workspace.apply([pr])
        self.assertEqual(result.reused, 0)
        self.assertIsNotNone(result.recovery_ref)
        self.assertEqual(workspace.load_state().entries[0].head_sha, pr.head_sha)

    def test_backup_failure_stops_before_reset_and_preserves_original_state(self):
        _repository, target, workspace, _pr = self.sandbox_workspace()
        original = run(["git", "rev-parse", "HEAD"], target)
        state = workspace._state_path().read_bytes()
        commands = []
        def failing_runner(command, cwd):
            commands.append(list(command))
            if command[:2] == ("git", "update-ref"):
                return subprocess.CompletedProcess(command, 1, "", "backup denied")
            return integration._run(command, cwd)
        workspace.runner = failing_runner
        with self.assertRaisesRegex(integration.IntegrationError, "backup denied"):
            workspace.apply([])
        self.assertFalse(any(command[:2] == ["git", "reset"] for command in commands))
        self.assertEqual(run(["git", "rev-parse", "HEAD"], target), original)
        self.assertEqual(workspace._state_path().read_bytes(), state)
        self.assert_caches_preserved(target)

    def test_fetch_and_state_write_failures_restore_original_tree_and_state(self):
        for failure in ("fetch", "save"):
            with self.subTest(failure=failure):
                _repository, target, workspace, pr = self.sandbox_workspace()
                original = run(["git", "rev-parse", "HEAD"], target)
                state = workspace._state_path().read_bytes()
                if failure == "fetch":
                    with patch.object(workspace, "_fetch_pr_head", side_effect=integration.IntegrationError("fetch failed")):
                        with self.assertRaisesRegex(integration.IntegrationError, "fetch failed"):
                            workspace.apply([pr, self.pull_request(2, "2" * 40)])
                else:
                    with patch.object(workspace, "_save_state", side_effect=OSError("state write failed")):
                        with self.assertRaisesRegex(OSError, "state write failed"):
                            workspace.apply([])
                self.assertEqual(run(["git", "rev-parse", "HEAD"], target), original)
                self.assertEqual(workspace._state_path().read_bytes(), state)
                self.assertTrue((target / "feature.txt").exists())
                self.assert_caches_preserved(target)

    def test_clean_but_in_progress_git_operation_is_not_overwritten(self):
        _repository, target, workspace, _pr = self.sandbox_workspace()
        merge_head = Path(run(["git", "rev-parse", "--git-path", "MERGE_HEAD"], target))
        if not merge_head.is_absolute():
            merge_head = target / merge_head
        merge_head.write_text(run(["git", "rev-parse", "HEAD"], target) + "\n", encoding="utf-8")
        with self.assertRaisesRegex(integration.IntegrationError, "未完成的 Git 操作"):
            workspace.apply([])
        self.assertTrue(merge_head.exists())

    def test_tracked_local_edits_still_block_recovery(self):
        _repository, target, workspace, _pr = self.sandbox_workspace()
        (target / "feature.txt").write_text("unsaved work", encoding="utf-8")
        with self.assertRaisesRegex(integration.IntegrationError, "未提交修改"):
            workspace.apply([])
        self.assertEqual((target / "feature.txt").read_text(encoding="utf-8"), "unsaved work")


if __name__ == "__main__":
    unittest.main()
