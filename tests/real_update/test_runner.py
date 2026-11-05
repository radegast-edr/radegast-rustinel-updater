"""Orchestration regressions; these tests never install software or touch services."""

import argparse
import contextlib
import io
import json
import sqlite3
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import Mock, patch

import run as runner


def entry(version, filename="linux-amd64.zip", **kwargs):
    return dict(
        version=version,
        hash_sha256=f"{'a' * 64}  {filename}\n",
        sign_gpg="signature",
        **kwargs,
    )


class FixtureTests(unittest.TestCase):
    def test_selects_distinct_latest_platform_releases(self):
        entries = [
            entry("1.8.0"),
            entry("1.9.1r1"),
            entry("1.9.1"),
            entry("1.9.1r1"),
            entry("99.0.0", "windows-amd64.zip"),
        ]
        self.assertEqual(
            [e["version"] for e in runner.select_pair(entries, "linux", "amd64")],
            ["1.9.1", "1.9.1r1"],
        )

    def test_never_bypasses_cross_platform_autoupdate_veto(self):
        entries = [
            entry("1.8.0"),
            entry("1.9.1"),
            entry("1.9.0", "windows-amd64.zip", autoupdate=False),
        ]
        with self.assertRaisesRegex(ValueError, "blocked"):
            runner.select_pair(entries, "linux", "amd64")

    def test_requires_two_releases_and_exact_filename(self):
        with self.assertRaises(ValueError):
            runner.select_pair(
                [entry("1.0.0"), entry("2.0.0", "not-linux-amd64.zip")],
                "linux",
                "amd64",
            )

    def test_explicit_baseline_upgrades_to_latest_available_release(self):
        pair = runner.select_pair(
            [entry("1.7.1"), entry("1.9.1"), entry("2.0.0", autoupdate=True)],
            "linux",
            "amd64",
            "1.7.1",
        )
        self.assertEqual([item["version"] for item in pair], ["1.7.1", "2.0.0"])

    def test_explicit_candidate_and_invalid_pairs(self):
        entries = [entry("1.7.1"), entry("1.9.1"), entry("2.0.0")]
        pair = runner.select_pair(entries, "linux", "amd64", "1.7.1", "1.9.1")
        self.assertEqual(pair[1]["version"], "1.9.1")
        for baseline, candidate in [
            ("9.0.0", None),
            ("2.0.0", "1.9.1"),
            ("1.7.1", "9.0.0"),
        ]:
            with self.assertRaises(ValueError):
                runner.select_pair(entries, "linux", "amd64", baseline, candidate)

    def test_explicit_candidate_does_not_bypass_later_veto(self):
        entries = [entry("1.7.1"), entry("1.9.1"), entry("2.0.0", autoupdate=False)]
        with self.assertRaisesRegex(ValueError, "blocked"):
            runner.select_pair(entries, "linux", "amd64", "1.7.1", "1.9.1")

    def test_invalid_versions_rejected(self):
        for value in ("1.0.0/../../etc", "1.0", "1.0.0;exit", "1.0.0r"):
            with self.assertRaises(ValueError):
                runner.version_key(value)

    def test_hash_is_of_expected_executable_not_whole_archive(self):
        import hashlib

        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "release.zip"
            with zipfile.ZipFile(archive, "w") as bundle:
                bundle.writestr("rustinel", b"binary")
                bundle.writestr("migrate.sh", b"script")
            self.assertEqual(
                runner.archive_binary_hash(archive, "linux"),
                hashlib.sha256(b"binary").hexdigest(),
            )

    def test_installed_version_and_content_must_both_match(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "rustinel"
            binary.write_bytes(b"actual")
            release = {"version": "1.9.1", "binary_sha256": runner.file_hash(binary)}
            with (
                patch.object(runner, "checked", return_value="rustinel 1.8.0"),
                self.assertRaisesRegex(RuntimeError, "version"),
            ):
                runner.verify_installed(binary, release)
            with patch.object(runner, "checked", return_value="rustinel 1.9.1"):
                runner.verify_installed(binary, release)
            binary.write_bytes(b"tampered")
            with self.assertRaisesRegex(RuntimeError, "signed release"):
                runner.verify_installed(binary, release)

    def test_prepare_downloads_and_verifies_live_pair_and_freezes_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(
                assets=Path(directory),
                os="linux",
                arch="amd64",
                verifier=Path("verifier"),
                manifest=None,
                baseline_version="1.7.1",
                candidate_version=None,
            )
            entries = [entry("1.7.1"), entry("1.9.1"), entry("2.0.0", autoupdate=True)]
            urls = []

            def fetch(url, path, limit):
                urls.append(url)
                if path.name == "manifest.json":
                    path.write_text(json.dumps(entries))
                else:
                    with zipfile.ZipFile(path, "w") as archive:
                        archive.writestr("rustinel", path.name.encode())

            with (
                patch.object(runner, "fetch", side_effect=fetch),
                patch.object(runner.subprocess, "run") as verify,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                runner.prepare(args)
            metadata = json.loads((args.assets / "fixtures.json").read_text())
            self.assertEqual(
                [release["version"] for release in metadata["releases"]],
                ["1.7.1", "2.0.0"],
            )
            self.assertEqual(metadata["manifest_source"], runner.MANIFEST_URL)
            self.assertEqual(
                json.loads((args.assets / "snapshot.json").read_text()), entries
            )
            self.assertEqual(verify.call_count, 2)
            self.assertIn("/2.0.0/linux/amd64/download", urls[-1])

    def test_production_signature_verifier_rejects_tampering(self):
        # A command failure must stop fixture preparation before recording it as verified.
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(
                assets=Path(directory),
                os="linux",
                arch="amd64",
                verifier=Path("verifier"),
                manifest=None,
                baseline_version="1.0.0",
                candidate_version="2.0.0",
            )

            def fetch(url, path, limit):
                path.write_text(json.dumps([entry("1.0.0"), entry("2.0.0")]))

            with (
                patch.object(runner, "fetch", side_effect=fetch),
                patch.object(
                    runner.subprocess, "run", side_effect=RuntimeError("bad signature")
                ),
                self.assertRaisesRegex(RuntimeError, "bad signature"),
            ):
                runner.prepare(args)
            self.assertFalse((args.assets / "fixtures.json").exists())


class GuardTests(unittest.TestCase):
    def test_windows_defaults_to_live_manifest_and_latest_candidate(self):
        with (
            patch.object(
                runner.sys,
                "argv",
                ["run.py", "prepare", "--os", "windows", "--arch", "amd64"],
            ),
            patch.object(runner, "prepare") as prepare,
        ):
            self.assertEqual(runner.main(), 0)
            self.assertIsNone(prepare.call_args.args[0].manifest)
            self.assertIsNone(prepare.call_args.args[0].candidate_version)
            self.assertIsNone(prepare.call_args.args[0].baseline_version)

    def test_refuses_unacknowledged_host_before_installing(self):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(
                output=Path(directory), disposable_host=False, os="linux", arch="amd64"
            )
            with (
                patch.object(runner, "load_harness") as load,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                self.assertEqual(runner.run(args), 1)
                load.assert_not_called()
            result = json.loads((args.output / "update-result.json").read_text())
            self.assertEqual(result["stage"], "preflight")
            self.assertEqual(result["status"], "FAIL")

    def test_service_requires_real_running_pid(self):
        with (
            patch.object(runner, "checked", return_value="0\n"),
            self.assertRaisesRegex(RuntimeError, "running PID"),
        ):
            runner.sensor_pid("linux")
        with patch.object(
            runner, "checked", return_value="STATE : 4 RUNNING\nPID : 123"
        ):
            self.assertEqual(runner.sensor_pid("windows"), 123)
        with patch.object(runner, "checked", return_value="state = running\npid = 456"):
            self.assertEqual(runner.sensor_pid("mac"), 456)

    def test_waits_for_sensor_startup_but_never_accepts_timeout(self):
        with (
            patch.object(
                runner, "sensor_pid", side_effect=[RuntimeError("starting"), 123]
            ),
            patch.object(runner.time, "sleep"),
        ):
            self.assertEqual(runner.wait_sensor_pid("windows"), 123)
        with (
            patch.object(runner, "sensor_pid", side_effect=RuntimeError("stopped")),
            self.assertRaisesRegex(RuntimeError, "stopped"),
        ):
            runner.wait_sensor_pid("windows", timeout=0)

    def test_updater_exit_status_and_environment_are_enforced(self):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(output=Path(directory), updater=Path("updater"))
            with (
                patch.dict(
                    runner.os.environ,
                    {"UPDATER_AUTO_RESTART": "false", "UPDATER_LOG_LEVEL": "off"},
                ),
                patch.object(
                    runner.subprocess, "run", return_value=Mock(returncode=1)
                ) as execute,
            ):
                with self.assertRaisesRegex(RuntimeError, "returned 1"):
                    runner.run_updater(
                        args,
                        "http://localhost/manifest",
                        "http://localhost/api/v1",
                        Path("rustinel"),
                        "upgrade",
                        True,
                    )
                env = execute.call_args.kwargs["env"]
                self.assertEqual(env["UPDATER_AUTO_RESTART"], "true")
                self.assertEqual(env["UPDATER_LOG_LEVEL"], "info")


class BaselineHealthTests(unittest.TestCase):
    def test_requires_report_but_accepts_healthy_or_unhealthy(self):
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "db.sqlite"
            with contextlib.closing(sqlite3.connect(database)) as db, db:
                db.execute(
                    "CREATE TABLE devices (id INTEGER PRIMARY KEY, healthy INTEGER)"
                )
                db.execute("INSERT INTO devices VALUES (1, NULL)")
            with self.assertRaisesRegex(RuntimeError, "never reported"):
                runner.baseline_health(database, 1, 0)
            for value in (0, 1):
                with contextlib.closing(sqlite3.connect(database)) as db, db:
                    db.execute("UPDATE devices SET healthy = ?", (value,))
                self.assertIs(runner.baseline_health(database, 1, 0), bool(value))

    def test_waits_for_completed_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "db.sqlite"
            with contextlib.closing(sqlite3.connect(database)) as db, db:
                db.execute(
                    "CREATE TABLE devices (id INTEGER PRIMARY KEY, healthy INTEGER)"
                )
                db.execute("INSERT INTO devices VALUES (1, NULL)")

            def report(_):
                with contextlib.closing(sqlite3.connect(database)) as db, db:
                    db.execute("UPDATE devices SET healthy = 0 WHERE id = 1")

            with patch.object(runner.time, "sleep", side_effect=report):
                self.assertIs(runner.baseline_health(database, 1, 30), False)


class LifecycleTests(unittest.TestCase):
    def exercise(self, post_health=True, restart=True, baseline_healthy=True):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            assets = root / "assets"
            assets.mkdir()
            updater = root / "updater"
            updater.write_bytes(b"checkout updater")
            releases = []
            for label, version in (("baseline", "1.0.0"), ("candidate", "2.0.0")):
                archive = assets / f"{label}.zip"
                with zipfile.ZipFile(archive, "w") as bundle:
                    bundle.writestr("rustinel", version.encode())
                (assets / f"{label}.json").write_text(json.dumps(entry(version)))
                releases.append(
                    {
                        "label": label,
                        "version": version,
                        "archive_sha256": runner.file_hash(archive),
                        "binary_sha256": runner.archive_binary_hash(archive, "linux"),
                    }
                )
            (assets / "fixtures.json").write_text(
                json.dumps({"os": "linux", "arch": "amd64", "releases": releases})
            )
            (assets / "snapshot.json").write_text(
                json.dumps([entry("1.0.0"), entry("2.0.0")])
            )
            backend = Mock(
                db_path=root / "db.sqlite", base_url="http://127.0.0.1/api/v1"
            )
            with contextlib.closing(sqlite3.connect(backend.db_path)) as db, db:
                db.execute(
                    "CREATE TABLE devices (id INTEGER PRIMARY KEY, healthy INTEGER)"
                )
                db.execute("INSERT INTO devices VALUES (1, 1)")
            harness = Mock()
            harness.BackendManager.return_value = backend
            harness.setup_admin_and_crypto.return_value = ("session", 1)
            harness.create_device.return_value = (1, "token")
            harness.prepare_linux_perf_events.return_value = None
            events = []

            def health(*args, **kwargs):
                with contextlib.closing(sqlite3.connect(backend.db_path)) as db, db:
                    self.assertIsNone(
                        db.execute("SELECT healthy FROM devices").fetchone()[0]
                    )
                self.assertEqual(events[-1], "agent started")
                events.append("fresh probe")
                return post_health

            harness.verify_healthcheck_alert.side_effect = health
            args = argparse.Namespace(
                output=root / "output",
                assets=assets,
                updater=updater,
                verifier=root / "verifier",
                backend_dir=root,
                os="linux",
                arch="amd64",
                timeout=1,
            )
            with (
                patch.object(runner, "preflight"),
                patch.object(runner, "load_harness", return_value=harness),
                patch.object(runner.subprocess, "run"),
                patch.object(runner, "verify_installed"),
                patch.object(runner, "baseline_health", return_value=baseline_healthy),
                patch.object(
                    runner, "run_updater", side_effect=lambda *a: events.append(a[-2])
                ),
                patch.object(
                    runner,
                    "sensor_pid",
                    side_effect=[10, 10, 11 if restart else 10, 11, 11],
                ),
                patch.object(
                    runner,
                    "agent_control",
                    side_effect=lambda os_name, start: events.append(
                        "agent started" if start else "agent stopped"
                    ),
                ),
                patch.object(
                    runner,
                    "serve",
                    return_value=contextlib.nullcontext("http://localhost"),
                ),
                patch.object(runner.time, "sleep"),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                status = runner.run(args)
            harness.execute_uninstallation.assert_called_once_with("linux")
            backend.stop.assert_called_once()
            return (
                status,
                json.loads((args.output / "update-result.json").read_text()),
                events,
            )

    def test_success_requires_fresh_probe_and_noop(self):
        status, result, events = self.exercise()
        self.assertEqual(status, 0)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(
            events,
            [
                "tampered",
                "agent stopped",
                "upgrade",
                "agent started",
                "fresh probe",
                "noop",
            ],
        )

    def test_unhealthy_baseline_recovers_only_with_fresh_health(self):
        status, result, events = self.exercise(baseline_healthy=False)
        self.assertEqual(status, 0)
        self.assertIs(result["baseline_healthy"], False)
        self.assertEqual(result["scenario"], "unhealthy baseline recovery")
        self.assertIn("fresh probe", events)
        self.assertIn("noop", events)

    def test_unhealthy_candidate_still_fails_recovery(self):
        status, result, events = self.exercise(
            baseline_healthy=False, post_health=False
        )
        self.assertEqual(status, 1)
        self.assertEqual(result["stage"], "fresh post-update healthcheck")
        self.assertNotIn("noop", events)

    def test_stale_baseline_health_cannot_mask_broken_update(self):
        status, result, events = self.exercise(post_health=False)
        self.assertEqual(status, 1)
        self.assertEqual(result["stage"], "fresh post-update healthcheck")
        self.assertNotIn("noop", events)

    def test_success_exit_without_service_restart_is_failure(self):
        status, result, events = self.exercise(restart=False)
        self.assertEqual(status, 1)
        self.assertIn("did not restart", result["error"])
        self.assertNotIn("fresh probe", events)


if __name__ == "__main__":
    unittest.main()
