import importlib.util
import pathlib
import plistlib
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "smoke-packaged-macos-app.py"
SPEC = importlib.util.spec_from_file_location("smoke_packaged_macos_launch", MODULE_PATH)
SMOKE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = SMOKE
SPEC.loader.exec_module(SMOKE)


class LaunchServicesSmokeTests(unittest.TestCase):
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

    def test_run_uses_launch_services_with_only_the_isolated_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            app = self.fixture_bundle(pathlib.Path(directory))
            launcher = mock.Mock()
            launcher.wait.return_value = 0
            with (
                mock.patch.object(SMOKE.sys, "platform", "darwin"),
                mock.patch.dict(
                    SMOKE.os.environ,
                    {"LANG": "en_US.UTF-8", "OPENAI_API_KEY": "must-not-leak"},
                    clear=True,
                ),
                mock.patch.object(SMOKE, "running_executable_process_ids", return_value=set()),
                mock.patch.object(SMOKE.subprocess, "Popen", return_value=launcher) as popen,
                mock.patch.object(SMOKE, "validate_runtime_evidence", return_value=99999999),
            ):
                SMOKE.run(app, timeout=1)

            command = popen.call_args.args[0]
            environment = popen.call_args.kwargs["env"]
            bundle = SMOKE.load_bundle(app)
            self.assertEqual(command[0], "/usr/bin/open")
            self.assertIn("-W", command)
            self.assertIn("-n", command)
            self.assertIn("-g", command)
            self.assertEqual(command[-2:], ["-a", str(bundle.app)])
            self.assertNotIn(str(bundle.executable), command)
            self.assertIn("--stdout", command)
            self.assertIn("--stderr", command)
            self.assertIn("--env", command)
            self.assertIn("BRIDGE_PACKAGED_SMOKE=1", command)
            self.assertIn("BRIDGE_DESKTOP_HOST=daemon", command)
            self.assertNotIn("OPENAI_API_KEY", environment)
            self.assertFalse(any("OPENAI_API_KEY" in argument for argument in command))

    def test_process_inventory_matches_only_the_exact_bundle_executable(self):
        executable = pathlib.Path(
            "/tmp/Bundle With Spaces/Bridge.app/Contents/MacOS/bridge-deck"
        )
        listing = "\n".join(
            (
                f"  101 {executable}",
                f"  102 {executable} -psn_0_123",
                f"  103 {executable}-helper",
                "  104 /other/Bridge.app/Contents/MacOS/bridge-deck",
                "garbage",
            )
        )
        self.assertEqual(SMOKE.executable_process_ids(listing, executable), {101, 102})

    def test_failed_launch_cleanup_targets_reported_and_new_app_processes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            bundle = SMOKE.load_bundle(self.fixture_bundle(root))
            app_log = root / "Bridge.log"
            app_log.write_text(
                "bridge: starting Bridge 9.8.7 pid=321 executable=/fixture/bridge-deck\n"
                "bridge: started bridged (pid 654) for /fixture/data\n"
            )
            launcher = mock.Mock()
            launcher.poll.return_value = 0
            with (
                mock.patch.object(
                    SMOKE, "running_executable_process_ids", return_value={100, 333}
                ),
                mock.patch.object(SMOKE, "terminate_processes") as terminate,
                mock.patch.object(SMOKE, "process_exists", return_value=False),
            ):
                SMOKE.force_cleanup(launcher, bundle, {100}, app_log)

            terminate.assert_called_once_with({321, 333})

    def test_failed_launch_cleanup_kills_an_unresponsive_open_waiter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            bundle = SMOKE.load_bundle(self.fixture_bundle(root))
            app_log = root / "Bridge.log"
            app_log.write_text("")
            launcher = mock.Mock()
            launcher.poll.return_value = None
            launcher.wait.side_effect = [
                SMOKE.subprocess.TimeoutExpired("/usr/bin/open", 5),
                0,
            ]
            with (
                mock.patch.object(
                    SMOKE, "running_executable_process_ids", return_value={100}
                ),
                mock.patch.object(SMOKE, "terminate_processes") as terminate,
            ):
                SMOKE.force_cleanup(launcher, bundle, {100}, app_log)

            terminate.assert_called_once_with(set())
            launcher.terminate.assert_called_once_with()
            launcher.kill.assert_called_once_with()

    def test_cleanup_finds_an_app_that_launches_while_open_is_stopping(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            bundle = SMOKE.load_bundle(self.fixture_bundle(root))
            app_log = root / "Bridge.log"
            app_log.write_text("")
            events = []
            launcher = mock.Mock()
            launcher.poll.return_value = None
            launcher.terminate.side_effect = lambda: events.append("terminate open")
            launcher.wait.side_effect = lambda timeout: events.append("wait open") or 0

            def inventory(_executable):
                events.append("inventory app")
                return {100, 444}

            with (
                mock.patch.object(
                    SMOKE, "running_executable_process_ids", side_effect=inventory
                ),
                mock.patch.object(SMOKE, "terminate_processes") as terminate,
            ):
                SMOKE.force_cleanup(launcher, bundle, {100}, app_log)

            self.assertEqual(events, ["terminate open", "wait open", "inventory app"])
            terminate.assert_called_once_with({444})


if __name__ == "__main__":
    unittest.main()
