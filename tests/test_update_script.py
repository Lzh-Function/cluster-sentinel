"""更新スクリプトを一時ディレクトリで実行し、実機を変更せずに検証する。"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]

FAKE_COMMAND = r'''#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys

name = Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["UPDATE_TEST_LOG"], "a") as log:
    log.write(json.dumps([name, args, os.getcwd()]) + "\n")
if name == "sudo":
    if args == ["-v"]:
        sys.exit(0)
    if args[:2] == ["-u", "sentinel"]:
        args = args[2:]
    os.execvp(args[0], args)
elif name == "install":
    if os.environ.get("FAIL_INSTALL"):
        sys.exit(1)
    args = args[4:]  # -o root -g rootを省き、一時ディレクトリだけを更新する。
    os.execv("/usr/bin/install", ["install", *args])
elif name == "curl":
    if os.environ.get("FAIL_DOWNLOAD"):
        sys.exit(22)
    dest = Path(args[args.index("--output") + 1])
    fixture = Path(os.environ["UPDATE_TEST_DOWNLOAD"])
    if args[-1].endswith(".sha256"):
        digest = hashlib.sha256(fixture.read_bytes()).hexdigest()
        if os.environ.get("BAD_CHECKSUM"):
            digest = "0" * 64
        dest.write_text(digest + "  " + dest.name.removesuffix(".sha256") + "\n")
    else:
        shutil.copyfile(fixture, dest)
elif name == "systemctl":
    if args[0] == "show":
        print("loaded")
    elif args[0] == "restart" and os.environ.get("FAIL_RESTART"):
        sys.exit(1)
    elif args[0] == "is-active":
        if "sentinel-agent.service" in args and not os.environ.get("ACTIVE_AGENT"):
            sys.exit(3)
        if "sentinel-controller.service" in args and os.environ.get("INACTIVE_PARENT"):
            sys.exit(3)
elif name == "ansible-playbook":
    sys.exit(int(os.environ.get("ANSIBLE_EXIT", "0")))
elif name == "uname":
    print("Linux" if args == ["-s"] else os.environ.get("UPDATE_TEST_ARCH", "x86_64"))
elif name == "sleep":
    pass
elif name == "getent":
    assert args == ["group", "systemd-journal"]
    if os.environ.get("NO_JOURNAL_GROUP"):
        sys.exit(2)
    print("systemd-journal:x:190:")
elif name == "id":
    assert args == ["-nG", "sentinel"]
    print(os.environ.get("UPDATE_TEST_GROUPS", "sentinel"))
elif name == "usermod":
    assert args == ["-aG", "systemd-journal", "sentinel"]
    if os.environ.get("FAIL_JOURNAL_GROUP"):
        sys.exit(1)
else:
    raise RuntimeError(name)
'''


def fake_binary(version):
    return f'''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
args = sys.argv[1:]
with open(os.environ["UPDATE_TEST_LOG"], "a") as log:
    log.write(json.dumps(["sentinel", args, str(Path(sys.argv[0]))]) + "\\n")
if args == ["version"]:
    print("sentinel {version}")
    if os.environ.get("FAIL_VERSION"):
        sys.exit(1)
elif args[-2:] == ["config", "check"]:
    if os.environ.get("FAIL_CONFIG"):
        sys.exit(1)
elif args[-1:] == ["status"]:
    sys.exit(int(os.environ.get("STATUS_EXIT", "0")))
elif args[-3:] == ["state", "reset", "--help"]:
    if os.environ.get("NO_STATE_RESET"):
        sys.exit(2)
elif args[-2:] == ["state", "reset"]:
    if os.environ.get("FAIL_STATE_RESET"):
        sys.exit(1)
else:
    raise RuntimeError(args)
'''


class UpdateScriptTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="sentinel-update-test-")
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        self.bin = self.directory / "commands"
        self.bin.mkdir()
        dispatcher = self.bin / "mock-command"
        dispatcher.write_text(FAKE_COMMAND)
        dispatcher.chmod(0o755)
        for name in ("sudo", "install", "curl", "systemctl", "ansible-playbook", "uname", "sleep", "getent", "id", "usermod"):
            (self.bin / name).symlink_to(dispatcher)

        deploy = self.directory / "checkout" / "deploy"
        ansible = deploy / "ansible"
        ansible.mkdir(parents=True)
        (ansible / "site.yml").write_text("---\n")
        self.inventory = ansible / "inventory.ini"
        self.inventory.write_text("[agents]\nnode01\n")
        self.binary = self.directory / "installed" / "sentinel"
        self.binary.parent.mkdir()
        self.binary.write_text(fake_binary("1.0.4"))
        self.binary.chmod(0o755)
        self.original = self.binary.read_bytes()
        self.config = self.directory / "config.toml"
        self.config.write_text("config_version = 1\n")
        self.download = self.directory / "download"
        self.download.write_text(fake_binary("1.0.5"))
        self.log = self.directory / "calls.jsonl"
        self.script = deploy / "update.sh"
        source = (ROOT / "deploy" / "update.sh").read_text()
        source = source.replace("binary=/usr/local/bin/sentinel", f'binary="{self.binary}"')
        source = source.replace("config=/etc/sentinel/config.toml", f'config="{self.config}"')
        self.script.write_text(source)
        self.env = os.environ.copy()
        self.env.pop("SUDO_USER", None)
        self.env.update(
            PATH=f"{self.bin}:{self.env['PATH']}",
            UPDATE_TEST_LOG=str(self.log),
            UPDATE_TEST_DOWNLOAD=str(self.download),
        )

    def run_update(self, *args, **environment):
        return subprocess.run(
            ["bash", str(self.script), *args],
            cwd=self.directory,
            env={**self.env, **environment},
            text=True,
            capture_output=True,
            timeout=20,
        )

    def calls(self, name):
        if not self.log.exists():
            return []
        return [call for line in self.log.read_text().splitlines() if (call := json.loads(line))[0] == name]

    def assert_original_kept(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.binary.read_bytes(), self.original)
        self.assertEqual(self.calls("ansible-playbook"), [])
        self.assertFalse(any(call[1][0] == "restart" for call in self.calls("systemctl")))

    def test_full_update_passes_pinned_version_and_ssh_vault_options(self):
        # インベントリの相対パスと空白を、作業ディレクトリを移った後も保持する。
        inventory = self.directory / "existing inventory.ini"
        shutil.copyfile(self.inventory, inventory)
        result = self.run_update(
            "1.0.5", "--inventory", inventory.name, "--repo", "Lzh-Function/cluster-sentinel",
            "--", "--ask-vault-pass", "-k", "-K", "--limit", "node01", ACTIVE_AGENT="1",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.binary.read_bytes(), self.download.read_bytes())
        self.assertEqual(Path(f"{self.binary}.previous").read_bytes(), self.original)
        self.assertEqual(self.config.read_text(), "config_version = 1\n")
        restarts = [call[1] for call in self.calls("systemctl") if call[1][0] == "restart"]
        self.assertEqual(restarts, [["restart", "sentinel-controller.service"], ["restart", "sentinel-agent.service"]])
        ansible = self.calls("ansible-playbook")
        self.assertEqual(len(ansible), 1)
        self.assertEqual(ansible[0][1], [
            "-i", str(inventory), "site.yml", "--ask-vault-pass", "-k", "-K", "--limit", "node01",
            "-e", "sentinel_version=v1.0.5", "-e", "sentinel_repo=Lzh-Function/cluster-sentinel",
        ])
        self.assertEqual(Path(ansible[0][2]), self.inventory.parent)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        parent_restart = next(i for i, call in enumerate(calls) if call[:2] == ["systemctl", ["restart", "sentinel-controller.service"]])
        downstream = next(i for i, call in enumerate(calls) if call[0] == "ansible-playbook")
        self.assertLess(parent_restart, downstream)
        self.assertFalse(list(self.binary.parent.glob("sentinel.update.*")))
        self.assertFalse(list(self.binary.parent.glob("sentinel.backup.*")))

    def test_parent_only_does_not_require_inventory_or_restart_inactive_agent(self):
        self.inventory.unlink()
        result = self.run_update("v1.0.5", "--parent-only", UPDATE_TEST_ARCH="aarch64")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls("ansible-playbook"), [])
        self.assertFalse(any("sentinel-agent.service" in call[1] for call in self.calls("systemctl") if call[1][0] == "restart"))
        self.assertIn("sentinel-aarch64-unknown-linux-musl", self.calls("curl")[0][1][-1])

    def test_missing_inventory_fails_before_changing_parent(self):
        self.inventory.unlink()
        self.assert_original_kept(self.run_update("v1.0.5"))

    def test_parent_journal_permission_is_added_before_restarting_services(self):
        result = self.run_update("v1.0.5", "--parent-only", ACTIVE_AGENT="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[1] for call in self.calls("usermod")], [["-aG", "systemd-journal", "sentinel"]])
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        added = next(i for i, call in enumerate(calls) if call[0] == "usermod")
        restarted = [i for i, call in enumerate(calls) if call[0] == "systemctl" and call[1][0] == "restart"]
        self.assertEqual(len(restarted), 2)
        self.assertTrue(all(added < i for i in restarted))

    def test_existing_journal_membership_is_kept(self):
        result = self.run_update("v1.0.5", "--parent-only", UPDATE_TEST_GROUPS="sentinel systemd-journal extra-group")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls("usermod"), [])

    def test_journal_permission_failure_keeps_original_and_blocks_downstream(self):
        self.assert_original_kept(self.run_update("v1.0.5", FAIL_JOURNAL_GROUP="1"))

    def test_missing_journal_group_reports_required_permission_check(self):
        result = self.run_update("v1.0.5", "--parent-only", NO_JOURNAL_GROUP="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls("usermod"), [])
        self.assertIn("ログの閲覧権限を確認", result.stderr)

    def test_state_reset_runs_after_controller_stops_and_before_restart(self):
        result = self.run_update("v1.0.5", "--parent-only")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        stopped = next(i for i, call in enumerate(calls) if call[:2] == ["systemctl", ["stop", "sentinel-controller.service"]])
        reset = [i for i, call in enumerate(calls) if call[0] == "sentinel" and call[1][-2:] == ["state", "reset"]]
        restarted = next(i for i, call in enumerate(calls) if call[:2] == ["systemctl", ["restart", "sentinel-controller.service"]])
        self.assertEqual(len(reset), 1)
        self.assertLess(stopped, reset[0])
        self.assertLess(reset[0], restarted)

    def test_failed_state_reset_blocks_downstream_and_keeps_controller_stopped(self):
        result = self.run_update("v1.0.5", FAIL_STATE_RESET="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls("ansible-playbook"), [])
        self.assertFalse(any(call[1][0] == "restart" for call in self.calls("systemctl")))
        self.assertIn("controllerは停止したまま", result.stderr)

    def test_binary_without_reset_support_is_rejected_before_changing_services(self):
        result = self.run_update("v1.0.5", NO_STATE_RESET="1")
        self.assert_original_kept(result)
        self.assertFalse(any(call[1][0] == "stop" for call in self.calls("systemctl")))

    def test_download_checksum_config_and_version_failures_keep_original(self):
        for failure in ("FAIL_DOWNLOAD", "BAD_CHECKSUM", "FAIL_CONFIG", "FAIL_VERSION", "wrong_version"):
            with self.subTest(failure=failure):
                self.log.unlink(missing_ok=True)
                self.download.write_text(fake_binary("1.0.6" if failure == "wrong_version" else "1.0.5"))
                environment = {} if failure == "wrong_version" else {failure: "1"}
                self.assert_original_kept(self.run_update("v1.0.5", **environment))

    def test_install_failure_cleans_staged_files_and_keeps_original(self):
        self.assert_original_kept(self.run_update("v1.0.5", FAIL_INSTALL="1"))
        self.assertFalse(list(self.binary.parent.glob("sentinel.update.*")))
        self.assertFalse(list(self.binary.parent.glob("sentinel.backup.*")))

    def test_failed_restart_blocks_downstream_and_reports_backup(self):
        result = self.run_update("v1.0.5", FAIL_RESTART="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls("ansible-playbook"), [])
        self.assertEqual(self.binary.read_bytes(), self.download.read_bytes())
        self.assertIn("置き換え済み", result.stderr)
        self.assertIn(f"{self.binary}.previous", result.stderr)

    def test_agents_only_requires_updated_running_parent_and_skips_download(self):
        self.assertNotEqual(self.run_update("v1.0.5", "--agents-only").returncode, 0)
        self.assertEqual(self.calls("ansible-playbook"), [])
        self.binary.write_bytes(self.download.read_bytes())
        self.assertNotEqual(self.run_update("v1.0.5", "--agents-only", INACTIVE_PARENT="1").returncode, 0)
        self.assertEqual(self.calls("ansible-playbook"), [])
        result = self.run_update("v1.0.5", "--agents-only", "--", "-K")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls("ansible-playbook")), 1)
        self.assertEqual(self.calls("curl"), [])
        self.assertEqual(
            [call[1] for call in self.calls("systemctl") if call[1][0] == "restart"],
            [["restart", "sentinel-controller.service"]],
        )

    def test_repeat_update_restarts_parent_and_preserves_previous_backup(self):
        self.binary.write_bytes(self.download.read_bytes())
        backup = Path(f"{self.binary}.previous")
        backup.write_bytes(self.original)
        result = self.run_update("v1.0.5", "--parent-only")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(backup.read_bytes(), self.original)
        self.assertIn(["restart", "sentinel-controller.service"], [call[1] for call in self.calls("systemctl")])

    def test_incident_exit_code_does_not_block_update_but_status_error_does(self):
        result = self.run_update("v1.0.5", STATUS_EXIT="2")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls("ansible-playbook")), 1)
        self.log.unlink()
        result = self.run_update("v1.0.5", STATUS_EXIT="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls("ansible-playbook"), [])

    def test_ansible_failure_is_not_reported_as_success(self):
        result = self.run_update("v1.0.5", ANSIBLE_EXIT="2")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("更新が完了", result.stdout)
        self.assertEqual(self.binary.read_bytes(), self.download.read_bytes())

    def test_help_and_invalid_arguments_do_not_touch_system(self):
        self.assertEqual(self.run_update("--help").returncode, 0)
        for args in (
            (), ("latest",), ("v1.0.5", "--parent-only", "--agents-only"),
            ("v1.0.5", "--inventory"), ("v1.0.5", "-K"),
            ("v1.0.5", "--", "--check"), ("v1.0.5", "--parent-only", "--", "-K"),
            ("v1.0.5", "--", "--help"), ("v1.0.5", "--", "--version"),
        ):
            with self.subTest(args=args):
                self.assertNotEqual(self.run_update(*args).returncode, 0)
        self.assertEqual(self.calls("sudo"), [])


if __name__ == "__main__":
    unittest.main()
