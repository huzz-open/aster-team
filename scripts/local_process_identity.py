"""Read-only process provenance used by the local development manager."""

from __future__ import annotations

import ctypes
import ctypes.wintypes
import os
import shlex
import shutil
import subprocess
import sys
from functools import lru_cache
from pathlib import Path


class _DarwinProcessInfo(ctypes.Structure):
    _fields_ = [
        ("pbi_flags", ctypes.c_uint32),
        ("pbi_status", ctypes.c_uint32),
        ("pbi_xstatus", ctypes.c_uint32),
        ("pbi_pid", ctypes.c_uint32),
        ("pbi_ppid", ctypes.c_uint32),
        ("pbi_uid", ctypes.c_uint32),
        ("pbi_gid", ctypes.c_uint32),
        ("pbi_ruid", ctypes.c_uint32),
        ("pbi_rgid", ctypes.c_uint32),
        ("pbi_svuid", ctypes.c_uint32),
        ("pbi_svgid", ctypes.c_uint32),
        ("rfu_1", ctypes.c_uint32),
        ("pbi_comm", ctypes.c_char * 16),
        ("pbi_name", ctypes.c_char * 32),
        ("pbi_nfiles", ctypes.c_uint32),
        ("pbi_pgid", ctypes.c_uint32),
        ("pbi_pjobc", ctypes.c_uint32),
        ("e_tdev", ctypes.c_uint32),
        ("e_tpgid", ctypes.c_uint32),
        ("pbi_nice", ctypes.c_int32),
        ("pbi_start_tvsec", ctypes.c_uint64),
        ("pbi_start_tvusec", ctypes.c_uint64),
    ]


@lru_cache(maxsize=1)
def _darwin_libproc() -> ctypes.CDLL:
    library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    library.proc_pidinfo.argtypes = [
        ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int,
    ]
    library.proc_pidinfo.restype = ctypes.c_int
    return library


def darwin_process_info(pid: int) -> tuple[str, str] | None:
    """Return a native process name and PID-reuse-safe creation identity on macOS."""
    if sys.platform != "darwin" or pid <= 0:
        return None
    try:
        info = _DarwinProcessInfo()
        received = _darwin_libproc().proc_pidinfo(
            pid, 3, 0, ctypes.byref(info), ctypes.sizeof(info),  # PROC_PIDTBSDINFO
        )
        if received != ctypes.sizeof(info) or info.pbi_pid != pid:
            return None
        encoded_name = bytes(info.pbi_name).split(b"\0", 1)[0] or bytes(info.pbi_comm).split(b"\0", 1)[0]
        name = encoded_name.decode("utf-8", "replace")
        identity = f"darwin:{info.pbi_start_tvsec}:{info.pbi_start_tvusec}"
        return name, identity
    except (OSError, AttributeError, ValueError):
        return None


def command_arguments(command: str) -> list[str]:
    if not command.strip():
        return []
    if os.name != "nt":
        try:
            return shlex.split(command)
        except ValueError:
            return []
    shell = ctypes.WinDLL("shell32", use_last_error=True)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    shell.CommandLineToArgvW.argtypes = [ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_int)]
    shell.CommandLineToArgvW.restype = ctypes.POINTER(ctypes.c_wchar_p)
    kernel.LocalFree.argtypes = [ctypes.c_void_p]
    kernel.LocalFree.restype = ctypes.c_void_p
    count = ctypes.c_int()
    arguments = shell.CommandLineToArgvW(command, ctypes.byref(count))
    if not arguments:
        return []
    try:
        return [arguments[index] for index in range(count.value)]
    finally:
        kernel.LocalFree(arguments)


def process_working_directory(pid: int) -> Path | None:
    """Return an OS-observed cwd, never one inferred from a relative command."""
    if pid <= 0:
        return None
    try:
        if sys.platform == "darwin":
            return _darwin_working_directory(pid)
        if os.name != "nt":
            return (Path("/proc") / str(pid) / "cwd").resolve(strict=True)
        return _windows_working_directory(pid)
    except (OSError, ValueError, UnicodeError, AttributeError):
        # Access denial, unsupported process architecture or a disappearing PID
        # must prevent takeover, not turn into a guessed repository directory.
        return None


def _darwin_working_directory(pid: int) -> Path | None:
    lsof = shutil.which("lsof") or "/usr/sbin/lsof"
    completed = subprocess.run(
        [lsof, "-a", "-p", str(pid), "-d", "cwd", "-Fn"],
        capture_output=True, text=True, encoding="utf-8", errors="replace",
        timeout=4, check=False,
    )
    if completed.returncode != 0:
        return None
    for line in completed.stdout.splitlines():
        if line.startswith("n") and len(line) > 1:
            directory = Path(line[1:])
            return directory.resolve(strict=True) if directory.is_absolute() else None
    return None


def _windows_working_directory(pid: int) -> Path | None:
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    native = ctypes.WinDLL("ntdll", use_last_error=True)
    kernel.OpenProcess.argtypes = [ctypes.wintypes.DWORD, ctypes.wintypes.BOOL, ctypes.wintypes.DWORD]
    kernel.OpenProcess.restype = ctypes.wintypes.HANDLE
    kernel.CloseHandle.argtypes = [ctypes.wintypes.HANDLE]
    kernel.ReadProcessMemory.argtypes = [ctypes.wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)]
    kernel.ReadProcessMemory.restype = ctypes.wintypes.BOOL
    kernel.IsWow64Process.argtypes = [ctypes.wintypes.HANDLE, ctypes.POINTER(ctypes.wintypes.BOOL)]
    kernel.IsWow64Process.restype = ctypes.wintypes.BOOL
    native.NtQueryInformationProcess.argtypes = [ctypes.wintypes.HANDLE, ctypes.c_ulong, ctypes.c_void_p, ctypes.c_ulong, ctypes.c_void_p]
    native.NtQueryInformationProcess.restype = ctypes.c_long
    handle = kernel.OpenProcess(0x0400 | 0x0010, False, pid)  # QUERY_INFORMATION | VM_READ
    if not handle:
        return None
    try:
        def read(address: int, size: int) -> bytes:
            buffer = ctypes.create_string_buffer(size)
            received = ctypes.c_size_t()
            if not address or not kernel.ReadProcessMemory(handle, address, buffer, size, ctypes.byref(received)) or received.value != size:
                raise OSError("Process parameters are unavailable")
            return buffer.raw

        width = ctypes.sizeof(ctypes.c_void_p)
        wow = ctypes.wintypes.BOOL()
        if not kernel.IsWow64Process(handle, ctypes.byref(wow)):
            return None
        basic = (ctypes.c_void_p * 6)()
        if native.NtQueryInformationProcess(handle, 0, basic, ctypes.sizeof(basic), None) < 0:
            return None
        peb = basic[1]
        if width == 8 and wow.value:
            wow_peb = ctypes.c_size_t()
            if native.NtQueryInformationProcess(handle, 26, ctypes.byref(wow_peb), ctypes.sizeof(wow_peb), None) < 0:
                return None
            peb, width = wow_peb.value, 4
        elif width == 4 and not wow.value and os.environ.get("PROCESSOR_ARCHITEW6432"):
            return None  # Do not truncate a 64-bit target address in 32-bit Python.
        if not peb:
            return None
        # NT's PEB -> RTL_USER_PROCESS_PARAMETERS -> CURDIR.DosPath. These
        # layouts are architecture-specific; failed/invalid reads fail closed.
        # https://learn.microsoft.com/windows/win32/api/winternl/ns-winternl-peb
        parameters = int.from_bytes(read(peb + (0x20 if width == 8 else 0x10), width), "little")
        descriptor = read(parameters + (0x38 if width == 8 else 0x24), 2 * width)
        length = int.from_bytes(descriptor[:2], "little")
        maximum = int.from_bytes(descriptor[2:4], "little")
        pointer = int.from_bytes(descriptor[width:], "little")
        if not length or length % 2 or length > maximum or length > 65534:
            return None
        directory = Path(read(pointer, length).decode("utf-16-le"))
        return directory.resolve(strict=True) if directory.is_absolute() else None
    finally:
        kernel.CloseHandle(handle)


def is_operations_binary(executable: Path) -> bool:
    """Inspect Go build metadata; never execute the candidate service binary."""
    try:
        metadata = executable.stat()
        return _operations_build_info(str(executable.resolve()), metadata.st_size, metadata.st_mtime_ns)
    except OSError:
        return False


@lru_cache(maxsize=32)
def _operations_build_info(executable: str, size: int, modified: int) -> bool:
    go = shutil.which("go")
    if not go:
        return False
    try:
        result = subprocess.run(
            [go, "version", "-m", executable], capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=4, check=False,
            env={**os.environ, "GOTOOLCHAIN": "local"},
            **({"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}),
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return result.returncode == 0 and any(
        line.split() == ["path", "aster.local/team/operations/backend/cmd/api"]
        for line in result.stdout.splitlines()
    )
