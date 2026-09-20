import fcntl
import importlib.util
import json
import pathlib
import plistlib
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "smoke-packaged-macos-app.py"
SPEC = importlib.util.spec_from_file_location("smoke_packaged_macos_app", MODULE_PATH)
SMOKE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = SMOKE
SPEC.loader.exec_module(SMOKE)


class PackagedMacOSSmokeTests(unittest.TestCase):
    def fixture_bundle(self, root: pathlib.Path) -> pathlib.Path:
        app = root / "Bundle With Spaces" / "Bridge.app"
        macos = app / "Contents" / "MacOS"
        macos.mkdir(parents=True)
        with (app / "Contents" / "Info.plist").open("wb") as target:
            plistlib.dump(
                {
                    "CFBundleExecutable": "bridge-deck",
                    "CFBundleIdentifier": "dev.bridge.deck",
                    "CFBundleShortVersionString": "9.8.7",
                },
                target,
            )
        for name in ("bridge-deck", "bridged"):
            binary = macos / name
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o700)
        return app

    def fixture_state(self, root: pathlib.Path) -> pathlib.Path:
        data_dir = root / "data"
        data_dir.mkdir()
        (data_dir / "bridge.db").write_bytes(b"fixture")
        token = data_dir / "daemon.token"
        token.write_text("a" * 64)
        token.chmod(0o600)
        (data_dir / "owner.lock").write_bytes(b"fixture")
        return data_dir

    def test_bundle_metadata_selects_the_exact_packaged_executables(self):
        with tempfile.TemporaryDirectory() as directory:
            app = self.fixture_bundle(pathlib.Path(directory))
            bundle = SMOKE.load_bundle(app)
            self.assertEqual(bundle.app, app.resolve())
            self.assertEqual(bundle.executable, app.resolve() / "Contents/MacOS/bridge-deck")
            self.assertEqual(bundle.daemon, app.resolve() / "Contents/MacOS/bridged")
            self.assertEqual(bundle.identifier, "dev.bridge.deck")
            self.assertEqual(bundle.version, "9.8.7")

            bundle.daemon.unlink()
            with self.assertRaisesRegex(SMOKE.SmokeFailure, "bundled bridged daemon"):
                SMOKE.load_bundle(app)

    def test_runtime_environment_is_isolated_and_credential_free(self):
        original = {
            "LANG": "en_US.UTF-8",
            "USER": "fixture-user",
            "OPENAI_API_KEY": "must-not-leak",
            "SSH_AUTH_SOCK": "/must/not/leak",
            "OPENCODE_CONFIG_DIR": "/must/not/leak",
            "BRIDGE_CLAUDE_SIDECAR": "/must/not/leak",
            "FUTURE_PROVIDER_SECRET": "must-not-leak",
        }
        home = pathlib.Path("/tmp/isolated-home")
        data_dir = pathlib.Path("/tmp/isolated-data")
        temp_dir = pathlib.Path("/tmp/isolated-temp")
        environment = SMOKE.smoke_environment(original, home, data_dir, temp_dir)

        self.assertEqual(environment["PATH"], "/usr/bin:/bin:/usr/sbin:/sbin")
        self.assertEqual(environment["SHELL"], "/bin/sh")
        self.assertEqual(environment["LANG"], "en_US.UTF-8")
        self.assertEqual(environment["USER"], "fixture-user")
        self.assertEqual(environment["HOME"], str(home))
        self.assertEqual(environment["TMPDIR"], str(temp_dir))
        self.assertEqual(environment["BRIDGE_DATA_DIR"], str(data_dir))
        self.assertEqual(environment["BRIDGE_DESKTOP_HOST"], "daemon")
        self.assertEqual(environment["BRIDGE_PACKAGED_SMOKE"], "1")
        for name in (
            "OPENAI_API_KEY",
            "SSH_AUTH_SOCK",
            "OPENCODE_CONFIG_DIR",
            "BRIDGE_CLAUDE_SIDECAR",
            "FUTURE_PROVIDER_SECRET",
        ):
            self.assertNotIn(name, environment)

    def test_complete_runtime_evidence_accepts_only_a_clean_app_owned_shutdown(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            bundle = SMOKE.load_bundle(self.fixture_bundle(root))
            data_dir = self.fixture_state(root)
            app_log = "\n".join(
                [
                    "bridge: started bridged (pid 99999999) for fixture",
                    SMOKE.APP_ATTACHED,
                    SMOKE.SMOKE_PASSED,
                    SMOKE.DAEMON_EXITED_CLEANLY,
                ]
            )
            daemon_log = "\n".join(
                [
                    f"bridged: serving {data_dir}/bridged.sock (data dir {data_dir})",
                    SMOKE.DAEMON_SHUTDOWN_COMPLETE,
                ]
            )

            self.assertEqual(
                SMOKE.validate_runtime_evidence(bundle, data_dir, app_log, daemon_log, 0),
                99999999,
            )

            failures = [
                (app_log.replace(SMOKE.APP_ATTACHED, ""), daemon_log, 0, "missing"),
                (app_log.replace(SMOKE.SMOKE_PASSED, ""), daemon_log, 0, "missing"),
                (
                    app_log.replace(SMOKE.DAEMON_EXITED_CLEANLY, ""),
                    daemon_log,
                    0,
                    "missing",
                ),
                (app_log + "\n" + SMOKE.SMOKE_FAILED, daemon_log, 0, "failed"),
                (
                    app_log + "\n" + SMOKE.DAEMON_EXITED_UNCLEANLY,
                    daemon_log,
                    0,
                    "uncleanly",
                ),
                (
                    app_log + "\n" + SMOKE.DAEMON_EXITED_EARLY,
                    daemon_log,
                    0,
                    "before desktop shutdown",
                ),
                (app_log, daemon_log, 7, "status 7"),
                (app_log, "", 0, "isolated data directory"),
                (
                    app_log,
                    daemon_log.replace(SMOKE.DAEMON_SHUTDOWN_COMPLETE, ""),
                    0,
                    "acknowledge graceful shutdown",
                ),
            ]
            for candidate_app_log, candidate_daemon_log, status, message in failures:
                with self.subTest(message=message):
                    with self.assertRaisesRegex(SMOKE.SmokeFailure, message):
                        SMOKE.validate_runtime_evidence(
                            bundle, data_dir, candidate_app_log, candidate_daemon_log, status
                        )

            socket_path = data_dir / "bridged.sock"
            socket_path.write_bytes(b"stale")
            with self.assertRaisesRegex(SMOKE.SmokeFailure, "socket survived"):
                SMOKE.validate_runtime_evidence(bundle, data_dir, app_log, daemon_log, 0)

    def test_token_permissions_and_owner_lease_are_enforced(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            bundle = SMOKE.load_bundle(self.fixture_bundle(root))
            data_dir = self.fixture_state(root)
            app_log = "\n".join(
                [
                    "bridge: started bridged (pid 99999999) for fixture",
                    SMOKE.APP_ATTACHED,
                    SMOKE.SMOKE_PASSED,
                    SMOKE.DAEMON_EXITED_CLEANLY,
                ]
            )
            daemon_log = "\n".join(
                [
                    f"bridged: serving socket (data dir {data_dir})",
                    SMOKE.DAEMON_SHUTDOWN_COMPLETE,
                ]
            )
            token = data_dir / "daemon.token"
            token.chmod(0o644)
            with self.assertRaisesRegex(SMOKE.SmokeFailure, "permissions are 644"):
                SMOKE.validate_runtime_evidence(bundle, data_dir, app_log, daemon_log, 0)
            token.chmod(0o600)

            owner = data_dir / "owner.lock"
            with owner.open("r+b") as held:
                fcntl.flock(held.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                # BSD and Linux both report a second open description as busy.
                self.assertFalse(SMOKE.owner_lease_is_free(owner))
                fcntl.flock(held.fileno(), fcntl.LOCK_UN)
            self.assertTrue(SMOKE.owner_lease_is_free(owner))

    def test_ci_job_and_build_profile_cannot_consume_release_credentials(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        job = workflow.split("  macos-package-smoke:\n", 1)[1].split("\n  sidecar:\n", 1)[0]
        self.assertIn("runs-on: macos-15", job)
        self.assertIn("timeout-minutes: 60", job)
        self.assertIn("npm run build:macos-smoke", job)
        self.assertIn("npm run smoke:macos-app", job)
        for forbidden in ("secrets.", "release-dmg", "notarize", "verify-macos-app"):
            self.assertNotIn(forbidden, job)

        profile = json.loads((ROOT / "src-tauri/tauri.macos-smoke.conf.json").read_text())
        self.assertFalse(profile["bundle"]["createUpdaterArtifacts"])
        self.assertEqual(profile["bundle"]["targets"], ["app"])
        self.assertEqual(profile["bundle"]["macOS"]["signingIdentity"], "-")

        build_script = (ROOT / "scripts/build-macos-smoke.sh").read_text()
        for name in (
            "APPLE_CERTIFICATE",
            "APPLE_SIGNING_IDENTITY",
            "APPLE_API_KEY",
            "APPLE_ID",
            "BRIDGE_RELEASE_ENV",
            "TAURI_SIGNING_PRIVATE_KEY",
        ):
            self.assertIn(f"unset {name}", build_script)
        self.assertIn("tauri.macos-smoke.conf.json", build_script)
        self.assertNotIn("release-dmg.sh", build_script)
        self.assertNotIn("notarize-dmg.sh", build_script)


if __name__ == "__main__":
    unittest.main()
