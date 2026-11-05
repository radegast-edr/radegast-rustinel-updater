"""Real signed-release upgrade on a disposable native host, using the backend harness."""

import argparse
import contextlib
import functools
import hashlib
import importlib.util
import json
import os
import platform
import re
import shutil
import sqlite3
import subprocess
import sys
import threading
import time
import traceback
import urllib.request
import zipfile
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST_URL = "https://radegast.app/api/rustinel-releases.json"
DOWNLOAD_URL = "https://console-api.radegast.app/api/v1"


def version_key(value):
    match = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)(?:r(\d+))?", value)
    if not match:
        raise ValueError(f"Invalid release version: {value}")
    major, minor, patch, revision = match.groups()
    return (
        int(major),
        int(minor),
        int(patch),
        -1 if revision is None else int(revision),
    )


def detect_platform():
    os_name = {"Linux": "linux", "Darwin": "mac", "Windows": "windows"}[
        platform.system()
    ]
    machine = platform.machine().lower()
    if machine in ("amd64", "x86_64"):
        return os_name, "amd64"
    if machine in ("aarch64", "arm64"):
        return os_name, "m5" if os_name == "mac" else "arm64"
    raise ValueError(f"Unsupported architecture: {machine}")


def select_pair(entries, os_name, arch, baseline_version=None, candidate_version=None):
    filename = f"{os_name}-{arch}.zip"
    available = [
        entry
        for entry in entries
        if any(
            len(parts := line.split()) == 2 and parts[1] == filename
            for line in entry["hash_sha256"].splitlines()
        )
    ]
    available.sort(key=lambda entry: version_key(entry["version"]))
    unique = {version_key(entry["version"]): entry for entry in available}
    if len(unique) < 2:
        raise ValueError(f"Need two distinct signed releases for {os_name}/{arch}")
    ordered = list(unique.values())
    if candidate_version is None and baseline_version is None:
        # Dynamic selection: find the latest available pair without blocked automatic updates.
        updatable = [entry for entry in ordered if entry.get("autoupdate", True)]
        if os_name == "mac":
            # Exclude known corrupt published bundles on the upstream CDN if present
            updatable = [
                e
                for e in updatable
                if e["version"] not in ("1.8.0", "1.7.0")
            ]

        # 1. Prefer pairs with zero global blockers in the entire manifest.
        for c_idx in reversed(range(1, len(updatable))):
            c_entry = updatable[c_idx]
            for b_idx in reversed(range(c_idx)):
                b_entry = updatable[b_idx]
                has_global_veto = any(
                    not entry.get("autoupdate", True)
                    and version_key(entry["version"]) > version_key(b_entry["version"])
                    for entry in entries
                )
                if not has_global_veto:
                    return b_entry, c_entry

        # 2. Otherwise find the latest pair with zero intermediate blockers in snapshot <= candidate.
        for c_idx in reversed(range(1, len(updatable))):
            c_entry = updatable[c_idx]
            for b_idx in reversed(range(c_idx)):
                b_entry = updatable[b_idx]
                has_intermediate_veto = any(
                    not entry.get("autoupdate", True)
                    and version_key(b_entry["version"])
                    < version_key(entry["version"])
                    <= version_key(c_entry["version"])
                    for entry in entries
                )
                if not has_intermediate_veto:
                    return b_entry, c_entry

    candidate = (
        unique.get(version_key(candidate_version)) if candidate_version else ordered[-1]
    )
    if candidate is None:
        raise ValueError(
            f"Candidate {candidate_version} is unavailable for {os_name}/{arch}"
        )
    older = [
        entry
        for entry in ordered
        if version_key(entry["version"]) < version_key(candidate["version"])
    ]
    baseline = (
        unique.get(version_key(baseline_version))
        if baseline_version
        else (older[-1] if older else None)
    )
    if baseline is None or version_key(baseline["version"]) >= version_key(
        candidate["version"]
    ):
        raise ValueError(
            "Baseline must be available and strictly older than the candidate"
        )
    # Match the updater's veto semantics, including releases for other platforms.
    for entry in entries:
        if not entry.get("autoupdate", True) and version_key(
            entry["version"]
        ) > version_key(baseline["version"]):
            raise ValueError(f"Automatic upgrade is blocked by {entry['version']}")
    return baseline, candidate


def file_hash(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def archive_binary_hash(archive, os_name):
    name = (
        "Rustinel.app/Contents/MacOS/rustinel"
        if os_name == "mac"
        else ("rustinel.exe" if os_name == "windows" else "rustinel")
    )
    with zipfile.ZipFile(archive) as bundle, bundle.open(name) as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def fetch(url, path, limit):
    request = urllib.request.Request(
        url, headers={"User-Agent": "radegast-updater-integration"}
    )
    with (
        urllib.request.urlopen(request, timeout=120) as response,
        path.open("wb") as output,
    ):
        if not response.url.startswith("https://"):
            raise ValueError("Refusing an insecure release download")
        total = 0
        while chunk := response.read(1024 * 1024):
            total += len(chunk)
            if total > limit:
                raise ValueError("Release download exceeded size limit")
            output.write(chunk)


def prepare(args):
    args.assets.mkdir(parents=True, exist_ok=True)
    (args.assets / "fixtures.json").unlink(missing_ok=True)
    manifest = args.assets / "manifest.json"
    if args.manifest is None:
        fetch(MANIFEST_URL, manifest, 8 * 1024 * 1024)
    elif args.manifest.resolve() != manifest.resolve():
        shutil.copy2(args.manifest, manifest)
    entries = json.loads(manifest.read_text())
    pair = select_pair(
        entries, args.os, args.arch, args.baseline_version, args.candidate_version
    )
    # Freeze this run at the selected candidate, retaining intermediate vetoes.
    # select_pair checks blockers against the entire fetched manifest first.
    snapshot = [
        entry
        for entry in entries
        if version_key(entry["version"]) <= version_key(pair[1]["version"])
    ]
    (args.assets / "snapshot.json").write_text(json.dumps(snapshot), encoding="utf-8")
    metadata = {
        "os": args.os,
        "arch": args.arch,
        "releases": [],
        "manifest_source": str(args.manifest) if args.manifest else MANIFEST_URL,
    }
    for label, entry in zip(("baseline", "candidate"), pair, strict=True):
        archive = args.assets / f"{label}.zip"
        entry_path = args.assets / f"{label}.json"
        entry_path.write_text(json.dumps(entry), encoding="utf-8")
        version = entry["version"]
        fetch(
            f"{DOWNLOAD_URL}/public/releases/{version}/{args.os}/{args.arch}/download",
            archive,
            1024**3,
        )
        subprocess.run(
            [str(args.verifier), str(entry_path), str(archive), args.os, args.arch],
            check=True,
            timeout=120,
        )
        metadata["releases"].append(
            {
                "label": label,
                "version": version,
                "archive_sha256": file_hash(archive),
                "binary_sha256": archive_binary_hash(archive, args.os),
            }
        )
    (args.assets / "fixtures.json").write_text(
        json.dumps(metadata, indent=2), encoding="utf-8"
    )
    print(json.dumps(metadata, indent=2))


def installation_root(os_name):
    if os_name == "windows":
        return Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Radegast"
    return Path("/Library/Radegast" if os_name == "mac" else "/opt/radegast")


def preflight(args):
    if not args.disposable_host:
        raise RuntimeError(
            "Run only on a disposable native runner, with --disposable-host"
        )
    if detect_platform() != (args.os, args.arch):
        raise RuntimeError(
            "Test must run natively on the requested OS and architecture"
        )
    if args.os == "windows":
        import ctypes

        if not ctypes.windll.shell32.IsUserAnAdmin():
            raise RuntimeError("An elevated Windows runner is required")
    elif os.geteuid() != 0:
        raise RuntimeError(
            "Root is required for the real installer and service managers"
        )
    if args.os == "linux" and not Path("/run/systemd/system").is_dir():
        raise RuntimeError(
            "A native systemd host is required; this environment cannot run the deployment test"
        )
    if installation_root(args.os).exists():
        raise RuntimeError("Refusing to overwrite an existing Radegast installation")
    for path in (
        args.updater,
        args.verifier,
        args.backend_dir / ".github/tests/integration_test.py",
    ):
        if not path.is_file():
            raise FileNotFoundError(path)


def load_harness(backend):
    directory = backend / ".github/tests"
    sys.path.insert(0, str(directory))
    spec = importlib.util.spec_from_file_location(
        "backend_integration", directory / "integration_test.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@contextlib.contextmanager
def serve(directory):
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0),
        functools.partial(SimpleHTTPRequestHandler, directory=str(directory)),
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def checked(command):
    res = subprocess.run(
        command, capture_output=True, text=True, timeout=60
    )
    if res.returncode != 0:
        raise RuntimeError(
            f"Command {command} failed with exit code {res.returncode}:\n"
            f"STDOUT: {res.stdout.strip()}\n"
            f"STDERR: {res.stderr.strip()}"
        )
    return res.stdout


def sensor_pid(os_name):
    if os_name == "linux":
        value = checked(
            ["systemctl", "show", "rustinel", "--property=MainPID", "--value"]
        ).strip()
    elif os_name == "windows":
        match = re.search(
            r"PID\s*:\s*(\d+)", checked(["sc.exe", "queryex", "RadegastRustinel"])
        )
        value = match[1] if match else "0"
    else:
        match = re.search(
            r"\bpid\s*=\s*(\d+)",
            checked(["launchctl", "print", "system/io.rustinel.daemon"]),
        )
        value = match[1] if match else "0"
    pid = int(value)
    if pid <= 0:
        raise RuntimeError("Rustinel service has no running PID")
    return pid


def wait_sensor_pid(os_name, timeout=30):
    deadline = time.monotonic() + timeout
    while True:
        try:
            return sensor_pid(os_name)
        except (RuntimeError, subprocess.CalledProcessError):
            if time.monotonic() >= deadline:
                raise
            time.sleep(1)


def ensure_log_permissions(os_name):
    if os_name == "mac":
        log_dir = Path("/Library/Logs/Radegast")
        if log_dir.exists():
            subprocess.run(
                ["chmod", "2750", str(log_dir)],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["chown", "-R", ":_radegast", str(log_dir)],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["chmod", "-R", "g+rX", str(log_dir)],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                [
                    "chmod",
                    "+a",
                    "group:_radegast allow read,readattr,readextattr,readsecurity,file_inherit",
                    str(log_dir),
                ],
                check=False,
                capture_output=True,
            )
            for path in log_dir.glob("alerts.json*"):
                try:
                    mode = path.stat().st_mode
                    if mode & 0o040 == 0:
                        path.chmod(mode | 0o640)
                except OSError:
                    pass
    elif os_name == "linux":
        for dir_str in ("/var/log/radegast", "/opt/radegast/logs"):
            log_dir = Path(dir_str)
            if log_dir.exists():
                subprocess.run(
                    ["chmod", "2750", str(log_dir)],
                    check=False,
                    capture_output=True,
                )
                for path in log_dir.glob("alerts.json*"):
                    try:
                        mode = path.stat().st_mode
                        if mode & 0o040 == 0:
                            path.chmod(mode | 0o640)
                    except OSError:
                        pass


@contextlib.contextmanager
def log_permission_enforcer(os_name):
    if os_name not in ("mac", "linux"):
        yield
        return

    stop_event = threading.Event()

    def _loop():
        while not stop_event.is_set():
            ensure_log_permissions(os_name)
            stop_event.wait(0.5)

    thread = threading.Thread(target=_loop, daemon=True)
    thread.start()
    try:
        yield
    finally:
        stop_event.set()


def agent_control(os_name, start):
    if os_name == "linux":
        checked(["systemctl", "start" if start else "stop", "radegast-agent"])
    elif os_name == "windows":
        checked(["sc.exe", "start" if start else "stop", "RadegastAgent"])
        desired = "RUNNING" if start else "STOPPED"
        for _ in range(30):
            if desired in checked(["sc.exe", "query", "RadegastAgent"]):
                return
            time.sleep(1)
        raise RuntimeError(f"Agent did not reach {desired}")
    else:
        plist = "/Library/LaunchDaemons/app.radegast.agent.plist"
        label = "app.radegast.agent"
        if start:
            ensure_log_permissions("mac")
            subprocess.run(
                ["chown", "root:wheel", plist],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["chmod", "644", plist],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["launchctl", "enable", f"system/{label}"],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["launchctl", "load", "-w", plist],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["launchctl", "bootstrap", "system", plist],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["launchctl", "kickstart", "-k", f"system/{label}"],
                check=False,
                capture_output=True,
            )

            for _ in range(30):
                res = subprocess.run(
                    ["launchctl", "print", f"system/{label}"],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                if res.returncode == 0 and re.search(r"state\s*=\s*running", res.stdout):
                    return
                time.sleep(1)
                subprocess.run(
                    ["launchctl", "kickstart", "-k", f"system/{label}"],
                    check=False,
                    capture_output=True,
                )
        else:
            subprocess.run(
                ["launchctl", "bootout", f"system/{label}"],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["launchctl", "bootout", "system", plist],
                check=False,
                capture_output=True,
            )
            subprocess.run(
                ["launchctl", "unload", plist],
                check=False,
                capture_output=True,
            )
            for _ in range(30):
                res = subprocess.run(
                    ["launchctl", "print", f"system/{label}"],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                if res.returncode != 0 or not re.search(r"state\s*=\s*running", res.stdout):
                    return
                time.sleep(1)


def verify_installed(path, release):
    if file_hash(path) != release["binary_sha256"]:
        raise RuntimeError("Installed binary does not match the signed release archive")
    output = checked([str(path), "--version"])
    if not any(
        token.removeprefix("v") == release["version"].removeprefix("v")
        for token in output.split()
    ):
        raise RuntimeError(f"Unexpected installed version: {output.strip()}")


def baseline_health(db_path, device, timeout):
    """Require a completed agent probe; an unhealthy baseline may be recovered."""
    deadline = time.monotonic() + timeout
    while True:
        with contextlib.closing(sqlite3.connect(db_path)) as db:
            row = db.execute(
                "SELECT healthy FROM devices WHERE id = ?", (device,)
            ).fetchone()
        if row is not None and row[0] is not None:
            return bool(row[0])
        if time.monotonic() >= deadline:
            raise RuntimeError("Baseline agent never reported a completed healthcheck")
        time.sleep(2)


def run_updater(args, manifest_url, download_url, binary, label, success):
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("UPDATER_") and key != "RUST_LOG"
    }
    env.update(
        UPDATER_MANIFEST_URL=manifest_url,
        UPDATER_DOWNLOAD_URL=download_url,
        UPDATER_RUSTINEL_PATH=str(binary),
        UPDATER_AUTO_RESTART="true",
        UPDATER_LOG_LEVEL="info",
    )
    with (args.output / f"updater-{label}.log").open("w", encoding="utf-8") as log:
        result = subprocess.run(
            [str(args.updater), "--once"],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=900,
            check=False,
        )
    if (result.returncode == 0) != success:
        raise RuntimeError(
            f"Updater {label} returned {result.returncode}; see updater-{label}.log"
        )


def run(args):
    args.output.mkdir(parents=True, exist_ok=True)
    result = {
        "status": "FAIL",
        "stage": "preflight",
        "os": args.os,
        "arch": args.arch,
        "checks": [],
    }
    backend = harness = None
    install_attempted = False
    original_perf = None
    try:
        preflight(args)
        harness = load_harness(args.backend_dir)
        metadata = json.loads((args.assets / "fixtures.json").read_text())
        if (metadata["os"], metadata["arch"]) != (args.os, args.arch):
            raise RuntimeError("Release fixtures do not match this runner")
        baseline, candidate = metadata["releases"]
        if version_key(candidate["version"]) <= version_key(baseline["version"]):
            raise RuntimeError("Candidate must be strictly newer than baseline")
        for release in (baseline, candidate):
            archive = args.assets / f"{release['label']}.zip"
            subprocess.run(
                [
                    str(args.verifier),
                    str(args.assets / f"{release['label']}.json"),
                    str(archive),
                    args.os,
                    args.arch,
                ],
                check=True,
                timeout=120,
            )
            if (
                file_hash(archive) != release["archive_sha256"]
                or archive_binary_hash(archive, args.os) != release["binary_sha256"]
            ):
                raise RuntimeError("Fixture provenance changed after preparation")
        result.update(
            baseline=baseline["version"],
            candidate=candidate["version"],
            updater_sha256=file_hash(args.updater),
        )
        shutil.copy2(args.assets / "fixtures.json", args.output / "fixtures.json")
        result["stage"] = "prepare native runner"
        if args.os == "linux":
            original_perf = harness.prepare_linux_perf_events()
        elif args.os == "mac" and os.environ.get("GITHUB_ACTIONS") == "true":
            harness.grant_macos_full_disk_access(
                Path("/Library/Application Support/com.apple.TCC/TCC.db")
            )
        result["stage"] = "start backend"
        backend = harness.BackendManager(args.backend_dir, output_dir=args.output)
        backend.start()
        session, group = harness.setup_admin_and_crypto(
            backend.base_url, backend.db_path
        )
        for release in (baseline, candidate):
            harness.upload_release_zip(
                backend.base_url,
                session,
                args.assets / f"{release['label']}.zip",
                release["version"],
                args.arch,
                args.os,
            )
        device, token = harness.create_device(backend.base_url, session, group)
        result["stage"] = "install baseline"
        install_attempted = True
        harness.execute_installation(
            backend.base_url, token, baseline["version"], args.os, args.output
        )
        with log_permission_enforcer(args.os):
            binary = installation_root(args.os) / (
                "rustinel/rustinel/rustinel.exe"
                if args.os == "windows"
                else "rustinel/rustinel"
            )
            harness.verify_services_running(args.os)
            verify_installed(binary, baseline)
            result["stage"] = "observe baseline healthcheck"
            result["baseline_healthy"] = baseline_health(
                backend.db_path, device, args.timeout
            )
            result["scenario"] = (
                "healthy upgrade"
                if result["baseline_healthy"]
                else "unhealthy baseline recovery"
            )
            print(
                f"Baseline health: {result['baseline_healthy']}; testing {result['scenario']}",
                flush=True,
            )
            result["checks"].append(
                "baseline installation and completed native healthcheck"
            )
            previous_pid = sensor_pid(args.os)
            # Serve one verified candidate so later upstream publications cannot change this run.
            candidate_entry = json.loads((args.assets / "candidate.json").read_text())
            manifest_dir = args.output / "http"
            manifest_dir.mkdir(exist_ok=True)
            manifest = manifest_dir / "update-manifest.json"
            snapshot = json.loads((args.assets / "snapshot.json").read_text())
            bad_snapshot = [
                dict(entry, sign_gpg="invalid signature")
                if entry["version"] == candidate_entry["version"]
                else entry
                for entry in snapshot
            ]
            manifest.write_text(json.dumps(bad_snapshot), encoding="utf-8")
            with serve(manifest_dir) as server:
                url = server + "/update-manifest.json"
                result["stage"] = "reject tampered update"
                run_updater(args, url, backend.base_url, binary, "tampered", False)
                verify_installed(binary, baseline)
                if sensor_pid(args.os) != previous_pid:
                    raise RuntimeError("Rejected update disrupted the running sensor")
                result["checks"].append(
                    "tampered signature rejected without service disruption"
                )
                # Discard all prior health state. A freshly started agent generates a new
                # probe UUID, so a cached healthy=True or old queued alert cannot pass.
                agent_control(args.os, False)
                with contextlib.closing(sqlite3.connect(backend.db_path)) as db, db:
                    db.execute("UPDATE devices SET healthy = NULL WHERE id = ?", (device,))
                manifest.write_text(json.dumps(snapshot), encoding="utf-8")
                result["stage"] = "apply real signed update"
                run_updater(args, url, backend.base_url, binary, "upgrade", True)
                verify_installed(binary, candidate)
                new_pid = wait_sensor_pid(args.os)
                if new_pid == previous_pid:
                    raise RuntimeError("Updater did not restart the sensor")
                result["checks"].append(
                    "signed upgrade, exact installed bytes, version, and new service PID"
                )
                result["stage"] = "fresh post-update healthcheck"
                agent_control(args.os, True)
                harness.verify_services_running(args.os)
                if not harness.verify_healthcheck_alert(
                    backend.base_url, session, device, timeout_seconds=args.timeout
                ):
                    raise RuntimeError("Updated sensor failed its fresh native healthcheck")
                time.sleep(5)
                harness.verify_services_running(args.os)
                if sensor_pid(args.os) != new_pid:
                    raise RuntimeError(
                        "Updated sensor restarted unexpectedly during healthcheck"
                    )
                result["checks"].append(
                    "fresh post-update sensor-to-agent-to-backend healthcheck"
                )
                result["stage"] = "repeat update is a no-op"
                run_updater(args, url, backend.base_url, binary, "noop", True)
                verify_installed(binary, candidate)
                if sensor_pid(args.os) != new_pid:
                    raise RuntimeError("No-op update restarted the sensor")
                result["checks"].append("repeat update preserves binary and service PID")
        result.update(status="PASS", stage="complete")
    except Exception as error:  # noqa: BLE001 - persist stage and traceback on every test failure
        result["error"] = str(error)
        (args.output / "failure.log").write_text(
            traceback.format_exc(), encoding="utf-8"
        )
    finally:
        if backend is not None:
            try:
                harness.collect_diagnostics(
                    args.output,
                    result["status"] == "PASS",
                    result.get("candidate", "unknown"),
                    args.os,
                    args.arch,
                    result["stage"],
                    result.get("error"),
                )
            except Exception:  # noqa: BLE001 - diagnostics must not prevent cleanup
                (args.output / "diagnostics-error.log").write_text(
                    traceback.format_exc()
                )
        cleanup = []
        if backend is not None:
            if install_attempted:
                cleanup.append(lambda: harness.execute_uninstallation(args.os))
            cleanup.append(backend.stop)
        if original_perf is not None:
            cleanup.append(lambda: harness.set_linux_perf_paranoid(original_perf))
        for action in cleanup:
            try:
                action()
            except Exception as error:  # noqa: BLE001 - attempt all cleanup and record failure
                result["status"] = "FAIL"
                result.setdefault("cleanup_errors", []).append(str(error))
        (args.output / "update-result.json").write_text(
            json.dumps(result, indent=2), encoding="utf-8"
        )
    print(json.dumps(result, indent=2), flush=True)
    return 0 if result["status"] == "PASS" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("prepare", "run"))
    parser.add_argument(
        "--backend-dir", type=Path, default=ROOT.parent / "radegast-console-backend"
    )
    parser.add_argument(
        "--assets", type=Path, default=ROOT / "target/real-update-assets"
    )
    parser.add_argument(
        "--output", type=Path, default=ROOT / "target/real-update-output"
    )
    parser.add_argument(
        "--updater",
        type=Path,
        default=ROOT
        / "target/debug"
        / (
            "radegast-rustinel-updater.exe"
            if os.name == "nt"
            else "radegast-rustinel-updater"
        ),
    )
    parser.add_argument(
        "--verifier",
        type=Path,
        default=ROOT
        / "target/debug/examples"
        / ("verify_release.exe" if os.name == "nt" else "verify_release"),
    )
    parser.add_argument("--os", default=detect_platform()[0])
    parser.add_argument("--arch", default=detect_platform()[1])
    parser.add_argument("--manifest", type=Path, default=None)
    parser.add_argument(
        "--baseline-version", default=None, help="Installed release to upgrade from"
    )
    parser.add_argument(
        "--candidate-version",
        help="Target release; defaults to the latest for this platform",
    )
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument("--disposable-host", action="store_true")
    args = parser.parse_args()
    if args.manifest is not None:
        args.manifest = args.manifest.resolve()
    for name in ("backend_dir", "assets", "output", "updater", "verifier"):
        setattr(args, name, getattr(args, name).resolve())
    if args.mode == "prepare":
        prepare(args)
        return 0
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
