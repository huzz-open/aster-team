#!/usr/bin/env python3
"""Aster Team local development process manager with a Tk GUI."""

from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes
import hashlib
import http.cookies
import ipaddress
import json
import os
import queue
import shlex
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
import webbrowser
from collections.abc import Callable, Mapping
from dataclasses import dataclass, replace
from datetime import datetime, timezone
from pathlib import Path, PureWindowsPath
from urllib.parse import parse_qsl, unquote, urlencode, urlparse

try:
    import winreg
except ImportError:  # pragma: no cover - only available on Windows
    winreg = None

try:
    from scripts import local_pr_integration, local_process_identity
except ImportError:  # Direct execution adds scripts/, rather than the repository root, to sys.path.
    import local_pr_integration
    import local_process_identity

try:
    import tkinter as tk
    from tkinter import filedialog, font as tkfont, messagebox, ttk
except ImportError as exc:  # pragma: no cover - depends on the local Python installation
    tk = None
    filedialog = None
    tkfont = None
    ttk = None
    messagebox = None
    TK_IMPORT_ERROR = exc
else:
    TK_IMPORT_ERROR = None


ROOT = Path(__file__).resolve().parents[1]
IS_WINDOWS = os.name == "nt"
UI_FONT_FAMILY = "Microsoft YaHei UI"
MONO_FONT_FAMILY = "Consolas"
SYMBOL_FONT_FAMILY = "Segoe UI Symbol"
LOCAL_ADMIN_CREDENTIALS_FILE = ROOT / "data/local/local-admin-credentials.env"
INITIALIZATION_FILES = (
    ROOT / "data/local/customer.env",
    ROOT / "data/local/operations.env",
    ROOT / "data/control/license/installation.json",
    LOCAL_ADMIN_CREDENTIALS_FILE,
)
PROCESS_STATE_FILE = ROOT / "data/local/dev-manager-processes.json"
SERVICE_LOG_ROOT = ROOT / "data/local/dev-manager-logs"
STARTUP_GRACE_SECONDS = 30.0
GO_DEPENDENCIES_READY_MARKER = "@@ASTER_GO_DEPENDENCIES_READY@@"
CUSTOMER_CONTROL_BUILD_READY_MARKER = "@@ASTER_CUSTOMER_CONTROL_BUILD_READY@@"
RESTORED_LOG_BYTES = 64 * 1024
PASSWORD_CHANGE_TIMEOUT_SECONDS = 30.0
NODE_DEPENDENCY_MARKER = ".aster-package-lock.sha256"
MANAGER_READY_ENV = "ASTER_LOCAL_MANAGER_READY_FILE"
LAN_ENABLED_ENV = "ASTER_LOCAL_LAN_ENABLED"
LAN_HOST_ENV = "ASTER_LOCAL_LAN_HOST"
BROWSER_CHROME = "Chrome"
BROWSER_SYSTEM_DEFAULT = "系统默认浏览器"
BROWSER_OPTIONS = (BROWSER_CHROME, BROWSER_SYSTEM_DEFAULT)
DEFAULT_BROWSER_OPTION = BROWSER_CHROME

PRIMARY_SERVICE_PORTS = {
    "operations_api": 12090,
    "operations_console": 12080,
    "customer_control": 11080,
    "customer_member": 11081,
    "customer_admin": 11082,
    "website": 14080,
    "website_backend": 8788,
}
INTEGRATION_SERVICE_PORTS = {
    key: port + 10_000 for key, port in PRIMARY_SERVICE_PORTS.items()
}
LOCAL_PORT_ENV = {
    "operations_api": "ASTER_LOCAL_OPERATIONS_API_PORT",
    "operations_console": "ASTER_LOCAL_OPERATIONS_CONSOLE_PORT",
    "customer_control": "ASTER_LOCAL_CUSTOMER_CONTROL_PORT",
    "customer_member": "ASTER_LOCAL_CUSTOMER_MEMBER_PORT",
    "customer_admin": "ASTER_LOCAL_CUSTOMER_ADMIN_PORT",
    "website": "ASTER_LOCAL_WEBSITE_FRONTEND_PORT",
    "website_backend": "ASTER_LOCAL_WEBSITE_BACKEND_PORT",
}


def is_integration_environment(source: Path = ROOT) -> bool:
    try:
        target = local_pr_integration.default_integration_path(source)
    except local_pr_integration.IntegrationError:
        return False
    return source.resolve() == target.resolve()


IS_INTEGRATION_RUNTIME = is_integration_environment()


def local_service_ports(
    source: Path = ROOT,
    environment: Mapping[str, str] | None = None,
) -> dict[str, int]:
    """Resolve the complete port profile once for this manager invocation."""
    values = os.environ if environment is None else environment
    integration = (
        IS_INTEGRATION_RUNTIME
        if source.resolve() == ROOT.resolve()
        else is_integration_environment(source)
    )
    defaults = (
        INTEGRATION_SERVICE_PORTS
        if integration
        else PRIMARY_SERVICE_PORTS
    )
    result = dict(defaults)
    for key, variable in LOCAL_PORT_ENV.items():
        configured = str(values.get(variable, "")).strip()
        if not configured:
            continue
        try:
            port = int(configured)
        except ValueError:
            raise ValueError(f"{variable} 必须是 1-65535 的端口号。") from None
        if not 1 <= port <= 65535:
            raise ValueError(f"{variable} 必须是 1-65535 的端口号。")
        result[key] = port
    if len(set(result.values())) != len(result):
        raise ValueError("本地服务端口配置不能重复。")
    return result


ACTIVE_SERVICE_PORTS = local_service_ports()


@dataclass(frozen=True)
class ServiceSpec:
    key: str
    name: str
    npm_arguments: tuple[str, ...]
    health_url: str | None
    address_url: str | None
    required_files: tuple[str, ...] = ()
    core: bool = True
    note: str = ""
    expected_health_service: str | None = None
    port_env: tuple[str, str] | None = None
    preparation_marker: str | None = None
    preparation_message: str = "正在准备服务"
    preparation_ready_message: str = "服务准备已完成"


@dataclass(frozen=True)
class ProbeResult:
    healthy: bool
    detail: str = ""


@dataclass(frozen=True)
class LocalAuthorizationSnapshot:
    license_status: str
    license_id: str = ""
    expires_at: str = ""
    runner_registered: bool = False
    member_account_prepared: bool = False
    free_certificate_prepared: bool = False


SERVICES = (
    ServiceSpec(
        "operations_api", "Operations API", ("run", "dev:operations:api"),
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_api']}/health",
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_api']}/api/operations/v1",
        ("data/local/operations.env",), note="运营后端",
        preparation_marker=GO_DEPENDENCIES_READY_MARKER,
        preparation_message="正在获取 Go 模块依赖",
        preparation_ready_message="Go 模块依赖已就绪",
    ),
    ServiceSpec(
        "operations_console", "Operations Console",
        (
            "run", "dev", "--workspace", "@aster/operations-console", "--",
            "--port", str(ACTIVE_SERVICE_PORTS["operations_console"]),
        ),
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_console']}",
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_console']}", note="运营管理页面",
    ),
    ServiceSpec(
        "customer_control", "Customer Control", ("run", "dev:api"),
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_control']}/healthz",
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_control']}",
        ("data/local/customer.env",), note="仅本机内部业务后端",
        expected_health_service="aster-control",
        port_env=("data/local/customer.env", "ASTER_CONTROL_PORT"),
        preparation_marker=CUSTOMER_CONTROL_BUILD_READY_MARKER,
        preparation_message="正在准备 asterctl 与 Customer Control",
        preparation_ready_message="Customer Control 已准备就绪",
    ),
    ServiceSpec(
        "customer_admin", "Customer Admin",
        (
            "run", "dev", "--workspace", "@aster/admin", "--",
            "--port", str(ACTIVE_SERVICE_PORTS["customer_admin"]),
        ),
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_admin']}",
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_admin']}", note="客户管理端",
    ),
    ServiceSpec(
        "customer_member", "Customer Member",
        (
            "run", "dev", "--workspace", "@aster/member", "--",
            "--port", str(ACTIVE_SERVICE_PORTS["customer_member"]),
        ),
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_member']}",
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_member']}", note="客户用户端",
    ),
    ServiceSpec(
        "website", "Website", ("run", "dev:website:cf"),
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['website']}",
        f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['website']}", core=False, note="官网 HMR 与试用申请后端",
    ),
    ServiceSpec(
        "runner", "Runner", ("run", "dev:runner"),
        None, None, ("data/runner/identity.json", "data/runner/task-keys.json"), core=False, note="本地授权后可启动",
    ),
)

SERVICE_BY_KEY = {service.key: service for service in SERVICES}
LAN_ADDRESS_SERVICE_KEYS = {
    "operations_console",
    "customer_control",
    "customer_admin",
    "customer_member",
    "website",
}


def service_log_path(key: str) -> Path:
    return SERVICE_LOG_ROOT / f"{key}.log"


LOG_TAB_LABELS = {
    "operations_api": "Ops API",
    "operations_console": "Ops Console",
    "customer_control": "Control",
    "customer_admin": "Admin",
    "customer_member": "Member",
    "website": "Website",
    "runner": "Runner",
}
LOG_SEVERITY_ICONS = {"warning": "⚠", "error": "✖"}
AUTHORIZATION_PROGRESS_PREFIX = "@@ASTER_PROGRESS@@"
AUTHORIZATION_STEPS = (
    ("credentials", "验证本地账号"),
    ("operations", "连接运营平台"),
    ("order", "创建客户与订单"),
    ("policy", "生成许可证策略"),
    ("request", "读取机器申请文件"),
    ("issuance", "签发机器许可证"),
    ("install", "导入本机许可证"),
    ("runner", "注册本机 Runner"),
    ("member", "准备用户侧账号与额度"),
)
AUTHORIZATION_STEP_KEYS = {key for key, _label in AUTHORIZATION_STEPS}
TERMINAL_SEQUENCE_RE = re.compile(
    r"(?:\x1b\][^\x07\x1b]*(?:\x07|\x1b\\))"  # OSC, including titles and hyperlinks
    r"|(?:\x1b\[([0-?]*)([ -/]*)([@-~]))"        # CSI
    r"|(?:\x1b[@-_])"                            # Other two-byte escape sequences
)
ANSI_COLORS = {
    30: "#5b6578", 31: "#d55f63", 32: "#74c991", 33: "#e5c07b",
    34: "#61afef", 35: "#c678dd", 36: "#56b6c2", 37: "#d8dee9",
    90: "#7f8a9e", 91: "#ff7a85", 92: "#8ddeaa", 93: "#f0d58a",
    94: "#7ab7ff", 95: "#dc91f2", 96: "#72d5df", 97: "#ffffff",
}


@dataclass(frozen=True)
class TerminalStyle:
    foreground: str | None = None
    background: str | None = None
    bold: bool = False
    dim: bool = False
    italic: bool = False
    underline: bool = False


def _ansi_256_color(index: int) -> str:
    base = (
        "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
        "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
    )
    if index < 16:
        return base[max(0, index)]
    if index < 232:
        value = index - 16
        red, green, blue = value // 36, (value % 36) // 6, value % 6
        channel = lambda item: 0 if item == 0 else 55 + item * 40
        return f"#{channel(red):02x}{channel(green):02x}{channel(blue):02x}"
    gray = 8 + min(index - 232, 23) * 10
    return f"#{gray:02x}{gray:02x}{gray:02x}"


def _apply_sgr(style: TerminalStyle, source: str) -> TerminalStyle:
    try:
        codes = [int(value or "0") for value in source.replace(":", ";").split(";")] if source else [0]
    except ValueError:
        return style
    index = 0
    while index < len(codes):
        code = codes[index]
        if code == 0:
            style = TerminalStyle()
        elif code == 1:
            style = replace(style, bold=True, dim=False)
        elif code == 2:
            style = replace(style, dim=True, bold=False)
        elif code == 3:
            style = replace(style, italic=True)
        elif code == 4:
            style = replace(style, underline=True)
        elif code == 22:
            style = replace(style, bold=False, dim=False)
        elif code == 23:
            style = replace(style, italic=False)
        elif code == 24:
            style = replace(style, underline=False)
        elif code in ANSI_COLORS:
            style = replace(style, foreground=ANSI_COLORS[code])
        elif code == 39:
            style = replace(style, foreground=None)
        elif 40 <= code <= 47 or 100 <= code <= 107:
            foreground_code = code - 10
            style = replace(style, background=ANSI_COLORS.get(foreground_code))
        elif code == 49:
            style = replace(style, background=None)
        elif code in {38, 48} and index + 2 < len(codes):
            color: str | None = None
            if codes[index + 1] == 5:
                color = _ansi_256_color(codes[index + 2])
                index += 2
            elif codes[index + 1] == 2 and index + 4 < len(codes):
                red, green, blue = (max(0, min(codes[index + offset], 255)) for offset in (2, 3, 4))
                color = f"#{red:02x}{green:02x}{blue:02x}"
                index += 4
            if color:
                style = replace(style, **{"foreground" if code == 38 else "background": color})
        index += 1
    return style


def terminal_segments(value: str, initial: TerminalStyle | None = None) -> tuple[list[tuple[str, TerminalStyle]], TerminalStyle]:
    """Parse terminal output into visible text segments while preserving SGR styles."""
    style = initial or TerminalStyle()
    segments: list[tuple[str, TerminalStyle]] = []
    position = 0
    for match in TERMINAL_SEQUENCE_RE.finditer(value):
        text = value[position:match.start()].replace("\r", "")
        text = "".join(character for character in text if character in "\t" or ord(character) >= 32)
        if text:
            segments.append((text, style))
        if match.group(3) == "m" and not match.group(2):
            style = _apply_sgr(style, match.group(1) or "")
        position = match.end()
    text = value[position:].replace("\r", "")
    text = "".join(character for character in text if character in "\t" or ord(character) >= 32)
    if text:
        segments.append((text, style))
    return segments, style


def terminal_plain_text(value: str) -> str:
    """Return visible terminal text for diagnostics; never use this for log rendering."""
    return "".join(text for text, _style in terminal_segments(value)[0])


def log_severity(value: str) -> str | None:
    if re.search(r'(?i)(?:\b(?:error|fatal|panic)\b|"level"\s*:\s*"error"|启动失败|失败[：:]|异常|退出码\s*[1-9]\d*)', value):
        return "error"
    if re.search(r'(?i)(?:\bwarn(?:ing)?\b|"level"\s*:\s*"warn(?:ing)?"|警告|告警|deprecated|experimentalwarning)', value):
        return "warning"
    return None


def higher_log_severity(current: str | None, incoming: str | None) -> str | None:
    levels = {None: 0, "warning": 1, "error": 2}
    return incoming if levels[incoming] > levels[current] else current


def parse_authorization_progress(value: str) -> tuple[str, str, str] | None:
    if not value.startswith(AUTHORIZATION_PROGRESS_PREFIX):
        return None
    try:
        payload = json.loads(value[len(AUTHORIZATION_PROGRESS_PREFIX):])
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    step = str(payload.get("step") or "")
    state = str(payload.get("state") or "")
    detail = str(payload.get("detail") or "").strip()
    if step not in AUTHORIZATION_STEP_KEYS or state not in {"running", "completed", "failed"} or not detail:
        return None
    return step, state, detail[:1000]


def authorization_detail_lines(value: str) -> list[str]:
    lines = [line.rstrip() for line in value.splitlines() if line.strip()]
    return lines or ["—"]


def authorization_detail_summary(value: str, limit: int = 180) -> str:
    lines = authorization_detail_lines(value)
    preferred = next(
        (
            line.strip()
            for line in reversed(lines)
            if any(marker in line.lower() for marker in ("error:", "failed", "失败", "异常"))
        ),
        lines[0].strip(),
    )
    return preferred if len(preferred) <= limit else f"{preferred[:limit - 1]}…"


def process_identity(pid: int) -> str | None:
    """Return a PID-reuse-safe process creation identity when the OS exposes one."""
    if pid <= 0:
        return None
    if sys.platform == "darwin":
        info = local_process_identity.darwin_process_info(pid)
        return info[1] if info is not None else None
    if IS_WINDOWS:
        process_query_limited_information = 0x1000
        kernel32 = ctypes.windll.kernel32
        kernel32.OpenProcess.argtypes = [ctypes.wintypes.DWORD, ctypes.wintypes.BOOL, ctypes.wintypes.DWORD]
        kernel32.OpenProcess.restype = ctypes.wintypes.HANDLE
        kernel32.GetProcessTimes.argtypes = [
            ctypes.wintypes.HANDLE,
            ctypes.POINTER(ctypes.wintypes.FILETIME), ctypes.POINTER(ctypes.wintypes.FILETIME),
            ctypes.POINTER(ctypes.wintypes.FILETIME), ctypes.POINTER(ctypes.wintypes.FILETIME),
        ]
        kernel32.GetProcessTimes.restype = ctypes.wintypes.BOOL
        kernel32.CloseHandle.argtypes = [ctypes.wintypes.HANDLE]
        handle = kernel32.OpenProcess(process_query_limited_information, False, pid)
        if not handle:
            return None
        try:
            creation = ctypes.wintypes.FILETIME()
            exit_time = ctypes.wintypes.FILETIME()
            kernel = ctypes.wintypes.FILETIME()
            user = ctypes.wintypes.FILETIME()
            if not kernel32.GetProcessTimes(
                handle, ctypes.byref(creation), ctypes.byref(exit_time), ctypes.byref(kernel), ctypes.byref(user),
            ):
                return None
            value = (creation.dwHighDateTime << 32) | creation.dwLowDateTime
            return f"windows:{value}"
        finally:
            kernel32.CloseHandle(handle)
    try:
        source = (Path("/proc") / str(pid) / "stat").read_text(encoding="utf-8")
        fields_after_name = source[source.rfind(")") + 2:].split()
        return f"proc:{fields_after_name[19]}"
    except (OSError, IndexError):
        return None


class RestoredProcess:
    """Minimal Popen-compatible handle for a process restored from local state."""

    def __init__(self, pid: int, identity: str) -> None:
        self.pid = pid
        self.identity = identity

    def poll(self) -> int | None:
        return None if process_identity(self.pid) == self.identity else 0

    def wait(self, timeout: float | None = None) -> int:
        deadline = None if timeout is None else time.monotonic() + timeout
        while self.poll() is None:
            if deadline is not None and time.monotonic() >= deadline:
                raise subprocess.TimeoutExpired(str(self.pid), timeout)
            time.sleep(0.05)
        return 0

    def kill(self) -> None:
        os.kill(self.pid, signal.SIGTERM if IS_WINDOWS else signal.SIGKILL)


def terminate_process_tree(process: subprocess.Popen[bytes] | RestoredProcess) -> str | None:
    """Stop one PID-reuse-checked process tree and return a user-facing failure."""
    if process.poll() is not None:
        return None
    failure = ""
    try:
        if IS_WINDOWS:
            completed = subprocess.run(
                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                capture_output=True,
                text=True,
                errors="replace",
                timeout=8,
                creationflags=subprocess.CREATE_NO_WINDOW,
                check=False,
            )
            if completed.returncode != 0 and process.poll() is None:
                output = (completed.stderr or completed.stdout).strip()
                failure = output[:800] or f"taskkill 退出码 {completed.returncode}"
        else:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=6)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
    except (OSError, subprocess.SubprocessError) as exc:
        failure = str(exc)
    if failure and process.poll() is None:
        try:
            process.kill()
        except OSError as exc:
            failure += f"；备用终止失败：{exc}"
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        return f"PID {process.pid} 在终止请求后仍未退出" + (f"；{failure}" if failure else "")
    except OSError as exc:
        return f"无法确认 PID {process.pid} 已退出：{exc}"
    return None

def npm_executable() -> str | None:
    candidates = ("npm.cmd", "npm") if IS_WINDOWS else ("npm",)
    return next((path for name in candidates if (path := shutil.which(name))), None)


def node_executable() -> str | None:
    candidates = ("node.exe", "node") if IS_WINDOWS else ("node",)
    return next((path for name in candidates if (path := shutil.which(name))), None)


def node_dependencies_ready(workspace: Path) -> bool:
    lock_file = workspace / "package-lock.json"
    marker = workspace / "node_modules" / NODE_DEPENDENCY_MARKER
    installed_lock = workspace / "node_modules" / ".package-lock.json"
    if not lock_file.is_file() or not installed_lock.is_file():
        return False
    try:
        return marker.read_text(encoding="utf-8").strip() == hashlib.sha256(lock_file.read_bytes()).hexdigest()
    except OSError:
        return False


def ensure_node_dependencies(workspace: Path, npm: str | None = None) -> bool:
    """Install locked Node.js dependencies when this worktree has not prepared them."""
    lock_file = workspace / "package-lock.json"
    if not lock_file.is_file():
        raise RuntimeError(f"未找到依赖锁文件：{lock_file}")
    if node_dependencies_ready(workspace):
        return False
    digest = hashlib.sha256(lock_file.read_bytes()).hexdigest()
    marker = workspace / "node_modules" / NODE_DEPENDENCY_MARKER

    executable = npm or npm_executable()
    if not executable:
        raise RuntimeError("PATH 中没有找到 npm，无法准备集成环境依赖。")
    creationflags = subprocess.CREATE_NO_WINDOW if IS_WINDOWS else 0
    completed = subprocess.run(
        [executable, "ci"], cwd=workspace, capture_output=True, text=True,
        encoding="utf-8", errors="replace", check=False, creationflags=creationflags,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or f"退出码 {completed.returncode}"
        raise RuntimeError(f"集成环境执行 npm ci 失败：\n{detail}")
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.write_text(f"{digest}\n", encoding="utf-8")
    return True


def notify_parent_manager_ready(root: tk.Tk) -> None:
    ready_file = os.environ.pop(MANAGER_READY_ENV, "").strip()
    if not ready_file:
        return
    try:
        root.update_idletasks()
        root.update()
        Path(ready_file).write_text("ready\n", encoding="utf-8")
    except (OSError, tk.TclError):
        pass


def pull_request_local_state(
    pull_request: local_pr_integration.PullRequest,
    selected: bool,
    applied: dict[int, str],
) -> str:
    applied_sha = applied.get(pull_request.number)
    if applied_sha and not selected:
        return "待移除"
    if applied_sha == pull_request.head_sha:
        return "已集成"
    if applied_sha:
        return "有更新"
    return "待加入" if selected else "未选择"


class PullRequestSelectionTable:
    """Shared compact PR picker used by the environment chooser and manager dialog."""

    def __init__(
        self, parent: tk.Misc, on_toggle: Callable[[int], None],
        *, height: int = 5, style: str = "PullRequest.Treeview",
    ) -> None:
        columns = ("selection", "number", "title", "branch", "github", "integration")
        tree = ttk.Treeview(
            parent, columns=columns, show="headings", selectmode="browse",
            height=height, style=style,
        )
        self.tree = tree
        self.on_toggle = on_toggle
        for column, label in (
            ("selection", "选择"), ("number", "PR"), ("title", "标题"),
            ("branch", "分支"), ("github", "GitHub"), ("integration", "本地集成"),
        ):
            tree.heading(column, text=label)
        tree.column("selection", width=48, minwidth=48, stretch=False, anchor="center")
        tree.column("number", width=52, minwidth=52, stretch=False, anchor="center")
        tree.column("title", width=220, minwidth=120)
        tree.column("branch", width=120, minwidth=90)
        tree.column("github", width=68, minwidth=64, stretch=False, anchor="center")
        tree.column("integration", width=76, minwidth=72, stretch=False, anchor="center")
        tree.bind("<Button-1>", self._clicked, add="+")
        tree.bind("<MouseWheel>", lambda event: tree.yview_scroll(int(-event.delta / 120), "units"), add="+")

    def pack(self, **kwargs) -> None:
        self.tree.pack(**kwargs)

    def render(
        self, pull_requests: list[local_pr_integration.PullRequest],
        selected: set[int], applied: dict[int, str],
    ) -> None:
        for item in self.tree.get_children():
            self.tree.delete(item)
        for pull_request in pull_requests:
            is_selected = pull_request.number in selected
            self.tree.insert(
                "", "end", iid=str(pull_request.number),
                values=(
                    "☑" if is_selected else "☐", f"#{pull_request.number}",
                    pull_request.title, pull_request.head_ref_name,
                    "Draft" if pull_request.is_draft else pull_request.merge_state_status,
                    pull_request_local_state(pull_request, is_selected, applied),
                ),
            )

    def _clicked(self, event: tk.Event) -> str | None:
        if self.tree.identify_region(event.x, event.y) != "cell" or self.tree.identify_column(event.x) != "#1":
            return None
        row = self.tree.identify_row(event.y)
        if row:
            self.on_toggle(int(row))
            return "break"
        return None


class ToolTip:
    """Small delayed tooltip for compact UI controls."""

    def __init__(self, widget: tk.Widget, text: str) -> None:
        self.widget = widget
        self.text = text
        self.pending: str | None = None
        self.window: tk.Toplevel | None = None
        widget.bind("<Enter>", self._schedule, add="+")
        widget.bind("<Leave>", self._hide, add="+")
        widget.bind("<ButtonPress>", self._hide, add="+")

    def _schedule(self, _event: tk.Event | None = None) -> None:
        self._cancel()
        if not self.text:
            return
        self.pending = self.widget.after(450, self._show)

    def set_text(self, text: str) -> None:
        if text == self.text:
            return
        self.text = text
        self._hide()
        try:
            pointer_x, pointer_y = self.widget.winfo_pointerxy()
            if self.widget.winfo_containing(pointer_x, pointer_y) is self.widget:
                self._schedule()
        except tk.TclError:
            pass

    def _cancel(self) -> None:
        if self.pending is not None:
            self.widget.after_cancel(self.pending)
            self.pending = None

    def _show(self) -> None:
        self.pending = None
        if self.window is not None or not self.widget.winfo_exists():
            return
        window = tk.Toplevel(self.widget)
        window.wm_overrideredirect(True)
        label = tk.Label(
            window, text=self.text, justify="left", padx=8, pady=5,
            bg="#20242c", fg="#ffffff", relief="solid", borderwidth=1,
            font=(UI_FONT_FAMILY, 9), wraplength=720,
        )
        label.pack()
        window.update_idletasks()
        x = self.widget.winfo_rootx() + self.widget.winfo_width() // 2 - window.winfo_reqwidth() // 2
        y = self.widget.winfo_rooty() + self.widget.winfo_height() + 6
        x = max(0, min(x, self.widget.winfo_screenwidth() - window.winfo_reqwidth()))
        y = max(0, min(y, self.widget.winfo_screenheight() - window.winfo_reqheight()))
        window.wm_geometry(f"+{x}+{y}")
        try:
            window.transient(self.widget.winfo_toplevel())
            window.wm_attributes("-topmost", True)
            window.lift()
        except tk.TclError:
            pass
        self.window = window

    def _hide(self, _event: tk.Event | None = None) -> None:
        self._cancel()
        if self.window is not None:
            self.window.destroy()
            self.window = None


def workspace_title(title: str, directory: Path = ROOT) -> str:
    """Build a compact title that still distinguishes parallel worktrees."""
    leaf = directory.resolve().name or str(directory.resolve())
    return f"{title} · {leaf}"


def centered_window_geometry(
    parent_x: int, parent_y: int, parent_width: int, parent_height: int,
    width: int, height: int, screen_x: int, screen_y: int,
    screen_width: int, screen_height: int, margin: int = 16,
) -> str:
    """Center a child window over its parent and keep it inside the visible screen."""
    width = min(width, max(1, screen_width - margin * 2))
    height = min(height, max(1, screen_height - margin * 2))
    x = parent_x + max(0, (parent_width - width) // 2)
    y = parent_y + max(0, (parent_height - height) // 2)
    minimum_x = screen_x + margin
    minimum_y = screen_y + margin
    maximum_x = max(minimum_x, screen_x + screen_width - width - margin)
    maximum_y = max(minimum_y, screen_y + screen_height - height - margin)
    x = max(minimum_x, min(x, maximum_x))
    y = max(minimum_y, min(y, maximum_y))
    return f"{width}x{height}+{x}+{y}"


def local_authorization_snapshot(
    directory: Path = ROOT, now: datetime | None = None,
) -> LocalAuthorizationSnapshot:
    """Read local authorization artifacts without contacting a running service."""
    license_path = directory / "data/control/license/license.json"
    runner_root = directory / "data/runner"
    runner_registered = (
        (runner_root / "identity.json").is_file()
        and (runner_root / "task-keys.json").is_file()
    )
    member_account_prepared = False
    free_certificate_prepared = False
    credentials_path = directory / "data/local/local-admin-credentials.env"
    try:
        credentials = parse_env_text(credentials_path.read_text(encoding="utf-8"))
        member_account_prepared = (
            credentials.get("ASTER_LOCAL_MEMBER_EMAIL") == "test@at.com"
            and bool(credentials.get("ASTER_LOCAL_MEMBER_PASSWORD"))
            and credentials.get("ASTER_LOCAL_MEMBER_QUOTA_READY") == "true"
        )
    except (OSError, UnicodeDecodeError):
        pass
    try:
        free_document = json.loads(
            (directory / "data/local/demo-delivery/free-license.json").read_text(encoding="utf-8"),
        )
        free_claims = free_document.get("claims", {}) if isinstance(free_document, dict) else {}
        free_certificate_prepared = (
            isinstance(free_document, dict)
            and isinstance(free_claims, dict)
            and isinstance(free_document.get("signature"), str)
            and bool(free_document["signature"])
            and free_claims.get("source", {}).get("kind") == "free_distribution"
            and free_claims.get("binding", {}).get("mode") == "unbound"
            and free_claims.get("validity", {}).get("expiry", {}).get("mode") == "none"
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, KeyError, TypeError):
        pass
    if not license_path.is_file():
        return LocalAuthorizationSnapshot(
            "missing", runner_registered=runner_registered,
            member_account_prepared=member_account_prepared,
            free_certificate_prepared=free_certificate_prepared,
        )
    try:
        document = json.loads(license_path.read_text(encoding="utf-8"))
        license_id = str(document.get("license_id", "")).strip()
        expires_at = str(document.get("expires_at", "")).strip()
        parsed_expiry = datetime.fromisoformat(
            expires_at[:-1] + "+00:00" if expires_at.endswith("Z") else expires_at,
        )
        if parsed_expiry.tzinfo is None:
            parsed_expiry = parsed_expiry.replace(tzinfo=timezone.utc)
        current_time = now or datetime.now(timezone.utc)
        if current_time.tzinfo is None:
            current_time = current_time.replace(tzinfo=timezone.utc)
        status = "expired" if parsed_expiry <= current_time else "active"
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, TypeError, ValueError):
        status, license_id, expires_at = "unreadable", "", ""
    return LocalAuthorizationSnapshot(
        status, license_id, expires_at, runner_registered, member_account_prepared,
        free_certificate_prepared,
    )


def local_authorization_prompt(snapshot: LocalAuthorizationSnapshot) -> tuple[str, str]:
    runner_status = "Runner 已注册" if snapshot.runner_registered else "Runner 待补齐"
    member_status = (
        "test@at.com 账号与测试额度已准备"
        if snapshot.member_account_prepared
        else "test@at.com 账号或测试额度待补齐"
    )
    free_status = (
        "本地测试免费证书已生成"
        if snapshot.free_certificate_prepared
        else "本地测试免费证书待补齐"
    )
    if snapshot.license_status == "active":
        expiry = snapshot.expires_at.split("T", 1)[0] if snapshot.expires_at else "未知日期"
        if snapshot.runner_registered and snapshot.member_account_prepared and snapshot.free_certificate_prepared:
            return (
                f"授权已经完成：许可证有效期至 {expiry}，{free_status}，{runner_status}，{member_status}。现有数据保持不变。",
                "",
            )
        return (
            f"许可证有效期至 {expiry}，{free_status}，{runner_status}，{member_status}。确认后仅补齐缺失项，现有数据保持不变。",
            "确认并补齐缺失项",
        )
    if snapshot.license_status == "expired":
        return (
            f"许可证已过期（{snapshot.expires_at.split('T', 1)[0] if snapshot.expires_at else '日期未知'}）。"
            f"此流程不用于续期；确认后只检查现有授权状态。",
            "确认检查",
        )
    if snapshot.license_status == "unreadable":
        return (
            f"许可证文件无法读取；{runner_status}，{member_status}。确认后将重新校验并补齐缺失项。",
            "确认并重新校验",
        )
    return (
        f"尚未授权；{free_status}，{runner_status}，{member_status}。确认后将完成免费证书、许可证、Runner 和用户侧账号准备。",
        "确认并开始授权",
    )


def missing_files(service: ServiceSpec) -> list[str]:
    return [path for path in service.required_files if not (ROOT / path).is_file()]


def probe_result(url: str | None, expected_service: str | None = None, timeout: float = 0.65) -> ProbeResult:
    if not url:
        return ProbeResult(False, "服务没有配置健康检查地址")
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "aster-team-local-dev-manager/1"})
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read(16_384)
            if not 200 <= response.status < 500:
                return ProbeResult(False, f"健康检查返回 HTTP {response.status}")
            if expected_service:
                try:
                    payload = json.loads(body.decode("utf-8"))
                    actual_service = payload.get("service") if isinstance(payload, dict) else None
                except (UnicodeDecodeError, json.JSONDecodeError):
                    actual_service = None
                if actual_service != expected_service:
                    actual = actual_service or "未提供 service 标识"
                    return ProbeResult(
                        False,
                        f"健康检查地址由其他服务响应：期望 {expected_service}，实际 {actual}",
                    )
            return ProbeResult(True)
    except urllib.error.HTTPError as exc:
        return ProbeResult(False, f"健康检查返回 HTTP {exc.code}")
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        return ProbeResult(False, f"健康检查不可达：{exc}")


def probe(url: str | None, timeout: float = 0.65, expected_service: str | None = None) -> bool:
    return probe_result(url, expected_service, timeout).healthy


def service_port(service: ServiceSpec) -> int | None:
    url = service_health_url(service)
    if not url:
        return None
    try:
        return urlparse(url).port
    except ValueError:
        return None


def operations_service_origin() -> str:
    """Use the same environment-over-file precedence as the Go API startup."""
    values: dict[str, str] = {}
    env_path = ROOT / "data/local/operations.env"
    if env_path.is_file():
        for line in env_path.read_text(encoding="utf-8").splitlines():
            # Go's loader accepts export and keeps the first value for a key.
            for key, value in parse_env_text(line.strip().removeprefix("export ")).items():
                values.setdefault(key, value)
    configured = os.environ.get("ASTER_OPERATIONS_ADDR")
    if configured is not None:
        address = configured.strip() or f"127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_api']}"
    elif (
        IS_INTEGRATION_RUNTIME
        or ACTIVE_SERVICE_PORTS["operations_api"] != PRIMARY_SERVICE_PORTS["operations_api"]
    ):
        address = f"127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_api']}"
    else:
        address = values.get("ASTER_OPERATIONS_ADDR", "").strip() or "127.0.0.1:12090"
    try:
        if any(character.isspace() for character in address) or "\\" in address:
            raise ValueError
        parsed = urlparse("http://" + address)
        port = parsed.port
        if port is None or not 1 <= port <= 65535 or parsed.username or parsed.password or parsed.path or parsed.query or parsed.fragment:
            raise ValueError
    except ValueError:
        raise ValueError("ASTER_OPERATIONS_ADDR 必须是有效的 host:port，不能回退到默认端口。") from None
    host = parsed.hostname or "127.0.0.1"
    if host == "0.0.0.0":
        host = "127.0.0.1"
    elif host == "::":
        host = "::1"
    return f"http://{'[' + host + ']' if ':' in host else host}:{port}"


def service_health_url(service: ServiceSpec) -> str | None:
    if service.key == "operations_api":
        try:
            return operations_service_origin() + "/health"
        except (OSError, ValueError):
            return None
    return service.health_url


def valid_lan_ipv4(value: str | None) -> str | None:
    try:
        address = ipaddress.ip_address((value or "").strip())
    except ValueError:
        return None
    if not isinstance(address, ipaddress.IPv4Address) or address.is_loopback or address.is_link_local or address.is_unspecified:
        return None
    return str(address)


def detect_lan_ipv4() -> str | None:
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as connection:
            connection.connect(("192.0.2.1", 9))
            if address := valid_lan_ipv4(connection.getsockname()[0]):
                return address
    except OSError:
        pass
    try:
        candidates = socket.gethostbyname_ex(socket.gethostname())[2]
    except OSError:
        candidates = []
    return next((address for value in candidates if (address := valid_lan_ipv4(value))), None)


def replace_url_host(url: str | None, host: str | None) -> str | None:
    if not url or not host:
        return url
    parsed = urlparse(url)
    port = f":{parsed.port}" if parsed.port is not None else ""
    return parsed._replace(netloc=f"{host}{port}").geturl()


def service_address_url(service: ServiceSpec, advertised_host: str | None = None) -> str | None:
    if service.key == "operations_api":
        try:
            url = operations_service_origin() + "/api/operations/v1"
        except (OSError, ValueError):
            return None
    else:
        url = service.address_url
    host = advertised_host if service.key in LAN_ADDRESS_SERVICE_KEYS else None
    return replace_url_host(url, host)


def local_service_environment(
    enabled: bool,
    advertised_host: str | None,
    base: dict[str, str] | None = None,
) -> dict[str, str]:
    environment = development_tool_environment(ROOT, os.environ if base is None else base)
    operations_origin = operations_service_origin()
    environment.update({
        "ASTER_OPERATIONS_ADDR": operations_origin.removeprefix("http://"),
        "ASTER_OPERATIONS_CONSOLE_PORT": str(ACTIVE_SERVICE_PORTS["operations_console"]),
        "ASTER_CONTROL_PORT": str(ACTIVE_SERVICE_PORTS["customer_control"]),
        "ASTER_CUSTOMER_ADMIN_PORT": str(ACTIVE_SERVICE_PORTS["customer_admin"]),
        "ASTER_CUSTOMER_MEMBER_PORT": str(ACTIVE_SERVICE_PORTS["customer_member"]),
        "ASTER_RUNNER_CONTROL_WSS": (
            f"ws://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_control']}/api/runner/channel"
        ),
        "ASTER_WEBSITE_FRONTEND_PORT": str(ACTIVE_SERVICE_PORTS["website"]),
        "ASTER_WEBSITE_BACKEND_PORT": str(ACTIVE_SERVICE_PORTS["website_backend"]),
    })
    configured_origins = read_env_file(ROOT / "data/local/operations.env").get(
        "ASTER_OPERATIONS_TRUSTED_ORIGINS",
        (
            f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_console']},"
            f"http://localhost:{ACTIVE_SERVICE_PORTS['operations_console']}"
        ),
    )
    origins = [value.strip().rstrip("/") for value in configured_origins.split(",") if value.strip()]
    if IS_INTEGRATION_RUNTIME:
        primary_origins = {
            f"http://127.0.0.1:{PRIMARY_SERVICE_PORTS['operations_console']}",
            f"http://localhost:{PRIMARY_SERVICE_PORTS['operations_console']}",
        }
        origins = [value for value in origins if value not in primary_origins]
        for host in ("127.0.0.1", "localhost"):
            origin = f"http://{host}:{ACTIVE_SERVICE_PORTS['operations_console']}"
            if origin not in origins:
                origins.append(origin)
    environment["ASTER_OPERATIONS_TRUSTED_ORIGINS"] = ",".join(origins)
    environment[LAN_ENABLED_ENV] = "true" if enabled else "false"
    if not enabled:
        environment.pop(LAN_HOST_ENV, None)
        return environment
    host = valid_lan_ipv4(advertised_host)
    if host is None:
        raise ValueError("无法确定有效的局域网 IPv4 地址。")
    lan_origin = f"http://{host}:{ACTIVE_SERVICE_PORTS['operations_console']}"
    if lan_origin not in origins:
        origins.append(lan_origin)
    environment.update({
        LAN_HOST_ENV: host,
        "ASTER_OPERATIONS_TRUSTED_ORIGINS": ",".join(origins),
    })
    return environment


def saved_lan_access_enabled(path: Path = PROCESS_STATE_FILE) -> bool:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return False
    return isinstance(payload, dict) and payload.get("lan_access_enabled") is True


def service_configuration_error(service: ServiceSpec) -> str | None:
    if service.key == "operations_api":
        try:
            operations_service_origin()
        except (OSError, ValueError) as exc:
            return f"Operations API 端口配置无效：{exc}"
    if not service.port_env:
        return None
    relative_path, key = service.port_env
    configured = read_env_file(ROOT / relative_path).get(key, "").strip()
    expected = service_port(service)
    if IS_INTEGRATION_RUNTIME and configured == str(PRIMARY_SERVICE_PORTS["customer_control"]):
        configured = str(ACTIVE_SERVICE_PORTS["customer_control"])
    if configured and expected is not None and configured != str(expected):
        return (
            f"端口配置不一致：{relative_path} 中 {key}={configured}，"
            f"但 {service.name} 应监听 {expected}。请修正配置后重试。"
        )
    return None


def managed_services_in_workspace(workspace: Path) -> list[str]:
    state_path = workspace / "data/local/dev-manager-processes.json"
    try:
        payload = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return []
    records = payload.get("services", {}) if isinstance(payload, dict) else {}
    if not isinstance(records, dict):
        return []
    running: list[str] = []
    for key, record in records.items():
        if not isinstance(record, dict):
            continue
        pid = record.get("pid")
        identity = record.get("identity")
        if isinstance(pid, int) and isinstance(identity, str) and process_identity(pid) == identity:
            running.append(SERVICE_BY_KEY[key].name if key in SERVICE_BY_KEY else key)
    return sorted(running)


def environment_roots(source: Path = ROOT) -> tuple[Path, Path]:
    """Return the fixed primary and PR-integration worktrees for this repository."""
    primary = local_pr_integration.primary_worktree(source)
    return primary, local_pr_integration.default_integration_path(source)


def shared_development_tools_root(
    source: Path = ROOT,
    environment: Mapping[str, str] | None = None,
) -> str | None:
    """Resolve the machine-wide developer tools root shared by every worktree."""
    values = os.environ if environment is None else environment
    explicit = values.get("ASTER_TOOLS_ROOT", "").strip()
    if explicit:
        return explicit
    if IS_WINDOWS:
        try:
            primary, _integration = environment_roots(source)
            payload = json.loads((primary / ".aster-tools/windows-setup.json").read_text(encoding="utf-8"))
            install_root = payload.get("installRoot", "") if isinstance(payload, dict) else ""
        except (OSError, json.JSONDecodeError, local_pr_integration.IntegrationError):
            return None
        return install_root.strip() if isinstance(install_root, str) else None
    home = Path.home()
    if sys.platform == "darwin":
        return str(home / "Library/Application Support/AsterDev")
    if sys.platform.startswith("linux"):
        data_home = values.get("XDG_DATA_HOME", "").strip()
        return str((Path(data_home) if data_home else home / ".local/share") / "aster-dev")
    return None


def development_tool_environment(
    workspace: Path,
    base: Mapping[str, str] | None = None,
) -> dict[str, str]:
    """Pass the shared tools root across worktrees, bridging older Go resolvers when needed."""
    environment = dict(os.environ if base is None else base)
    tools_root = shared_development_tools_root(workspace, environment)
    if not tools_root:
        return environment
    environment.setdefault("ASTER_TOOLS_ROOT", tools_root)
    resolver = workspace / "tools/toolchains/go-toolchain.mjs"
    try:
        supports_shared_root = "ASTER_TOOLS_ROOT" in resolver.read_text(encoding="utf-8")
    except OSError:
        supports_shared_root = False
    if not supports_shared_root and not environment.get("ASTER_GO_ROOT", "").strip():
        environment["ASTER_GO_ROOT"] = str(PureWindowsPath(tools_root) / "Go") if IS_WINDOWS else str(Path(tools_root) / "Go")
    return environment


def pr_integration_settings_file(source: Path = ROOT) -> Path:
    primary, _target = environment_roots(source)
    return primary / "data/local/pr-integration-settings.json"


def read_pr_integration_settings(source: Path = ROOT) -> dict[str, object]:
    path = pr_integration_settings_file(source)
    legacy_path = source.resolve() / "data/local/pr-integration-settings.json"
    candidates = (path, legacy_path) if legacy_path != path else (path,)
    for candidate in candidates:
        try:
            payload = json.loads(candidate.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(payload, dict):
            return payload
    return {}


def write_pr_integration_settings(
    selected: set[int], seen: set[int], drafts: set[int], source: Path = ROOT,
) -> None:
    path = pr_integration_settings_file(source)
    payload = {
        "version": 1,
        "selected": sorted(selected),
        "seen": sorted(seen),
        "drafts": sorted(drafts),
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def local_database_defaults(source: Path = ROOT) -> dict[str, str]:
    suffix = "_integration" if is_integration_environment(source) else ""
    return {
        "ASTER_OPERATIONS_DB_NAME": f"aster_operations{suffix}",
        "ASTER_OPERATIONS_DB_SERVICE_USER": f"aster_operations{suffix}",
        "ASTER_CUSTOMER_DB_NAME": f"aster_customer{suffix}",
        "ASTER_CUSTOMER_DB_SERVICE_USER": f"aster_customer{suffix}",
    }


def selected_pull_request_numbers(settings: dict[str, object], key: str) -> set[int]:
    values = settings.get(key, [])
    if not isinstance(values, list):
        return set()
    return {
        int(value) for value in values
        if isinstance(value, int) or (isinstance(value, str) and value.isdigit())
    }


def prepare_integration_environment(source: Path = ROOT) -> local_pr_integration.IntegrationResult:
    settings = read_pr_integration_settings(source)
    pull_requests = local_pr_integration.list_open_pull_requests(source)
    selected, seen, drafts = local_pr_integration.reconcile_selection(
        pull_requests,
        selected_pull_request_numbers(settings, "selected"),
        selected_pull_request_numbers(settings, "seen"),
        selected_pull_request_numbers(settings, "drafts"),
    )
    write_pr_integration_settings(selected, seen, drafts, source)
    chosen = [item for item in pull_requests if item.number in selected]
    _primary, target = environment_roots(source)
    return local_pr_integration.IntegrationWorkspace(source, target).apply(chosen)


def listening_pids(port: int) -> list[int]:
    if port <= 0:
        return []
    if IS_WINDOWS:
        try:
            completed = subprocess.run(
                ["netstat", "-ano", "-p", "tcp"], capture_output=True, text=True,
                encoding="utf-8", errors="replace", timeout=4, check=False,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
        except (OSError, subprocess.SubprocessError):
            return []
        result: set[int] = set()
        for line in completed.stdout.splitlines():
            fields = line.split()
            if len(fields) < 5 or fields[0].upper() != "TCP" or not fields[-1].isdigit():
                continue
            if re.search(rf":{port}$", fields[1]) and fields[2].endswith(":0"):
                result.add(int(fields[-1]))
        return sorted(result)
    try:
        completed = subprocess.run(
            ["lsof", "-nP", f"-iTCP:{port}", "-sTCP:LISTEN", "-t"],
            capture_output=True, text=True, timeout=4, check=False,
        )
        return sorted({int(line) for line in completed.stdout.splitlines() if line.strip().isdigit()})
    except (OSError, subprocess.SubprocessError):
        return []


def process_metadata(pid: int) -> tuple[str, str]:
    if IS_WINDOWS:
        powershell = shutil.which("powershell.exe") or shutil.which("pwsh.exe")
        if powershell:
            command = (
                f"$p=Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}'; "
                "if ($p) { [pscustomobject]@{Name=$p.Name;CommandLine=$p.CommandLine} | ConvertTo-Json -Compress }"
            )
            try:
                completed = subprocess.run(
                    [powershell, "-NoProfile", "-NonInteractive", "-Command", command],
                    capture_output=True, text=True, encoding="utf-8", errors="replace",
                    timeout=5, check=False, creationflags=subprocess.CREATE_NO_WINDOW,
                )
                payload = json.loads(completed.stdout.strip() or "{}")
                name = str(payload.get("Name") or "未知进程")
                command_line = str(payload.get("CommandLine") or "").strip()
                return name, command_line
            except (OSError, subprocess.SubprocessError, json.JSONDecodeError):
                pass
    elif sys.platform == "darwin":
        info = local_process_identity.darwin_process_info(pid)
        ps = shutil.which("ps") or "/bin/ps"
        try:
            completed = subprocess.run(
                [ps, "-ww", "-p", str(pid), "-o", "command="],
                capture_output=True, text=True, encoding="utf-8", errors="replace",
                timeout=4, check=False,
            )
            command_line = completed.stdout.strip() if completed.returncode == 0 else ""
            if info is not None and command_line:
                return info[0] or Path(command_line.split(None, 1)[0]).name, command_line
        except (OSError, subprocess.SubprocessError):
            pass
    else:
        try:
            arguments = [part.decode("utf-8", "replace") for part in (Path("/proc") / str(pid) / "cmdline").read_bytes().split(b"\0") if part]
            if arguments:
                return Path(arguments[0]).name, shlex.join(arguments)
        except OSError:
            pass
    return "", ""


def describe_process(pid: int) -> str:
    name, command_line = process_metadata(pid)
    return f"PID {pid}" + (f"，{name}" if name else "") + (f"；命令：{command_line}" if command_line else "")


def external_process_diagnostics(service: ServiceSpec) -> tuple[str, str]:
    """Describe the listener that answered an unmanaged service health probe."""
    health_url = service_health_url(service) or "未配置"
    port = service_port(service)
    if port is None:
        return "外部运行", f"健康地址：{health_url}\n监听端口：无法解析\n未找到可显示的端口占用信息。"
    owners = listening_pids(port)
    summary = f"外部运行 · {port}"
    lines = [f"健康地址：{health_url}", f"监听端口：{port}"]
    if not owners:
        lines.extend((
            "占用进程：未查询到监听 PID（进程可能刚刚退出，或系统拒绝查询）",
            "判定说明：健康检查刚才收到了 HTTP 响应，但控制台无法确认该服务属于当前工作树。",
        ))
        return summary, "\n".join(lines)
    if len(owners) == 1:
        summary += f" · PID {owners[0]}"
    else:
        summary += f" · {len(owners)} 个 PID"
    for pid in owners:
        name, command_line = process_metadata(pid)
        cwd = local_process_identity.process_working_directory(pid)
        lines.extend((
            "",
            f"占用进程：PID {pid}" + (f"，{name}" if name else "，进程名无法读取"),
            f"命令行：{command_line or '无法读取（可能是权限级别高于当前控制台）'}",
            f"工作目录：{cwd if cwd is not None else '无法读取（可能是权限级别高于当前控制台）'}",
        ))
    lines.extend((
        "",
        "判定说明：健康检查收到了 HTTP 响应，但进程身份、命令或工作目录未能全部确认属于当前 Aster 工作树，因此未自动接管或停止。",
    ))
    return summary, "\n".join(lines)


def service_process_source_matches(service: ServiceSpec, name: str, command_line: str) -> bool:
    source = f"{name} {command_line}".lower().replace("\\", "/")
    port = service_port(service)
    if service.key == "operations_api":
        return name.lower() in {"api", "api.exe", "operations-api", "operations-api.exe"}
    if service.key == "customer_control":
        return "aster-control" in source and " serve" in source
    if service.key in {"operations_console", "customer_admin", "customer_member", "website"}:
        return "vite" in source and port is not None and str(port) in source
    if service.key == "runner":
        return "aster-runner" in source and " serve" in source
    return False


def service_process_matches(service: ServiceSpec, pid: int) -> bool:
    """Only allow takeover when the process is recognizably this Aster service."""
    name, command = process_metadata(pid)
    if not service_process_source_matches(service, name, command):
        return False
    cwd = local_process_identity.process_working_directory(pid)
    roots = {ROOT.resolve()}
    frontend = {
        "operations_console": "operations/console", "customer_admin": "customer/admin",
        "customer_member": "customer/member", "website": "website",
    }.get(service.key)
    if frontend:
        roots.add((ROOT / frontend).resolve())
    if cwd is None or cwd not in roots:
        return False
    if service.key != "operations_api":
        return True
    arguments = local_process_identity.command_arguments(command)
    if not arguments:
        return False
    env_files: list[str] = []
    index = 1
    while index < len(arguments):
        argument = arguments[index]
        # Go's flag parser stops at -- or the first positional argument.
        if argument == "--" or not argument.startswith("-"):
            break
        if argument in {"--env-file", "-env-file"}:
            if index + 1 >= len(arguments) or arguments[index + 1].startswith("-"):
                return False
            env_files.append(arguments[index + 1])
            index += 1
        elif argument.startswith(("--env-file=", "-env-file=")):
            env_files.append(argument.split("=", 1)[1])
        elif argument.split("=", 1)[0] not in {"--create-database", "-create-database", "--migrate-only", "-migrate-only"}:
            return False
        index += 1
    if len(env_files) != 1 or not env_files[0]:
        return False
    try:
        if (cwd / env_files[0]).resolve() != (ROOT / "data/local/operations.env").resolve():
            return False
    except (OSError, ValueError):
        return False
    executable = Path(arguments[0])
    return executable.is_absolute() and local_process_identity.is_operations_binary(executable)


def all_process_metadata() -> list[tuple[int, str, str]]:
    if IS_WINDOWS:
        powershell = shutil.which("powershell.exe") or shutil.which("pwsh.exe")
        if not powershell:
            return []
        command = "Get-CimInstance Win32_Process | Select-Object ProcessId,Name,CommandLine | ConvertTo-Json -Compress"
        try:
            completed = subprocess.run(
                [powershell, "-NoProfile", "-NonInteractive", "-Command", command], capture_output=True, text=True,
                encoding="utf-8", errors="replace", timeout=8, check=False, creationflags=subprocess.CREATE_NO_WINDOW,
            )
            payload = json.loads(completed.stdout.strip() or "[]")
            records = payload if isinstance(payload, list) else [payload]
            return [
                (int(record["ProcessId"]), str(record.get("Name") or ""), str(record.get("CommandLine") or ""))
                for record in records if isinstance(record, dict) and isinstance(record.get("ProcessId"), int)
            ]
        except (OSError, subprocess.SubprocessError, json.JSONDecodeError, KeyError, ValueError):
            return []
    if sys.platform == "darwin":
        ps = shutil.which("ps") or "/bin/ps"
        try:
            completed = subprocess.run(
                [ps, "-ww", "-axo", "pid=,comm=,command="],
                capture_output=True, text=True, encoding="utf-8", errors="replace",
                timeout=8, check=False,
            )
        except (OSError, subprocess.SubprocessError):
            return []
        if completed.returncode != 0:
            return []
        result = []
        for line in completed.stdout.splitlines():
            fields = line.strip().split(None, 2)
            if len(fields) < 2 or not fields[0].isdigit():
                continue
            name = Path(fields[1]).name
            command_line = fields[2] if len(fields) == 3 else fields[1]
            result.append((int(fields[0]), name, command_line))
        return result
    result = []
    try:
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            arguments = [part.decode("utf-8", "replace") for part in (entry / "cmdline").read_bytes().split(b"\0") if part]
            if arguments:
                result.append((int(entry.name), Path(arguments[0]).name, shlex.join(arguments)))
    except OSError:
        return []
    return result


def controllable_external_process(service: ServiceSpec) -> RestoredProcess | None:
    """Return a PID-reuse-safe handle for a verified external Aster listener."""
    if service_configuration_error(service):
        return None
    port = service_port(service)
    if port is not None:
        owners = listening_pids(port)
        if len(owners) != 1:
            return None
        candidates = owners
    else:
        matches = [
            (pid, name) for pid, name, command_line in all_process_metadata()
            if service_process_source_matches(service, name, command_line)
        ]
        executable_matches = [pid for pid, name in matches if name.lower() in {"runner", "runner.exe"}]
        candidates = executable_matches or [pid for pid, _name in matches]
        if len(candidates) != 1:
            return None
    pid = candidates[0]
    if pid <= 0:
        return None
    identity = process_identity(pid)
    if not identity or not service_process_matches(service, pid) or process_identity(pid) != identity:
        return None
    if port is not None and listening_pids(port) != [pid]:
        return None
    return RestoredProcess(pid, identity)


def local_process_state_paths(workspace: Path = ROOT) -> list[Path]:
    """Return only the state file belonging to the current runtime configuration."""
    return [workspace.resolve() / "data/local/dev-manager-processes.json"]


def identity_checked_state_processes(
    workspace: Path = ROOT,
) -> dict[str, list[RestoredProcess]]:
    """Read live, PID-reuse-checked handles from this workspace's manager state."""
    result: dict[str, list[RestoredProcess]] = {}
    seen: set[tuple[str, int]] = set()
    for state_path in local_process_state_paths(workspace):
        try:
            payload = json.loads(state_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        records = payload.get("services", {}) if isinstance(payload, dict) else {}
        if not isinstance(records, dict):
            continue
        for key, record in records.items():
            if key not in SERVICE_BY_KEY or not isinstance(record, dict):
                continue
            pid = record.get("pid")
            identity = record.get("identity")
            marker = (key, pid) if isinstance(pid, int) else None
            if (
                marker is None or marker in seen or not isinstance(identity, str)
                or process_identity(pid) != identity
            ):
                continue
            seen.add(marker)
            result.setdefault(key, []).append(RestoredProcess(pid, identity))
    return result


def clean_local_process_states(workspace: Path = ROOT) -> None:
    """Remove stale records while retaining any identity-checked processes that remain alive."""
    for state_path in local_process_state_paths(workspace):
        if not state_path.is_file():
            continue
        try:
            payload = json.loads(state_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        records = payload.get("services", {}) if isinstance(payload, dict) else {}
        if not isinstance(records, dict):
            records = {}
        remaining = {}
        for key, record in records.items():
            if key not in SERVICE_BY_KEY or not isinstance(record, dict):
                continue
            pid = record.get("pid")
            identity = record.get("identity")
            if isinstance(pid, int) and isinstance(identity, str) and process_identity(pid) == identity:
                remaining[key] = {"pid": pid, "identity": identity}
        temporary = state_path.with_suffix(".tmp")
        try:
            temporary.write_text(
                json.dumps({"version": 1, "services": remaining}, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
            os.replace(temporary, state_path)
        except OSError:
            try:
                temporary.unlink(missing_ok=True)
            except OSError:
                pass


def service_listener_ports(service: ServiceSpec) -> tuple[int, ...]:
    ports = []
    if (primary := service_port(service)) is not None:
        ports.append(primary)
    if service.key == "website":
        backend = ACTIVE_SERVICE_PORTS["website_backend"]
        if 1 <= backend <= 65535 and backend not in ports:
            ports.append(backend)
    return tuple(ports)


def stop_service_process(
    service: ServiceSpec,
    process: subprocess.Popen[bytes] | RestoredProcess,
) -> str | None:
    """Terminate a service tree and verify both its PID and listener ports are gone."""
    termination_failure = terminate_process_tree(process)
    deadline = time.monotonic() + 10
    occupied: dict[int, list[int]] = {}
    while True:
        occupied = {
            port: owners for port in service_listener_ports(service)
            if (owners := listening_pids(port))
        }
        if process.poll() is not None and not occupied:
            return None
        if time.monotonic() >= deadline:
            break
        time.sleep(0.1)
    detail = []
    if termination_failure:
        detail.append(termination_failure)
    if process.poll() is None:
        detail.append(f"PID {process.pid} 仍在运行")
    for port, owners in occupied.items():
        detail.append(
            f"端口 {port} 仍被占用：" + "；".join(describe_process(pid) for pid in owners)
        )
    return "\n".join(detail) or f"{service.name} 未能完全停止"


def stop_local_services(workspace: Path = ROOT) -> tuple[list[str], list[str]]:
    """Stop only identity-checked Aster services, including services from another worktree."""
    state_processes = identity_checked_state_processes(workspace)
    stopped: list[str] = []
    failed: list[str] = []
    for service in SERVICES:
        processes: list[subprocess.Popen[bytes] | RestoredProcess] = list(state_processes.get(service.key, []))
        if not processes and (external := controllable_external_process(service)) is not None:
            processes.append(external)
        if not processes:
            continue
        service_failed = False
        for process in processes:
            if stop_service_process(service, process):
                service_failed = True
        if service_failed:
            failed.append(service.name)
        else:
            stopped.append(service.name)
    clean_local_process_states(workspace)
    return stopped, failed


def port_conflict_detail(service: ServiceSpec, health_detail: str = "") -> str | None:
    port = service_port(service)
    if port is None:
        return None
    owners = listening_pids(port)
    if not owners:
        return None
    detail = f"端口 {port} 已被占用：" + "；".join(describe_process(pid) for pid in owners)
    if health_detail:
        detail += f"\n{health_detail}"
    return detail


def initialization_state() -> str:
    present = sum(path.is_file() for path in INITIALIZATION_FILES)
    if present == len(INITIALIZATION_FILES):
        return "complete"
    return "partial" if present else "missing"


def read_env_file(path: Path) -> dict[str, str]:
    if not path.is_file():
        return {}
    return parse_env_text(path.read_text(encoding="utf-8"))


def parse_env_text(source: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for source_line in source.splitlines():
        line = source_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
            value = value[1:-1]
        result[key.strip()] = value
    return result


def local_browser_url(url: str) -> str:
    parsed = urlparse(url)
    login_ports = {
        ACTIVE_SERVICE_PORTS["customer_member"],
        ACTIVE_SERVICE_PORTS["customer_admin"],
        ACTIVE_SERVICE_PORTS["operations_console"],
    }
    if parsed.scheme != "http" or parsed.hostname not in {"127.0.0.1", "localhost", "::1"} or parsed.port not in login_ports:
        return url
    destination = parsed.path or "/"
    if parsed.query:
        destination = f"{destination}?{urlencode(parse_qsl(parsed.query, keep_blank_values=True))}"
    query = {"local_login": "1"}
    if destination not in {"/", "/login"}:
        query["redirect"] = destination
    return parsed._replace(path="/login", query=urlencode(query), fragment="").geturl()


def _windows_app_path(executable_name: str) -> str | None:
    if not IS_WINDOWS or winreg is None:
        return None
    subkey = rf"Software\Microsoft\Windows\CurrentVersion\App Paths\{executable_name}"
    views = (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY)
    for root in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
        for view in views:
            try:
                with winreg.OpenKey(root, subkey, 0, winreg.KEY_READ | view) as key:
                    value = winreg.QueryValue(key, None)
            except OSError:
                continue
            candidate = Path(os.path.expandvars(value.strip().strip('"')))
            if candidate.is_file():
                return str(candidate)
    return None


def chrome_executable() -> str | None:
    command_names = ("chrome.exe", "chrome") if IS_WINDOWS else (
        "google-chrome", "google-chrome-stable", "chromium", "chromium-browser",
    )
    command = next((path for name in command_names if (path := shutil.which(name))), None)
    if command:
        return command
    if not IS_WINDOWS:
        return None
    registered = _windows_app_path("chrome.exe")
    if registered:
        return registered
    install_roots = (
        os.environ.get("LOCALAPPDATA"),
        os.environ.get("ProgramFiles"),
        os.environ.get("ProgramFiles(x86)"),
    )
    for install_root in install_roots:
        if not install_root:
            continue
        candidate = Path(install_root) / "Google" / "Chrome" / "Application" / "chrome.exe"
        if candidate.is_file():
            return str(candidate)
    return None


def open_browser_target(url: str, browser_option: str) -> bool:
    if browser_option == BROWSER_CHROME:
        executable = chrome_executable()
        if executable:
            return webbrowser.BackgroundBrowser(executable).open(url, new=2)
        return webbrowser.get("chrome").open(url, new=2)
    if browser_option == BROWSER_SYSTEM_DEFAULT:
        return webbrowser.open(url, new=2)
    raise ValueError(f"不支持的浏览器打开方式：{browser_option}")


def update_env_file(path: Path, values: dict[str, str]) -> None:
    def encode(value: str) -> str:
        if re.fullmatch(r"[A-Za-z0-9_./:@%+!$^&*()=-]*", value):
            return value
        if "'" not in value:
            return f"'{value}'"
        if '"' not in value:
            return f'"{value}"'
        raise ValueError("配置值不能同时包含单引号和双引号")

    lines = path.read_text(encoding="utf-8").splitlines() if path.is_file() else []
    updated: list[str] = []
    remaining = dict(values)
    pattern = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)=")
    for line in lines:
        match = pattern.match(line.strip())
        if match and match.group(1) in remaining:
            key = match.group(1)
            updated.append(f"{key}={encode(remaining.pop(key))}")
        else:
            updated.append(line)
    if remaining:
        if updated and updated[-1] != "":
            updated.append("")
        updated.extend(f"{key}={encode(value)}" for key, value in remaining.items())
    temporary = path.with_name(f"{path.name}.tmp")
    temporary.write_text("\n".join(updated).rstrip() + "\n", encoding="utf-8")
    os.chmod(temporary, 0o600)
    os.replace(temporary, path)


def save_local_account_passwords(
    path: Path, customer_password: str, operations_password: str,
    member_password: str | None = None, member_email: str | None = None,
) -> None:
    """Update local account records; this never changes application accounts."""
    entries = [("Customer", customer_password), ("Operations", operations_password)]
    if member_password is not None:
        entries.append(("用户侧", member_password))
    for label, value in entries:
        if not value:
            raise ValueError(f"{label} 本地记录密码不能为空")
        if "\n" in value or "\r" in value:
            raise ValueError(f"{label} 本地记录密码不能包含换行符")
    updates = {
        "ASTER_LOCAL_CUSTOMER_PASSWORD": customer_password,
        "ASTER_LOCAL_OPERATIONS_PASSWORD": operations_password,
    }
    if member_password is not None:
        updates["ASTER_LOCAL_MEMBER_PASSWORD"] = member_password
    if member_email is not None:
        normalized_email = member_email.strip()
        if member_password is None or not re.fullmatch(r"[^\s@]+@[^\s@]+\.[^\s@]+", normalized_email):
            raise ValueError("用户侧本地记录需要有效邮箱和密码")
        updates["ASTER_LOCAL_MEMBER_EMAIL"] = normalized_email
    update_env_file(path, updates)


def save_local_admin_passwords(path: Path, customer_password: str, operations_password: str) -> None:
    """Backward-compatible wrapper for callers that only edit administrator records."""
    save_local_account_passwords(path, customer_password, operations_password)


def quick_password_validation_error(new_password: str) -> str | None:
    if "\n" in new_password or "\r" in new_password:
        return "新密码不能包含换行符。"
    byte_length = len(new_password.encode("utf-8"))
    if not 12 <= byte_length <= 72:
        return "新密码必须为 12—72 字节。"
    return None


def authorization_requires_password_change(error: str | None) -> bool:
    return bool(error and ("尚未完成首次改密" in error or "仍在使用初始化密码" in error))


def password_change_required_accounts(error: str | None) -> set[str]:
    if not error:
        return set()
    return {
        label for label in ("Operations", "Customer")
        if any(
            label in line and ("尚未完成首次改密" in line or "仍在使用初始化密码" in line)
            for line in error.splitlines()
        )
    }


def _http_error_message(exc: urllib.error.HTTPError) -> str:
    try:
        payload = json.loads(exc.read().decode("utf-8"))
        detail = payload.get("error", {}).get("message", "") if isinstance(payload, dict) else ""
    except (UnicodeDecodeError, json.JSONDecodeError):
        detail = ""
    return str(detail).strip() or f"HTTP {exc.code}"


def change_operations_password(
    email: str, current_password: str, new_password: str, timeout: float = 4.0,
    base_url: str | None = None,
) -> str | None:
    """Change the local Operations password through the same authenticated API as the console."""
    if base_url is None:
        base_url = service_address_url(SERVICE_BY_KEY["operations_api"])
        if base_url is None:
            return "Operations API 端口配置无效，未修改密码。"
    parsed_base_url = urlparse(base_url)
    if parsed_base_url.scheme != "http" or parsed_base_url.hostname not in {"127.0.0.1", "localhost", "::1"}:
        return "快速改密只允许连接本机 HTTP Operations API。"
    origin = f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_console']}"
    session_url = base_url.rstrip("/") + "/session"
    login = urllib.request.Request(
        session_url,
        data=json.dumps({"email": email, "password": current_password}).encode("utf-8"),
        method="POST",
        headers={"Content-Type": "application/json", "Accept": "application/json", "Origin": origin},
    )
    cookie = http.cookies.SimpleCookie()
    try:
        with urllib.request.urlopen(login, timeout=timeout) as response:
            for value in response.headers.get_all("Set-Cookie", []):
                cookie.load(value)
    except urllib.error.HTTPError as exc:
        if exc.code == 401 and current_password != new_password:
            recovered = validate_operations_credentials(
                email, new_password, timeout=max(timeout, 12.0), base_url=base_url,
            )
            if recovered is None:
                return None
        return f"Operations 当前密码无效或登录失败：{_http_error_message(exc)}"
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        return f"无法连接 Operations API：{exc}"
    session = cookie.get("aster_operations_session")
    csrf = cookie.get("aster_operations_csrf")
    if not session or not csrf:
        return "Operations 登录未返回完整会话，未修改密码。"
    cookie_header = f"aster_operations_session={session.value}; aster_operations_csrf={csrf.value}"
    change = urllib.request.Request(
        base_url.rstrip("/") + "/session/password",
        data=json.dumps({"current_password": current_password, "new_password": new_password}).encode("utf-8"),
        method="PUT",
        headers={
            "Content-Type": "application/json", "Accept": "application/json", "Origin": origin,
            "Cookie": cookie_header, "X-CSRF-Token": unquote(csrf.value),
        },
    )
    try:
        with urllib.request.urlopen(change, timeout=max(timeout, PASSWORD_CHANGE_TIMEOUT_SECONDS)):
            pass
    except urllib.error.HTTPError as exc:
        return f"Operations 修改失败：{_http_error_message(exc)}"
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        recovered = validate_operations_credentials(
            email, new_password, timeout=max(timeout, 12.0), base_url=base_url,
        )
        if recovered is None:
            return None
        return f"Operations 修改失败：{exc}"
    logout = urllib.request.Request(
        session_url, method="DELETE",
        headers={
            "Accept": "application/json", "Origin": origin, "Cookie": cookie_header,
            "X-CSRF-Token": unquote(csrf.value),
        },
    )
    try:
        with urllib.request.urlopen(logout, timeout=timeout):
            pass
    except (OSError, TimeoutError, urllib.error.HTTPError, urllib.error.URLError):
        pass
    return None


def change_customer_password(
    email: str, current_password: str, new_password: str, timeout: float = 4.0,
    base_url: str | None = None,
) -> str | None:
    """Change the local Customer administrator password through the normal Admin API."""
    if base_url is None:
        base_url = service_address_url(SERVICE_BY_KEY["customer_control"])
        if base_url is None:
            return "Customer Control 端口配置无效，未修改密码。"
    parsed_base_url = urlparse(base_url)
    if parsed_base_url.scheme != "http" or parsed_base_url.hostname not in {"127.0.0.1", "localhost", "::1"}:
        return "快速改密只允许连接本机 HTTP Customer Control。"
    origin = f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_admin']}"
    login = urllib.request.Request(
        base_url.rstrip("/") + "/api/admin/auth/login",
        data=json.dumps({"email": email, "password": current_password}).encode("utf-8"),
        method="POST",
        headers={"Content-Type": "application/json", "Accept": "application/json", "Origin": origin},
    )
    cookie = http.cookies.SimpleCookie()
    try:
        with urllib.request.urlopen(login, timeout=timeout) as response:
            for value in response.headers.get_all("Set-Cookie", []):
                cookie.load(value)
    except urllib.error.HTTPError as exc:
        if exc.code == 401 and current_password != new_password:
            recovered = validate_customer_credentials(
                email, new_password, timeout=max(timeout, 12.0), base_url=base_url,
            )
            if recovered is None:
                return None
        return f"Customer 当前密码无效或登录失败：{_http_error_message(exc)}"
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        return f"无法连接 Rust Customer Control：{exc}"
    session = cookie.get("aster_admin_session")
    if not session:
        return "Customer 登录未返回管理员会话，未修改密码。"
    change = urllib.request.Request(
        base_url.rstrip("/") + "/api/admin/auth/password",
        data=json.dumps({"current_password": current_password, "new_password": new_password}).encode("utf-8"),
        method="POST",
        headers={
            "Content-Type": "application/json", "Accept": "application/json", "Origin": origin,
            "Cookie": f"aster_admin_session={session.value}",
        },
    )
    try:
        with urllib.request.urlopen(change, timeout=max(timeout, PASSWORD_CHANGE_TIMEOUT_SECONDS)):
            pass
    except urllib.error.HTTPError as exc:
        return f"Customer 修改失败：{_http_error_message(exc)}"
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        recovered = validate_customer_credentials(
            email, new_password, timeout=max(timeout, 12.0), base_url=base_url,
        )
        if recovered is None:
            return None
        return f"Customer 修改失败：{exc}"
    return None


def validate_operations_credentials(
    email: str, password: str, timeout: float = 4.0,
    base_url: str | None = None,
) -> str | None:
    """Validate current Operations credentials and remove the short-lived probe session."""
    if base_url is None:
        base_url = service_address_url(SERVICE_BY_KEY["operations_api"])
        if base_url is None:
            return "Operations API 端口配置无效，未验证凭据。"
    parsed_base_url = urlparse(base_url)
    if parsed_base_url.scheme != "http" or parsed_base_url.hostname not in {"127.0.0.1", "localhost", "::1"}:
        return "凭据验证只允许连接本机 HTTP Operations API。"
    url = base_url.rstrip("/") + "/session"
    origin = f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['operations_console']}"
    body = json.dumps({"email": email, "password": password}).encode("utf-8")
    request = urllib.request.Request(
        url, data=body, method="POST",
        headers={"Content-Type": "application/json", "Accept": "application/json", "Origin": origin},
    )
    response_error: str | None = None
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            cookie = http.cookies.SimpleCookie()
            for value in response.headers.get_all("Set-Cookie", []):
                cookie.load(value)
            try:
                payload = json.loads(response.read().decode("utf-8"))
                password_change_required = payload["operator"]["password_change_required"]
                if not isinstance(password_change_required, bool):
                    raise TypeError("password_change_required must be boolean")
                if password_change_required:
                    response_error = (
                        "Operations 仍在使用初始化密码，尚未完成首次改密。请先打开 Operations Console 完成首次改密，"
                        "再在“查看本地账号”中同步新密码后重试。"
                    )
            except (KeyError, TypeError, UnicodeDecodeError, json.JSONDecodeError):
                response_error = "Operations 登录响应缺少有效的首次改密状态，已停止授权。"
    except urllib.error.HTTPError as exc:
        if exc.code == 401:
            return "Operations 当前密码无效。请输入现在登录 Operations Console 使用的密码，而不是初始化时的临时密码。"
        return f"Operations 凭据验证失败：HTTP {exc.code}"
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        return f"无法连接 Operations API：{exc}"

    session = cookie.get("aster_operations_session")
    csrf = cookie.get("aster_operations_csrf")
    if session and csrf:
        logout = urllib.request.Request(
            url, method="DELETE",
            headers={
                "Accept": "application/json", "Origin": origin,
                "Cookie": f"aster_operations_session={session.value}; aster_operations_csrf={csrf.value}",
                "X-CSRF-Token": unquote(csrf.value),
            },
        )
        try:
            with urllib.request.urlopen(logout, timeout=timeout):
                pass
        except (OSError, TimeoutError, urllib.error.HTTPError, urllib.error.URLError):
            pass
    return response_error


def validate_customer_credentials(
    email: str, password: str, timeout: float = 4.0,
    base_url: str | None = None,
) -> str | None:
    """Validate the current local Customer administrator credentials."""
    if base_url is None:
        base_url = service_address_url(SERVICE_BY_KEY["customer_control"])
        if base_url is None:
            return "Customer Control 端口配置无效，未验证凭据。"
    parsed_base_url = urlparse(base_url)
    if parsed_base_url.scheme != "http" or parsed_base_url.hostname not in {"127.0.0.1", "localhost", "::1"}:
        return "本地凭据验证只允许连接本机 HTTP Customer Control。"
    url = base_url.rstrip("/") + "/api/admin/auth/login"
    origin = f"http://127.0.0.1:{ACTIVE_SERVICE_PORTS['customer_admin']}"
    body = json.dumps({"email": email, "password": password}).encode("utf-8")
    request = urllib.request.Request(
        url, data=body, method="POST",
        headers={"Content-Type": "application/json", "Accept": "application/json", "Origin": origin},
    )
    cookie = http.cookies.SimpleCookie()
    response_error: str | None = None
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            for value in response.headers.get_all("Set-Cookie", []):
                cookie.load(value)
            try:
                payload = json.loads(response.read().decode("utf-8"))
                password_change_required = payload["password_change_required"]
                if not isinstance(password_change_required, bool):
                    raise TypeError("password_change_required must be boolean")
                if password_change_required:
                    response_error = (
                        "Customer 仍在使用初始化密码，尚未完成首次改密。请先打开 Customer Admin 完成首次改密，"
                        "再在“查看本地账号”中同步新密码后重试。"
                    )
            except (KeyError, TypeError, UnicodeDecodeError, json.JSONDecodeError):
                response_error = "Customer 登录响应缺少有效的首次改密状态，已停止授权。"
    except urllib.error.HTTPError as exc:
        if exc.code == 401:
            return "Customer 当前密码无效。请先把浏览器中使用的当前密码写入本地管理员凭据文件。"
        return f"Customer 凭据验证失败：HTTP {exc.code}"
    except (OSError, TimeoutError, urllib.error.URLError) as exc:
        return f"无法连接 Rust Customer Control：{exc}"

    session = cookie.get("aster_admin_session")
    if not session:
        return "Customer 登录未返回本地管理员会话。"

    logout = urllib.request.Request(
        base_url.rstrip("/") + "/api/admin/auth/logout", data=b"{}", method="POST",
        headers={
            "Content-Type": "application/json", "Origin": origin,
            "Cookie": f"aster_admin_session={session.value}",
        },
    )
    try:
        with urllib.request.urlopen(logout, timeout=timeout):
            pass
    except (OSError, TimeoutError, urllib.error.HTTPError, urllib.error.URLError):
        pass
    return response_error


class LocalDevManager:
    def __init__(self, root: tk.Tk) -> None:
        self.root = root
        self.npm = npm_executable()
        self.node = node_executable()
        self.events: queue.Queue[tuple] = queue.Queue()
        self.processes: dict[str, subprocess.Popen[bytes] | RestoredProcess] = {}
        self.process_state_lock = threading.RLock()
        self.started_at: dict[str, float] = {}
        self.preparing: set[str] = set()
        self.busy: set[str] = set()
        self.health: dict[str, bool] = {service.key: False for service in SERVICES}
        self.health_initialized = False
        self.external_process_details: dict[str, tuple[str, str]] = {}
        self.stop_failures: dict[str, str] = {}
        self.failures: dict[str, str] = {}
        self.recent_output: dict[str, list[str]] = {service.key: [] for service in SERVICES}
        self.rows: dict[str, dict[str, object]] = {}
        self.log_history: list[tuple[str, str, str]] = []
        self.log_texts: dict[str, tk.Text] = {}
        self.log_tabs: dict[str, ttk.Frame] = {}
        self.unread_log_severity: dict[str, str] = {}
        self.active_log_key = SERVICES[0].key
        self.terminal_states: dict[str, TerminalStyle] = {}
        self.page = ""
        self.setup_process: subprocess.Popen[str] | None = None
        self.demo_process: subprocess.Popen[str] | None = None
        self.authorization_dialog: tk.Toplevel | None = None
        self.authorization_step_rows: dict[str, dict[str, object]] = {}
        self.authorization_step_started: dict[str, float] = {}
        self.authorization_active_step: str | None = None
        self.authorization_detail: ttk.Treeview | None = None
        self.authorization_detail_text: tk.Text | None = None
        self.authorization_detail_values: dict[str, str] = {}
        self.authorization_detail_groups: dict[str, str] = {}
        self.authorization_status = tk.StringVar(value="")
        self.authorization_start_button: ttk.Button | None = None
        self.authorization_close_button: ttk.Button | None = None
        self.authorization_in_progress = False
        self.quick_password_dialog: tk.Toplevel | None = None
        self.quick_password_status: tk.StringVar | None = None
        self.quick_password_confirm_button: ttk.Button | None = None
        self.quick_password_cancel_button: ttk.Button | None = None
        self.quick_password_in_progress = False
        self.quick_password_required_accounts: set[str] = set()
        self.reset_button: ttk.Button | None = None
        self.setup_vars: dict[str, tk.StringVar] = {}
        self.health_probe_running = False
        self.closing = False
        self.include_website = tk.BooleanVar(value=False)
        self.include_runner = tk.BooleanVar(value=False)
        self.browser_option = tk.StringVar(value=DEFAULT_BROWSER_OPTION)
        self.lan_access = saved_lan_access_enabled()
        self.lan_host = detect_lan_ipv4() if self.lan_access else None
        self.lan_access = self.lan_access and self.lan_host is not None
        self.lan_access_enabled = tk.BooleanVar(value=self.lan_access)
        self.local_admin_credentials_sha256 = ""
        self.local_admin_credentials: dict[str, str] = {}
        self.credentials_dialog: tk.Toplevel | None = None
        self.integration_dialog: tk.Toplevel | None = None
        self.integration_tree: ttk.Treeview | None = None
        self.integration_picker: PullRequestSelectionTable | None = None
        self.integration_status = tk.StringVar(value="")
        self.integration_busy = False
        self.integration_buttons: list[ttk.Button] = []
        self.integration_pull_requests: list[local_pr_integration.PullRequest] = []
        self.integration_applied: dict[int, str] = {}
        integration_settings = read_pr_integration_settings(ROOT)
        self.integration_selected = selected_pull_request_numbers(integration_settings, "selected")
        self.integration_seen = selected_pull_request_numbers(integration_settings, "seen")
        self.integration_drafts = selected_pull_request_numbers(integration_settings, "drafts")
        try:
            _primary_root, default_integration_path = environment_roots(ROOT)
        except local_pr_integration.IntegrationError:
            default_integration_path = ROOT.parent / f"{ROOT.name}_worktrees" / "integration-test"
        self.integration_target = default_integration_path
        self.footer_message = tk.StringVar(value=f"项目目录：{ROOT}")
        self._restore_process_state()
        self._configure_window()
        self._build_ui()
        self.root.protocol("WM_DELETE_WINDOW", self.close_requested)
        self.root.after(100, self._drain_events)
        self.root.after(150, self._schedule_health_probe)

    def _restore_process_state(self) -> None:
        try:
            payload = json.loads(PROCESS_STATE_FILE.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return
        records = payload.get("services", {}) if isinstance(payload, dict) else {}
        if not isinstance(records, dict):
            return
        changed = False
        for key, record in records.items():
            if key not in SERVICE_BY_KEY or not isinstance(record, dict):
                changed = True
                continue
            pid = record.get("pid")
            identity = record.get("identity")
            if not isinstance(pid, int) or not isinstance(identity, str) or process_identity(pid) != identity:
                changed = True
                continue
            self.processes[key] = RestoredProcess(pid, identity)
            self.started_at[key] = time.monotonic()
            self._start_log_reader(SERVICE_BY_KEY[key], self.processes[key], restored=True)
            threading.Thread(
                target=self._watch_process, args=(SERVICE_BY_KEY[key], self.processes[key]), daemon=True,
            ).start()
        if changed:
            self._save_process_state()

    def _save_process_state(self) -> None:
        with self.process_state_lock:
            services: dict[str, dict[str, object]] = {}
            for key, process in self.processes.items():
                if process.poll() is not None:
                    continue
                identity = process_identity(process.pid)
                if identity:
                    services[key] = {"pid": process.pid, "identity": identity}
            PROCESS_STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
            temporary = PROCESS_STATE_FILE.with_suffix(".tmp")
            temporary.write_text(json.dumps({
                "version": 2,
                "lan_access_enabled": self.lan_access,
                "services": services,
            }, indent=2) + "\n", encoding="utf-8")
            os.replace(temporary, PROCESS_STATE_FILE)

    def _configure_window(self) -> None:
        self.root.title(workspace_title("Aster Team 本地开发控制台"))
        self.root.geometry("1240x820")
        self.root.minsize(1040, 680)
        style = ttk.Style(self.root)
        available = style.theme_names()
        if "vista" in available:
            style.theme_use("vista")
        style.layout("TButton", [
            ("Button.button", {"sticky": "nswe", "children": [
                ("Button.padding", {"sticky": "nswe", "children": [
                    ("Button.label", {"sticky": "nswe"}),
                ]}),
            ]}),
        ])
        style.layout("TCheckbutton", [
            ("Checkbutton.padding", {"sticky": "nswe", "children": [
                ("Checkbutton.indicator", {"side": "left", "sticky": ""}),
                ("Checkbutton.label", {"side": "left", "sticky": "w"}),
            ]}),
        ])
        style.configure("Title.TLabel", font=(UI_FONT_FAMILY, 18, "bold"))
        style.configure("Subtitle.TLabel", foreground="#536275")
        style.configure("Running.TLabel", foreground="#157347", font=(UI_FONT_FAMILY, 9, "bold"))
        style.configure("Starting.TLabel", foreground="#9a6700", font=(UI_FONT_FAMILY, 9, "bold"))
        style.configure("Stopped.TLabel", foreground="#8b3030", font=(UI_FONT_FAMILY, 9, "bold"))
        style.configure("Muted.TLabel", foreground="#6b7280")
        style.configure("Icon.TButton", font=(SYMBOL_FONT_FAMILY, 11), padding=(2, 1))
        surface = style.lookup("TFrame", "background") or self.root.cget("bg")
        style.configure("Authorization.Treeview", rowheight=30, font=(UI_FONT_FAMILY, 9))
        style.configure("Authorization.Treeview.Heading", font=(UI_FONT_FAMILY, 9, "bold"))
        style.configure("PullRequest.Treeview", rowheight=30, font=(UI_FONT_FAMILY, 10))
        style.configure("PullRequest.Treeview.Heading", font=(UI_FONT_FAMILY, 9, "bold"))
        style.configure(
            "Log.TNotebook", borderwidth=1, relief="solid", background=surface,
            tabmargins=(2, 2, 2, 0),
        )
        style.configure(
            "Log.TNotebook.Tab", padding=(11, 5), borderwidth=2, relief="solid",
            background=surface, foreground="#273142",
            font=(UI_FONT_FAMILY, 9),
        )
        style.map(
            "Log.TNotebook.Tab",
            background=[("selected", surface), ("active", surface), ("!selected", surface)],
            foreground=[("selected", "#111827"), ("active", "#111827"), ("!selected", "#465366")],
            expand=[("selected", (1, 2, 1, 0))],
        )

    @staticmethod
    def _workspace_title_label(parent: ttk.Frame, title: str) -> ttk.Label:
        label = ttk.Label(parent, text=workspace_title(title), style="Title.TLabel")
        ToolTip(label, str(ROOT.resolve()))
        return label

    def _show_modeless_dialog(self, dialog: tk.Toplevel, width: int, height: int) -> None:
        """Place and reveal an independent tool window without locking the main window."""
        self.root.update_idletasks()
        dialog.update_idletasks()
        screen_width = dialog.winfo_vrootwidth()
        screen_height = dialog.winfo_vrootheight()
        minimum_width, minimum_height = dialog.minsize()
        dialog.minsize(
            min(minimum_width, max(1, screen_width - 32)),
            min(minimum_height, max(1, screen_height - 32)),
        )
        dialog.geometry(centered_window_geometry(
            self.root.winfo_rootx(), self.root.winfo_rooty(),
            self.root.winfo_width(), self.root.winfo_height(),
            width, height,
            dialog.winfo_vrootx(), dialog.winfo_vrooty(),
            screen_width, screen_height,
        ))
        dialog.deiconify()
        dialog.lift()
        dialog.focus_set()

    @staticmethod
    def _icon_button(parent: ttk.Frame, icon: str, tooltip: str, command) -> ttk.Button:
        button = ttk.Button(parent, text=icon, width=3, style="Icon.TButton", command=command)
        ToolTip(button, tooltip)
        return button

    @staticmethod
    def _terminal_tag(text_widget: tk.Text, style: TerminalStyle) -> str:
        identity = "_".join((
            (style.foreground or "default").replace("#", ""),
            (style.background or "default").replace("#", ""),
            "b" if style.bold else "n", "d" if style.dim else "n",
            "i" if style.italic else "n", "u" if style.underline else "n",
        ))
        tag = f"ansi_{identity}"
        if tag not in text_widget.tag_names():
            options: dict[str, object] = {}
            if style.foreground:
                options["foreground"] = style.foreground
            elif style.dim:
                options["foreground"] = "#8491a7"
            if style.background:
                options["background"] = style.background
            traits = " ".join(value for enabled, value in ((style.bold, "bold"), (style.italic, "italic")) if enabled)
            if traits:
                options["font"] = (MONO_FONT_FAMILY, 9, traits)
            if style.underline:
                options["underline"] = True
            text_widget.tag_configure(tag, **options)
        return tag

    def _insert_terminal_line(self, key: str, text_widget: tk.Text, prefix: str, value: str) -> None:
        text_widget.insert("end", prefix)
        segments, final_style = terminal_segments(value, self.terminal_states.get(key))
        for text, style in segments:
            text_widget.insert("end", text, (self._terminal_tag(text_widget, style),))
        text_widget.insert("end", "\n")
        self.terminal_states[key] = final_style

    def _selectable_cell(
        self, parent: ttk.Frame, text: str, *, width: int,
        foreground: str = "#111111", address_url: str | None = None,
    ) -> tk.Entry:
        entry = tk.Entry(
            parent, width=width, relief="flat", borderwidth=0, highlightthickness=0,
            bg=self.root.cget("bg"), readonlybackground=self.root.cget("bg"),
            fg=foreground, selectbackground="#0b63ce", selectforeground="#ffffff",
            font=(UI_FONT_FAMILY, 9), takefocus=True,
        )
        entry.insert(0, text)
        entry.configure(state="readonly")
        entry.bind("<Control-c>", self._copy_cell_selection, add="+")
        entry.bind("<Control-C>", self._copy_cell_selection, add="+")
        if address_url:
            entry.configure(cursor="xterm", fg="#0b63ce", font=(UI_FONT_FAMILY, 9, "underline"))
            entry.bind("<ButtonPress-1>", self._address_press, add="+")
            entry.bind("<ButtonRelease-1>", lambda event, url=address_url: self._address_release(event, url), add="+")
            entry.bind("<Motion>", self._address_motion, add="+")
            entry.bind("<Leave>", lambda event: event.widget.configure(cursor="xterm"), add="+")
            entry.bind("<Return>", lambda _event, url=address_url: self.open_url(url))
        return entry

    def _copy_cell_selection(self, event: tk.Event) -> str | None:
        entry: tk.Entry = event.widget
        if not entry.selection_present():
            return None
        selected = entry.get()[entry.index("sel.first"):entry.index("sel.last")]
        self.root.clipboard_clear()
        self.root.clipboard_append(selected)
        self.root.update_idletasks()
        self.footer_message.set(f"已复制：{selected}")
        return "break"

    @staticmethod
    def _address_text_width(entry: tk.Entry) -> int:
        return tkfont.Font(font=entry.cget("font")).measure(entry.get()) + 4

    def _address_press(self, event: tk.Event) -> None:
        event.widget._address_press_position = (event.x_root, event.y_root)

    def _address_release(self, event: tk.Event, url: str) -> None:
        entry: tk.Entry = event.widget
        press = getattr(entry, "_address_press_position", None)
        entry._address_press_position = None
        if press is None:
            return
        moved = abs(event.x_root - press[0]) > 4 or abs(event.y_root - press[1]) > 4
        if not moved and 0 <= event.x <= self._address_text_width(entry):
            self.root.after_idle(lambda: self.open_url(url))

    def _address_motion(self, event: tk.Event) -> None:
        cursor = "hand2" if 0 <= event.x <= self._address_text_width(event.widget) else "xterm"
        event.widget.configure(cursor=cursor)

    def _build_ui(self) -> None:
        self.shell = ttk.Frame(self.root, padding=16)
        self.shell.pack(fill="both", expand=True)
        if initialization_state() == "complete":
            self._show_main()
        else:
            self._show_setup()

    def _clear_shell(self) -> None:
        for child in self.shell.winfo_children():
            child.destroy()
        self.rows.clear()
        self.log_texts.clear()
        self.log_tabs.clear()
        self.unread_log_severity.clear()
        self.terminal_states.clear()

    def _show_main(self) -> None:
        self._clear_shell()
        self.page = "main"
        self._build_main_content()
        self.refresh_rows()

    def _build_main_content(self) -> None:
        shell = self.shell

        header = ttk.Frame(shell)
        header.pack(fill="x")
        self._workspace_title_label(header, "Aster Team 本地开发控制台").pack(anchor="w")
        ttk.Label(
            header,
            text="集中管理本地服务；关闭控制台不会停止服务，重新打开后可继续管理并查看日志。",
            style="Subtitle.TLabel",
        ).pack(anchor="w", pady=(3, 12))

        actions = ttk.Frame(shell)
        actions.pack(fill="x", pady=(0, 6))
        ttk.Button(actions, text="▶ 一键启动", command=self.start_all).pack(side="left")
        ttk.Button(actions, text="↻ 重启已管理服务", command=self.restart_all).pack(side="left", padx=6)
        ttk.Button(actions, text="■ 停止已管理服务", command=self.stop_all).pack(side="left")
        self.demo_button = ttk.Button(actions, text="本地快速授权", command=self.open_local_authorization)
        self.demo_button.pack(side="left", padx=(14, 0))
        ttk.Button(actions, text="重新初始化", command=self.request_reinitialize).pack(side="right")
        ttk.Button(actions, text="查看本地账号", command=self.show_local_admin_credentials).pack(side="right", padx=6)
        ttk.Button(actions, text="管理 PR 集成", command=self.open_pr_integration).pack(side="right", padx=6)

        options = ttk.Frame(shell)
        options.pack(fill="x", pady=(0, 12))
        ttk.Checkbutton(options, text="包含官网（含表单后端）", variable=self.include_website).pack(side="left", padx=(0, 8))
        ttk.Checkbutton(options, text="包含 Runner", variable=self.include_runner).pack(side="left", padx=8)
        lan_access = ttk.Checkbutton(
            options,
            text="允许局域网 IP 访问（HTTP）",
            variable=self.lan_access_enabled,
            command=self.toggle_lan_access,
        )
        lan_access.pack(side="left", padx=(12, 4))
        ToolTip(
            lan_access,
            "开启后，Customer API 与各开发页面允许通过局域网 IP 访问；当前托管服务会自动重启。"
            "内部 API、数据库和本地自动登录凭据仍仅限本机。",
        )
        browser_picker = ttk.Combobox(
            options,
            textvariable=self.browser_option,
            values=BROWSER_OPTIONS,
            state="readonly",
            width=15,
        )
        browser_picker.pack(side="right")
        ttk.Label(options, text="打开链接：").pack(side="right", padx=(8, 4))
        ToolTip(
            browser_picker,
            "仅控制本地开发控制台中的链接，不会修改操作系统的默认浏览器。",
        )

        services_box = ttk.LabelFrame(shell, text="服务", padding=8)
        services_box.pack(fill="x", pady=(0, 12))
        headings = (("状态", 0), ("服务", 1), ("说明", 2), ("地址", 3), ("操作", 4))
        for text, column in headings:
            ttk.Label(services_box, text=text, font=(UI_FONT_FAMILY, 9, "bold")).grid(row=0, column=column, sticky="w", padx=6, pady=(0, 6))
        services_box.columnconfigure(3, weight=1)
        for row_index, service in enumerate(SERVICES, start=1):
            status = self._selectable_cell(services_box, "检测中…", width=28, foreground="#9a6700")
            status.grid(row=row_index, column=0, sticky="w", padx=6, pady=3)
            status_tooltip = ToolTip(status, "")
            self._selectable_cell(services_box, service.name, width=22).grid(row=row_index, column=1, sticky="w", padx=6, pady=3)
            self._selectable_cell(services_box, service.note, width=28, foreground="#536275").grid(row=row_index, column=2, sticky="w", padx=6, pady=3)
            address_url = service_address_url(service, self.lan_host if self.lan_access else None)
            address = address_url or "—"
            self._selectable_cell(
                services_box, address, width=38, foreground="#536275", address_url=address_url,
            ).grid(row=row_index, column=3, sticky="ew", padx=6, pady=3)
            buttons = ttk.Frame(services_box)
            buttons.grid(row=row_index, column=4, sticky="w", padx=6, pady=2)
            for column in range(3):
                buttons.columnconfigure(column, minsize=36)
            start = self._icon_button(buttons, "▶", f"启动 {service.name}", lambda key=service.key: self.start_service(key))
            restart = self._icon_button(buttons, "↻", f"重启 {service.name}", lambda key=service.key: self.restart_service(key))
            stop = self._icon_button(buttons, "■", f"停止 {service.name}", lambda key=service.key: self.stop_service(key))
            start.grid(row=0, column=0, padx=2)
            restart.grid(row=0, column=1, padx=2)
            stop.grid(row=0, column=2, padx=2)
            self.rows[service.key] = {
                "status": status, "status_tooltip": status_tooltip,
                "start": start, "restart": restart, "stop": stop,
            }

        log_head = ttk.Frame(shell)
        log_head.pack(fill="x")
        ttk.Label(log_head, text="服务日志", font=(UI_FONT_FAMILY, 10, "bold")).pack(side="left")
        ttk.Button(log_head, text="清空当前", command=self.clear_logs).pack(side="right")

        self.log_notebook = ttk.Notebook(shell, style="Log.TNotebook")
        self.log_notebook.pack(fill="both", expand=True, pady=(6, 8))
        for service in SERVICES:
            log_frame = ttk.Frame(self.log_notebook, padding=1, relief="solid", borderwidth=1)
            log_text = tk.Text(
                log_frame, wrap="none", height=14, bg="#10141f", fg="#d8e4ff",
                insertbackground="white", font=(MONO_FONT_FAMILY, 9), state="disabled",
            )
            scroll_y = ttk.Scrollbar(log_frame, orient="vertical", command=log_text.yview)
            scroll_x = ttk.Scrollbar(log_frame, orient="horizontal", command=log_text.xview)
            log_text.configure(yscrollcommand=scroll_y.set, xscrollcommand=scroll_x.set)
            log_text.grid(row=0, column=0, sticky="nsew")
            scroll_y.grid(row=0, column=1, sticky="ns")
            scroll_x.grid(row=1, column=0, sticky="ew")
            log_frame.columnconfigure(0, weight=1)
            log_frame.rowconfigure(0, weight=1)
            self.log_notebook.add(log_frame, text=LOG_TAB_LABELS[service.key])
            self.log_tabs[service.key] = log_frame
            self.log_texts[service.key] = log_text
        self.active_log_key = SERVICES[0].key
        self.log_notebook.bind("<<NotebookTabChanged>>", self._log_tab_changed, add="+")
        self.render_logs()
        ttk.Label(shell, textvariable=self.footer_message, style="Muted.TLabel").pack(fill="x")

    def _save_pr_integration_settings(self) -> None:
        write_pr_integration_settings(
            self.integration_selected, self.integration_seen, self.integration_drafts, ROOT,
        )

    def open_pr_integration(self) -> None:
        if self.integration_dialog is not None and self.integration_dialog.winfo_exists():
            self.integration_dialog.deiconify()
            self.integration_dialog.lift()
            self.integration_dialog.focus_set()
            return
        dialog = tk.Toplevel(self.root)
        dialog.withdraw()
        self.integration_dialog = dialog
        dialog.title("本地 PR 集成测试")
        dialog.minsize(900, 520)
        dialog.protocol("WM_DELETE_WINDOW", self._close_pr_integration)

        header = ttk.Frame(dialog, padding=(16, 14, 16, 8))
        header.pack(fill="x")
        ttk.Label(header, text="本地 PR 集成测试", style="Title.TLabel").pack(anchor="w")
        ttk.Label(
            header,
            text="集成代码严格由最新 origin/main 与所选远端 PR 组成；不会读取本地未提交内容，也不会 push 或合并 PR。",
            style="Subtitle.TLabel",
        ).pack(anchor="w", pady=(3, 0))

        path_row = ttk.Frame(dialog, padding=(16, 4, 16, 8))
        path_row.pack(fill="x")
        ttk.Label(path_row, text="固定目录", font=(UI_FONT_FAMILY, 9, "bold")).pack(side="left")
        ttk.Label(
            path_row, text="../aster-team_worktrees/integration-test", style="Muted.TLabel",
        ).pack(side="left", padx=8)
        ToolTip(path_row, str(self.integration_target))

        toolbar = ttk.Frame(dialog, padding=(16, 0, 16, 8))
        toolbar.pack(fill="x")
        refresh_button = ttk.Button(toolbar, text="刷新 PR", command=self.refresh_pull_requests)
        refresh_button.pack(side="left")
        all_button = ttk.Button(toolbar, text="全选", command=lambda: self._set_all_pull_requests("all"))
        all_button.pack(side="left", padx=(8, 0))
        none_button = ttk.Button(toolbar, text="全部取消", command=lambda: self._set_all_pull_requests("none"))
        none_button.pack(side="left", padx=(8, 0))
        add_button = ttk.Button(toolbar, text="加入高亮 PR", command=lambda: self._apply_highlighted_pull_request(True))
        add_button.pack(side="right")
        remove_button = ttk.Button(toolbar, text="移除高亮 PR", command=lambda: self._apply_highlighted_pull_request(False))
        remove_button.pack(side="right", padx=6)
        view_button = ttk.Button(toolbar, text="查看高亮 PR", command=self._open_highlighted_pull_request)
        view_button.pack(side="right")
        self.integration_buttons = [
            refresh_button, all_button, none_button,
            add_button, remove_button, view_button,
        ]

        table = ttk.Frame(dialog, padding=(16, 0, 16, 8))
        table.pack(fill="both", expand=True)
        self.integration_picker = PullRequestSelectionTable(table, self._toggle_pull_request, height=10)
        self.integration_picker.pack(fill="both", expand=True)
        self.integration_tree = self.integration_picker.tree

        footer = ttk.Frame(dialog, padding=(16, 0, 16, 14))
        footer.pack(fill="x")
        ttk.Label(footer, textvariable=self.integration_status, style="Muted.TLabel").pack(side="left", fill="x", expand=True)
        open_button: ttk.Button | None = None
        if self.integration_target.resolve() != ROOT.resolve():
            open_button = ttk.Button(footer, text="打开集成控制台", command=self.open_integrated_dev_manager)
            open_button.pack(side="right")
        apply_button = ttk.Button(footer, text="应用所选 PR", command=self.apply_selected_pull_requests)
        apply_button.pack(side="right", padx=8)
        self.integration_buttons.append(apply_button)
        if open_button is not None:
            self.integration_buttons.append(open_button)

        self._load_applied_integration_state()
        self._render_pull_requests()
        self._show_modeless_dialog(dialog, 1080, 650)
        self.refresh_pull_requests()

    def _close_pr_integration(self) -> None:
        if self.integration_dialog is not None:
            self.integration_dialog.destroy()
        self.integration_dialog = None
        self.integration_tree = None
        self.integration_picker = None

    def _integration_workspace(self) -> local_pr_integration.IntegrationWorkspace:
        return local_pr_integration.IntegrationWorkspace(
            ROOT, self.integration_target,
            progress=lambda message: self.events.put(("integration_progress", message)),
        )

    def _load_applied_integration_state(self) -> None:
        self.integration_applied = {}
        try:
            workspace = self._integration_workspace()
            state = workspace.load_state()
        except local_pr_integration.IntegrationError:
            return
        if state is not None:
            self.integration_applied = {item.number: item.head_sha for item in state.entries}

    def _set_integration_busy(self, busy: bool, message: str = "") -> None:
        self.integration_busy = busy
        state = "disabled" if busy else "normal"
        for button in self.integration_buttons:
            if button.winfo_exists():
                button.configure(state=state)
        if message:
            self.integration_status.set(message)

    def refresh_pull_requests(self) -> None:
        if self.integration_busy:
            return
        try:
            self._save_pr_integration_settings()
        except OSError as exc:
            messagebox.showerror("无法保存设置", str(exc), parent=self.integration_dialog)
            return
        self._set_integration_busy(True, "正在从 GitHub 刷新 Open PR…")
        threading.Thread(target=self._refresh_pull_requests_worker, daemon=True).start()

    def _refresh_pull_requests_worker(self) -> None:
        try:
            pull_requests = local_pr_integration.list_open_pull_requests(ROOT)
            self.events.put(("integration_prs", pull_requests))
        except Exception as exc:
            self.events.put(("integration_error", "刷新 PR 失败", str(exc)))

    def _render_pull_requests(self) -> None:
        picker = self.integration_picker
        if picker is None or not picker.tree.winfo_exists():
            return
        picker.render(self.integration_pull_requests, self.integration_selected, self.integration_applied)
        selected_count = len(self.integration_selected & {item.number for item in self.integration_pull_requests})
        applied_count = sum(
            self.integration_applied.get(item.number) == item.head_sha for item in self.integration_pull_requests
        )
        removal_count = len(set(self.integration_applied) - self.integration_selected)
        if not self.integration_busy:
            self.integration_status.set(
                f"共 {len(self.integration_pull_requests)} 个 Open PR；已选择 {selected_count} 个，"
                f"当前版本已集成 {applied_count} 个，待移除 {removal_count} 个。"
            )

    def _toggle_pull_request(self, number: int) -> None:
        if self.integration_busy:
            return
        if number in self.integration_selected:
            self.integration_selected.remove(number)
        else:
            self.integration_selected.add(number)
        self._save_pr_integration_settings()
        self._render_pull_requests()

    def _set_all_pull_requests(self, mode: str) -> None:
        if mode == "all":
            self.integration_selected = {item.number for item in self.integration_pull_requests if not item.is_draft}
        else:
            self.integration_selected.clear()
        self.integration_seen = {item.number for item in self.integration_pull_requests}
        self._save_pr_integration_settings()
        self._render_pull_requests()

    def _highlighted_pull_request(self) -> local_pr_integration.PullRequest | None:
        tree = self.integration_tree
        selected_rows = tree.selection() if tree is not None else ()
        if not selected_rows:
            messagebox.showinfo("请选择 PR", "请先在列表中高亮一个 PR。", parent=self.integration_dialog)
            return None
        number = int(selected_rows[0])
        return next((item for item in self.integration_pull_requests if item.number == number), None)

    def _apply_highlighted_pull_request(self, include: bool) -> None:
        pull_request = self._highlighted_pull_request()
        if pull_request is None:
            return
        if include:
            self.integration_selected.add(pull_request.number)
        else:
            self.integration_selected.discard(pull_request.number)
        self.integration_seen.add(pull_request.number)
        self._save_pr_integration_settings()
        self._render_pull_requests()
        self.apply_selected_pull_requests()

    def _open_highlighted_pull_request(self) -> None:
        pull_request = self._highlighted_pull_request()
        if pull_request is not None:
            webbrowser.open(pull_request.url, new=2)
            self.integration_status.set(f"已请求浏览器打开 PR #{pull_request.number}。")

    def apply_selected_pull_requests(self) -> None:
        if self.integration_busy:
            return
        selected = [
            item for item in self.integration_pull_requests if item.number in self.integration_selected
        ]
        try:
            workspace = self._integration_workspace()
            target = workspace.target
        except local_pr_integration.IntegrationError as exc:
            messagebox.showerror("集成配置无效", str(exc), parent=self.integration_dialog)
            return
        if target == ROOT.resolve() and any(process.poll() is None for process in self.processes.values()):
            messagebox.showwarning(
                "请先停止服务", "当前控制台正在从该目录运行服务，请先停止服务后再更新集成分支。",
                parent=self.integration_dialog,
            )
            return
        running_in_target = managed_services_in_workspace(target)
        if running_in_target:
            messagebox.showwarning(
                "请先停止集成服务",
                "以下服务仍从集成目录运行，请先在集成控制台中停止：\n\n" + "、".join(running_in_target),
                parent=self.integration_dialog,
            )
            return
        self._save_pr_integration_settings()
        self._set_integration_busy(True, f"正在应用 {len(selected)} 个 PR…")
        threading.Thread(
            target=self._apply_selected_pull_requests_worker, args=(workspace, selected), daemon=True,
        ).start()

    def _apply_selected_pull_requests_worker(
        self, workspace: local_pr_integration.IntegrationWorkspace,
        selected: list[local_pr_integration.PullRequest],
    ) -> None:
        try:
            result = workspace.apply(selected)
            if not node_dependencies_ready(result.path):
                self.events.put(("integration_progress", "正在安装集成环境的 Node.js 依赖…"))
            ensure_node_dependencies(result.path)
            self.events.put(("integration_applied", result))
        except Exception as exc:
            self.events.put(("integration_error", "更新集成分支失败", str(exc)))

    def open_integrated_dev_manager(self) -> None:
        try:
            workspace = self._integration_workspace()
        except local_pr_integration.IntegrationError as exc:
            messagebox.showerror("集成配置无效", str(exc), parent=self.integration_dialog)
            return
        script = workspace.target / "scripts/local_dev_manager.py"
        if not script.is_file():
            messagebox.showinfo(
                "尚未创建集成环境", "请先刷新 PR 并点击“应用所选 PR”。",
                parent=self.integration_dialog,
            )
            return
        try:
            subprocess.Popen(
                [sys.executable, str(script)], cwd=workspace.target,
                env=development_tool_environment(workspace.target),
            )
        except OSError as exc:
            messagebox.showerror("无法打开集成控制台", str(exc), parent=self.integration_dialog)
            return
        self.integration_status.set(f"已从 {workspace.target} 打开集成控制台。")

    def _show_setup(self) -> None:
        self._clear_shell()
        self.page = "setup"
        self.reset_button = None
        state = initialization_state()
        source = read_env_file(ROOT / ".env")
        defaults = {
            "ASTER_LOCAL_ADMIN_EMAIL": "admin@example.com",
            "ASTER_OPERATIONS_DB_HOST": "127.0.0.1", "ASTER_OPERATIONS_DB_PORT": "3306",
            "ASTER_OPERATIONS_DB_NAME": "aster_operations", "ASTER_OPERATIONS_DB_ADMIN_USER": "root",
            "ASTER_OPERATIONS_DB_ADMIN_PASSWORD": "", "ASTER_OPERATIONS_DB_ADMIN_TLS": "false",
            "ASTER_OPERATIONS_DB_SERVICE_USER": "aster_operations", "ASTER_OPERATIONS_DB_SERVICE_HOST": "127.0.0.1",
            "ASTER_CUSTOMER_DB_HOST": "127.0.0.1", "ASTER_CUSTOMER_DB_PORT": "3306",
            "ASTER_CUSTOMER_DB_NAME": "aster_customer", "ASTER_CUSTOMER_DB_ADMIN_USER": "root",
            "ASTER_CUSTOMER_DB_ADMIN_PASSWORD": "", "ASTER_CUSTOMER_DB_ADMIN_TLS": "false",
            "ASTER_CUSTOMER_DB_SERVICE_USER": "aster_customer", "ASTER_CUSTOMER_DB_SERVICE_HOST": "127.0.0.1",
            "ASTER_LOCAL_SECURITY_CONFIG_DIR": "",
        }
        defaults.update(local_database_defaults(ROOT))
        self.setup_vars = {key: tk.StringVar(value=source.get(key, default)) for key, default in defaults.items()}
        header = ttk.Frame(self.shell)
        header.pack(fill="x")
        title = "首次初始化 Aster Team" if state == "missing" else "重新初始化 Aster Team"
        self._workspace_title_label(header, title).pack(anchor="w")
        detail = "尚未检测到运行配置。填写下面的连接信息后，界面会创建两套独立数据库、专用账号和本地签名密钥。"
        if state == "partial":
            detail = "检测到不完整的初始化文件。请核对配置并重新初始化；旧数据库和残留运行数据会在确认后清理。"
        elif state == "complete":
            detail = "重新初始化会删除并重建 Customer、Operations 数据库，同时重置授权密钥、Runner 和本地交付数据。"
        ttk.Label(header, text=detail, style="Subtitle.TLabel", wraplength=1120).pack(anchor="w", pady=(3, 12))

        account = ttk.LabelFrame(self.shell, text="本地管理员", padding=10)
        account.pack(fill="x", pady=(0, 10))
        ttk.Label(account, text="管理员邮箱").grid(row=0, column=0, sticky="w", padx=(0, 8))
        ttk.Entry(account, textvariable=self.setup_vars["ASTER_LOCAL_ADMIN_EMAIL"], width=42).grid(row=0, column=1, sticky="w")
        ttk.Label(account, text="数据库管理员密码只保存在根目录 .env，不会写入运行配置。", style="Muted.TLabel").grid(row=1, column=0, columnspan=2, sticky="w", pady=(7, 0))

        self._common_database_form(self.shell)

        databases = ttk.Frame(self.shell)
        databases.pack(fill="x", pady=(0, 10))
        databases.columnconfigure(0, weight=1)
        databases.columnconfigure(1, weight=1)
        self._database_identity_form(databases, "ASTER_OPERATIONS_DB", "Operations 数据库", 0)
        self._database_identity_form(databases, "ASTER_CUSTOMER_DB", "Customer 数据库", 1)

        self._security_config_form(self.shell)

        actions = ttk.Frame(self.shell)
        actions.pack(fill="x", pady=(0, 8))
        self.setup_button = ttk.Button(actions, text="初始化并进入控制台", command=self.initialize_from_ui)
        self.setup_button.pack(side="left")
        if state != "missing":
            self.reset_button = ttk.Button(actions, text="清除本地环境", command=self.clear_environment_from_ui)
            self.reset_button.pack(side="left", padx=8)
        ttk.Button(actions, text="退出", command=self.close_requested).pack(side="right")

        ttk.Label(self.shell, text="初始化日志", font=(UI_FONT_FAMILY, 10, "bold")).pack(anchor="w")
        setup_log_frame = ttk.Frame(self.shell)
        setup_log_frame.pack(fill="both", expand=True, pady=(6, 8))
        self.setup_log_text = tk.Text(setup_log_frame, wrap="word", height=10, bg="#10141f", fg="#d8e4ff", insertbackground="white", font=(MONO_FONT_FAMILY, 9), state="disabled")
        scroll = ttk.Scrollbar(setup_log_frame, orient="vertical", command=self.setup_log_text.yview)
        self.setup_log_text.configure(yscrollcommand=scroll.set)
        self.setup_log_text.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")
        ttk.Label(self.shell, textvariable=self.footer_message, style="Muted.TLabel").pack(fill="x")

    def _common_database_form(self, parent: ttk.Frame) -> None:
        frame = ttk.LabelFrame(parent, text="数据库管理员连接（两套业务库共用）", padding=10)
        frame.pack(fill="x", pady=(0, 10))
        fields = (
            ("HOST", "主机", 22, False), ("PORT", "端口", 12, False),
            ("ADMIN_USER", "建库管理员", 22, False), ("ADMIN_PASSWORD", "管理员密码", 26, True),
            ("SERVICE_HOST", "运行账号来源主机", 22, False),
        )
        for column, (suffix, label, width, secret) in enumerate(fields):
            cell = ttk.Frame(frame)
            cell.grid(row=0, column=column, sticky="ew", padx=(0, 10) if column < len(fields) - 1 else 0)
            frame.columnconfigure(column, weight=1)
            ttk.Label(cell, text=label).pack(anchor="w")
            ttk.Entry(cell, textvariable=self.setup_vars[f"ASTER_OPERATIONS_DB_{suffix}"], width=width, show="●" if secret else "").pack(fill="x", pady=(3, 0))
        ttk.Checkbutton(
            frame, text="数据库管理员连接使用 TLS",
            variable=self.setup_vars["ASTER_OPERATIONS_DB_ADMIN_TLS"], onvalue="true", offvalue="false",
        ).grid(row=1, column=0, columnspan=len(fields), sticky="w", pady=(8, 0))

    def _database_identity_form(self, parent: ttk.Frame, prefix: str, title: str, column: int) -> None:
        frame = ttk.LabelFrame(parent, text=title, padding=10)
        frame.grid(row=0, column=column, sticky="nsew", padx=(0, 5) if column == 0 else (5, 0))
        frame.columnconfigure(1, weight=1)
        fields = (
            ("NAME", "数据库名"), ("SERVICE_USER", "运行专用账号"),
        )
        for row, (suffix, label) in enumerate(fields):
            ttk.Label(frame, text=label, width=16).grid(row=row, column=0, sticky="w", pady=3)
            ttk.Entry(frame, textvariable=self.setup_vars[f"{prefix}_{suffix}"]).grid(row=row, column=1, sticky="ew", pady=3)
        ttk.Label(frame, text="初始化时自动创建或重置该专用账号，并只授权访问本业务库。", style="Muted.TLabel", wraplength=480).grid(row=len(fields), column=0, columnspan=2, sticky="w", pady=(6, 0))

    def _security_config_form(self, parent: ttk.Frame) -> None:
        frame = ttk.LabelFrame(parent, text="发布安全配置（可选）", padding=10)
        frame.pack(fill="x", pady=(0, 10))
        frame.columnconfigure(1, weight=1)
        ttk.Label(frame, text="安全配置目录").grid(row=0, column=0, sticky="w", padx=(0, 8))
        entry = ttk.Entry(frame, textvariable=self.setup_vars["ASTER_LOCAL_SECURITY_CONFIG_DIR"])
        entry.grid(row=0, column=1, sticky="ew")
        ttk.Button(frame, text="选择…", command=self._choose_security_config_directory).grid(row=0, column=2, padx=(8, 0))
        ttk.Button(frame, text="清空", command=lambda: self.setup_vars["ASTER_LOCAL_SECURITY_CONFIG_DIR"].set("")).grid(row=0, column=3, padx=(6, 0))
        self.security_config_status = tk.StringVar()
        ttk.Label(frame, textvariable=self.security_config_status, style="Muted.TLabel", wraplength=1060).grid(
            row=1, column=0, columnspan=4, sticky="w", pady=(7, 0),
        )
        self.setup_vars["ASTER_LOCAL_SECURITY_CONFIG_DIR"].trace_add("write", lambda *_args: self._refresh_security_config_status())
        self._refresh_security_config_status()

    def _choose_security_config_directory(self) -> None:
        initial = self.setup_vars["ASTER_LOCAL_SECURITY_CONFIG_DIR"].get().strip()
        selected = filedialog.askdirectory(
            parent=self.root,
            title="选择仓库外安全配置目录",
            mustexist=True,
            initialdir=initial if initial and Path(initial).is_dir() else str(ROOT.parent),
        )
        if selected:
            self.setup_vars["ASTER_LOCAL_SECURITY_CONFIG_DIR"].set(str(Path(selected).resolve()))

    def _refresh_security_config_status(self) -> None:
        raw = self.setup_vars["ASTER_LOCAL_SECURITY_CONFIG_DIR"].get().strip()
        if not raw:
            self.security_config_status.set("未指定：初始化会生成仅供本地开发的 License/Release 密钥，发布中心保持只读。")
            return
        directory = Path(raw).expanduser()
        required = (
            "license-v2.signers.json",
            "license-v2.public-keyring.json",
            "release-v1.public-keyring.json",
        )
        missing = [name for name in required if not (directory / name).is_file()]
        if missing:
            self.security_config_status.set(f"目录不完整，缺少：{', '.join(missing)}")
        elif (directory / "operations-release-center.json").is_file():
            self.security_config_status.set("将导入 License 私钥、License/Release 公钥环和 GitHub App 配置；Release seed 不会读取或复制。")
        else:
            self.security_config_status.set("签名材料完整；未发现 operations-release-center.json，远端验证构建仍保持只读。")

    def _setup_values(self) -> dict[str, str]:
        values = {key: variable.get().strip() for key, variable in self.setup_vars.items()}
        for suffix in ("HOST", "PORT", "ADMIN_USER", "ADMIN_PASSWORD", "ADMIN_TLS", "SERVICE_HOST"):
            values[f"ASTER_CUSTOMER_DB_{suffix}"] = values[f"ASTER_OPERATIONS_DB_{suffix}"]
            self.setup_vars[f"ASTER_CUSTOMER_DB_{suffix}"].set(values[f"ASTER_OPERATIONS_DB_{suffix}"])
        email = values["ASTER_LOCAL_ADMIN_EMAIL"]
        if not re.fullmatch(r"[^\s@]+@[^\s@]+\.[^\s@]+", email):
            raise ValueError("管理员邮箱格式无效")
        for prefix in ("ASTER_OPERATIONS_DB", "ASTER_CUSTOMER_DB"):
            for suffix in ("HOST", "NAME", "ADMIN_USER", "ADMIN_PASSWORD", "SERVICE_USER", "SERVICE_HOST"):
                if not values[f"{prefix}_{suffix}"] or "\n" in values[f"{prefix}_{suffix}"]:
                    raise ValueError(f"{prefix}_{suffix} 不能为空")
            try:
                port = int(values[f"{prefix}_PORT"])
            except ValueError as exc:
                raise ValueError(f"{prefix}_PORT 必须是端口号") from exc
            if port < 1 or port > 65535:
                raise ValueError(f"{prefix}_PORT 必须在 1-65535 之间")
            for suffix in ("NAME", "SERVICE_USER"):
                if not re.fullmatch(r"[A-Za-z0-9_]+", values[f"{prefix}_{suffix}"]):
                    raise ValueError(f"{prefix}_{suffix} 只能包含字母、数字和下划线")
        if values["ASTER_CUSTOMER_DB_NAME"] == values["ASTER_OPERATIONS_DB_NAME"] and values["ASTER_CUSTOMER_DB_HOST"].lower() == values["ASTER_OPERATIONS_DB_HOST"].lower() and values["ASTER_CUSTOMER_DB_PORT"] == values["ASTER_OPERATIONS_DB_PORT"]:
            raise ValueError("Customer 与 Operations 必须使用不同数据库")
        security_directory = values["ASTER_LOCAL_SECURITY_CONFIG_DIR"]
        if security_directory:
            security_path = Path(security_directory).expanduser()
            if not security_path.is_absolute():
                raise ValueError("安全配置目录必须使用绝对路径")
            security_path = security_path.resolve()
            if not security_path.is_dir():
                raise ValueError("安全配置目录不存在或不是目录")
            try:
                security_path.relative_to(ROOT.resolve())
            except ValueError:
                pass
            else:
                raise ValueError("安全配置目录必须位于源码仓库之外")
            values["ASTER_LOCAL_SECURITY_CONFIG_DIR"] = str(security_path)
        return values

    def _running_core_services(self) -> list[str]:
        return [service.name for service in SERVICES if service.core and probe(service_health_url(service), timeout=0.25)]

    def initialize_from_ui(self) -> None:
        if self.setup_process and self.setup_process.poll() is None:
            return
        if not self.node:
            messagebox.showerror("找不到 Node.js", "PATH 中没有找到 node。请安装 Node.js 22.13+ 后重试。")
            return
        try:
            values = self._setup_values()
        except ValueError as exc:
            messagebox.showerror("配置无效", str(exc))
            return
        running_services = [
            service for service in SERVICES
            if service.core and probe(service_health_url(service), timeout=0.25)
        ]
        if running_services:
            state_processes = identity_checked_state_processes(ROOT)
            processes: dict[str, subprocess.Popen[bytes] | RestoredProcess] = {}
            unrecognized: list[str] = []
            for service in running_services:
                candidates: list[subprocess.Popen[bytes] | RestoredProcess] = []
                process = self.processes.get(service.key)
                if process is not None and process.poll() is None:
                    candidates.append(process)
                known_pids = {candidate.pid for candidate in candidates}
                candidates.extend(
                    candidate for candidate in state_processes.get(service.key, [])
                    if candidate.pid not in known_pids
                )
                if not candidates and (external := controllable_external_process(service)) is not None:
                    candidates.append(external)
                if not candidates:
                    unrecognized.append(service.name)
                    continue
                for candidate in candidates:
                    processes[f"{service.key}:{candidate.pid}"] = candidate
            if unrecognized:
                messagebox.showwarning(
                    "无法安全停止服务",
                    "初始化期间不能运行以下服务：\n\n"
                    + "、".join(service.name for service in running_services)
                    + "\n\n其中以下进程无法确认属于 Aster，本工具不会自动结束它们：\n\n"
                    + "、".join(unrecognized)
                    + "\n\n请关闭对应外部终端后重试。",
                )
                return
            self._show_stop_before_setup_dialog(
                values,
                tuple(processes.items()),
                [service.name for service in running_services],
            )
            return
        self._confirm_and_start_initialization(values)

    def _show_stop_before_setup_dialog(
        self,
        values: dict[str, str],
        processes: tuple[tuple[str, subprocess.Popen[bytes] | RestoredProcess], ...],
        service_names: list[str],
    ) -> None:
        dialog = tk.Toplevel(self.root)
        dialog.title("停止服务并继续初始化")
        dialog.transient(self.root)
        dialog.resizable(False, False)
        body = ttk.Frame(dialog, padding=(22, 20, 22, 12))
        body.pack(fill="both", expand=True)
        ttk.Label(
            body,
            text="初始化期间不能运行以下服务",
            font=(UI_FONT_FAMILY, 12, "bold"),
        ).pack(anchor="w")
        ttk.Label(
            body,
            text="、".join(service_names),
            wraplength=500,
            justify="left",
        ).pack(anchor="w", fill="x", pady=(10, 14))
        ttk.Label(
            body,
            text="这些进程已确认属于 Aster。可以直接停止全部服务，然后继续初始化。",
            style="Muted.TLabel",
            wraplength=500,
            justify="left",
        ).pack(anchor="w", fill="x")

        actions = ttk.Frame(dialog, padding=(22, 10, 22, 18))
        actions.pack(fill="x")

        def cancel() -> None:
            dialog.grab_release()
            dialog.destroy()

        def confirm() -> None:
            dialog.grab_release()
            dialog.destroy()
            self._begin_stop_before_setup(values, processes)

        ttk.Button(actions, text="取消", command=cancel).pack(side="right")
        confirm_button = ttk.Button(actions, text="停止服务并继续", command=confirm)
        confirm_button.pack(side="right", padx=(0, 10))
        dialog.protocol("WM_DELETE_WINDOW", cancel)
        dialog.update_idletasks()
        dialog.geometry(centered_window_geometry(
            self.root.winfo_rootx(), self.root.winfo_rooty(),
            self.root.winfo_width(), self.root.winfo_height(),
            560, max(230, dialog.winfo_reqheight()),
            self.root.winfo_vrootx(), self.root.winfo_vrooty(),
            self.root.winfo_screenwidth(), self.root.winfo_screenheight(),
        ))
        dialog.grab_set()
        confirm_button.focus_set()

    def _begin_stop_before_setup(
        self,
        values: dict[str, str],
        processes: tuple[tuple[str, subprocess.Popen[bytes] | RestoredProcess], ...],
    ) -> None:
        self._set_setup_busy(True)
        self._append_setup_log("正在停止运行中的 Aster 服务…")
        threading.Thread(
            target=self._stop_before_setup_worker,
            args=(values, processes),
            daemon=True,
        ).start()

    def _confirm_and_start_initialization(self, values: dict[str, str]) -> None:
        summary = (
            f"Operations：{values['ASTER_OPERATIONS_DB_HOST']}:{values['ASTER_OPERATIONS_DB_PORT']}/{values['ASTER_OPERATIONS_DB_NAME']}\n"
            f"Customer：{values['ASTER_CUSTOMER_DB_HOST']}:{values['ASTER_CUSTOMER_DB_PORT']}/{values['ASTER_CUSTOMER_DB_NAME']}\n\n"
            f"发布安全配置：{values['ASTER_LOCAL_SECURITY_CONFIG_DIR'] or '未指定（使用本地临时信任根，发布中心只读）'}\n\n"
            "若数据库已经存在，将被删除并重建；同名专用账号的密码会被重置。继续吗？"
        )
        if not messagebox.askyesno("确认初始化", summary, icon="warning"):
            return
        try:
            update_env_file(ROOT / ".env", values)
        except ValueError as exc:
            messagebox.showerror("配置无法保存", str(exc))
            return
        self._set_setup_busy(True)
        self._append_setup_log("已保存 .env，正在连接数据库并初始化…")
        threading.Thread(target=self._ui_command_worker, args=("setup_done", "scripts/setup-local-stack-ui.mjs", []), daemon=True).start()

    def _stop_before_setup_worker(
        self,
        values: dict[str, str],
        processes: tuple[tuple[str, subprocess.Popen[bytes] | RestoredProcess], ...],
    ) -> None:
        failures = []
        for process_key, process in processes:
            service_key = process_key.split(":", 1)[0]
            service = SERVICE_BY_KEY[service_key]
            if failure := stop_service_process(service, process):
                failures.append(f"{service.name}：{failure}")
        clean_local_process_states(ROOT)
        remaining = self._running_core_services()
        if failures:
            self.events.put(("setup_stop_failed", "\n\n".join(failures)))
            return
        self.events.put(("setup_stop_done", values, remaining))

    def clear_environment_from_ui(self) -> None:
        if self.setup_process and self.setup_process.poll() is None:
            return
        if not self.node:
            messagebox.showerror("找不到 Node.js", "PATH 中没有找到 node。请安装 Node.js 22.13+ 后重试。")
            return
        running = self._running_core_services()
        if running:
            messagebox.showwarning("请先停止服务", f"清理期间不能运行以下服务：\n\n{'、'.join(running)}")
            return
        if not messagebox.askyesno(
            "清除本地环境",
            "这会删除 Customer 与 Operations 数据库及专用账号，并清理本地授权、Runner、ArtifactStore 和演示交付数据。\n\n根目录 .env 会保留，源码、node_modules、.idea 和 dist 不受影响。确定继续吗？",
            icon="warning",
        ):
            return
        self._set_setup_busy(True)
        self._append_setup_log("正在清除本地数据库和运行数据…")
        threading.Thread(target=self._ui_command_worker, args=("reset_done", "scripts/reset-local-stack-ui.mjs", ["--confirmed-by-local-ui"]), daemon=True).start()

    def _set_setup_busy(self, busy: bool) -> None:
        self.setup_button.configure(state="disabled" if busy else "normal")
        if self.reset_button is not None:
            self.reset_button.configure(state="disabled" if busy else "normal")

    def _ui_command_worker(self, done_event: str, script: str, arguments: list[str]) -> None:
        creationflags = subprocess.CREATE_NO_WINDOW if IS_WINDOWS else 0
        try:
            environment = local_service_environment(
                False,
                None,
                {**os.environ, "ASTER_LOCAL_UI_CONFIRMED": "true"},
            )
            process = subprocess.Popen(
                [self.node, str(ROOT / script), *arguments], cwd=ROOT, env=environment, stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
                creationflags=creationflags,
            )
            self.setup_process = process
            if process.stdout:
                for line in iter(process.stdout.readline, ""):
                    value = line.rstrip("\r\n")
                    if value:
                        self.events.put(("setup_log", value))
            code = process.wait()
            self.events.put((done_event, code))
        except Exception as exc:
            self.events.put((done_event, 1, str(exc)))

    def _append_setup_log(self, value: str) -> None:
        if self.page != "setup" or not hasattr(self, "setup_log_text"):
            return
        self.setup_log_text.configure(state="normal")
        self._insert_terminal_line("__setup__", self.setup_log_text, f"[{time.strftime('%H:%M:%S')}] ", value)
        self.setup_log_text.see("end")
        self.setup_log_text.configure(state="disabled")

    def show_local_admin_credentials(self) -> None:
        if self.credentials_dialog is not None and self.credentials_dialog.winfo_exists():
            self.credentials_dialog.deiconify()
            self.credentials_dialog.lift()
            self.credentials_dialog.focus_set()
            return
        values = self.read_current_local_admin_credentials()
        if not values:
            messagebox.showinfo("本地账号", "没有找到本地管理员账号文件。重新初始化后会自动生成。")
            return
        dialog = tk.Toplevel(self.root)
        dialog.withdraw()
        self.credentials_dialog = dialog
        dialog.title("本地账号")
        dialog.protocol("WM_DELETE_WINDOW", self._close_local_admin_credentials)
        ttk.Label(
            dialog,
            text="可在此更新本地记录的密码和用户侧邮箱；不会修改 Customer、Operations 或用户侧的真实账号密码。",
            style="Subtitle.TLabel", wraplength=660,
        ).pack(anchor="w", padx=18, pady=(16, 10))
        content = ttk.Frame(dialog, padding=(18, 0, 18, 12))
        content.pack(fill="x")
        rows = (
            ("Customer", values.get("ASTER_LOCAL_CUSTOMER_EMAIL", ""), values.get("ASTER_LOCAL_CUSTOMER_PASSWORD", "")),
            ("Operations", values.get("ASTER_LOCAL_OPERATIONS_EMAIL", ""), values.get("ASTER_LOCAL_OPERATIONS_PASSWORD", "")),
            ("用户侧", values.get("ASTER_LOCAL_MEMBER_EMAIL", ""), values.get("ASTER_LOCAL_MEMBER_PASSWORD", "")),
        )
        password_values: dict[str, tk.StringVar] = {}
        member_email_value = tk.StringVar(value=values.get("ASTER_LOCAL_MEMBER_EMAIL", ""))
        for row, (label, email, password) in enumerate(rows):
            ttk.Label(content, text=label, width=14, font=(UI_FONT_FAMILY, 9, "bold")).grid(row=row, column=0, sticky="w", pady=6)
            if label == "用户侧":
                email_entry = ttk.Entry(content, width=28, textvariable=member_email_value)
            else:
                email_entry = ttk.Entry(content, width=28)
                email_entry.insert(0, email)
                email_entry.configure(state="readonly")
            email_entry.grid(row=row, column=1, sticky="ew", padx=4)
            password_value = tk.StringVar(value=password)
            password_values[label] = password_value
            password_entry = ttk.Entry(content, width=32, textvariable=password_value)
            password_entry.grid(row=row, column=2, sticky="ew", padx=4)
            ttk.Button(
                content, text="复制密码", command=lambda value=password_value: self.copy_url(value.get()),
            ).grid(row=row, column=3, padx=4)
        content.columnconfigure(1, weight=1)
        content.columnconfigure(2, weight=1)

        status = tk.StringVar(value="用户侧请填写真实成员邮箱与密码；保存不会验证它们是否与真实账号一致。")
        ttk.Label(dialog, textvariable=status, style="Muted.TLabel", wraplength=660).pack(
            anchor="w", padx=18, pady=(0, 8),
        )

        def save() -> None:
            try:
                member_email = member_email_value.get()
                member_password = password_values["用户侧"].get()
                member_configured = bool(
                    member_email or member_password
                    or values.get("ASTER_LOCAL_MEMBER_EMAIL")
                    or values.get("ASTER_LOCAL_MEMBER_PASSWORD")
                )
                save_local_account_passwords(
                    LOCAL_ADMIN_CREDENTIALS_FILE,
                    password_values["Customer"].get(),
                    password_values["Operations"].get(),
                    member_password if member_configured else None,
                    member_email=member_email if member_configured else None,
                )
            except (OSError, ValueError) as exc:
                messagebox.showerror("无法保存本地账号", str(exc), parent=dialog)
                return
            self.local_admin_credentials_sha256 = ""
            self.read_current_local_admin_credentials()
            status.set("已同步到 local-admin-credentials.env；真实账号密码未被修改。")
            self.footer_message.set("本地账号文件中的记录已更新。")

        actions = ttk.Frame(dialog, padding=(18, 0, 18, 14))
        actions.pack(fill="x")
        ttk.Button(actions, text="保存到文件", command=save).pack(side="left")
        ttk.Button(actions, text="关闭", command=self._close_local_admin_credentials).pack(side="right")
        self._show_modeless_dialog(dialog, 720, 255)

    def _close_local_admin_credentials(self) -> None:
        if self.credentials_dialog is not None:
            self.credentials_dialog.destroy()
        self.credentials_dialog = None

    def read_current_local_admin_credentials(self) -> dict[str, str]:
        if not LOCAL_ADMIN_CREDENTIALS_FILE.is_file():
            self.local_admin_credentials_sha256 = ""
            self.local_admin_credentials = {}
            return {}
        source = LOCAL_ADMIN_CREDENTIALS_FILE.read_bytes()
        digest = hashlib.sha256(source).hexdigest()
        if digest != self.local_admin_credentials_sha256:
            self.local_admin_credentials = parse_env_text(source.decode("utf-8"))
            self.local_admin_credentials_sha256 = digest
        return dict(self.local_admin_credentials)

    def open_local_authorization(self) -> None:
        if self.authorization_dialog is not None and self.authorization_dialog.winfo_exists():
            self.authorization_dialog.deiconify()
            self.authorization_dialog.lift()
            self.authorization_dialog.focus_set()
            return
        if self.authorization_in_progress or (self.demo_process and self.demo_process.poll() is None):
            return
        self._open_authorization_progress(local_authorization_snapshot())

    def _start_local_authorization(self) -> None:
        if self.authorization_in_progress or (self.demo_process and self.demo_process.poll() is None):
            return
        if self.quick_password_dialog is not None and self.quick_password_dialog.winfo_exists():
            self.quick_password_dialog.lift()
            self.quick_password_dialog.focus_set()
            self.authorization_status.set("请先完成或取消“快速一键改密”。")
            return
        missing = [service.name for service in SERVICES if service.core and not self.health.get(service.key, False)]
        if missing:
            detail = f"请先一键启动并等待以下服务就绪：{'、'.join(missing)}"
            self.authorization_status.set(detail)
            messagebox.showwarning("服务尚未就绪", detail, parent=self.authorization_dialog)
            return
        credentials = self.read_current_local_admin_credentials()
        payload = {
            "operations_email": credentials.get("ASTER_LOCAL_OPERATIONS_EMAIL", "").strip(),
            "operations_password": credentials.get("ASTER_LOCAL_OPERATIONS_PASSWORD", ""),
            "customer_email": credentials.get("ASTER_LOCAL_CUSTOMER_EMAIL", "").strip(),
            "customer_password": credentials.get("ASTER_LOCAL_CUSTOMER_PASSWORD", ""),
        }
        self._reset_authorization_progress()
        self.authorization_in_progress = True
        self.demo_button.configure(state="disabled")
        if self.authorization_start_button is not None and self.authorization_start_button.winfo_exists():
            self.authorization_start_button.configure(state="disabled")
        if self.authorization_close_button is not None and self.authorization_close_button.winfo_exists():
            self.authorization_close_button.configure(state="disabled")
        self.footer_message.set("正在检查本地账号、当前密码和首次改密状态…")
        self._set_authorization_step("credentials", "running", "正在验证 Operations 与 Customer 本地账号")
        threading.Thread(
            target=self._validate_demo_credentials_worker, args=(payload,), daemon=True,
        ).start()

    def _validate_demo_credentials_worker(self, payload: dict[str, str]) -> None:
        try:
            missing = [key for key, value in payload.items() if not value]
            error = None
            if missing:
                error = "本地账号文件中的邮箱或密码不完整。请先在“查看本地账号”中补全密码记录。"
            if not error:
                errors = [
                    validate_operations_credentials(payload["operations_email"], payload["operations_password"]),
                    validate_customer_credentials(payload["customer_email"], payload["customer_password"]),
                ]
                error = "\n".join(value for value in errors if value) or None
        except Exception as exc:
            error = f"本地账号预检异常：{exc}"
        self.events.put(("demo_credentials_checked", error))

    def _show_quick_password_change(self, reason: str) -> None:
        if self.quick_password_dialog is not None and self.quick_password_dialog.winfo_exists():
            self.quick_password_dialog.deiconify()
            self.quick_password_dialog.lift()
            self.quick_password_dialog.focus_set()
            return
        dialog = tk.Toplevel(self.root)
        dialog.withdraw()
        dialog.title("快速一键改密")
        dialog.resizable(False, False)
        self.quick_password_dialog = dialog
        self.quick_password_status = tk.StringVar(value=reason)
        self.quick_password_required_accounts = password_change_required_accounts(reason)
        new_password = tk.StringVar(value="")
        show_password = tk.BooleanVar(value=False)
        dialog.protocol("WM_DELETE_WINDOW", self._close_quick_password_change)

        header = ttk.Frame(dialog, padding=(18, 16, 18, 10))
        header.pack(fill="x")
        ttk.Label(header, text="快速一键改密", style="Title.TLabel").pack(anchor="w")
        ttk.Label(
            header,
            text="输入一次新密码，同时修改 Customer 与 Operations 管理员密码，并把成功结果同步到本地凭据文件。",
            style="Subtitle.TLabel", wraplength=600,
        ).pack(anchor="w", pady=(3, 0))

        form = ttk.Frame(dialog, padding=(18, 0, 18, 8))
        form.pack(fill="x")
        ttk.Label(form, text="两个账号的新密码").grid(row=0, column=0, sticky="w", pady=5)
        password_entry = ttk.Entry(form, textvariable=new_password, show="●", width=46)
        password_entry.grid(row=1, column=0, sticky="ew", pady=(0, 5))
        ttk.Checkbutton(
            form, text="显示密码", variable=show_password,
            command=lambda: password_entry.configure(show="" if show_password.get() else "●"),
        ).grid(row=2, column=0, sticky="w")
        ttk.Label(
            form, text="要求 12—72 字节；若某个账号已是该密码，将只修改另一个账号。",
            style="Muted.TLabel",
        ).grid(row=3, column=0, sticky="w", pady=(5, 0))
        ttk.Label(
            form, textvariable=self.quick_password_status, style="Muted.TLabel", wraplength=600,
        ).grid(row=4, column=0, sticky="w", pady=(8, 0))
        form.columnconfigure(0, weight=1)

        def submit() -> None:
            value = new_password.get()
            error = quick_password_validation_error(value)
            credentials = self.read_current_local_admin_credentials()
            current_by_account = {
                "Customer": credentials.get("ASTER_LOCAL_CUSTOMER_PASSWORD", ""),
                "Operations": credentials.get("ASTER_LOCAL_OPERATIONS_PASSWORD", ""),
            }
            conflicts = [
                label for label in self.quick_password_required_accounts
                if current_by_account.get(label) == value
            ]
            if not error and conflicts:
                error = f"新密码必须不同于仍待首次改密的账号：{'、'.join(sorted(conflicts))}。"
            if error:
                self.quick_password_status.set(error)
                return
            payload = {
                "operations_email": credentials.get("ASTER_LOCAL_OPERATIONS_EMAIL", "").strip(),
                "operations_password": credentials.get("ASTER_LOCAL_OPERATIONS_PASSWORD", ""),
                "customer_email": credentials.get("ASTER_LOCAL_CUSTOMER_EMAIL", "").strip(),
                "customer_password": credentials.get("ASTER_LOCAL_CUSTOMER_PASSWORD", ""),
            }
            if not all(payload.values()):
                self.quick_password_status.set("本地账号文件中的邮箱或密码不完整，无法执行快速改密。")
                return
            self.quick_password_in_progress = True
            self.quick_password_status.set("正在修改两个本地管理员账号…")
            self.quick_password_confirm_button.configure(state="disabled")
            self.quick_password_cancel_button.configure(state="disabled")
            threading.Thread(
                target=self._quick_password_change_worker, args=(payload, value), daemon=True,
            ).start()

        actions = ttk.Frame(dialog, padding=(18, 4, 18, 16))
        actions.pack(fill="x")
        self.quick_password_confirm_button = ttk.Button(actions, text="确定", command=submit)
        self.quick_password_confirm_button.pack(side="left")
        self.quick_password_cancel_button = ttk.Button(
            actions, text="取消", command=self._close_quick_password_change,
        )
        self.quick_password_cancel_button.pack(side="right")
        dialog.bind("<Return>", lambda _event: submit())
        dialog.bind("<Escape>", lambda _event: self._close_quick_password_change())
        self._show_modeless_dialog(dialog, 650, 330)
        password_entry.focus_set()

    def _close_quick_password_change(self) -> None:
        if self.quick_password_in_progress:
            return
        if self.quick_password_dialog is not None:
            self.quick_password_dialog.destroy()
        self.quick_password_dialog = None
        self.quick_password_status = None
        self.quick_password_confirm_button = None
        self.quick_password_cancel_button = None
        self.quick_password_required_accounts.clear()

    def _quick_password_change_worker(self, payload: dict[str, str], new_password: str) -> None:
        errors: list[str] = []
        changed: list[str] = []
        targets = (
            (
                "Operations", "operations_email", "operations_password",
                "ASTER_LOCAL_OPERATIONS_PASSWORD", change_operations_password,
            ),
            (
                "Customer", "customer_email", "customer_password",
                "ASTER_LOCAL_CUSTOMER_PASSWORD", change_customer_password,
            ),
        )
        for label, email_key, password_key, environment_key, change_password in targets:
            current_password = payload[password_key]
            if current_password == new_password:
                changed.append(label)
                continue
            try:
                error = change_password(payload[email_key], current_password, new_password)
            except Exception as exc:
                error = f"{label} 修改异常：{exc}"
            if error:
                errors.append(error)
                continue
            changed.append(label)
            try:
                update_env_file(LOCAL_ADMIN_CREDENTIALS_FILE, {environment_key: new_password})
            except (OSError, ValueError) as exc:
                errors.append(f"{label} 密码已经修改，但同步本地凭据文件失败：{exc}")
        self.events.put(("quick_password_change_done", errors, changed))

    def _open_authorization_progress(self, snapshot: LocalAuthorizationSnapshot) -> None:
        if self.authorization_dialog is not None and self.authorization_dialog.winfo_exists():
            self.authorization_dialog.destroy()
        dialog = tk.Toplevel(self.root)
        dialog.withdraw()
        dialog.title("本地快速授权")
        dialog.minsize(880, 540)
        dialog.columnconfigure(0, weight=1)
        dialog.rowconfigure(1, weight=1)
        self.authorization_dialog = dialog
        self.authorization_step_rows.clear()
        self.authorization_step_started.clear()
        self.authorization_active_step = None
        self.authorization_detail_values.clear()
        self.authorization_detail_groups.clear()
        prompt, button_text = local_authorization_prompt(snapshot)
        self.authorization_status.set(
            "本地快速授权已完成，可关闭窗口。" if not button_text else "等待用户确认；尚未执行任何操作。",
        )
        dialog.protocol("WM_DELETE_WINDOW", self._authorization_close_requested)

        header = ttk.Frame(dialog, padding=(18, 12, 18, 8))
        header.grid(row=0, column=0, sticky="ew")
        ttk.Label(header, text="本地快速授权", style="Title.TLabel").pack(anchor="w")
        ttk.Label(
            header,
            text="授权操作仅在确认后执行；已完成时可直接关闭窗口。",
            style="Subtitle.TLabel", wraplength=1040,
        ).pack(anchor="w", pady=(3, 0))
        ttk.Label(header, text=prompt, style="Muted.TLabel", wraplength=1040).pack(anchor="w", pady=(6, 0))

        body = ttk.Frame(dialog, padding=(18, 0, 18, 10))
        body.grid(row=1, column=0, sticky="nsew")
        body.columnconfigure(0, minsize=275)
        body.columnconfigure(1, weight=1)
        body.rowconfigure(0, weight=1)

        steps = ttk.LabelFrame(body, text="执行步骤", padding=(12, 8))
        steps.grid(row=0, column=0, sticky="nsew", padx=(0, 12))
        for row_index, (key, title) in enumerate(AUTHORIZATION_STEPS):
            icon = tk.Label(steps, text="○", fg="#9aa4b2", font=(SYMBOL_FONT_FAMILY, 13), width=2, anchor="center")
            icon.grid(row=row_index, column=0, sticky="n", pady=(2, 4))
            content = ttk.Frame(steps)
            content.grid(row=row_index, column=1, sticky="ew", pady=(0, 4))
            ttk.Label(content, text=title, font=(UI_FONT_FAMILY, 9, "bold")).pack(anchor="w")
            status = tk.StringVar(value="未开始")
            ttk.Label(content, textvariable=status, style="Muted.TLabel").pack(anchor="w")
            self.authorization_step_rows[key] = {"icon": icon, "status": status}
        steps.columnconfigure(1, weight=1)

        detail_box = ttk.LabelFrame(body, text="执行详情", padding=(8, 8))
        detail_box.grid(row=0, column=1, sticky="nsew")
        detail_box.columnconfigure(0, weight=1)
        detail_box.rowconfigure(0, weight=3)
        detail_box.rowconfigure(1, weight=2, minsize=110)
        detail_table = ttk.Frame(detail_box)
        detail_table.grid(row=0, column=0, sticky="nsew")
        detail_table.columnconfigure(0, weight=1)
        detail_table.rowconfigure(0, weight=1)
        detail = ttk.Treeview(
            detail_table, columns=("time", "stage", "detail"), show="headings",
            style="Authorization.Treeview", selectmode="browse",
        )
        detail.heading("time", text="时间")
        detail.heading("stage", text="阶段")
        detail.heading("detail", text="详情摘要")
        detail.column("time", width=80, minwidth=70, stretch=False, anchor="w")
        detail.column("stage", width=135, minwidth=115, stretch=False, anchor="w")
        detail.column("detail", width=480, minwidth=260, stretch=True, anchor="w")
        detail.tag_configure("completed", foreground="#157347")
        detail.tag_configure("running", foreground="#1d5fa7")
        detail.tag_configure("failed", foreground="#a12622")
        scroll = ttk.Scrollbar(detail_table, orient="vertical", command=detail.yview)
        detail.configure(yscrollcommand=scroll.set)
        detail.grid(row=0, column=0, sticky="nsew")
        scroll.grid(row=0, column=1, sticky="ns")
        detail.bind("<<TreeviewSelect>>", self._authorization_detail_selected)
        self.authorization_detail = detail

        selected_box = ttk.LabelFrame(detail_box, text="完整详情", padding=(6, 6))
        selected_box.grid(row=1, column=0, sticky="nsew", pady=(8, 0))
        selected_box.columnconfigure(0, weight=1)
        selected_box.rowconfigure(0, weight=1)
        detail_text = tk.Text(
            selected_box, height=5, wrap="word", state="disabled", relief="flat",
            font=("Cascadia Mono", 9), padx=6, pady=5,
        )
        detail_text_scroll = ttk.Scrollbar(selected_box, orient="vertical", command=detail_text.yview)
        detail_text.configure(yscrollcommand=detail_text_scroll.set)
        detail_text.grid(row=0, column=0, sticky="nsew")
        detail_text_scroll.grid(row=0, column=1, sticky="ns")
        self.authorization_detail_text = detail_text

        footer = ttk.Frame(dialog, padding=(18, 0, 18, 12))
        footer.grid(row=2, column=0, sticky="ew")
        footer.columnconfigure(0, weight=1)
        ttk.Label(footer, textvariable=self.authorization_status, style="Muted.TLabel").grid(
            row=0, column=0, sticky="w",
        )
        self.authorization_close_button = ttk.Button(footer, text="关闭", command=self._authorization_close_requested)
        self.authorization_close_button.grid(row=0, column=2, sticky="e")
        self.authorization_start_button = None
        if button_text:
            self.authorization_start_button = ttk.Button(
                footer, text=button_text, command=self._start_local_authorization,
            )
            self.authorization_start_button.grid(row=0, column=1, sticky="e", padx=(8, 8))
        self._show_modeless_dialog(dialog, 1080, 620)

    def _reset_authorization_progress(self) -> None:
        self.authorization_step_started.clear()
        self.authorization_active_step = None
        self.authorization_detail_values.clear()
        self.authorization_detail_groups.clear()
        for row in self.authorization_step_rows.values():
            row["icon"].configure(text="○", fg="#9aa4b2")
            row["status"].set("未开始")
        if self.authorization_detail is not None and self.authorization_detail.winfo_exists():
            self.authorization_detail.delete(*self.authorization_detail.get_children())
        if self.authorization_detail_text is not None and self.authorization_detail_text.winfo_exists():
            self.authorization_detail_text.configure(state="normal")
            self.authorization_detail_text.delete("1.0", "end")
            self.authorization_detail_text.configure(state="disabled")

    def _authorization_close_requested(self) -> None:
        if self.authorization_in_progress or getattr(self, "quick_password_in_progress", False):
            messagebox.showinfo("授权流程执行中", "请等待当前步骤完成后再关闭控制台。", parent=self.authorization_dialog)
            return
        self._close_quick_password_change()
        if self.authorization_dialog is not None:
            self.authorization_dialog.destroy()
        self.authorization_dialog = None
        self.authorization_start_button = None
        self.authorization_close_button = None

    def _set_authorization_step(self, step: str, state: str, detail: str) -> None:
        row = self.authorization_step_rows.get(step)
        if row is None:
            return
        now = time.monotonic()
        if state == "running":
            self.authorization_active_step = step
            self.authorization_step_started.setdefault(step, now)
            icon, color, status = "●", "#2f80d1", "执行中"
        elif state == "completed":
            started = self.authorization_step_started.get(step)
            elapsed = f" · {now - started:.1f} 秒" if started is not None else ""
            icon, color, status = "✓", "#2e9d45", f"已完成{elapsed}"
            if self.authorization_active_step == step:
                self.authorization_active_step = None
        else:
            started = self.authorization_step_started.get(step)
            elapsed = f" · {now - started:.1f} 秒" if started is not None else ""
            icon, color, status = "✖", "#c43c35", f"失败{elapsed}"
            self.authorization_active_step = step
        row["icon"].configure(text=icon, fg=color)
        row["status"].set(status)
        self.authorization_status.set(authorization_detail_summary(detail))
        self._append_authorization_detail(step, state, detail)

    def _append_authorization_detail(
        self, step: str, state: str, detail: str, *, group: str | None = None,
    ) -> None:
        if self.authorization_detail is None or not self.authorization_detail.winfo_exists():
            return
        title = dict(AUTHORIZATION_STEPS).get(step, step)
        timestamp = time.strftime("%H:%M:%S")
        normalized = "\n".join(authorization_detail_lines(detail))
        item = self.authorization_detail_groups.get(group) if group else None
        if item and self.authorization_detail.exists(item):
            previous = self.authorization_detail_values.get(item, "")
            normalized = f"{previous}\n{normalized}" if previous else normalized
            if len(normalized) > 20_000:
                normalized = normalized[-20_000:]
            values = self.authorization_detail.item(item, "values")
            original_time = values[0] if values else timestamp
            self.authorization_detail.item(
                item,
                values=(original_time, title, authorization_detail_summary(normalized)),
                tags=(state,),
            )
        else:
            item = self.authorization_detail.insert(
                "", "end", values=(timestamp, title, authorization_detail_summary(normalized)), tags=(state,),
            )
            if group:
                self.authorization_detail_groups[group] = item
        self.authorization_detail_values[item] = normalized
        self.authorization_detail.selection_set(item)
        self.authorization_detail.focus(item)
        self.authorization_detail.see(item)
        self._authorization_detail_selected()

    def _authorization_detail_selected(self, _event: tk.Event | None = None) -> None:
        if (
            self.authorization_detail is None
            or self.authorization_detail_text is None
            or not self.authorization_detail.winfo_exists()
            or not self.authorization_detail_text.winfo_exists()
        ):
            return
        selection = self.authorization_detail.selection()
        value = self.authorization_detail_values.get(selection[0], "") if selection else ""
        self.authorization_detail_text.configure(state="normal")
        self.authorization_detail_text.delete("1.0", "end")
        self.authorization_detail_text.insert("1.0", value)
        self.authorization_detail_text.configure(state="disabled")

    def _finish_authorization_progress(self, success: bool, _detail: str) -> None:
        if not success and self.authorization_active_step:
            self._set_authorization_step(self.authorization_active_step, "failed", _detail)
        self.authorization_status.set(
            "本地快速授权完成，可关闭窗口。" if success else authorization_detail_summary(_detail),
        )
        if self.authorization_start_button is not None and self.authorization_start_button.winfo_exists():
            if success:
                self.authorization_start_button.pack_forget()
                self.authorization_start_button = None
            else:
                self.authorization_start_button.configure(state="normal", text="重新检查并执行")
        if self.authorization_close_button is not None and self.authorization_close_button.winfo_exists():
            self.authorization_close_button.configure(state="normal")

    def _demo_authorization_worker(self) -> None:
        creationflags = subprocess.CREATE_NO_WINDOW if IS_WINDOWS else 0
        recent: list[str] = []
        current_step = "operations"
        try:
            commands = (
                ("operations", [self.npm, "run", "seed:local"], "开始调用 Operations 离线签发接口", ""),
                (
                    "member", [self.npm, "run", "seed:local-member"],
                    "正在通过 Customer Admin 创建或复用 test@at.com，并补齐密码与测试额度",
                    "用户侧测试账号与初始额度已就绪，本地状态已安全写入 local-admin-credentials.env",
                ),
            )
            for step, command, start_detail, completed_detail in commands:
                current_step = step
                self.events.put(("demo_progress", step, "running", start_detail))
                process = subprocess.Popen(
                    command, cwd=ROOT, stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                    encoding="utf-8", errors="replace", creationflags=creationflags,
                    env=local_service_environment(False, None),
                )
                self.demo_process = process
                if process.stdout:
                    for line in iter(process.stdout.readline, ""):
                        value = line.rstrip("\r\n")
                        if not value:
                            continue
                        plain_value = terminal_plain_text(value)
                        recent.append(plain_value)
                        if len(recent) > 20:
                            del recent[:-12]
                        parsed = parse_authorization_progress(plain_value)
                        if parsed:
                            current_step = parsed[0]
                            self.events.put(("demo_progress", *parsed))
                            continue
                        self.events.put(("log", "operations_api", f"[本地授权] {value}"))
                        self.events.put(("demo_detail", current_step, plain_value))
                code = process.wait()
                if code != 0:
                    summary = "\n".join(recent[-8:]) or f"命令退出码 {code}"
                    self.events.put(("demo_done", code, summary))
                    return
                if completed_detail:
                    self.events.put(("demo_progress", step, "completed", completed_detail))
            self.events.put(("demo_done", 0))
        except Exception as exc:
            self.events.put(("demo_detail", current_step, str(exc)))
            self.events.put(("demo_done", 1, str(exc)))

    def request_reinitialize(self) -> None:
        self._show_setup()

    def toggle_lan_access(self) -> None:
        enabled = bool(self.lan_access_enabled.get())
        host = detect_lan_ipv4() if enabled else None
        if enabled and host is None:
            self.lan_access_enabled.set(False)
            self.footer_message.set("未检测到可用的局域网 IPv4 地址，局域网访问未开启。")
            messagebox.showerror("无法开启局域网访问", "未检测到可用的局域网 IPv4 地址。请先连接局域网后重试。")
            return
        self.lan_access = enabled
        self.lan_host = host
        self._save_process_state()
        managed = [
            key for key, process in self.processes.items()
            if process.poll() is None
        ]
        external = [
            service.name for service in SERVICES
            if self.health.get(service.key, False) and service.key not in managed
        ]
        self._show_main()
        for key in managed:
            self.restart_service(key)
        mode = f"局域网 HTTP 访问已开启：{host}" if enabled else "局域网访问已关闭"
        if managed:
            mode += "；正在重启已管理服务。"
        else:
            mode += "；下次启动服务时生效。"
        self.footer_message.set(mode)
        if external:
            messagebox.showwarning(
                "部分服务需要手工重启",
                f"以下服务不由本控制台管理，无法自动重启：{'、'.join(external)}。\n\n请先停止它们，再通过本控制台启动。",
            )

    def selected_services(self) -> list[ServiceSpec]:
        selected = [service for service in SERVICES if service.core]
        if self.include_website.get():
            selected.append(SERVICE_BY_KEY["website"])
        if self.include_runner.get():
            selected.append(SERVICE_BY_KEY["runner"])
        return selected

    def start_all(self) -> None:
        for service in self.selected_services():
            self.start_service(service.key)

    def restart_all(self) -> None:
        managed = [key for key, process in self.processes.items() if process.poll() is None]
        if not managed:
            self.footer_message.set("当前没有由本控制台管理的运行中服务。")
            return
        for key in managed:
            self.restart_service(key)

    def stop_all(self) -> None:
        for service in SERVICES:
            process = self.processes.get(service.key)
            if (process is not None and process.poll() is None) or self.health.get(service.key, False):
                self.stop_service(service.key)

    def start_service(self, key: str) -> None:
        service = SERVICE_BY_KEY[key]
        if key in self.busy or self.closing:
            return
        current = self.processes.get(key)
        if current and current.poll() is None:
            self.footer_message.set(f"{service.name} 已由本控制台管理。")
            return
        missing = missing_files(service)
        if missing:
            paths = "、".join(missing)
            self.footer_message.set(f"{service.name} 缺少 {paths}")
            messagebox.showwarning("缺少初始化文件", f"{service.name} 无法启动。\n\n缺少：{paths}\n\n请先使用本工具完成初始化和本地快速授权；也可以在 Customer Admin 手工注册 Runner。")
            return
        if not self.npm:
            messagebox.showerror("找不到 npm", "PATH 中没有找到 npm。请安装 Node.js 22.13+ 后重新打开控制台。")
            return
        self.failures.pop(key, None)
        self.stop_failures.pop(key, None)
        self.recent_output[key] = []
        self.busy.add(key)
        self.refresh_rows()
        threading.Thread(
            target=self._launch_worker,
            args=(service, False, self.lan_access, self.lan_host),
            daemon=True,
        ).start()

    def restart_service(self, key: str) -> None:
        service = SERVICE_BY_KEY[key]
        if key in self.busy or self.closing:
            return
        process = self.processes.get(key)
        if not process or process.poll() is not None:
            if self.health.get(key):
                self.footer_message.set(f"{service.name} 不是由本工具启动，无法安全重启。")
                return
            self.start_service(key)
            return
        self.busy.add(key)
        self.stop_failures.pop(key, None)
        self.refresh_rows()
        threading.Thread(
            target=self._restart_worker,
            args=(service, process, self.lan_access, self.lan_host),
            daemon=True,
        ).start()

    def stop_service(self, key: str) -> None:
        service = SERVICE_BY_KEY[key]
        if key in self.busy or self.closing:
            return
        process = self.processes.get(key)
        if not process or process.poll() is not None:
            if not self.health.get(key, False):
                self.footer_message.set(f"{service.name} 当前没有运行。")
                return
            self.busy.add(key)
            self.stop_failures.pop(key, None)
            self.refresh_rows()
            self.events.put(("log", key, "正在核验外部监听进程…"))
            threading.Thread(target=self._stop_external_worker, args=(service,), daemon=True).start()
            return
        self.busy.add(key)
        self.stop_failures.pop(key, None)
        self.refresh_rows()
        self.events.put(("log", key, "正在停止进程树…"))
        threading.Thread(target=self._stop_worker, args=(service, process), daemon=True).start()

    def _launch_worker(
        self,
        service: ServiceSpec,
        ignore_health: bool,
        lan_access: bool,
        lan_host: str | None,
    ) -> None:
        configuration_error = service_configuration_error(service)
        if configuration_error:
            self.events.put(("launch_error", service.key, configuration_error))
            return
        outcome = probe_result(service_health_url(service), service.expected_health_service)
        if not ignore_health and outcome.healthy:
            self.events.put(("external", service.key))
            return
        conflict = port_conflict_detail(service, outcome.detail)
        if conflict:
            self.events.put(("launch_error", service.key, conflict))
            return
        command = [self.npm, *service.npm_arguments]
        creationflags = 0
        popen_options: dict[str, object] = {}
        if IS_WINDOWS:
            creationflags = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW
        else:
            popen_options["start_new_session"] = True
        log_path = service_log_path(service.key)
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_output = None
        try:
            environment = local_service_environment(
                lan_access,
                lan_host,
                {**os.environ, "ASTER_LOCAL_DEV_MANAGER": "true"},
            )
            log_output = log_path.open("w", encoding="utf-8", newline="", buffering=1)
            process = subprocess.Popen(
                command,
                cwd=ROOT,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=log_output,
                stderr=subprocess.STDOUT,
                creationflags=creationflags,
                **popen_options,
            )
        except Exception as exc:
            self.events.put(("launch_error", service.key, str(exc)))
            return
        finally:
            if log_output is not None:
                log_output.close()
        with self.process_state_lock:
            self.processes[service.key] = process
            self._save_process_state()
        self.events.put(("started", service.key, process.pid))
        self._start_log_reader(service, process, restored=False)
        threading.Thread(target=self._watch_process, args=(service, process), daemon=True).start()

    def _start_log_reader(
        self, service: ServiceSpec, process: subprocess.Popen[bytes] | RestoredProcess, *, restored: bool,
    ) -> None:
        path = service_log_path(service.key)
        if not path.is_file():
            return
        threading.Thread(
            target=self._read_output_file, args=(service, process, path, restored), daemon=True,
        ).start()

    def _read_output_file(
        self, service: ServiceSpec, process: subprocess.Popen[bytes] | RestoredProcess,
        path: Path, restored: bool,
    ) -> None:
        try:
            with path.open("r", encoding="utf-8", errors="replace") as output:
                if restored:
                    offset = max(0, path.stat().st_size - RESTORED_LOG_BYTES)
                    output.seek(offset)
                    if offset:
                        output.readline()
                while not self.closing:
                    line = output.readline()
                    if line:
                        value = line.rstrip("\r\n")
                        if service.preparation_marker and value == service.preparation_marker:
                            self.events.put(("prepared", service.key, process.pid))
                        elif value:
                            self.events.put(("log", service.key, value))
                        continue
                    if process.poll() is not None:
                        return
                    time.sleep(0.08)
        except OSError as exc:
            self.events.put(("log", service.key, f"日志文件读取失败：{exc}"))

    def _watch_process(self, service: ServiceSpec, process: subprocess.Popen[bytes] | RestoredProcess) -> None:
        code = process.wait()
        self.events.put(("exited", service.key, code, process.pid))

    def _stop_worker(self, service: ServiceSpec, process: subprocess.Popen[bytes] | RestoredProcess) -> None:
        failure = self._terminate_process_tree(service, process)
        self.events.put(("stop_failed", service.key, failure) if failure else ("stopped", service.key))

    def _stop_external_worker(self, service: ServiceSpec) -> None:
        process = controllable_external_process(service)
        if process is None:
            _summary, detail = external_process_diagnostics(service)
            self.events.put((
                "stop_failed",
                service.key,
                "无法安全停止外部进程。控制台不能确认监听者属于当前 Aster 工作树；"
                "若详情显示权限不足，请使用与启动服务相同权限级别的控制台重试。\n\n" + detail,
            ))
            return
        with self.process_state_lock:
            self.processes[service.key] = process
            self._save_process_state()
        self.events.put(("log", service.key, f"已确认外部 Aster 进程 PID {process.pid}，正在停止进程树…"))
        self._stop_worker(service, process)

    def _restart_worker(
        self,
        service: ServiceSpec,
        process: subprocess.Popen[bytes] | RestoredProcess,
        lan_access: bool,
        lan_host: str | None,
    ) -> None:
        self.events.put(("log", service.key, "正在重启进程树…"))
        failure = self._terminate_process_tree(service, process)
        if failure:
            self.events.put(("stop_failed", service.key, "无法重启：旧服务停止失败。\n" + failure))
            return
        time.sleep(0.25)
        self._launch_worker(service, True, lan_access, lan_host)

    def _terminate_process_tree(
        self,
        service: ServiceSpec,
        process: subprocess.Popen[bytes] | RestoredProcess,
    ) -> str | None:
        return stop_service_process(service, process)

    def _schedule_health_probe(self) -> None:
        if self.closing:
            return
        if self.page == "main" and not self.health_probe_running:
            self.health_probe_running = True
            threading.Thread(target=self._health_worker, daemon=True).start()
        self.root.after(1800, self._schedule_health_probe)

    def _health_worker(self) -> None:
        result = {}
        for service in SERVICES:
            process = self.processes.get(service.key)
            managed = process is not None and process.poll() is None
            configuration_error = service_configuration_error(service)
            health_url = service_health_url(service)
            if configuration_error:
                outcome, takeover = ProbeResult(False, configuration_error), None
            elif health_url:
                outcome = probe_result(health_url, service.expected_health_service)
                takeover = controllable_external_process(service) if outcome.healthy and not managed else None
            else:
                takeover = controllable_external_process(service) if not managed else None
                outcome = ProbeResult(managed or takeover is not None)
            external_detail = (
                external_process_diagnostics(service)
                if outcome.healthy and not managed and takeover is None
                else ("", "")
            )
            result[service.key] = (outcome, takeover, external_detail)
        self.events.put(("health", result))

    def _drain_events(self) -> None:
        try:
            while True:
                event = self.events.get_nowait()
                kind = event[0]
                if kind == "log":
                    self.append_log(event[1], event[2])
                elif kind == "setup_log":
                    self._append_setup_log(event[1])
                elif kind == "integration_prs":
                    self.integration_pull_requests = event[1]
                    (
                        self.integration_selected, self.integration_seen, self.integration_drafts,
                    ) = local_pr_integration.reconcile_selection(
                        self.integration_pull_requests, self.integration_selected,
                        self.integration_seen, self.integration_drafts,
                    )
                    self._load_applied_integration_state()
                    self._save_pr_integration_settings()
                    self._set_integration_busy(False)
                    self._render_pull_requests()
                elif kind == "integration_applied":
                    result: local_pr_integration.IntegrationResult = event[1]
                    self.integration_applied = {item.number: item.head_sha for item in result.entries}
                    self._set_integration_busy(False)
                    self._render_pull_requests()
                    if result.replayed:
                        detail = f"复用前 {result.reused} 个检查点，重新合并 {result.replayed} 个 PR。"
                    elif result.recovery_ref:
                        detail = "已按当前选择恢复到 main 基线。"
                    else:
                        detail = "当前组合已经是最新状态，没有执行新的合并。"
                    if result.recovery_ref:
                        detail += "原集成提交已保留为本地恢复备份。"
                    self.integration_status.set(f"集成分支已更新：{detail}")
                elif kind == "integration_progress":
                    self.integration_status.set(event[1])
                elif kind == "integration_error":
                    self._set_integration_busy(False)
                    self._load_applied_integration_state()
                    self._render_pull_requests()
                    self.integration_status.set(event[1])
                    if self.integration_dialog is not None and self.integration_dialog.winfo_exists():
                        messagebox.showerror(event[1], event[2], parent=self.integration_dialog)
                elif kind == "setup_done":
                    self.setup_process = None
                    if event[1] == 0 and initialization_state() == "complete":
                        self.footer_message.set("初始化完成。可以一键启动核心服务。")
                        self._show_main()
                        messagebox.showinfo("初始化完成", "本地数据库、运行配置和签名密钥已生成。管理员账号已保存，可在主页面点击“查看本地账号”。")
                    else:
                        self._set_setup_busy(False)
                        detail = event[2] if len(event) > 2 else "请查看初始化日志。"
                        self._append_setup_log(f"初始化失败：{detail}")
                        messagebox.showerror("初始化失败", detail)
                elif kind == "setup_stop_failed":
                    self._set_setup_busy(False)
                    self._append_setup_log(f"服务停止失败：{event[1]}")
                    messagebox.showerror("服务停止失败", event[1])
                elif kind == "setup_stop_done":
                    self._set_setup_busy(False)
                    remaining = event[2]
                    if remaining:
                        detail = "、".join(remaining)
                        self._append_setup_log(f"服务未能全部停止：{detail}")
                        messagebox.showerror(
                            "服务停止失败",
                            f"以下服务仍在运行：\n\n{detail}\n\n请关闭对应终端后重试初始化。",
                        )
                    else:
                        self._append_setup_log("运行中的服务已全部停止。")
                        self._confirm_and_start_initialization(event[1])
                elif kind == "reset_done":
                    self.setup_process = None
                    if event[1] == 0:
                        self.footer_message.set("本地环境已清理，现在可以从头初始化。")
                        self._show_setup()
                        self._append_setup_log("本地环境已恢复为未初始化状态。")
                    else:
                        self._set_setup_busy(False)
                        detail = event[2] if len(event) > 2 else "请查看清理日志。"
                        self._append_setup_log(f"清理失败：{detail}")
                        messagebox.showerror("清理失败", detail)
                elif kind == "demo_done":
                    self.demo_process = None
                    self.authorization_in_progress = False
                    self.demo_button.configure(state="normal")
                    if event[1] == 0:
                        self.footer_message.set("本地快速授权完成，免费证书、许可证与用户侧测试账号均已就绪。")
                        self._finish_authorization_progress(True, "全部步骤完成；本地测试免费证书已生成，许可证已生效，用户侧测试账号已就绪。")
                        messagebox.showinfo(
                            "授权完成",
                            "本地测试免费证书已经生成，本地许可证已经生效，Runner 与用户侧测试账号也已补齐。\n\n授权已经完成，可关闭本窗口。免费证书、离线申请与许可证副本位于 data/local/demo-delivery。",
                            parent=self.authorization_dialog,
                        )
                    else:
                        detail = event[2] if len(event) > 2 else "请切换到 Ops API 日志页查看详情。"
                        self.footer_message.set("本地授权失败。")
                        self._finish_authorization_progress(False, detail)
                        messagebox.showerror("授权失败", detail, parent=self.authorization_dialog)
                elif kind == "demo_progress":
                    self._set_authorization_step(event[1], event[2], event[3])
                elif kind == "demo_detail":
                    self._append_authorization_detail(
                        event[1], "running", event[2], group=f"process-output:{event[1]}",
                    )
                elif kind == "demo_credentials_checked":
                    error = event[1]
                    if error:
                        self.authorization_in_progress = False
                        self.demo_button.configure(state="normal")
                        self.footer_message.set("本地账号预检失败，尚未执行授权操作。")
                        self._finish_authorization_progress(False, error)
                        if authorization_requires_password_change(error):
                            self._show_quick_password_change(error)
                        continue
                    self._set_authorization_step(
                        "credentials", "completed", "Operations 与 Customer 当前密码有效，且均已完成首次改密",
                    )
                    self.footer_message.set("本地账号预检通过，正在执行本地快速授权…")
                    threading.Thread(target=self._demo_authorization_worker, daemon=True).start()
                elif kind == "quick_password_change_done":
                    errors, changed = event[1], event[2]
                    self.quick_password_in_progress = False
                    self.quick_password_required_accounts.difference_update(changed)
                    self.local_admin_credentials_sha256 = ""
                    self.read_current_local_admin_credentials()
                    if errors:
                        changed_text = f"已完成并同步：{'、'.join(changed)}。" if changed else ""
                        detail = changed_text + "\n".join(errors)
                        if self.quick_password_status is not None:
                            self.quick_password_status.set(detail)
                        if self.quick_password_confirm_button is not None:
                            self.quick_password_confirm_button.configure(state="normal")
                        if self.quick_password_cancel_button is not None:
                            self.quick_password_cancel_button.configure(state="normal")
                        self.footer_message.set("快速改密未全部完成，请按弹窗提示处理。")
                        continue
                    self._close_quick_password_change()
                    self._reset_authorization_progress()
                    self.authorization_status.set("两个管理员账号已使用同一新密码，且已同步到本地凭据文件。")
                    if self.authorization_start_button is not None and self.authorization_start_button.winfo_exists():
                        self.authorization_start_button.configure(text="继续快速授权", state="normal")
                    self.footer_message.set("两个本地管理员账号已完成改密；可继续快速授权。")
                elif kind == "health":
                    health_values: dict[str, bool] = {}
                    for key, health_result in event[1].items():
                        outcome, takeover, external_detail = health_result
                        healthy = outcome.healthy
                        health_values[key] = healthy
                        current = self.processes.get(key)
                        if takeover is not None and (current is None or current.poll() is not None):
                            with self.process_state_lock:
                                self.processes[key] = takeover
                                self.started_at[key] = time.monotonic()
                                self._save_process_state()
                            self.append_log(key, f"已接管外部启动的 Aster 进程 PID {takeover.pid}。")
                            self.footer_message.set(f"{SERVICE_BY_KEY[key].name} 已由本控制台接管。")
                            self.external_process_details.pop(key, None)
                        elif healthy and external_detail[1]:
                            self.external_process_details[key] = external_detail
                        else:
                            self.external_process_details.pop(key, None)
                        if healthy and not self.health.get(key, False):
                            process = self.processes.get(key)
                            if process is not None and process.poll() is None:
                                self.append_log(key, "HTTP 健康检查通过，服务已就绪。")
                        if healthy:
                            self.preparing.discard(key)
                            self.failures.pop(key, None)
                        elif "其他服务响应" in outcome.detail:
                            self.failures.setdefault(key, outcome.detail)
                    self.health.update(health_values)
                    self.health_initialized = True
                    self.health_probe_running = False
                    self.refresh_rows()
                elif kind == "started":
                    service = SERVICE_BY_KEY[event[1]]
                    self.busy.discard(service.key)
                    self.failures.pop(service.key, None)
                    if service.preparation_marker:
                        self.preparing.add(service.key)
                        self.started_at.pop(service.key, None)
                        self.append_log(service.key, f"已启动准备进程 PID {event[2]}，{service.preparation_message}。")
                        self.footer_message.set(f"{service.name}：{service.preparation_message}。")
                    else:
                        self.started_at[service.key] = time.monotonic()
                        self.append_log(service.key, f"已启动 PID {event[2]}")
                        self.footer_message.set(f"{service.name} 正在启动。")
                    self.refresh_rows()
                elif kind == "prepared":
                    service = SERVICE_BY_KEY[event[1]]
                    current = self.processes.get(service.key)
                    if current is None or current.pid != event[2] or current.poll() is not None:
                        continue
                    self.preparing.discard(service.key)
                    self.started_at[service.key] = time.monotonic()
                    self.append_log(
                        service.key,
                        f"{service.preparation_ready_message}，开始启动服务并计时健康检查。",
                    )
                    self.footer_message.set(f"{service.name} 正在启动。")
                    self.refresh_rows()
                elif kind == "external":
                    service = SERVICE_BY_KEY[event[1]]
                    self.busy.discard(service.key)
                    self.preparing.discard(service.key)
                    self.health[service.key] = True
                    self.failures.pop(service.key, None)
                    self.footer_message.set(f"{service.name} 已经在运行，本控制台未重复启动。")
                    self.refresh_rows()
                elif kind == "launch_error":
                    service = SERVICE_BY_KEY[event[1]]
                    self.busy.discard(service.key)
                    self.preparing.discard(service.key)
                    self.failures[service.key] = event[2]
                    self.append_log(service.key, f"启动失败：{event[2]}")
                    self.footer_message.set(f"{service.name} 启动失败。")
                    self.refresh_rows()
                elif kind == "exited":
                    service = SERVICE_BY_KEY[event[1]]
                    was_busy = service.key in self.busy
                    self.preparing.discard(service.key)
                    self.started_at.pop(service.key, None)
                    current = self.processes.get(service.key)
                    if current is not None and current.pid == event[3] and not was_busy:
                        with self.process_state_lock:
                            self.processes.pop(service.key, None)
                            self._save_process_state()
                    if event[2] != 0 and not was_busy:
                        recent = self.recent_output.get(service.key, [])[-8:]
                        detail = f"进程 PID {event[3]} 已退出，退出码 {event[2]}"
                        if recent:
                            detail += "\n\n最近输出：\n" + "\n".join(recent)
                        conflict = port_conflict_detail(service)
                        if conflict:
                            detail += "\n\n" + conflict
                        self.failures[service.key] = detail
                    self.append_log(service.key, f"进程 PID {event[3]} 已退出，退出码 {event[2]}")
                    self.refresh_rows()
                elif kind == "stopped":
                    self.busy.discard(event[1])
                    self.preparing.discard(event[1])
                    self.started_at.pop(event[1], None)
                    self.failures.pop(event[1], None)
                    self.stop_failures.pop(event[1], None)
                    with self.process_state_lock:
                        self.processes.pop(event[1], None)
                        self._save_process_state()
                    self.footer_message.set(f"{SERVICE_BY_KEY[event[1]].name} 已停止。")
                    self.refresh_rows()
                elif kind == "stop_failed":
                    service = SERVICE_BY_KEY[event[1]]
                    self.busy.discard(service.key)
                    self.stop_failures[service.key] = event[2]
                    with self.process_state_lock:
                        self._save_process_state()
                    self.append_log(service.key, f"停止失败：{event[2]}")
                    self.footer_message.set(f"{service.name} 停止失败；端口或进程仍在运行。")
                    self.refresh_rows()
        except queue.Empty:
            pass
        self.root.after(100, self._drain_events)

    def refresh_rows(self) -> None:
        if self.page != "main" or not self.rows:
            return
        for service in SERVICES:
            row = self.rows[service.key]
            process = self.processes.get(service.key)
            managed = process is not None and process.poll() is None
            healthy = self.health.get(service.key, False)
            if managed and service.key in self.preparing:
                status_text, status_color = "准备中", "#9a6700"
            elif managed and (healthy or not service.health_url):
                status_text, status_color = "运行中", "#157347"
            elif managed:
                elapsed = time.monotonic() - self.started_at.get(service.key, time.monotonic())
                if elapsed <= STARTUP_GRACE_SECONDS:
                    status_text, status_color = "启动中", "#9a6700"
                else:
                    status_text, status_color = "启动异常", "#8b3030"
            elif self.stop_failures.get(service.key):
                status_text, status_color = "停止失败", "#8b3030"
            elif not self.health_initialized:
                status_text, status_color = "检测中…", "#9a6700"
            elif healthy:
                status_text, status_color = "外部运行", "#9a6700"
            elif self.failures.get(service.key):
                status_text, status_color = "启动失败", "#8b3030"
            else:
                status_text, status_color = "已停止", "#8b3030"
            status: tk.Entry = row["status"]
            display_text = status_text
            if status_text == "外部运行":
                display_text = self.external_process_details.get(service.key, (status_text, ""))[0] or status_text
            if status.get() != display_text:
                status.configure(state="normal")
                status.delete(0, "end")
                status.insert(0, display_text)
                status.configure(state="readonly")
            status.configure(fg=status_color)
            tooltip: ToolTip = row["status_tooltip"]
            detail = self.stop_failures.get(service.key) or self.failures.get(service.key, "")
            if status_text == "外部运行" and not detail:
                detail = self.external_process_details.get(service.key, ("", ""))[1]
                if not detail:
                    detail = "端口健康，但进程来源或工作目录无法确认属于当前 Aster 服务；正在读取端口占用详情。"
            if status_text == "启动异常" and not detail:
                detail = f"进程仍在运行，但 {service_health_url(service)} 在 {STARTUP_GRACE_SECONDS:.0f} 秒内未通过健康检查。"
            if status_text == "准备中" and not detail:
                detail = f"{service.preparation_message}；完成后才开始计算健康检查时间。"
            tooltip.set_text(detail)
            status.configure(cursor="question_arrow" if detail else "xterm")
            busy = service.key in self.busy
            row["start"].configure(state="disabled" if busy or managed or healthy else "normal")
            row["restart"].configure(state="normal" if managed and not busy else "disabled")
            row["stop"].configure(state="normal" if (managed or healthy) and not busy else "disabled")

    def append_log(self, key: str, value: str) -> None:
        plain_value = terminal_plain_text(value)
        timestamp = time.strftime("%H:%M:%S")
        recent = self.recent_output.setdefault(key, [])
        recent.append(plain_value)
        if len(recent) > 30:
            del recent[:-20]
        self.log_history.append((key, timestamp, value))
        trimmed = False
        if len(self.log_history) > 5000:
            self.log_history = self.log_history[-4000:]
            self.render_logs()
            trimmed = True
        log_text = self.log_texts.get(key)
        if log_text is not None and not trimmed:
            log_text.configure(state="normal")
            self._insert_terminal_line(key, log_text, f"[{timestamp}] ", value)
            log_text.see("end")
            log_text.configure(state="disabled")
        severity = log_severity(plain_value)
        if severity and key != self.active_log_key and key in self.log_tabs:
            self.unread_log_severity[key] = higher_log_severity(self.unread_log_severity.get(key), severity) or severity
            self._update_log_tab_label(key)

    def render_logs(self) -> None:
        self.terminal_states = {key: TerminalStyle() for key in self.log_texts}
        for key, log_text in self.log_texts.items():
            log_text.configure(state="normal")
            log_text.delete("1.0", "end")
            for log_key, timestamp, value in self.log_history:
                if log_key == key:
                    self._insert_terminal_line(key, log_text, f"[{timestamp}] ", value)
            log_text.see("end")
            log_text.configure(state="disabled")

    def _update_log_tab_label(self, key: str) -> None:
        tab = self.log_tabs.get(key)
        if tab is None or not hasattr(self, "log_notebook"):
            return
        severity = self.unread_log_severity.get(key)
        icon = LOG_SEVERITY_ICONS.get(severity, "")
        label = LOG_TAB_LABELS[key]
        self.log_notebook.tab(tab, text=f"{icon} {label}" if icon else label)

    def _log_tab_changed(self, _event: tk.Event | None = None) -> None:
        if not hasattr(self, "log_notebook"):
            return
        selected = self.log_notebook.select()
        for key, tab in self.log_tabs.items():
            if str(tab) == selected:
                self.active_log_key = key
                self.unread_log_severity.pop(key, None)
                self._update_log_tab_label(key)
                return

    def clear_logs(self) -> None:
        key = self.active_log_key
        self.log_history = [entry for entry in self.log_history if entry[0] != key]
        self.unread_log_severity.pop(key, None)
        self._update_log_tab_label(key)
        self.render_logs()

    def open_url(self, url: str) -> None:
        target = local_browser_url(url)
        browser_option = self.browser_option.get()
        try:
            opened = open_browser_target(target, browser_option)
        except (OSError, ValueError, webbrowser.Error) as exc:
            opened = False
            detail = str(exc)
        else:
            detail = ""
        if opened:
            self.footer_message.set(f"已请求 {browser_option} 打开：{url}")
            return
        message = f"无法使用 {browser_option} 打开链接。"
        if browser_option == BROWSER_CHROME:
            message += "\n\n请确认已安装 Google Chrome，或将“打开链接”切换为“系统默认浏览器”。"
        if detail:
            message += f"\n\n{detail}"
        self.footer_message.set(message.split("\n", 1)[0])
        messagebox.showerror("无法打开浏览器", message, parent=self.root)

    def copy_url(self, url: str) -> None:
        self.root.clipboard_clear()
        self.root.clipboard_append(url)
        self.root.update_idletasks()
        self.footer_message.set(f"已复制：{url}")

    def close_requested(self) -> None:
        if self.setup_process and self.setup_process.poll() is None:
            messagebox.showinfo("操作进行中", "初始化或清理正在进行，请等待完成后再退出。")
            return
        if self.authorization_in_progress or getattr(self, "quick_password_in_progress", False):
            messagebox.showinfo("授权流程执行中", "请等待本地授权流程完成后再退出。")
            return
        if getattr(self, "integration_busy", False):
            messagebox.showinfo("PR 集成操作进行中", "请等待 PR 刷新或集成分支更新完成后再退出。")
            return
        self.closing = True
        self._save_process_state()
        self.root.destroy()


class EnvironmentChooser:
    """Small bootstrap window that hands control to one fixed development environment."""

    def __init__(self, root: tk.Tk) -> None:
        self.root = root
        self.primary, self.integration = environment_roots(ROOT)
        self.busy = False
        self.pr_busy = False
        self.prs_loaded = False
        self.events: queue.Queue[tuple[str, object]] = queue.Queue()
        self.integration_detail = tk.StringVar(value=self._integration_summary())
        self.pr_status = tk.StringVar(value="正在读取 GitHub Open PR…")
        self.pr_count = tk.StringVar(value="尚未刷新")
        self.loading_detail = tk.StringVar()
        self.loading_progress: ttk.Progressbar | None = None
        self.launch_process: subprocess.Popen[bytes] | None = None
        self.launch_ready_file: Path | None = None
        self.launch_deadline = 0.0
        self.pr_tree: ttk.Treeview | None = None
        self.pr_picker: PullRequestSelectionTable | None = None
        self.pr_controls: list[ttk.Button] = []
        self.pr_selection_controls: list[ttk.Button] = []
        self.integration_enter_button: ttk.Button | None = None
        self.pull_requests: list[local_pr_integration.PullRequest] = []
        self.applied: dict[int, str] = {}
        settings = read_pr_integration_settings(ROOT)
        self.selected = selected_pull_request_numbers(settings, "selected")
        self.seen = selected_pull_request_numbers(settings, "seen")
        self.drafts = selected_pull_request_numbers(settings, "drafts")
        self._load_applied_pull_requests()
        self.root.protocol("WM_DELETE_WINDOW", self._close_requested)
        self._configure_style()
        self._show_chooser()
        self.root.after(100, self._drain_events)
        self.root.after(150, self._refresh_pull_requests)

    def _load_applied_pull_requests(self) -> None:
        self.applied = {}
        try:
            state = local_pr_integration.IntegrationWorkspace(ROOT, self.integration).load_state()
        except local_pr_integration.IntegrationError:
            return
        if state is not None:
            self.applied = {item.number: item.head_sha for item in state.entries}

    def _configure_style(self) -> None:
        style = ttk.Style(self.root)
        if "vista" in style.theme_names():
            style.theme_use("vista")
        style.configure("ChooserTitle.TLabel", font=(UI_FONT_FAMILY, 19, "bold"))
        style.configure("ChooserSubtitle.TLabel", foreground="#5b6678")
        style.configure("PullRequest.Treeview", rowheight=30, font=(UI_FONT_FAMILY, 10))
        style.configure("PullRequest.Treeview.Heading", font=(UI_FONT_FAMILY, 9, "bold"))

    def _clear_window(self) -> None:
        if self.loading_progress is not None:
            self.loading_progress.stop()
            self.loading_progress = None
        for child in self.root.winfo_children():
            child.destroy()
        self.pr_tree = None
        self.pr_picker = None
        self.pr_controls = []
        self.pr_selection_controls = []
        self.integration_enter_button = None

    def _show_chooser(self) -> None:
        self._clear_window()
        self.root.title("选择 Aster Team 开发环境")
        self.root.geometry("780x540")
        self.root.minsize(700, 500)
        self.root.resizable(True, True)
        self._build_ui()

    def _show_loading(self, title: str, detail: str) -> None:
        self.root.update_idletasks()
        center_x = self.root.winfo_rootx() + self.root.winfo_width() // 2
        center_y = self.root.winfo_rooty() + self.root.winfo_height() // 2
        width, height = 560, 220
        x = max(0, center_x - width // 2)
        y = max(0, center_y - height // 2)

        self._clear_window()
        self.root.title(title)
        self.root.geometry(f"{width}x{height}+{x}+{y}")
        self.root.minsize(width, height)
        self.root.resizable(False, False)
        background = "#f3f5f9"
        self.root.configure(bg=background)
        self.loading_detail.set(detail)

        shell = tk.Frame(self.root, bg=background, padx=34, pady=30)
        shell.pack(fill="both", expand=True)
        tk.Label(
            shell, text=title, bg=background, fg="#172033",
            font=(UI_FONT_FAMILY, 18, "bold"), anchor="w",
        ).pack(fill="x")
        tk.Label(
            shell, textvariable=self.loading_detail, bg=background, fg="#5b6678",
            font=(UI_FONT_FAMILY, 10), anchor="w", justify="left", wraplength=490,
        ).pack(fill="x", pady=(10, 22))
        self.loading_progress = ttk.Progressbar(shell, mode="indeterminate")
        self.loading_progress.pack(fill="x")
        self.loading_progress.start(12)
        self.root.update_idletasks()

    @staticmethod
    def _git_value(workspace: Path, *arguments: str) -> str:
        try:
            result = subprocess.run(
                ["git", *arguments], cwd=workspace, capture_output=True, text=True,
                encoding="utf-8", errors="replace", timeout=5, check=False,
                creationflags=subprocess.CREATE_NO_WINDOW if IS_WINDOWS else 0,
            )
        except (OSError, subprocess.SubprocessError):
            return "未知"
        return result.stdout.strip() if result.returncode == 0 else "未知"

    def _main_summary(self) -> str:
        branch = self._git_value(self.primary, "branch", "--show-current")
        sha = self._git_value(self.primary, "rev-parse", "--short=10", "HEAD")
        return f"当前 {branch} · {sha}"

    def _integration_summary(self) -> str:
        if not self.integration.is_dir():
            return "尚未创建；首次进入时自动建立"
        try:
            state = local_pr_integration.IntegrationWorkspace(ROOT, self.integration).load_state()
        except local_pr_integration.IntegrationError:
            state = None
        if state is None:
            return "已有目录；进入时校验并更新"
        numbers = "、".join(f"#{item.number}" for item in state.entries) or "未选择 PR"
        return f"origin/main + {numbers}"

    def _build_ui(self) -> None:
        background = "#f3f5f9"
        self.root.configure(bg=background)
        shell = tk.Frame(self.root, bg=background, padx=24, pady=18)
        shell.pack(fill="both", expand=True)
        ttk.Label(shell, text="选择开发环境", style="ChooserTitle.TLabel").pack(anchor="w")
        ttk.Label(
            shell,
            text="主环境与 PR 集成环境使用独立目录、数据库和端口，可以同时运行。",
            style="ChooserSubtitle.TLabel",
        ).pack(anchor="w", pady=(4, 14))

        self.main_card = self._main_environment_card(shell)
        relative_integration = os.path.relpath(self.integration, self.primary).replace("\\", "/")
        self.integration_card = tk.Frame(
            shell, bg="#ffffff", highlightthickness=1,
            highlightbackground="#cbd3df", padx=18, pady=14,
        )
        self.integration_card.pack(fill="both", expand=True, pady=(12, 0))

        heading = tk.Frame(self.integration_card, bg="#ffffff")
        heading.pack(fill="x")
        tk.Label(
            heading, text="PR 集成环境", bg="#ffffff", fg="#172033",
            font=(UI_FONT_FAMILY, 13, "bold"),
        ).pack(side="left")
        path_label = tk.Label(
            heading, text=relative_integration, bg="#ffffff", fg="#2864dc",
            font=(MONO_FONT_FAMILY, 9),
        )
        path_label.pack(side="left", padx=(14, 0))
        ToolTip(path_label, str(self.integration))
        tk.Label(
            heading, textvariable=self.integration_detail, bg="#ffffff", fg="#687386",
            font=(UI_FONT_FAMILY, 9),
        ).pack(side="right")

        toolbar = tk.Frame(self.integration_card, bg="#ffffff")
        toolbar.pack(fill="x", pady=(12, 8))
        refresh = ttk.Button(toolbar, text="刷新 PR", command=self._refresh_pull_requests)
        refresh.pack(side="left")
        select_all = ttk.Button(toolbar, text="全选", command=lambda: self._set_all_pull_requests(True))
        select_all.pack(side="left", padx=(8, 0))
        clear_all = ttk.Button(toolbar, text="全部取消", command=lambda: self._set_all_pull_requests(False))
        clear_all.pack(side="left", padx=(8, 0))
        self.pr_controls = [refresh, select_all, clear_all]
        self.pr_selection_controls = [select_all, clear_all]
        tk.Label(
            toolbar, textvariable=self.pr_count, bg="#ffffff", fg="#5b6678",
            font=(UI_FONT_FAMILY, 9),
        ).pack(side="right")

        self.pr_picker = PullRequestSelectionTable(self.integration_card, self._toggle_pull_request, height=4)
        self.pr_picker.pack(fill="both", expand=True)
        self.pr_tree = self.pr_picker.tree

        footer = tk.Frame(self.integration_card, bg="#ffffff")
        footer.pack(fill="x", pady=(10, 0))
        tk.Label(
            footer, textvariable=self.pr_status, bg="#ffffff", fg="#5b6678",
            font=(UI_FONT_FAMILY, 9), anchor="w",
        ).pack(side="left", fill="x", expand=True)
        self.integration_enter_button = ttk.Button(
            footer, text="更新并进入集成环境", command=self._enter_integration,
        )
        self.integration_enter_button.pack(side="right", padx=(12, 0))
        self._render_pull_requests()
        self._update_pr_control_state()

    def _main_environment_card(self, parent: tk.Frame) -> tk.Frame:
        card = tk.Frame(
            parent, bg="#ffffff", cursor="hand2", takefocus=True,
            highlightthickness=1, highlightbackground="#cbd3df", highlightcolor="#2864dc",
            padx=18, pady=12,
        )
        card.pack(fill="x")
        title_row = tk.Frame(card, bg="#ffffff", cursor="hand2")
        title_row.pack(fill="x")
        tk.Label(
            title_row, text="主环境", bg="#ffffff", fg="#172033", cursor="hand2",
            font=(UI_FONT_FAMILY, 13, "bold"),
        ).pack(side="left")
        tk.Label(
            title_row, text="→", bg="#ffffff", fg="#2864dc", cursor="hand2",
            font=(SYMBOL_FONT_FAMILY, 14, "bold"),
        ).pack(side="right")
        meta = tk.Frame(card, bg="#ffffff", cursor="hand2")
        meta.pack(fill="x", pady=(8, 0))
        tk.Label(
            meta, text=self._main_summary(),
            bg="#ffffff", fg="#273142", cursor="hand2", font=(UI_FONT_FAMILY, 9, "bold"),
        ).pack(side="left")
        tk.Label(
            meta, text="使用主工作区中的本地配置和数据", bg="#ffffff", fg="#687386", cursor="hand2",
            font=(UI_FONT_FAMILY, 9),
        ).pack(side="right")

        def bind_tree(widget: tk.Misc) -> None:
            widget.bind("<Button-1>", lambda _event: self._enter_main(), add="+")
            widget.bind("<Enter>", lambda _event: card.configure(highlightbackground="#2864dc"), add="+")
            widget.bind("<Leave>", lambda _event: card.configure(highlightbackground="#cbd3df"), add="+")
            for child in widget.winfo_children():
                bind_tree(child)

        bind_tree(card)
        card.bind("<Return>", lambda _event: self._enter_main(), add="+")
        card.bind("<space>", lambda _event: self._enter_main(), add="+")
        ToolTip(card, str(self.primary))
        return card

    def _save_pr_integration_settings(self) -> None:
        write_pr_integration_settings(self.selected, self.seen, self.drafts, ROOT)

    def _update_pr_control_state(self) -> None:
        controls_state = "disabled" if self.busy or self.pr_busy else "normal"
        for control in self.pr_controls:
            if control.winfo_exists():
                control.configure(state=controls_state)
        if not self.prs_loaded:
            for control in self.pr_selection_controls:
                if control.winfo_exists():
                    control.configure(state="disabled")
        if self.integration_enter_button is not None and self.integration_enter_button.winfo_exists():
            enter_state = "normal" if self.prs_loaded and not self.busy and not self.pr_busy else "disabled"
            self.integration_enter_button.configure(state=enter_state)

    def _set_pr_busy(self, busy: bool, message: str = "") -> None:
        self.pr_busy = busy
        if message:
            self.pr_status.set(message)
        self._update_pr_control_state()

    def _refresh_pull_requests(self) -> None:
        if self.busy or self.pr_busy:
            return
        try:
            self._save_pr_integration_settings()
        except OSError as exc:
            self.pr_status.set(f"无法保存 PR 选择：{exc}")
            return
        self._set_pr_busy(True, "正在从 GitHub 刷新 Open PR…")
        threading.Thread(target=self._refresh_pull_requests_worker, daemon=True).start()

    def _refresh_pull_requests_worker(self) -> None:
        try:
            pull_requests = local_pr_integration.list_open_pull_requests(ROOT)
            self.events.put(("pull_requests", pull_requests))
        except Exception as exc:
            self.events.put(("pull_request_error", str(exc)))

    def _render_pull_requests(self) -> None:
        picker = self.pr_picker
        if picker is None or not picker.tree.winfo_exists():
            return
        picker.render(self.pull_requests, self.selected, self.applied)
        open_numbers = {item.number for item in self.pull_requests}
        selected_count = len(self.selected & open_numbers)
        removal_count = len(set(self.applied) - self.selected)
        suffix = f" · 待移除 {removal_count}" if removal_count else ""
        self.pr_count.set(f"已选择 {selected_count} / {len(self.pull_requests)}{suffix}")
        if self.prs_loaded and not self.pr_busy:
            if self.pull_requests:
                self.pr_status.set("勾选需要验证的 PR，然后更新并进入集成环境。")
            else:
                self.pr_status.set("当前没有 Open PR，将仅使用最新 origin/main。")

    def _toggle_pull_request(self, number: int) -> None:
        if self.busy or self.pr_busy:
            return
        if number in self.selected:
            self.selected.remove(number)
        else:
            self.selected.add(number)
        self.seen.add(number)
        self._save_pr_integration_settings()
        self._render_pull_requests()

    def _set_all_pull_requests(self, selected: bool) -> None:
        if self.busy or self.pr_busy:
            return
        if selected:
            self.selected = {item.number for item in self.pull_requests if not item.is_draft}
        else:
            self.selected.clear()
        self.seen = {item.number for item in self.pull_requests}
        self.drafts = {item.number for item in self.pull_requests if item.is_draft}
        self._save_pr_integration_settings()
        self._render_pull_requests()

    def _enter_main(self) -> None:
        if self.busy:
            return
        self.busy = True
        self._show_loading("正在进入主环境", "正在打开主工作区中的开发控制台…")
        self._launch_manager(self.primary)

    def _enter_integration(self) -> None:
        if self.busy or self.pr_busy or not self.prs_loaded:
            return
        try:
            self._save_pr_integration_settings()
        except OSError as exc:
            messagebox.showerror("无法保存 PR 选择", str(exc), parent=self.root)
            return
        selected = [item for item in self.pull_requests if item.number in self.selected]
        running_here = managed_services_in_workspace(self.integration)
        if running_here:
            desired = {item.number: item.head_sha for item in selected}
            if desired != self.applied:
                messagebox.showinfo(
                    "PR 选择尚未应用",
                    "集成环境仍有服务运行，当前勾选变化暂时不能更新到代码。\n\n"
                    "将打开现有集成控制台；停止服务后，可在主页面点击“管理 PR 集成”完成加入或移除。",
                    parent=self.root,
                )
            self.busy = True
            self._show_loading("正在进入 PR 集成环境", "检测到服务已运行，正在打开现有开发控制台…")
            self._launch_manager(self.integration)
            return
        self.busy = True
        self._update_pr_control_state()
        self._show_loading(
            "正在准备 PR 集成环境",
            "正在同步最新 origin/main 与已选远端 PR，请稍候…",
        )
        threading.Thread(target=self._prepare_integration_worker, args=(selected,), daemon=True).start()

    def _prepare_integration_worker(self, selected: list[local_pr_integration.PullRequest]) -> None:
        try:
            workspace = local_pr_integration.IntegrationWorkspace(
                ROOT, self.integration,
                progress=lambda message: self.events.put(("progress", message)),
            )
            result = workspace.apply(selected)
            if not node_dependencies_ready(result.path):
                self.events.put(("progress", "正在安装 PR 集成环境的 Node.js 依赖（首次或锁文件变化时执行）…"))
            ensure_node_dependencies(result.path)
            self.events.put(("progress", "集成环境已准备完成，正在打开开发控制台…"))
            self.events.put(("ready", result))
        except Exception as exc:
            self.events.put(("error", str(exc)))

    def _drain_events(self) -> None:
        try:
            kind, payload = self.events.get_nowait()
        except queue.Empty:
            if self.root.winfo_exists():
                self.root.after(100, self._drain_events)
            return
        if kind == "pull_requests":
            self.pull_requests = list(payload)
            self.selected, self.seen, self.drafts = local_pr_integration.reconcile_selection(
                self.pull_requests, self.selected, self.seen, self.drafts,
            )
            self.prs_loaded = True
            self._load_applied_pull_requests()
            try:
                self._save_pr_integration_settings()
            except OSError as exc:
                self.pr_status.set(f"PR 已刷新，但无法保存选择：{exc}")
            self._set_pr_busy(False)
            self._render_pull_requests()
            self.root.after(100, self._drain_events)
            return
        if kind == "pull_request_error":
            self._set_pr_busy(False, f"刷新失败：{payload}")
            self._render_pull_requests()
            self.root.after(100, self._drain_events)
            return
        if kind == "progress":
            self.loading_detail.set(str(payload))
            self.root.after(100, self._drain_events)
            return
        self.busy = False
        if kind == "error":
            self.integration_detail.set(self._integration_summary())
            self._show_chooser()
            messagebox.showerror("无法进入 PR 集成环境", str(payload), parent=self.root)
            self.root.after(100, self._drain_events)
            return
        result = payload
        if isinstance(result, local_pr_integration.IntegrationResult):
            self.integration_detail.set(
                f"origin/main + {len(result.entries)} 个 PR；复用 {result.reused} 个，重放 {result.replayed} 个",
            )
            self._launch_manager(result.path)

    def _launch_manager(self, workspace: Path) -> None:
        script = workspace / "scripts/local_dev_manager.py"
        if not script.is_file():
            self.busy = False
            self._show_chooser()
            messagebox.showerror("无法进入环境", f"未找到开发控制台：\n{script}", parent=self.root)
            return
        ready_file = Path(tempfile.gettempdir()) / f"aster-manager-ready-{os.getpid()}-{time.time_ns()}.tmp"
        environment = development_tool_environment(
            workspace, {**os.environ, MANAGER_READY_ENV: str(ready_file)},
        )
        try:
            process = subprocess.Popen([sys.executable, str(script)], cwd=workspace, env=environment)
        except OSError as exc:
            self.busy = False
            self._show_chooser()
            messagebox.showerror("无法进入环境", str(exc), parent=self.root)
            return
        self.launch_process = process
        self.launch_ready_file = ready_file
        self.launch_deadline = time.monotonic() + 2.0
        self.loading_detail.set("开发控制台正在启动，主窗口显示后将自动关闭此页面…")
        self.root.after(50, self._finish_launch_when_ready)

    def _finish_launch_when_ready(self) -> None:
        process = self.launch_process
        ready_file = self.launch_ready_file
        if process is None or ready_file is None:
            return
        if ready_file.is_file():
            ready_file.unlink(missing_ok=True)
            self.root.destroy()
            return
        if process.poll() is not None:
            self.launch_process = None
            self.launch_ready_file = None
            self.busy = False
            self._show_chooser()
            messagebox.showerror("无法进入环境", "开发控制台进程在主窗口显示前已经退出。", parent=self.root)
            return
        if time.monotonic() >= self.launch_deadline:
            ready_file.unlink(missing_ok=True)
            self.root.destroy()
            return
        self.root.after(50, self._finish_launch_when_ready)

    def _close_requested(self) -> None:
        if self.busy:
            messagebox.showinfo("正在更新集成环境", "请等待 Git 操作完成后再关闭窗口。", parent=self.root)
            return
        self.root.destroy()


def check_configuration() -> int:
    payload = {
        "root": str(ROOT),
        "python": sys.version.split()[0],
        "tkinter": TK_IMPORT_ERROR is None,
        "npm": npm_executable(),
        "node": node_executable(),
        "services": [
            {
                "key": service.key,
                "command": ["npm", *service.npm_arguments],
                "missing_files": missing_files(service),
                "health_url": service_health_url(service),
                "configuration_error": service_configuration_error(service),
            }
            for service in SERVICES
        ],
    }
    print(json.dumps(payload, ensure_ascii=False, indent=2))
    return 0 if payload["tkinter"] and payload["npm"] and payload["node"] else 1


def stop_local_services_command() -> int:
    stopped, failed = stop_local_services(ROOT)
    if stopped:
        print("已停止：" + "、".join(stopped))
    elif not failed:
        print("没有检测到可安全停止的 Aster 本地服务。")
    if failed:
        print("停止失败：" + "、".join(failed), file=sys.stderr)
        return 1
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Aster Team local development GUI")
    parser.add_argument("--check", action="store_true", help="validate the local GUI runtime without opening a window")
    parser.add_argument("--stop-all", action="store_true", help="stop all identity-checked local Aster services")
    parser.add_argument(
        "--select-environment", action="store_true",
        help="choose the primary or fixed PR-integration worktree before opening the manager",
    )
    arguments = parser.parse_args()
    if arguments.stop_all:
        return stop_local_services_command()
    if arguments.check:
        return check_configuration()
    if TK_IMPORT_ERROR is not None:
        print(f"Python 缺少 tkinter，无法启动图形界面：{TK_IMPORT_ERROR}", file=sys.stderr)
        return 1
    if IS_WINDOWS:
        try:
            import ctypes
            ctypes.windll.shcore.SetProcessDpiAwareness(1)
        except Exception:
            pass
    root = tk.Tk()
    if arguments.select_environment:
        try:
            EnvironmentChooser(root)
        except local_pr_integration.IntegrationError as exc:
            messagebox.showerror("无法识别开发环境", str(exc), parent=root)
            root.destroy()
            return 1
    else:
        LocalDevManager(root)
        notify_parent_manager_ready(root)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
