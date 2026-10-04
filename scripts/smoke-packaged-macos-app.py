#!/usr/bin/env python3
"""Credential-free lifecycle smoke test for a packaged macOS Bridge.app."""

from __future__ import annotations

import argparse
import fcntl
import os
import pathlib
import plistlib
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from typing import Mapping


APP_STARTED = re.compile(r"bridge: started bridged \(pid (\d+)\)")
DESKTOP_STARTED = re.compile(r"bridge: starting Bridge .* pid=(\d+) executable=")
APP_ATTACHED = "bridge: attached to bridged (desktop runs as a daemon client)"
SMOKE_PASSED = "bridge: packaged smoke health check passed"
SMOKE_FAILED = "bridge: packaged smoke health check failed:"
DAEMON_EXITED_CLEANLY = "bridge: bridged process exited cleanly"
DAEMON_EXITED_UNCLEANLY = "bridge: bridged process exited uncleanly"
DAEMON_EXITED_EARLY = "bridge: bridged exited before desktop shutdown"
DAEMON_SERVING = "bridged: serving "
DAEMON_SHUTDOWN_COMPLETE = "bridged: graceful shutdown complete"
TOKEN = re.compile(r"^[0-9a-fA-F]{64}$")

# Locale and the macOS user-encoding hint are the only host settings worth
# retaining. Starting from an allowlist prevents future provider/config/agent
# variables from silently turning this credential-free test into a live one.
PASSTHROUGH_ENVIRONMENT = {
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LOGNAME",
    "USER",
    "__CF_USER_TEXT_ENCODING",
}


class SmokeFailure(RuntimeError):
    """A packaged-app acceptance condition was not met."""


@dataclass(frozen=True)
class Bundle:
    app: pathlib.Path
    executable: pathlib.Path
    daemon: pathlib.Path
    identifier: str
    version: str


def _executable(path: pathlib.Path, label: str) -> pathlib.Path:
    if not path.is_file() or not os.access(path, os.X_OK):
        raise SmokeFailure(f"{label} is missing or not executable: {path}")
    return path


def load_bundle(app: pathlib.Path) -> Bundle:
    app = app.resolve()
    info_path = app / "Contents" / "Info.plist"
    try:
        with info_path.open("rb") as source:
            info = plistlib.load(source)
    except (OSError, plistlib.InvalidFileException) as error:
        raise SmokeFailure(f"could not read {info_path}: {error}") from error

    executable_name = info.get("CFBundleExecutable")
    identifier = info.get("CFBundleIdentifier")
    version = info.get("CFBundleShortVersionString")
    for name, value in (
        ("CFBundleExecutable", executable_name),
        ("CFBundleIdentifier", identifier),
        ("CFBundleShortVersionString", version),
    ):
        if not isinstance(value, str) or not value:
            raise SmokeFailure(f"{info_path} has no valid {name}")

    macos = app / "Contents" / "MacOS"
    return Bundle(
        app=app,
        executable=_executable(macos / executable_name, "packaged Bridge executable"),
        daemon=_executable(macos / "bridged", "bundled bridged daemon"),
        identifier=identifier,
        version=version,
    )


def smoke_environment(
    original: Mapping[str, str],
    home: pathlib.Path,
    data_dir: pathlib.Path,
    temp_dir: pathlib.Path,
) -> dict[str, str]:
    environment = {
        name: value for name, value in original.items() if name in PASSTHROUGH_ENVIRONMENT
    }
    environment.update(
        {
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            "SHELL": "/bin/sh",
            "HOME": str(home),
            "TMPDIR": str(temp_dir),
            "XDG_CACHE_HOME": str(home / ".cache"),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_DATA_HOME": str(home / ".local" / "share"),
            "BRIDGE_DATA_DIR": str(data_dir),
            "BRIDGE_DESKTOP_HOST": "daemon",
            "BRIDGE_PACKAGED_SMOKE": "1",
        }
    )
    return environment


def read_text(path: pathlib.Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return ""


def daemon_pid(app_log: str) -> int | None:
    match = APP_STARTED.search(app_log)
    return int(match.group(1)) if match else None


def desktop_pid(app_log: str) -> int | None:
    match = DESKTOP_STARTED.search(app_log)
    return int(match.group(1)) if match else None


def launch_services_command(
    bundle: Bundle,
    environment: Mapping[str, str],
    stdout_path: pathlib.Path,
    stderr_path: pathlib.Path,
) -> list[str]:
    """Build an argv-only LaunchServices invocation for the exact app bundle."""
    command = [
        "/usr/bin/open",
        "-W",
        "-n",
        "-g",
        "--stdin",
        "/dev/null",
        "--stdout",
        str(stdout_path),
        "--stderr",
        str(stderr_path),
    ]
    # `open` inherits this same allowlisted environment, and explicit --env
    # entries make the launched application's environment deterministic too.
    for name, value in sorted(environment.items()):
        command.extend(("--env", f"{name}={value}"))
    command.extend(("-a", str(bundle.app)))
    return command


def executable_process_ids(listing: str, executable: pathlib.Path) -> set[int]:
    """Select only processes whose command is the exact packaged executable."""
    target = str(executable)
    matches: set[int] = set()
    for line in listing.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) != 2 or not fields[0].isdigit():
            continue
        command = fields[1]
        if command == target or command.startswith(f"{target} "):
            matches.add(int(fields[0]))
    return matches


def running_executable_process_ids(executable: pathlib.Path) -> set[int] | None:
    """Best-effort exact-path inventory used only for failed-launch cleanup."""
    try:
        result = subprocess.run(
            ["/bin/ps", "-ww", "-axo", "pid=,command="],
            env={"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"},
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
        )
    except OSError:
        return None
    if result.returncode != 0:
        return None
    return executable_process_ids(result.stdout, executable)


def process_exists(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def wait_for_process_exit(pid: int, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while process_exists(pid) and time.monotonic() < deadline:
        time.sleep(0.05)
    return not process_exists(pid)


def terminate_processes(pids: set[int], timeout: float = 5) -> None:
    targets = {pid for pid in pids if pid > 1 and pid != os.getpid() and process_exists(pid)}
    for pid in targets:
        try:
            os.kill(pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass

    deadline = time.monotonic() + timeout
    while any(process_exists(pid) for pid in targets) and time.monotonic() < deadline:
        time.sleep(0.05)
    for pid in targets:
        if not process_exists(pid):
            continue
        try:
            os.kill(pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
    # These LaunchServices-owned processes are not our children, so polling for
    # disappearance is the available equivalent of reaping after SIGKILL.
    kill_deadline = time.monotonic() + min(timeout, 1)
    while any(process_exists(pid) for pid in targets) and time.monotonic() < kill_deadline:
        time.sleep(0.05)


def owner_lease_is_free(path: pathlib.Path) -> bool:
    if not path.is_file():
        return False
    with path.open("r+b") as lease:
        try:
            fcntl.flock(lease.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return False
        finally:
            try:
                fcntl.flock(lease.fileno(), fcntl.LOCK_UN)
            except OSError:
                pass
    return True


def validate_runtime_evidence(
    bundle: Bundle,
    data_dir: pathlib.Path,
    app_log: str,
    daemon_log: str,
    app_status: int,
) -> int:
    if app_status != 0:
        raise SmokeFailure(f"packaged Bridge exited with status {app_status}")
    if SMOKE_FAILED in app_log:
        raise SmokeFailure("the packaged app reported a failed daemon health check")
    for marker, message in (
        (DAEMON_EXITED_UNCLEANLY, "the bundled daemon process exited uncleanly"),
        (DAEMON_EXITED_EARLY, "the bundled daemon exited before desktop shutdown"),
    ):
        if marker in app_log:
            raise SmokeFailure(message)
    for marker in (APP_ATTACHED, SMOKE_PASSED, DAEMON_EXITED_CLEANLY):
        if marker not in app_log:
            raise SmokeFailure(f"packaged app log is missing: {marker}")
    pid = daemon_pid(app_log)
    if pid is None:
        raise SmokeFailure("packaged app did not report starting its bundled daemon")
    if DAEMON_SERVING not in daemon_log or f"(data dir {data_dir})" not in daemon_log:
        raise SmokeFailure("bundled daemon did not report serving the isolated data directory")
    if DAEMON_SHUTDOWN_COMPLETE not in daemon_log:
        raise SmokeFailure("bundled daemon did not acknowledge graceful shutdown")

    database = data_dir / "bridge.db"
    token_path = data_dir / "daemon.token"
    socket_path = data_dir / "bridged.sock"
    owner_path = data_dir / "owner.lock"
    if not database.is_file():
        raise SmokeFailure(f"daemon did not create its database: {database}")
    try:
        token = token_path.read_text(encoding="ascii").strip()
    except OSError as error:
        raise SmokeFailure(f"daemon token is missing or unreadable: {error}") from error
    if not TOKEN.fullmatch(token):
        raise SmokeFailure("daemon token does not have the expected 64-hex format")
    token_mode = stat.S_IMODE(token_path.stat().st_mode)
    if token_mode != 0o600:
        raise SmokeFailure(f"daemon token permissions are {token_mode:03o}, expected 600")
    if socket_path.exists():
        raise SmokeFailure(f"daemon socket survived normal app shutdown: {socket_path}")
    if process_exists(pid):
        raise SmokeFailure(f"desktop-owned daemon watchdog {pid} survived app shutdown")
    if not owner_lease_is_free(owner_path):
        raise SmokeFailure("daemon ownership lease was not released after app shutdown")
    return pid


def print_logs(
    app_log_path: pathlib.Path,
    daemon_log_path: pathlib.Path,
    app_stdout_path: pathlib.Path | None = None,
    launcher_log_path: pathlib.Path | None = None,
) -> None:
    paths = [("Bridge.app stderr", app_log_path), ("bridged", daemon_log_path)]
    if app_stdout_path is not None:
        paths.append(("Bridge.app stdout", app_stdout_path))
    if launcher_log_path is not None:
        paths.append(("LaunchServices", launcher_log_path))
    for label, path in paths:
        contents = read_text(path)
        print(f"--- {label} log ({path}) ---", file=sys.stderr)
        print(contents[-64 * 1024 :] if contents else "<missing or empty>", file=sys.stderr)


def force_cleanup(
    launcher: subprocess.Popen[bytes] | None,
    bundle: Bundle,
    preexisting_app_pids: set[int] | None,
    app_log_path: pathlib.Path,
) -> None:
    # Stop and reap `open -W` before taking the final process inventory. If it
    # were still running, LaunchServices could create the app after our scan.
    if launcher is not None and launcher.poll() is None:
        launcher.terminate()
        try:
            launcher.wait(timeout=5)
        except subprocess.TimeoutExpired:
            launcher.kill()
            launcher.wait(timeout=5)

    app_log = read_text(app_log_path)
    app_pids: set[int] = set()
    reported_pid = desktop_pid(app_log)
    if reported_pid is not None:
        app_pids.add(reported_pid)
    current_app_pids = running_executable_process_ids(bundle.executable)
    if preexisting_app_pids is not None and current_app_pids is not None:
        app_pids.update(current_app_pids - preexisting_app_pids)
    terminate_processes(app_pids)

    # The app may have emitted its daemon PID while cleanup was terminating it,
    # so refresh once more before targeting the daemon process group.
    app_log = read_text(app_log_path)
    pid = daemon_pid(app_log)
    if pid is None or not process_exists(pid):
        return
    try:
        group = os.getpgid(pid)
        os.killpg(group, signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        return
    if not wait_for_process_exit(pid, 5):
        try:
            os.killpg(group, signal.SIGKILL)
        except ProcessLookupError:
            pass
        wait_for_process_exit(pid, 5)


def run(app: pathlib.Path, timeout: float) -> None:
    if sys.platform != "darwin":
        raise SmokeFailure("the packaged Bridge.app smoke test requires macOS")
    bundle = load_bundle(app)
    scratch = pathlib.Path(tempfile.mkdtemp(prefix="bridge-smoke-", dir="/tmp"))
    data_dir = scratch / "data"
    home = scratch / "home"
    temp_dir = scratch / "tmp"
    app_log_path = scratch / "Bridge.log"
    app_stdout_path = scratch / "Bridge.stdout.log"
    launcher_log_path = scratch / "LaunchServices.log"
    daemon_log_path = data_dir / "bridged.log"
    data_dir.mkdir(mode=0o700)
    home.mkdir(mode=0o700)
    temp_dir.mkdir(mode=0o700)
    launcher: subprocess.Popen[bytes] | None = None
    preexisting_app_pids = running_executable_process_ids(bundle.executable)
    succeeded = False
    try:
        environment = smoke_environment(os.environ, home, data_dir, temp_dir)
        command = launch_services_command(
            bundle, environment, app_stdout_path, app_log_path
        )
        with launcher_log_path.open("wb") as launcher_log:
            launcher = subprocess.Popen(
                command,
                cwd=bundle.app.parent,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=launcher_log,
                stderr=subprocess.STDOUT,
                start_new_session=False,
            )
            try:
                app_status = launcher.wait(timeout=timeout)
            except subprocess.TimeoutExpired as error:
                raise SmokeFailure(
                    f"packaged Bridge did not finish its smoke cycle within {timeout:.0f}s"
                ) from error

        app_log = "\n".join((read_text(app_log_path), read_text(app_stdout_path)))
        daemon_log = read_text(daemon_log_path)
        pid = daemon_pid(app_log)
        if pid is not None and not wait_for_process_exit(pid, 5):
            raise SmokeFailure(f"desktop-owned daemon watchdog {pid} did not exit within 5s")
        pid = validate_runtime_evidence(bundle, data_dir, app_log, daemon_log, app_status)
        succeeded = True
        print(
            f"Packaged Bridge.app smoke passed: v{bundle.version}, bundled daemon pid {pid}, "
            "authenticated desktop health RPC, clean app/daemon shutdown."
        )
    except Exception:
        print_logs(app_log_path, daemon_log_path, app_stdout_path, launcher_log_path)
        raise
    finally:
        if not succeeded:
            force_cleanup(launcher, bundle, preexisting_app_pids, app_log_path)
        shutil.rmtree(scratch, ignore_errors=True)


def main(argv: list[str] | None = None) -> int:
    root = pathlib.Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "app",
        nargs="?",
        type=pathlib.Path,
        default=root / "src-tauri" / "target" / "release" / "bundle" / "macos" / "Bridge.app",
        help="exact packaged Bridge.app to exercise",
    )
    parser.add_argument("--timeout", type=float, default=60, help="whole app lifecycle deadline")
    args = parser.parse_args(argv)
    try:
        run(args.app, args.timeout)
    except (SmokeFailure, OSError, subprocess.SubprocessError) as error:
        print(f"smoke-packaged-macos-app: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
