from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
import tempfile
import threading
import unittest
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import Mock, patch

from scripts import local_dev_manager as manager


class _HealthHandler(BaseHTTPRequestHandler):
    service = "aster-control"

    def do_GET(self) -> None:
        payload = json.dumps({"status": "ok", "service": self.service}).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, _format: str, *_args: object) -> None:
        pass


class _CredentialHandler(BaseHTTPRequestHandler):
    logout_count = 0
    customer_logout_count = 0
    customer_password = "current-password"
    operations_password = "current-password"
    operations_password_change_required = False
    customer_password_change_required = False

    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
        if self.path.endswith("/api/admin/auth/logout"):
            if "aster_admin_session=customer-session" in self.headers.get("Cookie", ""):
                type(self).customer_logout_count += 1
                self.send_response(204)
            else:
                self.send_response(403)
            self.end_headers()
            return
        if self.path.endswith("/api/admin/auth/password"):
            if (
                "aster_admin_session=customer-session" not in self.headers.get("Cookie", "")
                or body.get("current_password") != type(self).customer_password
                or len(body.get("new_password", "").encode("utf-8")) < 12
            ):
                self.send_response(401)
                self.end_headers()
                return
            type(self).customer_password = body["new_password"]
            type(self).customer_password_change_required = False
            payload = b'{"ok":true}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        expected_password = (
            type(self).customer_password
            if self.path.endswith("/api/admin/auth/login")
            else type(self).operations_password
        )
        if body.get("email") != "admin@example.com" or body.get("password") != expected_password:
            self.send_response(401)
            self.end_headers()
            return
        if self.path.endswith("/api/admin/auth/login"):
            payload = json.dumps({
                "password_change_required": type(self).customer_password_change_required,
            }).encode("utf-8")
        else:
            payload = json.dumps({
                "operator": {
                    "id": "operator_test",
                    "password_change_required": type(self).operations_password_change_required,
                },
            }).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        if self.path.endswith("/api/admin/auth/login"):
            self.send_header("Set-Cookie", "aster_admin_session=customer-session; Path=/; HttpOnly")
        else:
            self.send_header("Set-Cookie", "aster_operations_session=session-test; Path=/; HttpOnly")
            self.send_header("Set-Cookie", "aster_operations_csrf=csrf-test; Path=/")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_PUT(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
        if (
            not self.path.endswith("/session/password")
            or "aster_operations_session=session-test" not in self.headers.get("Cookie", "")
            or self.headers.get("X-CSRF-Token") != "csrf-test"
            or body.get("current_password") != type(self).operations_password
            or not 12 <= len(body.get("new_password", "").encode("utf-8")) <= 72
        ):
            self.send_response(401)
            self.end_headers()
            return
        type(self).operations_password = body["new_password"]
        type(self).operations_password_change_required = False
        self.send_response(204)
        self.end_headers()

    def do_DELETE(self) -> None:
        if "aster_operations_session=session-test" in self.headers.get("Cookie", "") and self.headers.get("X-CSRF-Token") == "csrf-test":
            type(self).logout_count += 1
            self.send_response(204)
        else:
            self.send_response(403)
        self.end_headers()

    def log_message(self, _format: str, *_args: object) -> None:
        pass


class LocalDevManagerDiagnosticsTest(unittest.TestCase):
    def test_tkinter_file_dialog_is_available_for_security_directory_selection(self) -> None:
        self.assertIsNotNone(manager.filedialog)

    def test_integration_dependencies_install_once_per_package_lock(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            workspace.joinpath("package-lock.json").write_text('{"lockfileVersion": 3}\n', encoding="utf-8")
            completed = Mock(returncode=0, stdout="", stderr="")
            def install(*_args, **_kwargs):
                installed_lock = workspace / "node_modules/.package-lock.json"
                installed_lock.parent.mkdir(parents=True, exist_ok=True)
                installed_lock.write_text("{}\n", encoding="utf-8")
                return completed

            with patch.object(manager.subprocess, "run", side_effect=install) as run:
                self.assertTrue(manager.ensure_node_dependencies(workspace, "npm"))
                self.assertFalse(manager.ensure_node_dependencies(workspace, "npm"))

            run.assert_called_once()
            self.assertEqual(run.call_args.args[0], ["npm", "ci"])
            marker = workspace / "node_modules" / manager.NODE_DEPENDENCY_MARKER
            self.assertTrue(marker.is_file())

            workspace.joinpath("package-lock.json").write_text('{"lockfileVersion": 4}\n', encoding="utf-8")
            with patch.object(manager.subprocess, "run", side_effect=install) as rerun:
                self.assertTrue(manager.ensure_node_dependencies(workspace, "npm"))
            rerun.assert_called_once()

    def test_pr_integration_settings_are_shared_from_the_primary_worktree(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            primary = root / "aster-team"
            feature = root / "aster-team_worktrees/feature"
            integration = root / "aster-team_worktrees/integration-test"
            primary.mkdir(parents=True)
            legacy = feature / "data/local/pr-integration-settings.json"
            legacy.parent.mkdir(parents=True)
            legacy.write_text(json.dumps({"selected": [9], "seen": [8, 9], "drafts": []}), encoding="utf-8")

            with patch.object(manager, "environment_roots", return_value=(primary, integration)):
                self.assertEqual(manager.read_pr_integration_settings(feature)["selected"], [9])
                manager.write_pr_integration_settings({8, 9}, {8, 9}, set(), feature)

            saved = json.loads((primary / "data/local/pr-integration-settings.json").read_text(encoding="utf-8"))
            self.assertEqual(saved, {
                "version": 1, "selected": [8, 9], "seen": [8, 9], "drafts": [],
            })

    def test_development_tool_root_is_shared_with_old_and_new_integration_worktrees(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            primary = root / "aster-team"
            integration = root / "aster-team_worktrees/integration-test"
            settings = primary / ".aster-tools/windows-setup.json"
            settings.parent.mkdir(parents=True)
            settings.write_text(json.dumps({"installRoot": "D:\\software"}), encoding="utf-8")
            resolver = integration / "tools/toolchains/go-toolchain.mjs"
            resolver.parent.mkdir(parents=True)
            resolver.write_text("// legacy resolver\n", encoding="utf-8")

            with patch.object(manager, "IS_WINDOWS", True), patch.object(
                manager, "environment_roots", return_value=(primary, integration),
            ):
                legacy = manager.development_tool_environment(integration, {})
                self.assertEqual(legacy["ASTER_TOOLS_ROOT"], "D:\\software")
                self.assertEqual(legacy["ASTER_GO_ROOT"], "D:\\software\\Go")

                resolver.write_text("// supports ASTER_TOOLS_ROOT\n", encoding="utf-8")
                current = manager.development_tool_environment(integration, {})
                self.assertEqual(current["ASTER_TOOLS_ROOT"], "D:\\software")
                self.assertNotIn("ASTER_GO_ROOT", current)

    def test_integration_database_defaults_use_dedicated_names_and_users(self) -> None:
        with patch.object(manager, "is_integration_environment", return_value=True):
            defaults = manager.local_database_defaults(Path("integration"))
        self.assertEqual(defaults, {
            "ASTER_OPERATIONS_DB_NAME": "aster_operations_integration",
            "ASTER_OPERATIONS_DB_SERVICE_USER": "aster_operations_integration",
            "ASTER_CUSTOMER_DB_NAME": "aster_customer_integration",
            "ASTER_CUSTOMER_DB_SERVICE_USER": "aster_customer_integration",
        })

    def test_integration_environment_uses_a_complete_dedicated_port_profile(self) -> None:
        with patch.object(manager, "is_integration_environment", return_value=True):
            ports = manager.local_service_ports(Path("integration"), {})

        self.assertEqual(ports, {
            "operations_api": 22090,
            "operations_console": 22080,
            "customer_control": 21080,
            "customer_member": 21081,
            "customer_admin": 21082,
            "website": 24080,
            "website_backend": 18788,
        })
        self.assertEqual(manager.SERVICE_BY_KEY["operations_api"].name, "Operations API")

    def test_port_profile_allows_explicit_startup_configuration(self) -> None:
        with patch.object(manager, "is_integration_environment", return_value=False):
            ports = manager.local_service_ports(Path("primary"), {
                "ASTER_LOCAL_OPERATIONS_API_PORT": "32090",
                "ASTER_LOCAL_CUSTOMER_ADMIN_PORT": "31082",
            })

        self.assertEqual(ports["operations_api"], 32090)
        self.assertEqual(ports["customer_admin"], 31082)
        with patch.object(manager, "is_integration_environment", return_value=False):
            with self.assertRaisesRegex(ValueError, "端口配置不能重复"):
                manager.local_service_ports(Path("primary"), {
                    "ASTER_LOCAL_OPERATIONS_API_PORT": "12080",
                })

    def test_explicit_operations_port_is_used_by_health_checks_and_child_processes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary, \
             patch.object(manager, "ROOT", Path(temporary)), \
             patch.object(manager, "IS_INTEGRATION_RUNTIME", False), \
             patch.dict(manager.ACTIVE_SERVICE_PORTS, {"operations_api": 32090}), \
             patch.dict(os.environ, {}, clear=True):
            self.assertEqual(manager.operations_service_origin(), "http://127.0.0.1:32090")
            environment = manager.local_service_environment(False, None, {})

        self.assertEqual(environment["ASTER_OPERATIONS_ADDR"], "127.0.0.1:32090")

    def test_prepare_integration_uses_only_selected_remote_pull_request_heads(self) -> None:
        pull_requests = [
            manager.local_pr_integration.PullRequest(
                8, "Integration", False, "integration", "8" * 40,
                "https://example.invalid/8", "CLEAN",
            ),
            manager.local_pr_integration.PullRequest(
                9, "Copy", False, "copy", "9" * 40,
                "https://example.invalid/9", "CLEAN",
            ),
        ]
        workspace = Mock()
        expected = Mock()
        workspace.apply.return_value = expected
        source = Path("feature-worktree")
        target = Path("integration-worktree")
        with patch.object(
            manager, "read_pr_integration_settings",
            return_value={"selected": [9], "seen": [8, 9], "drafts": []},
        ), patch.object(
            manager.local_pr_integration, "list_open_pull_requests", return_value=pull_requests,
        ), patch.object(
            manager, "write_pr_integration_settings",
        ) as save, patch.object(
            manager, "environment_roots", return_value=(Path("primary"), target),
        ), patch.object(
            manager.local_pr_integration, "IntegrationWorkspace", return_value=workspace,
        ) as workspace_factory:
            actual = manager.prepare_integration_environment(source)

        self.assertIs(actual, expected)
        workspace_factory.assert_called_once_with(source, target)
        workspace.apply.assert_called_once_with([pull_requests[1]])
        save.assert_called_once_with({9}, {8, 9}, set(), source)

    def test_environment_chooser_select_all_excludes_drafts_but_manual_selection_can_include_them(self) -> None:
        instance = object.__new__(manager.EnvironmentChooser)
        instance.busy = False
        instance.pr_busy = False
        instance.pull_requests = [
            manager.local_pr_integration.PullRequest(
                12, "Ready", False, "ready", "1" * 40, "https://example.invalid/12", "CLEAN",
            ),
            manager.local_pr_integration.PullRequest(
                13, "Draft", True, "draft", "2" * 40, "https://example.invalid/13", "UNKNOWN",
            ),
        ]
        instance.selected = {13}
        instance.seen = set()
        instance.drafts = set()
        instance._save_pr_integration_settings = Mock()
        instance._render_pull_requests = Mock()

        instance._set_all_pull_requests(True)
        self.assertEqual(instance.selected, {12})
        self.assertEqual(instance.seen, {12, 13})
        self.assertEqual(instance.drafts, {13})

        instance._toggle_pull_request(13)
        self.assertEqual(instance.selected, {12, 13})

    def test_unchecked_applied_pull_request_is_marked_for_removal(self) -> None:
        pull_request = manager.local_pr_integration.PullRequest(
            12, "Applied", False, "applied", "1" * 40,
            "https://example.invalid/12", "CLEAN",
        )
        self.assertEqual(
            manager.pull_request_local_state(pull_request, False, {12: pull_request.head_sha}),
            "待移除",
        )
        self.assertEqual(
            manager.pull_request_local_state(pull_request, True, {12: pull_request.head_sha}),
            "已集成",
        )

    def test_environment_chooser_applies_exactly_the_visible_selected_pull_requests(self) -> None:
        instance = object.__new__(manager.EnvironmentChooser)
        instance.integration = Path("integration-worktree")
        instance.events = manager.queue.Queue()
        selected = [
            manager.local_pr_integration.PullRequest(
                12, "Selected", False, "selected", "1" * 40,
                "https://example.invalid/12", "CLEAN",
            ),
        ]
        result = Mock(path=Path("integration-worktree"))
        workspace = Mock()
        workspace.apply.return_value = result

        with patch.object(
            manager.local_pr_integration, "IntegrationWorkspace", return_value=workspace,
        ), patch.object(manager, "node_dependencies_ready", return_value=True), patch.object(
            manager, "ensure_node_dependencies",
        ) as ensure:
            instance._prepare_integration_worker(selected)

        workspace.apply.assert_called_once_with(selected)
        ensure.assert_called_once_with(result.path)
        self.assertEqual(
            instance.events.get_nowait(),
            ("progress", "集成环境已准备完成，正在打开开发控制台…"),
        )
        self.assertEqual(instance.events.get_nowait(), ("ready", result))

    def test_environment_chooser_keeps_loading_visible_until_child_window_is_ready(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            script = workspace / "scripts/local_dev_manager.py"
            script.parent.mkdir(parents=True)
            script.write_text("# manager\n", encoding="utf-8")
            instance = object.__new__(manager.EnvironmentChooser)
            instance.root = Mock()
            instance.loading_detail = Mock()
            instance.busy = True
            instance.launch_process = None
            instance.launch_ready_file = None
            instance.launch_deadline = 0.0
            instance._show_chooser = Mock()
            process = Mock()
            process.poll.return_value = None

            with patch.object(manager, "development_tool_environment", side_effect=lambda _workspace, base: dict(base)), patch.object(
                manager.subprocess, "Popen", return_value=process,
            ) as launch:
                instance._launch_manager(workspace)

            instance.root.destroy.assert_not_called()
            instance.root.after.assert_called_once_with(50, instance._finish_launch_when_ready)
            environment = launch.call_args.kwargs["env"]
            ready_file = Path(environment[manager.MANAGER_READY_ENV])
            ready_file.write_text("ready\n", encoding="utf-8")
            instance._finish_launch_when_ready()

            instance.root.destroy.assert_called_once_with()
            self.assertFalse(ready_file.exists())

    def test_child_manager_signals_after_rendering_its_root_window(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            ready_file = Path(temporary) / "ready.tmp"
            root = Mock()
            with patch.dict(manager.os.environ, {manager.MANAGER_READY_ENV: str(ready_file)}):
                manager.notify_parent_manager_ready(root)

            root.update_idletasks.assert_called_once_with()
            root.update.assert_called_once_with()
            self.assertEqual(ready_file.read_text(encoding="utf-8"), "ready\n")
            self.assertNotIn(manager.MANAGER_READY_ENV, manager.os.environ)

    def test_workspace_title_shows_only_leaf_but_tooltip_can_use_full_path(self) -> None:
        directory = Path("D:/code/custom/aster-team_worktrees/integration-test")
        self.assertEqual(
            manager.workspace_title("Aster Team 本地开发控制台", directory),
            "Aster Team 本地开发控制台 · integration-test",
        )

    def test_child_window_is_centered_and_clamped_to_the_visible_screen(self) -> None:
        self.assertEqual(
            manager.centered_window_geometry(100, 80, 1200, 800, 720, 300, 0, 0, 1920, 1080),
            "720x300+340+330",
        )
        self.assertEqual(
            manager.centered_window_geometry(-600, -400, 300, 200, 1080, 650, 0, 0, 1920, 1080),
            "1080x650+16+16",
        )
        self.assertEqual(
            manager.centered_window_geometry(0, 0, 1024, 600, 1080, 620, 0, 0, 1024, 600),
            "992x568+16+16",
        )

    def test_local_authorization_snapshot_distinguishes_new_active_and_expired_workspaces(self) -> None:
        now = datetime(2026, 9, 4, tzinfo=timezone.utc)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertEqual(manager.local_authorization_snapshot(root, now).license_status, "missing")

            license_path = root / "data/control/license/license.json"
            license_path.parent.mkdir(parents=True)
            license_path.write_text(json.dumps({
                "license_id": "license_local_test",
                "expires_at": "2027-09-04T00:00:00.000Z",
            }), encoding="utf-8")
            runner_root = root / "data/runner"
            runner_root.mkdir(parents=True)
            (runner_root / "identity.json").write_text("{}", encoding="utf-8")
            (runner_root / "task-keys.json").write_text("{}", encoding="utf-8")
            local_credentials = root / "data/local/local-admin-credentials.env"
            local_credentials.parent.mkdir(parents=True)
            local_credentials.write_text(
                "ASTER_LOCAL_MEMBER_EMAIL=test@at.com\n"
                "ASTER_LOCAL_MEMBER_PASSWORD=member-password\n",
                encoding="utf-8",
            )
            self.assertFalse(
                manager.local_authorization_snapshot(root, now).member_account_prepared,
            )
            local_credentials.write_text(
                "ASTER_LOCAL_MEMBER_EMAIL=test@at.com\n"
                "ASTER_LOCAL_MEMBER_PASSWORD=member-password\n"
                "ASTER_LOCAL_MEMBER_QUOTA_READY=true\n",
                encoding="utf-8",
            )

            active = manager.local_authorization_snapshot(root, now)
            self.assertEqual(active.license_status, "active")
            self.assertEqual(active.license_id, "license_local_test")
            self.assertTrue(active.runner_registered)
            self.assertTrue(active.member_account_prepared)
            prompt, button = manager.local_authorization_prompt(active)
            self.assertIn("本地测试免费证书待补齐", prompt)
            self.assertIn("现有数据保持不变", prompt)
            self.assertNotIn("license_local_test", prompt)
            self.assertEqual(button, "确认并补齐缺失项")

            free_license_path = root / "data/local/demo-delivery/free-license.json"
            free_license_path.parent.mkdir(parents=True)
            free_license_path.write_text(json.dumps({
                "claims": {
                    "source": {"kind": "free_distribution"},
                    "binding": {"mode": "unbound"},
                    "validity": {"expiry": {"mode": "none"}},
                },
                "signature": "test-only-signature",
            }), encoding="utf-8")
            active = manager.local_authorization_snapshot(root, now)
            self.assertTrue(active.free_certificate_prepared)
            prompt, button = manager.local_authorization_prompt(active)
            self.assertIn("授权已经完成", prompt)
            self.assertIn("本地测试免费证书已生成", prompt)
            self.assertEqual(button, "")

            (runner_root / "task-keys.json").unlink()
            incomplete = manager.local_authorization_snapshot(root, now)
            prompt, button = manager.local_authorization_prompt(incomplete)
            self.assertIn("仅补齐缺失", prompt)
            self.assertEqual(button, "确认并补齐缺失项")

            license_path.write_text(json.dumps({
                "license_id": "license_local_test",
                "expires_at": "2025-09-04T00:00:00.000Z",
            }), encoding="utf-8")
            expired = manager.local_authorization_snapshot(root, now)
            self.assertEqual(expired.license_status, "expired")
            self.assertIn("不用于续期", manager.local_authorization_prompt(expired)[0])

    def test_opening_quick_authorization_waits_for_the_dialog_confirmation(self) -> None:
        instance = object.__new__(manager.LocalDevManager)
        instance.authorization_dialog = None
        instance.authorization_in_progress = False
        instance.demo_process = None
        instance._open_authorization_progress = Mock()
        snapshot = manager.LocalAuthorizationSnapshot("missing")

        with patch.object(manager, "local_authorization_snapshot", return_value=snapshot):
            instance.open_local_authorization()

        instance._open_authorization_progress.assert_called_once_with(snapshot)

    def test_local_password_editor_updates_only_the_credentials_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            credentials = Path(temporary) / "local-admin-credentials.env"
            credentials.write_text(
                "ASTER_LOCAL_CUSTOMER_EMAIL=customer@example.com\n"
                "ASTER_LOCAL_CUSTOMER_PASSWORD=old-customer\n"
                "ASTER_LOCAL_OPERATIONS_EMAIL=operations@example.com\n"
                "ASTER_LOCAL_OPERATIONS_PASSWORD=old-operations\n"
                "UNRELATED=value\n",
                encoding="utf-8",
            )

            manager.save_local_admin_passwords(credentials, "new customer", "new-operations")

            values = manager.parse_env_text(credentials.read_text(encoding="utf-8"))
            self.assertEqual(values["ASTER_LOCAL_CUSTOMER_PASSWORD"], "new customer")
            self.assertEqual(values["ASTER_LOCAL_OPERATIONS_PASSWORD"], "new-operations")
            self.assertEqual(values["ASTER_LOCAL_CUSTOMER_EMAIL"], "customer@example.com")
            self.assertEqual(values["ASTER_LOCAL_OPERATIONS_EMAIL"], "operations@example.com")
            self.assertEqual(values["UNRELATED"], "value")
            with self.assertRaisesRegex(ValueError, "不能为空"):
                manager.save_local_admin_passwords(credentials, "", "new-operations")
            with self.assertRaisesRegex(ValueError, "不能包含换行符"):
                manager.save_local_admin_passwords(credentials, "new\ncustomer", "new-operations")

            manager.save_local_account_passwords(
                credentials, "new customer", "new-operations", "member-password",
                member_email=" member@example.com ",
            )
            values = manager.parse_env_text(credentials.read_text(encoding="utf-8"))
            self.assertEqual(values["ASTER_LOCAL_MEMBER_PASSWORD"], "member-password")
            self.assertEqual(values["ASTER_LOCAL_MEMBER_EMAIL"], "member@example.com")
            self.assertEqual(values["UNRELATED"], "value")
            with self.assertRaisesRegex(ValueError, "有效邮箱和密码"):
                manager.save_local_account_passwords(
                    credentials, "new customer", "new-operations", "member-password",
                    member_email="",
                )
            self.assertEqual(
                manager.parse_env_text(credentials.read_text(encoding="utf-8"))["ASTER_LOCAL_MEMBER_EMAIL"],
                "member@example.com",
            )

    def test_quick_password_change_uses_normal_local_apis_for_both_accounts(self) -> None:
        _CredentialHandler.customer_password = "customer-current-password"
        _CredentialHandler.operations_password = "operations-current-password"
        server = ThreadingHTTPServer(("127.0.0.1", 0), _CredentialHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        operations_url = f"http://127.0.0.1:{server.server_port}/api/operations/v1"
        customer_url = f"http://127.0.0.1:{server.server_port}"
        try:
            self.assertIsNone(manager.change_operations_password(
                "admin@example.com", "operations-current-password", "shared-new-password",
                base_url=operations_url,
            ))
            self.assertIsNone(manager.change_customer_password(
                "admin@example.com", "customer-current-password", "shared-new-password",
                base_url=customer_url,
            ))
            self.assertEqual(_CredentialHandler.operations_password, "shared-new-password")
            self.assertEqual(_CredentialHandler.customer_password, "shared-new-password")
            self.assertIsNone(manager.change_operations_password(
                "admin@example.com", "operations-current-password", "shared-new-password",
                base_url=operations_url,
            ))
            self.assertIsNone(manager.change_customer_password(
                "admin@example.com", "customer-current-password", "shared-new-password",
                base_url=customer_url,
            ))
            self.assertIn("12—72", manager.quick_password_validation_error("short") or "")
            self.assertIsNone(manager.quick_password_validation_error("shared-new-password"))
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    def test_quick_password_change_worker_syncs_both_successful_passwords_to_env(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            credentials = Path(temporary) / "local-admin-credentials.env"
            credentials.write_text(
                "ASTER_LOCAL_CUSTOMER_EMAIL=admin@example.com\n"
                "ASTER_LOCAL_CUSTOMER_PASSWORD=customer-current-password\n"
                "ASTER_LOCAL_OPERATIONS_EMAIL=admin@example.com\n"
                "ASTER_LOCAL_OPERATIONS_PASSWORD=operations-current-password\n",
                encoding="utf-8",
            )
            instance = object.__new__(manager.LocalDevManager)
            instance.events = manager.queue.Queue()
            payload = {
                "operations_email": "admin@example.com",
                "operations_password": "operations-current-password",
                "customer_email": "admin@example.com",
                "customer_password": "customer-current-password",
            }
            with patch.object(manager, "LOCAL_ADMIN_CREDENTIALS_FILE", credentials), \
                 patch.object(manager, "change_operations_password", return_value=None), \
                 patch.object(manager, "change_customer_password", return_value=None):
                instance._quick_password_change_worker(payload, "shared-new-password")

            event = instance.events.get_nowait()
            values = manager.parse_env_text(credentials.read_text(encoding="utf-8"))
            self.assertEqual(event, ("quick_password_change_done", [], ["Operations", "Customer"]))
            self.assertEqual(values["ASTER_LOCAL_OPERATIONS_PASSWORD"], "shared-new-password")
            self.assertEqual(values["ASTER_LOCAL_CUSTOMER_PASSWORD"], "shared-new-password")
            self.assertTrue(manager.authorization_requires_password_change(
                "Customer 尚未完成首次改密",
            ))
            self.assertEqual(
                manager.password_change_required_accounts(
                    "Operations 尚未完成首次改密\nCustomer 尚未完成首次改密",
                ),
                {"Operations", "Customer"},
            )

    def test_integration_rebuild_detects_services_started_from_target_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            state = workspace / "data/local/dev-manager-processes.json"
            state.parent.mkdir(parents=True)
            state.write_text(json.dumps({
                "version": 1,
                "services": {
                    "customer_control": {"pid": 4321, "identity": "windows:123"},
                    "unknown": {"pid": 9999, "identity": "stale"},
                },
            }), encoding="utf-8")
            with patch.object(manager, "process_identity", side_effect=lambda pid: "windows:123" if pid == 4321 else None):
                self.assertEqual(manager.managed_services_in_workspace(workspace), ["Customer Control"])

    def test_stop_local_services_uses_only_pid_reuse_checked_state(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            state = workspace / "data/local/dev-manager-processes.json"
            state.parent.mkdir(parents=True)
            state.write_text(json.dumps({
                "version": 1,
                "services": {
                    "customer_control": {"pid": 4321, "identity": "windows:123"},
                },
            }), encoding="utf-8")
            with patch.object(
                manager, "process_identity", side_effect=["windows:123", None],
            ), patch.object(
                manager, "controllable_external_process", return_value=None,
            ), patch.object(
                manager, "stop_service_process", return_value=None,
            ) as stop_process:
                stopped, failed = manager.stop_local_services(workspace)

            self.assertEqual(stopped, ["Customer Control"])
            self.assertEqual(failed, [])
            stop_process.assert_called_once()
            self.assertEqual(stop_process.call_args.args[1].pid, 4321)
            self.assertEqual(json.loads(state.read_text(encoding="utf-8"))["services"], {})

    def test_stop_local_services_can_take_over_a_verified_listener_for_current_configuration(self) -> None:
        external = Mock(pid=9876)
        external.wait.return_value = 0
        with tempfile.TemporaryDirectory() as temporary, patch.object(
            manager, "controllable_external_process",
            side_effect=lambda service: external if service.key == "customer_control" else None,
        ), patch.object(manager, "stop_service_process", return_value=None) as stop_process:
            stopped, failed = manager.stop_local_services(Path(temporary))

        self.assertEqual(stopped, ["Customer Control"])
        self.assertEqual(failed, [])
        stop_process.assert_called_once_with(manager.SERVICE_BY_KEY["customer_control"], external)

    def test_process_discovery_does_not_cross_runtime_configurations(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            primary = parent / "aster-team"
            integration = parent / "aster-team_worktrees/integration-test"
            old_worktree = parent / "aster-team_worktrees/old-feature"
            primary.mkdir()
            integration.mkdir(parents=True)
            state = old_worktree / "data/local/dev-manager-processes.json"
            state.parent.mkdir(parents=True)
            state.write_text(json.dumps({
                "version": 1,
                "services": {
                    "operations_api": {"pid": 2468, "identity": "windows:456"},
                },
            }), encoding="utf-8")

            with patch.object(
                manager, "environment_roots", return_value=(primary, integration),
            ), patch.object(
                manager, "process_identity", return_value="windows:456",
            ):
                processes = manager.identity_checked_state_processes(primary)

            self.assertEqual(processes, {})

    def test_initialization_can_confirm_stopping_verified_running_services(self) -> None:
        instance = object.__new__(manager.LocalDevManager)
        instance.setup_process = None
        instance.node = "node"
        instance._setup_values = Mock(return_value={"setting": "value"})
        instance._set_setup_busy = Mock()
        instance._append_setup_log = Mock()
        instance._stop_before_setup_worker = Mock()
        instance._show_stop_before_setup_dialog = Mock()
        process = Mock()
        process.poll.return_value = None
        instance.processes = {"customer_control": process}

        with patch.object(
            manager, "probe", side_effect=lambda url, **_kwargs: bool(url and "11080" in url),
        ), patch.object(
            manager, "identity_checked_state_processes", return_value={},
        ):
            instance.initialize_from_ui()

        instance._show_stop_before_setup_dialog.assert_called_once_with(
            {"setting": "value"},
            ((f"customer_control:{process.pid}", process),),
            ["Customer Control"],
        )

    def test_reinitialize_opens_setup_and_defers_runtime_check_until_confirmation(self) -> None:
        instance = object.__new__(manager.LocalDevManager)
        instance._show_setup = Mock()
        instance._running_core_services = Mock(side_effect=AssertionError("must be deferred"))

        instance.request_reinitialize()

        instance._show_setup.assert_called_once_with()
        instance._running_core_services.assert_not_called()

    def test_confirmed_initialization_stop_starts_the_worker(self) -> None:
        instance = object.__new__(manager.LocalDevManager)
        instance._set_setup_busy = Mock()
        instance._append_setup_log = Mock()
        instance._stop_before_setup_worker = Mock()
        process = Mock()
        worker = Mock()
        processes = (("customer_control:123", process),)

        with patch.object(manager.threading, "Thread", return_value=worker) as thread:
            instance._begin_stop_before_setup({"setting": "value"}, processes)

        instance._set_setup_busy.assert_called_once_with(True)
        thread.assert_called_once_with(
            target=instance._stop_before_setup_worker,
            args=({"setting": "value"}, processes),
            daemon=True,
        )
        worker.start.assert_called_once_with()

    def test_closing_manager_preserves_running_services(self) -> None:
        instance = object.__new__(manager.LocalDevManager)
        instance.setup_process = None
        instance.authorization_in_progress = False
        instance.closing = False
        instance.root = Mock()
        instance._save_process_state = Mock()

        instance.close_requested()

        self.assertTrue(instance.closing)
        instance._save_process_state.assert_called_once_with()
        instance.root.destroy.assert_called_once_with()

    def test_service_output_is_read_from_a_persistent_log_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "customer_control.log"
            path.write_text("service started\n", encoding="utf-8")
            instance = object.__new__(manager.LocalDevManager)
            instance.closing = False
            instance.events = manager.queue.Queue()
            process = Mock(pid=1234)
            process.poll.return_value = 0

            instance._read_output_file(manager.SERVICE_BY_KEY["customer_control"], process, path, False)

            self.assertEqual(instance.events.get_nowait(), ("log", "customer_control", "service started"))

    def test_local_account_links_auto_sign_in(self) -> None:
        admin_port = manager.ACTIVE_SERVICE_PORTS["customer_admin"]
        member_port = manager.ACTIVE_SERVICE_PORTS["customer_member"]
        operations_port = manager.ACTIVE_SERVICE_PORTS["operations_console"]
        self.assertEqual(
            manager.local_browser_url(f"http://127.0.0.1:{admin_port}/login"),
            f"http://127.0.0.1:{admin_port}/login?local_login=1",
        )
        self.assertEqual(
            manager.local_browser_url(f"http://localhost:{operations_port}/?view=all"),
            f"http://localhost:{operations_port}/login?local_login=1&redirect=%2F%3Fview%3Dall",
        )
        self.assertEqual(
            manager.local_browser_url(f"http://127.0.0.1:{member_port}"),
            f"http://127.0.0.1:{member_port}/login?local_login=1",
        )
        self.assertEqual(
            manager.local_browser_url(f"http://127.0.0.1:{member_port}/models?view=all"),
            f"http://127.0.0.1:{member_port}/login?local_login=1&redirect=%2Fmodels%3Fview%3Dall",
        )
        self.assertEqual(manager.local_browser_url("https://admin.example.com"), "https://admin.example.com")

    def test_chrome_is_the_default_browser_option(self) -> None:
        self.assertEqual(manager.DEFAULT_BROWSER_OPTION, manager.BROWSER_CHROME)

    def test_browser_target_uses_explicit_chrome_controller(self) -> None:
        chrome = Mock()
        chrome.open.return_value = True
        chrome_path = r"C:\Program Files\Google\Chrome\Application\chrome.exe"
        with patch.object(manager, "chrome_executable", return_value=chrome_path), \
             patch.object(manager.webbrowser, "BackgroundBrowser", return_value=chrome) as browser_class, \
             patch.object(manager.webbrowser, "get") as get_browser, \
             patch.object(manager.webbrowser, "open") as open_default:
            self.assertTrue(manager.open_browser_target("http://127.0.0.1:11082", manager.BROWSER_CHROME))

        browser_class.assert_called_once_with(chrome_path)
        chrome.open.assert_called_once_with("http://127.0.0.1:11082", new=2)
        get_browser.assert_not_called()
        open_default.assert_not_called()

    def test_browser_target_falls_back_to_the_registered_python_controller(self) -> None:
        chrome = Mock()
        chrome.open.return_value = True
        with patch.object(manager, "chrome_executable", return_value=None), \
             patch.object(manager.webbrowser, "get", return_value=chrome) as get_browser:
            self.assertTrue(manager.open_browser_target("https://example.com", manager.BROWSER_CHROME))

        get_browser.assert_called_once_with("chrome")
        chrome.open.assert_called_once_with("https://example.com", new=2)

    def test_chrome_executable_uses_the_windows_registered_app_path(self) -> None:
        with patch.object(manager, "IS_WINDOWS", True), \
             patch.object(manager.shutil, "which", return_value=None), \
             patch.object(manager, "_windows_app_path", return_value=r"D:\Apps\Chrome\chrome.exe"):
            self.assertEqual(manager.chrome_executable(), r"D:\Apps\Chrome\chrome.exe")

    def test_browser_target_can_use_the_system_default(self) -> None:
        with patch.object(manager.webbrowser, "open", return_value=True) as open_default, \
             patch.object(manager.webbrowser, "get") as get_browser:
            self.assertTrue(manager.open_browser_target(
                "http://127.0.0.1:11082", manager.BROWSER_SYSTEM_DEFAULT,
            ))

        open_default.assert_called_once_with("http://127.0.0.1:11082", new=2)
        get_browser.assert_not_called()

    def test_manager_open_url_uses_selected_browser_and_local_login_url(self) -> None:
        instance = object.__new__(manager.LocalDevManager)
        instance.browser_option = Mock()
        instance.browser_option.get.return_value = manager.BROWSER_CHROME
        instance.footer_message = Mock()
        instance.root = Mock()
        admin_url = f"http://127.0.0.1:{manager.ACTIVE_SERVICE_PORTS['customer_admin']}"
        with patch.object(manager, "open_browser_target", return_value=True) as open_target:
            instance.open_url(admin_url)

        open_target.assert_called_once_with(
            f"{admin_url}/login?local_login=1", manager.BROWSER_CHROME,
        )
        instance.footer_message.set.assert_called_once_with(
            f"已请求 Chrome 打开：{admin_url}",
        )

    def test_local_credentials_are_reloaded_when_the_file_sha256_changes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            credentials = Path(temporary) / "local-admin-credentials.env"
            credentials.write_text("ASTER_LOCAL_CUSTOMER_PASSWORD=first-password\n", encoding="utf-8")
            instance = object.__new__(manager.LocalDevManager)
            instance.local_admin_credentials_sha256 = ""
            instance.local_admin_credentials = {}
            with patch.object(manager, "LOCAL_ADMIN_CREDENTIALS_FILE", credentials):
                self.assertEqual(instance.read_current_local_admin_credentials()["ASTER_LOCAL_CUSTOMER_PASSWORD"], "first-password")
                first_digest = instance.local_admin_credentials_sha256
                credentials.write_text("ASTER_LOCAL_CUSTOMER_PASSWORD=second-password\n", encoding="utf-8")
                self.assertEqual(instance.read_current_local_admin_credentials()["ASTER_LOCAL_CUSTOMER_PASSWORD"], "second-password")
                self.assertNotEqual(instance.local_admin_credentials_sha256, first_digest)

    def test_operations_api_defers_health_timeout_until_go_dependencies_are_ready(self) -> None:
        operations = manager.SERVICE_BY_KEY["operations_api"]
        self.assertEqual(operations.preparation_marker, manager.GO_DEPENDENCIES_READY_MARKER)
        self.assertIsNone(manager.SERVICE_BY_KEY["operations_console"].preparation_marker)

    def test_customer_control_defers_health_timeout_until_rust_build_is_ready(self) -> None:
        control = manager.SERVICE_BY_KEY["customer_control"]
        self.assertEqual(control.preparation_marker, manager.CUSTOMER_CONTROL_BUILD_READY_MARKER)
        self.assertEqual(control.preparation_message, "正在准备 asterctl 与 Customer Control")
        self.assertEqual(control.preparation_ready_message, "Customer Control 已准备就绪")

    def test_website_service_runs_the_full_pages_stack(self) -> None:
        website = manager.SERVICE_BY_KEY["website"]
        self.assertEqual(website.npm_arguments, ("run", "dev:website:cf"))
        self.assertEqual(website.health_url, "http://127.0.0.1:14080")
        self.assertEqual(website.address_url, "http://127.0.0.1:14080")

    def test_terminal_output_is_split_into_visible_text_and_render_styles(self) -> None:
        source = '\x1b[32m\x1b[1mVITE\x1b[22m v7.3.6\x1b[39m  ➜  Local: http://127.0.0.1:12080/'
        self.assertEqual(
            manager.terminal_plain_text(source),
            'VITE v7.3.6  ➜  Local: http://127.0.0.1:12080/',
        )
        self.assertEqual(
            manager.terminal_plain_text('\x1b]0;npm run dev\x07服务已就绪'),
            '服务已就绪',
        )
        segments, final_style = manager.terminal_segments(source)
        self.assertEqual(segments[0][0], 'VITE')
        self.assertEqual(segments[0][1].foreground, manager.ANSI_COLORS[32])
        self.assertTrue(segments[0][1].bold)
        self.assertFalse(segments[1][1].bold)
        self.assertIsNone(final_style.foreground)
        true_color, _ = manager.terminal_segments('\x1b[38;2;12;34;56mRGB\x1b[0m')
        self.assertEqual(true_color[0][1].foreground, '#0c2238')

    def test_authorization_progress_parser_accepts_only_known_structured_events(self) -> None:
        prefix = manager.AUTHORIZATION_PROGRESS_PREFIX
        self.assertEqual(
            manager.parse_authorization_progress(prefix + '{"step":"issuance","state":"running","detail":"正在签发"}'),
            ("issuance", "running", "正在签发"),
        )
        self.assertIsNone(manager.parse_authorization_progress('普通构建日志'))
        self.assertIsNone(manager.parse_authorization_progress(prefix + '{"step":"unknown","state":"running","detail":"x"}'))
        self.assertIsNone(manager.parse_authorization_progress(prefix + 'not-json'))

    def test_multiline_authorization_details_are_split_into_aligned_rows(self) -> None:
        self.assertEqual(
            manager.authorization_detail_lines("Error: import failed\n  caused by: access denied\n\n"),
            ["Error: import failed", "  caused by: access denied"],
        )
        self.assertEqual(manager.authorization_detail_lines(""), ["—"])

    def test_authorization_detail_summary_prefers_the_actionable_error(self) -> None:
        detail = "file:///seed-local-demo.mjs:43\nthrow new Error(...)\nError: POST /api/admin/license 失败: LICENSE_INVALID\nat APIClient.request"
        self.assertEqual(
            manager.authorization_detail_summary(detail),
            "Error: POST /api/admin/license 失败: LICENSE_INVALID",
        )

    def test_log_severity_prioritizes_errors_over_warnings(self) -> None:
        self.assertIsNone(manager.log_severity('HTTP 健康检查通过，服务已就绪。'))
        self.assertEqual(manager.log_severity('(node:1) ExperimentalWarning: SQLite is experimental'), 'warning')
        self.assertEqual(manager.log_severity('{"level":"ERROR","msg":"listen failed"}'), 'error')
        self.assertEqual(manager.log_severity('进程已退出，退出码 1'), 'error')
        self.assertEqual(manager.higher_log_severity('warning', 'error'), 'error')
        self.assertEqual(manager.higher_log_severity('error', 'warning'), 'error')

    def test_health_probe_rejects_another_service_on_the_same_port(self) -> None:
        server = ThreadingHTTPServer(("127.0.0.1", 0), _HealthHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = f"http://127.0.0.1:{server.server_port}/health"
            self.assertTrue(manager.probe_result(url, "aster-control").healthy)
            result = manager.probe_result(url, "another-service")
            self.assertFalse(result.healthy)
            self.assertIn("aster-control", result.detail)
            self.assertIn("another-service", result.detail)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    def test_configuration_drift_reports_actual_and_expected_ports(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            env_path = root / "data/local/customer.env"
            env_path.parent.mkdir(parents=True)
            env_path.write_text("ASTER_CONTROL_PORT=11080\n", encoding="utf-8")
            service = manager.ServiceSpec(
                "control", "Customer Control", (),
                "http://127.0.0.1:11083/health", "http://127.0.0.1:11083",
                port_env=("data/local/customer.env", "ASTER_CONTROL_PORT"),
            )
            with patch.object(manager, "ROOT", root):
                detail = manager.service_configuration_error(service)
            self.assertIn("ASTER_CONTROL_PORT=11080", detail or "")
            self.assertIn("11083", detail or "")

    def test_external_aster_listener_can_be_safely_taken_over(self) -> None:
        service = manager.SERVICE_BY_KEY["customer_control"]
        with patch.object(manager, "listening_pids", return_value=[4321]), \
             patch.object(manager, "process_metadata", return_value=("aster-control.exe", 'target/debug/aster-control.exe serve --listen 127.0.0.1:11080')), \
             patch.object(manager.local_process_identity, "process_working_directory", return_value=manager.ROOT.resolve()), \
             patch.object(manager, "process_identity", return_value="windows:123"):
            process = manager.controllable_external_process(service)
        self.assertIsNotNone(process)
        self.assertEqual(process.pid if process else None, 4321)

    def test_external_process_diagnostics_shows_port_pid_command_and_working_directory(self) -> None:
        service = manager.SERVICE_BY_KEY["website"]
        with patch.object(manager, "listening_pids", return_value=[4321]), \
             patch.object(manager, "process_metadata", return_value=("node.exe", "node vite.js --port 14080")), \
             patch.object(manager.local_process_identity, "process_working_directory", return_value=manager.ROOT / "website"):
            summary, detail = manager.external_process_diagnostics(service)

        self.assertEqual(summary, "外部运行 · 14080 · PID 4321")
        self.assertIn("健康地址：http://127.0.0.1:14080", detail)
        self.assertIn("占用进程：PID 4321，node.exe", detail)
        self.assertIn("命令行：node vite.js --port 14080", detail)
        self.assertIn(f"工作目录：{manager.ROOT / 'website'}", detail)

    def test_external_process_diagnostics_explains_inaccessible_process_metadata(self) -> None:
        service = manager.SERVICE_BY_KEY["customer_admin"]
        with patch.object(manager, "listening_pids", return_value=[9876]), \
             patch.object(manager, "process_metadata", return_value=("node.exe", "")), \
             patch.object(manager.local_process_identity, "process_working_directory", return_value=None):
            summary, detail = manager.external_process_diagnostics(service)

        self.assertEqual(summary, "外部运行 · 11082 · PID 9876")
        self.assertIn("命令行：无法读取（可能是权限级别高于当前控制台）", detail)
        self.assertIn("工作目录：无法读取（可能是权限级别高于当前控制台）", detail)

    def test_stop_service_process_requires_pid_exit_and_port_release(self) -> None:
        service = manager.SERVICE_BY_KEY["customer_admin"]
        process = Mock(pid=4321)
        process.poll.return_value = 0
        with patch.object(manager, "terminate_process_tree", return_value=None) as terminate, \
             patch.object(manager, "listening_pids", return_value=[]) as listeners:
            failure = manager.stop_service_process(service, process)

        self.assertIsNone(failure)
        terminate.assert_called_once_with(process)
        listeners.assert_called_once_with(11082)

    def test_stop_service_process_reports_listener_left_after_process_exit(self) -> None:
        service = manager.SERVICE_BY_KEY["customer_admin"]
        process = Mock(pid=4321)
        process.poll.return_value = 0
        with patch.object(manager, "terminate_process_tree", return_value="taskkill 拒绝访问"), \
             patch.object(manager, "listening_pids", return_value=[9876]), \
             patch.object(manager, "describe_process", return_value="PID 9876，node.exe"), \
             patch.object(manager.time, "monotonic", side_effect=[0.0, 11.0]):
            failure = manager.stop_service_process(service, process)

        self.assertIn("taskkill 拒绝访问", failure or "")
        self.assertIn("端口 11082 仍被占用：PID 9876，node.exe", failure or "")

    def test_windows_termination_does_not_hide_taskkill_failure(self) -> None:
        process = Mock(pid=4321)
        process.poll.return_value = None
        process.wait.side_effect = subprocess.TimeoutExpired("4321", 10)
        completed = subprocess.CompletedProcess([], 5, stdout="", stderr="拒绝访问")
        with patch.object(manager, "IS_WINDOWS", True), \
             patch.object(manager.subprocess, "run", return_value=completed):
            failure = manager.terminate_process_tree(process)

        self.assertIn("PID 4321 在终止请求后仍未退出", failure or "")
        self.assertIn("拒绝访问", failure or "")
        process.kill.assert_called_once_with()

    def test_unrelated_port_owner_is_never_taken_over(self) -> None:
        service = manager.SERVICE_BY_KEY["customer_member"]
        with patch.object(manager, "listening_pids", return_value=[9876]), \
             patch.object(manager, "process_metadata", return_value=("python.exe", 'python -m http.server 11081')), \
             patch.object(manager, "process_identity", return_value="windows:456"):
            self.assertIsNone(manager.controllable_external_process(service))

    def test_external_runner_requires_the_rust_serve_signature(self) -> None:
        service = manager.SERVICE_BY_KEY["runner"]
        self.assertEqual(service.required_files, (
            "data/runner/identity.json", "data/runner/task-keys.json",
        ))
        records = [(2468, "aster-runner.exe", 'target/debug/aster-runner.exe serve --control-wss ws://127.0.0.1:11080/api/runner/channel')]
        with patch.object(manager, "all_process_metadata", return_value=records), \
             patch.object(manager, "process_metadata", return_value=records[0][1:]), \
             patch.object(manager.local_process_identity, "process_working_directory", return_value=manager.ROOT.resolve()), \
             patch.object(manager, "process_identity", return_value="windows:789"):
            process = manager.controllable_external_process(service)
        self.assertEqual(process.pid if process else None, 2468)
        unrelated = [(1357, "runner.exe", 'runner.exe run --config C:/other/runner.json')]
        with patch.object(manager, "all_process_metadata", return_value=unrelated), \
             patch.object(manager, "process_identity", return_value="windows:987"):
            self.assertIsNone(manager.controllable_external_process(service))

    def test_operations_credentials_are_checked_before_build_and_probe_session_is_removed(self) -> None:
        _CredentialHandler.logout_count = 0
        _CredentialHandler.customer_logout_count = 0
        _CredentialHandler.customer_password = "current-password"
        _CredentialHandler.operations_password = "current-password"
        _CredentialHandler.operations_password_change_required = False
        _CredentialHandler.customer_password_change_required = False
        server = ThreadingHTTPServer(("127.0.0.1", 0), _CredentialHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base_url = f"http://127.0.0.1:{server.server_port}/api/operations/v1"
        try:
            error = manager.validate_operations_credentials(
                "admin@example.com", "wrong-password", base_url=base_url,
            )
            self.assertIn("当前密码无效", error or "")
            self.assertIsNone(manager.validate_operations_credentials(
                "admin@example.com", "current-password", base_url=base_url,
            ))
            self.assertEqual(_CredentialHandler.logout_count, 1)
            _CredentialHandler.operations_password_change_required = True
            self.assertIn("尚未完成首次改密", manager.validate_operations_credentials(
                "admin@example.com", "current-password", base_url=base_url,
            ) or "")
            self.assertEqual(_CredentialHandler.logout_count, 2)
            _CredentialHandler.operations_password_change_required = False
            self.assertIn("当前密码无效", manager.validate_customer_credentials(
                "admin@example.com", "wrong-password", base_url=base_url,
            ) or "")
            self.assertIn("只允许连接本机", manager.validate_customer_credentials(
                "admin@example.com", "current-password", base_url="https://admin.example.com",
            ) or "")
            self.assertIsNone(manager.validate_customer_credentials(
                "admin@example.com", "current-password", base_url=base_url,
            ))
            self.assertEqual(_CredentialHandler.customer_logout_count, 1)
            _CredentialHandler.customer_password_change_required = True
            self.assertIn("尚未完成首次改密", manager.validate_customer_credentials(
                "admin@example.com", "current-password", base_url=base_url,
            ) or "")
            self.assertEqual(_CredentialHandler.customer_logout_count, 2)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    def test_authorization_preflight_rejects_an_incomplete_credentials_file_without_network_calls(self) -> None:
        instance = object.__new__(manager.LocalDevManager)
        instance.events = manager.queue.Queue()
        payload = {
            "operations_email": "admin@example.com",
            "operations_password": "",
            "customer_email": "admin@example.com",
            "customer_password": "customer-password",
        }
        with patch.object(manager, "validate_operations_credentials") as validate_operations, \
             patch.object(manager, "validate_customer_credentials") as validate_customer:
            instance._validate_demo_credentials_worker(payload)

        self.assertIn("账号文件", instance.events.get_nowait()[1])
        validate_operations.assert_not_called()
        validate_customer.assert_not_called()


class OperationsProcessOwnershipTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="aster ownership 测试 ")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.env_path = self.root / "data/local/operations.env"
        self.env_path.parent.mkdir(parents=True)
        self.env_path.write_text("ASTER_OPERATIONS_ADDR=127.0.0.1:23190\n", encoding="utf-8")
        self.service = manager.SERVICE_BY_KEY["operations_api"]
        self.start_patch(patch.object(manager, "ROOT", self.root))
        self.start_patch(patch.dict(os.environ))
        os.environ.pop("ASTER_OPERATIONS_ADDR", None)

    def start_patch(self, patcher):
        result = patcher.start()
        self.addCleanup(patcher.stop)
        return result

    def command(self, *arguments: str) -> str:
        return subprocess.list2cmdline(arguments) if os.name == "nt" else shlex.join(arguments)

    def candidate(self, arguments: tuple[str, ...] = ("--env-file", "./data/local/operations.env")) -> None:
        executable = self.root / "Go cache/api.exe"
        self.start_patch(patch.object(manager, "process_metadata", return_value=(
            "api.exe", self.command(str(executable), *arguments),
        )))
        self.start_patch(patch.object(manager.local_process_identity, "process_working_directory", return_value=self.root))
        self.start_patch(patch.object(manager.local_process_identity, "is_operations_binary", return_value=True))
        self.start_patch(patch.object(manager, "process_identity", return_value="windows:original"))

    def test_custom_port_health_and_address_use_the_go_configuration(self) -> None:
        self.assertEqual(manager.service_port(self.service), 23190)
        self.assertEqual(manager.service_health_url(self.service), "http://127.0.0.1:23190/health")
        self.assertEqual(manager.service_address_url(self.service), "http://127.0.0.1:23190/api/operations/v1")
        self.assertIsNone(manager.service_configuration_error(self.service))

    def test_environment_precedence_matches_go_including_blank_override(self) -> None:
        for value, expected in ((" 127.0.0.1:24190 ", 24190), ("", 12090), ("  ", 12090)):
            with self.subTest(value=value), patch.dict(os.environ, {"ASTER_OPERATIONS_ADDR": value}):
                self.assertEqual(manager.service_port(self.service), expected)

    def test_export_quoted_and_duplicate_env_entries_match_go_first_value(self) -> None:
        self.env_path.write_text(
            'export ASTER_OPERATIONS_ADDR="127.0.0.1:24190"\nASTER_OPERATIONS_ADDR=127.0.0.1:25190\n', encoding="utf-8",
        )
        self.assertEqual(manager.service_port(self.service), 24190)

    def test_missing_address_uses_default_and_wildcards_use_loopback(self) -> None:
        for value, expected in (("", "127.0.0.1:12090"), ("0.0.0.0:24190", "127.0.0.1:24190"),
                                ("[::]:24190", "[::1]:24190"), (":24190", "127.0.0.1:24190")):
            with self.subTest(value=value), patch.dict(os.environ, {"ASTER_OPERATIONS_ADDR": value}):
                self.assertEqual(manager.operations_service_origin(), "http://" + expected)

    def test_invalid_port_never_falls_back_to_default_or_searches_processes(self) -> None:
        for value in ("bad", "localhost:0", "localhost:65536", "localhost:no", "[::1", "user@localhost:12090",
                      "localhost:12090/path", "localhost:12090?x=1", "local host:12090"):
            with self.subTest(value=value), patch.dict(os.environ, {"ASTER_OPERATIONS_ADDR": value}), \
                 patch.object(manager, "listening_pids") as listeners, \
                 patch.object(manager, "all_process_metadata") as metadata:
                self.assertIsNotNone(manager.service_configuration_error(self.service))
                self.assertIsNone(manager.service_port(self.service))
                self.assertIsNone(manager.controllable_external_process(self.service))
                listeners.assert_not_called()
                metadata.assert_not_called()

    def test_go_api_on_custom_port_is_taken_over_only_after_provenance_checks(self) -> None:
        self.candidate()
        with patch.object(manager, "listening_pids", return_value=[4567]) as listeners:
            process = manager.controllable_external_process(self.service)
        self.assertEqual(process.pid if process else None, 4567)
        self.assertEqual([call.args for call in listeners.call_args_list], [(23190,), (23190,)])

    def test_absolute_quoted_env_file_and_equals_syntax_are_supported(self) -> None:
        for arguments in (("--env-file", str(self.env_path)), ("--env-file=" + str(self.env_path),),
                          ("-env-file", "./data/local/operations.env")):
            with self.subTest(arguments=arguments):
                self.candidate(arguments)
                self.assertTrue(manager.service_process_matches(self.service, 4567))

    def test_wrong_missing_or_duplicate_env_file_is_not_adopted(self) -> None:
        for arguments in ((), ("--env-file",), ("--env-file=",), ("--env-file", "other.env"),
                          ("--", "--env-file", "./data/local/operations.env"),
                          ("positional", "--env-file", "./data/local/operations.env"),
                          ("--create-database", "false", "--env-file", "./data/local/operations.env"),
                          ("--env-file", "./data/local/operations.env", "--env-file"),
                          ("--env-file", "./data/local/operations.env", "--env-file=other.env")):
            with self.subTest(arguments=arguments):
                self.candidate(arguments)
                self.assertFalse(manager.service_process_matches(self.service, 4567))

    def test_other_workspace_installation_or_unknown_cwd_is_not_adopted(self) -> None:
        self.candidate()
        for directory in (None, self.root / "installed", self.root.parent / "other-worktree"):
            with self.subTest(directory=directory), patch.object(
                manager.local_process_identity, "process_working_directory", return_value=directory,
            ):
                self.assertFalse(manager.service_process_matches(self.service, 4567))

    def test_api_filename_without_correct_go_module_is_not_adopted(self) -> None:
        self.candidate()
        with patch.object(manager.local_process_identity, "is_operations_binary", return_value=False):
            self.assertFalse(manager.service_process_matches(self.service, 4567))

    def test_installed_control_on_same_port_is_not_adopted(self) -> None:
        with patch.object(manager, "process_metadata", return_value=("aster-control.exe", "aster-control.exe serve")), \
             patch.object(manager.local_process_identity, "process_working_directory", return_value=self.root / "install"):
            self.assertFalse(manager.service_process_matches(manager.SERVICE_BY_KEY["customer_control"], 4567))

    def test_ambiguous_listener_pid_reuse_and_listener_change_fail_closed(self) -> None:
        self.candidate()
        with patch.object(manager, "listening_pids", return_value=[4567, 6789]):
            self.assertIsNone(manager.controllable_external_process(self.service))
        with patch.object(manager, "listening_pids", return_value=[4567]), \
             patch.object(manager, "process_identity", side_effect=["windows:original", "windows:reused"]):
            self.assertIsNone(manager.controllable_external_process(self.service))
        with patch.object(manager, "listening_pids", side_effect=[[4567], [6789]]):
            self.assertIsNone(manager.controllable_external_process(self.service))

    def test_stop_targets_only_verified_custom_port_not_the_default_port(self) -> None:
        self.candidate()
        with patch.object(manager, "SERVICES", (self.service,)), \
             patch.object(manager, "identity_checked_state_processes", return_value={}), \
             patch.object(manager, "clean_local_process_states"), \
             patch.object(manager, "listening_pids", side_effect=lambda port: [4567] if port == 23190 else [6789]) as listeners, \
             patch.object(manager, "stop_service_process", return_value=None) as stop_process:
            stopped, failed = manager.stop_local_services(self.root)
        self.assertEqual(stopped, [self.service.name])
        self.assertEqual(failed, [])
        stop_process.assert_called_once()
        self.assertEqual(stop_process.call_args.args[1].pid, 4567)
        self.assertTrue(all(call.args == (23190,) for call in listeners.call_args_list))

    def test_health_worker_uses_custom_url_and_invalid_config_does_not_probe(self) -> None:
        instance = object.__new__(manager.LocalDevManager)
        instance.events, instance.processes = manager.queue.Queue(), {}
        with patch.object(manager, "SERVICES", (self.service,)), \
             patch.object(manager, "probe_result", return_value=manager.ProbeResult(False)) as probe, \
             patch.object(manager, "controllable_external_process") as takeover:
            instance._health_worker()
            probe.assert_called_once_with("http://127.0.0.1:23190/health", None)
            takeover.assert_not_called()
            probe.reset_mock()
            with patch.dict(os.environ, {"ASTER_OPERATIONS_ADDR": "invalid"}):
                instance._health_worker()
            probe.assert_not_called()
            takeover.assert_not_called()

    def test_real_health_probe_uses_configured_ephemeral_port(self) -> None:
        server = ThreadingHTTPServer(("127.0.0.1", 0), _HealthHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with patch.dict(os.environ, {"ASTER_OPERATIONS_ADDR": f"127.0.0.1:{server.server_port}"}):
                self.assertTrue(manager.probe_result(manager.service_health_url(self.service)).healthy)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    def test_invalid_operations_address_never_sends_credentials(self) -> None:
        with patch.dict(os.environ, {"ASTER_OPERATIONS_ADDR": "invalid"}), \
             patch.object(manager.urllib.request, "urlopen") as request:
            self.assertIn("配置无效", manager.validate_operations_credentials("test", "password") or "")
            self.assertIn("配置无效", manager.change_operations_password("test", "old", "new") or "")
            request.assert_not_called()


class LocalLanAccessTests(unittest.TestCase):
    def test_lan_address_validation_rejects_unsafe_interface_addresses(self) -> None:
        self.assertEqual(manager.valid_lan_ipv4("10.213.41.33"), "10.213.41.33")
        for value in (None, "", "localhost", "127.0.0.1", "169.254.1.2", "::1"):
            with self.subTest(value=value):
                self.assertIsNone(manager.valid_lan_ipv4(value))

    def test_service_addresses_use_the_advertised_lan_host(self) -> None:
        member = manager.SERVICE_BY_KEY["customer_member"]
        self.assertEqual(
            manager.service_address_url(member, "10.213.41.33"),
            "http://10.213.41.33:11081",
        )

    def test_lan_environment_enables_user_facing_development_connections(self) -> None:
        with tempfile.TemporaryDirectory(prefix="aster LAN 测试 ") as temporary:
            root = Path(temporary)
            env_file = root / "data/local/operations.env"
            env_file.parent.mkdir(parents=True)
            env_file.write_text(
                "ASTER_OPERATIONS_TRUSTED_ORIGINS=http://127.0.0.1:12080,http://localhost:12080\n",
                encoding="utf-8",
            )
            with patch.object(manager, "ROOT", root):
                environment = manager.local_service_environment(
                    True,
                    "10.213.41.33",
                    {"EXISTING": "kept"},
                )

        self.assertEqual(environment["EXISTING"], "kept")
        self.assertEqual(environment[manager.LAN_ENABLED_ENV], "true")
        self.assertEqual(environment[manager.LAN_HOST_ENV], "10.213.41.33")
        self.assertEqual(environment["ASTER_OPERATIONS_ADDR"], "127.0.0.1:12090")
        self.assertEqual(environment["ASTER_CONTROL_PORT"], "11080")
        self.assertEqual(
            environment["ASTER_RUNNER_CONTROL_WSS"],
            "ws://127.0.0.1:11080/api/runner/channel",
        )
        self.assertEqual(
            environment["ASTER_OPERATIONS_TRUSTED_ORIGINS"],
            "http://127.0.0.1:12080,http://localhost:12080,http://10.213.41.33:12080",
        )

    def test_internal_service_address_stays_on_loopback(self) -> None:
        operations_api = manager.SERVICE_BY_KEY["operations_api"]
        self.assertEqual(
            manager.service_address_url(operations_api, "10.213.41.33"),
            "http://127.0.0.1:12090/api/operations/v1",
        )

    def test_disabled_lan_environment_removes_stale_host(self) -> None:
        environment = manager.local_service_environment(
            False,
            None,
            {manager.LAN_HOST_ENV: "10.213.41.33"},
        )
        self.assertEqual(environment[manager.LAN_ENABLED_ENV], "false")
        self.assertNotIn(manager.LAN_HOST_ENV, environment)

    def test_integration_runtime_environment_overrides_legacy_primary_ports(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            operations_env = root / "data/local/operations.env"
            operations_env.parent.mkdir(parents=True)
            operations_env.write_text(
                "ASTER_OPERATIONS_ADDR=127.0.0.1:12090\n"
                "ASTER_OPERATIONS_TRUSTED_ORIGINS=http://127.0.0.1:12080,http://localhost:12080\n",
                encoding="utf-8",
            )
            with patch.object(manager, "ROOT", root), \
                 patch.object(manager, "IS_INTEGRATION_RUNTIME", True), \
                 patch.dict(manager.ACTIVE_SERVICE_PORTS, manager.INTEGRATION_SERVICE_PORTS, clear=True):
                environment = manager.local_service_environment(False, None, {})

        self.assertEqual(environment["ASTER_OPERATIONS_ADDR"], "127.0.0.1:22090")
        self.assertEqual(environment["ASTER_CONTROL_PORT"], "21080")
        self.assertEqual(environment["ASTER_CUSTOMER_ADMIN_PORT"], "21082")
        self.assertEqual(environment["ASTER_WEBSITE_FRONTEND_PORT"], "24080")
        self.assertEqual(environment["ASTER_WEBSITE_BACKEND_PORT"], "18788")
        self.assertEqual(
            environment["ASTER_OPERATIONS_TRUSTED_ORIGINS"],
            "http://127.0.0.1:22080,http://localhost:22080",
        )

    def test_saved_lan_setting_is_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="aster LAN state ") as temporary:
            state = Path(temporary) / "process-state.json"
            state.write_text('{"lan_access_enabled": true}', encoding="utf-8")
            self.assertTrue(manager.saved_lan_access_enabled(state))
            state.write_text('{"lan_access_enabled": "true"}', encoding="utf-8")
            self.assertFalse(manager.saved_lan_access_enabled(state))
            state.write_text("invalid", encoding="utf-8")
            self.assertFalse(manager.saved_lan_access_enabled(state))


class NativeProcessProvenanceTests(unittest.TestCase):
    def test_reads_real_child_cwd_with_unicode_and_spaces(self) -> None:
        with tempfile.TemporaryDirectory(prefix="aster child 测试 ") as temporary:
            process = subprocess.Popen(
                [sys.executable, "-c", "import sys; print('ready', flush=True); sys.stdin.read()"],
                cwd=temporary, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
                **({"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}),
            )
            try:
                self.assertEqual(process.stdout.readline().strip(), "ready")
                self.assertEqual(manager.local_process_identity.process_working_directory(process.pid), Path(temporary).resolve())
            finally:
                process.communicate(timeout=5)
            self.assertIsNone(manager.local_process_identity.process_working_directory(process.pid))

    @unittest.skipUnless(sys.platform == "darwin", "macOS process APIs only")
    def test_reads_real_macos_process_identity_metadata_and_process_list(self) -> None:
        process = subprocess.Popen(
            [sys.executable, "-c", "import sys; print('ready', flush=True); sys.stdin.read()"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        )
        try:
            self.assertEqual(process.stdout.readline().strip(), "ready")
            identity = manager.process_identity(process.pid)
            self.assertIsNotNone(identity)
            self.assertTrue(identity.startswith("darwin:") if identity else False)
            self.assertEqual(manager.process_identity(process.pid), identity)
            name, command_line = manager.process_metadata(process.pid)
            self.assertTrue(name)
            self.assertIn("sys.stdin.read()", command_line)
            records = {pid: (record_name, command) for pid, record_name, command in manager.all_process_metadata()}
            self.assertIn(process.pid, records)
            self.assertIn("sys.stdin.read()", records[process.pid][1])
        finally:
            process.communicate(timeout=5)
        self.assertIsNone(manager.process_identity(process.pid))

    def test_access_denial_and_invalid_pid_fail_closed(self) -> None:
        self.assertIsNone(manager.local_process_identity.process_working_directory(0))
        if os.name == "nt":
            with patch.object(manager.local_process_identity, "_windows_working_directory", side_effect=OSError("denied")):
                self.assertIsNone(manager.local_process_identity.process_working_directory(123))

    def test_go_metadata_is_read_without_executing_candidate_or_downloading_toolchain(self) -> None:
        module = manager.local_process_identity
        for output, expected in (("\tpath\taster.local/team/operations/backend/cmd/api\n", True),
                                 ("\tpath\tunrelated/cmd/api\n", False),
                                 ("\tpath\taster.local/team/operations/backend/cmd/api-fake\n", False)):
            with self.subTest(output=output), patch.object(module.shutil, "which", return_value="go"), \
                 patch.object(module.subprocess, "run", return_value=Mock(returncode=0, stdout=output)) as run:
                module._operations_build_info.cache_clear()
                self.assertEqual(module._operations_build_info("candidate-api.exe", 100, 200), expected)
                self.assertEqual(run.call_args.args[0], ["go", "version", "-m", "candidate-api.exe"])
                self.assertEqual(run.call_args.kwargs["env"]["GOTOOLCHAIN"], "local")
        module._operations_build_info.cache_clear()


if __name__ == "__main__":
    unittest.main()
