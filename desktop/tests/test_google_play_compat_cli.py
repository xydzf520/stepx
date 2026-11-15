"""CLI acceptance using a fake transport: no ADB, phone or account access."""

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import tempfile
import types
import unittest
from unittest import mock


SOURCE = Path(__file__).resolve().parents[2] / "scripts/google-play-compat.py"
SPEC = importlib.util.spec_from_file_location("google_play_compat_cli_test", SOURCE)
compat = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(compat)


class FakeTransport:
    def __init__(self):
        self.calls = []
        self.active = {scope: False for scope in compat.SCOPES}
        self.fail_apply = None
        self.fail_rollback = None
        self.guard_failure = False
        self.private_secret = "PRIVATE_ACCOUNT_OR_TOKEN_MUST_NOT_BE_PRINTED"
        self.before_apply = None
        self.before_rollback = None
        self.fail_status = None

    def check(self):
        self.calls.append(("check",))
        if self.guard_failure:
            raise compat.RepairError("unsupported_apk", "Google 组件版本不匹配。")

    def stop_apps(self):
        self.calls.append(("stop",))

    def run(self, scope, action, manifest=None):
        self.calls.append((scope, action))
        summary = {"cache_active": self.active[scope], "db_active": self.active[scope]}
        if action == "status":
            if scope == self.fail_status:
                return {"error": self.private_secret, "phase": "diagnostic_test"}
            return {"summary": summary, "private_token": self.private_secret}
        if action == "prepare":
            return {
                "summary": summary,
                "file_baseline": {"format": 1, "scope": scope, "entries": [{
                    "path": "/private/" + scope, "uid": 1, "gid": 1, "mode": 0o600,
                    "selinux_b64": None, "sha256": "a" * 64,
                    "data_zlib_b64": self.private_secret,
                }]},
                "db_baseline": {"format": 1, "scope": scope, "override": None},
            }
        if action == "apply":
            if self.before_apply:
                self.before_apply(scope, manifest)
            self.active[scope] = True
            if scope == self.fail_apply:
                return {"error": "scoped error " + self.private_secret}
            return {"summary": {"cache_active": True, "db_active": True}}
        if action == "rollback":
            if self.before_rollback:
                self.before_rollback(scope, manifest)
            if scope == self.fail_rollback:
                return {"error": "external change " + self.private_secret}
            self.active[scope] = False
            return {"summary": {"rolled_back": True, "cache_active": False, "db_active": False}}
        raise AssertionError(action)


class CliTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.directory.chmod(0o700)
        self.transport = FakeTransport()

    def execute(self, action):
        return compat.execute(action, self.directory, self.transport)

    def test_status_checks_real_scope_values_without_apply_or_stop(self):
        self.transport.active["gms"] = True
        result = self.execute("status")
        self.assertFalse(result["active"])
        self.assertFalse(result["can_rollback"])
        self.assertTrue(result["scopes"]["gms"]["cache_active"])
        self.assertFalse(result["scopes"]["play"]["cache_active"])
        self.assertEqual(self.transport.calls, [("check",), ("gms", "status"), ("play", "status")])
        self.assertNotIn(self.transport.private_secret, json.dumps(result))
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_guard_failure_prevents_any_scope_read_stop_or_write(self):
        self.transport.guard_failure = True
        with self.assertRaises(compat.RepairError):
            self.execute("apply")
        self.assertEqual(self.transport.calls, [("check",)])
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_private_backup_and_pointer_exist_before_first_mutation(self):
        seen = []

        def check(scope, manifest):
            stored = compat.current_receipt(self.directory)
            self.assertEqual(stored, manifest)
            path = self.directory / manifest["receipt_id"] / "manifest.json"
            pointer = self.directory / "current.json"
            self.assertTrue(path.is_file())
            self.assertTrue(pointer.is_file())
            if os.name == "posix":
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE(path.parent.stat().st_mode), 0o700)
                self.assertEqual(stat.S_IMODE(pointer.stat().st_mode), 0o600)
            seen.append(scope)

        self.transport.before_apply = check
        result = self.execute("apply")
        self.assertEqual(seen, ["gms", "play"])
        self.assertTrue(result["active"])
        self.assertTrue(result["can_rollback"])
        self.assertFalse(result["official_certification_changed"])
        self.assertLess(self.transport.calls.index(("stop",)), self.transport.calls.index(("gms", "prepare")))
        self.assertNotIn(self.transport.private_secret, json.dumps(result))
        self.assertFalse(any("status" in call for call in self.transport.calls))

    def test_repeated_apply_reuses_immutable_receipt_without_journal_overwrite(self):
        first = self.execute("apply")
        manifest = self.directory / first["receipt_id"] / "manifest.json"
        original = manifest.read_bytes()
        files = set(manifest.parent.iterdir())
        self.transport.calls.clear()
        second = self.execute("apply")
        self.assertEqual(first["receipt_id"], second["receipt_id"])
        self.assertEqual(manifest.read_bytes(), original)
        self.assertEqual(set(manifest.parent.iterdir()), files)
        self.assertEqual(self.transport.calls, [("check",), ("gms", "status"), ("play", "status")])

    def test_existing_receipt_with_new_state_is_not_rebased_or_overwritten(self):
        first = self.execute("apply")
        path = self.directory / first["receipt_id"] / "manifest.json"
        original = path.read_bytes()
        self.transport.active["play"] = False
        self.transport.calls.clear()
        second = self.execute("apply")
        self.assertTrue(second["active"])
        self.assertNotEqual(second["receipt_id"], first["receipt_id"])
        self.assertEqual(path.read_bytes(), original)
        second_manifest = compat._read_private(self.directory / second["receipt_id"] / "manifest.json")
        self.assertEqual(second_manifest["superseded_receipt_id"], first["receipt_id"])
        old_events = [compat._read_private(p) for p in (self.directory / first["receipt_id"]).glob("*.json")]
        self.assertTrue(any(e.get("phase") == "receipt_superseded" for e in old_events))

    def test_second_scope_failure_rolls_back_both_scopes(self):
        self.transport.fail_apply = "play"
        seen = []

        def before_rollback(scope, manifest):
            events = [compat._read_private(path) for path in
                      (self.directory / manifest["receipt_id"]).glob("*.json")]
            failure = [event for event in events if event.get("phase") == "apply_failed_before_rollback"]
            self.assertEqual(len(failure), 1)
            self.assertEqual(failure[0]["proof"]["exception_class"], "RepairError")
            self.assertEqual(failure[0]["proof"]["error_code"], "scoped_operation_failed")
            seen.append(scope)

        self.transport.before_rollback = before_rollback
        with self.assertRaises(compat.RepairError) as raised:
            self.execute("apply")
        self.assertEqual(raised.exception.code, "apply_rolled_back")
        self.assertEqual(self.transport.active, {"gms": False, "play": False})
        self.assertEqual(self.transport.calls[-2:], [("play", "rollback"), ("gms", "rollback")])
        self.assertIsNone(compat.current_receipt(self.directory))
        self.assertTrue(list(self.directory.glob("compat-*/manifest.json")))
        self.assertEqual(seen, ["play", "gms"])

    def test_failed_status_proof_is_saved_before_checked_can_throw(self):
        result = self.execute("apply")
        self.transport.fail_status = "gms"
        with self.assertRaises(compat.RepairError):
            self.execute("status")
        events = [compat._read_private(path) for path in
                  (self.directory / result["receipt_id"]).glob("*.json")]
        failures = [event for event in events if event.get("phase") == "gms_post_status_result"]
        self.assertEqual(len(failures), 1)
        self.assertEqual(failures[0]["proof"]["error"], self.transport.private_secret)
        self.assertEqual(failures[0]["proof"]["phase"], "diagnostic_test")
        self.assertTrue(any(event.get("phase") == "gms_post_status_exception" for event in events))

    def test_failure_rollback_conflict_keeps_current_receipt_and_external_changes(self):
        self.transport.fail_apply = "play"
        self.transport.fail_rollback = "play"
        with self.assertRaises(compat.RepairError) as raised:
            self.execute("apply")
        self.assertEqual(raised.exception.code, "apply_recovery_required")
        self.assertIsNotNone(compat.current_receipt(self.directory))
        self.assertTrue(self.transport.active["play"])
        self.assertFalse(self.transport.active["gms"])

    def test_explicit_rollback_stops_before_mutation_and_closes_receipt(self):
        self.execute("apply")
        self.transport.calls.clear()
        result = self.execute("rollback")
        self.assertFalse(result["active"])
        self.assertFalse(result["can_rollback"])
        self.assertEqual(self.transport.calls[:4], [("check",), ("stop",), ("play", "rollback"), ("gms", "rollback")])
        self.assertIsNone(compat.current_receipt(self.directory))
        self.assertFalse(any("status" in call for call in self.transport.calls))

    def test_rollback_without_receipt_is_read_only(self):
        result = self.execute("rollback")
        self.assertFalse(result["can_rollback"])
        self.assertNotIn(("stop",), self.transport.calls)
        self.assertFalse(any("rollback" in call for call in self.transport.calls))

    def test_manifest_is_exclusive_and_cannot_be_overwritten(self):
        path = self.directory / "manifest.json"
        compat._save_private(path, {"first": True})
        with self.assertRaises(FileExistsError):
            compat._save_private(path, {"first": False})
        self.assertEqual(compat._read_private(path), {"first": True})

    def test_public_errors_redact_exception_payload(self):
        output = io.StringIO()
        with mock.patch.object(compat, "state_directory", return_value=self.directory), \
             mock.patch.object(compat, "execute", side_effect=RuntimeError(self.transport.private_secret)), \
             contextlib.redirect_stdout(output):
            code = compat.main(["status"])
        self.assertEqual(code, 1)
        result = json.loads(output.getvalue())
        self.assertFalse(result["ok"])
        self.assertNotIn(self.transport.private_secret, output.getvalue())

    def test_receipt_symlink_is_rejected(self):
        path = self.directory / "receipt.json"
        compat._save_private(path, {"value": True})
        link = self.directory / "link.json"
        link.symlink_to(path)
        with self.assertRaises(OSError):
            compat._read_private(link)

    @unittest.skipUnless(os.name == "posix", "POSIX receipt mode bits")
    def test_receipt_permissive_mode_is_rejected(self):
        path = self.directory / "receipt.json"
        compat._save_private(path, {"value": True})
        path.chmod(0o644)
        with self.assertRaises(compat.RepairError):
            compat._read_private(path)

    def test_receipt_target_and_pin_tampering_are_rejected(self):
        result = self.execute("apply")
        path = self.directory / result["receipt_id"] / "manifest.json"
        receipt = compat._read_private(path)
        receipt["serial"] = "DIFFERENT_PHONE"
        compat._save_private(path, receipt, replace=True)
        self.transport.calls.clear()
        with self.assertRaises(compat.RepairError):
            self.execute("rollback")
        self.assertEqual(self.transport.calls, [])

    def test_sources_compile_and_never_embed_baseline_file_contents(self):
        self.execute("apply")
        receipt = compat.current_receipt(self.directory)
        for scope in compat.SCOPES:
            for action in ("status", "prepare", "apply", "rollback"):
                source = compat.worker_source(scope, action, receipt)
                compile(source, "bounded-compatibility-payload", "exec")
                self.assertNotIn(self.transport.private_secret, source)
                self.assertIn("sys.modules[", source)
                if action == "apply":
                    self.assertLess(source.index("if compact(baseline)!=expected"), source.index(".apply(connection,expected_db"))

    def test_missing_recovery_journal_must_match_original_before_success(self):
        self.execute("apply")
        source = compat.worker_source("gms", "rollback", compat.current_receipt(self.directory))
        self.assertIn("if compact(filesm.prepare(scope,discover()))!=expected", source)
        self.assertIn("if db_read()!=expected_db", source)
        self.assertIn("missing_cache_journal", source)

    def test_wireless_transport_is_rejected_before_import_or_adb(self):
        with mock.patch.dict(os.environ, {"STEPX_SERIAL": "192.0.2.1:5555"}):
            with self.assertRaises(compat.RepairError) as raised:
                compat.Transport().check()
        self.assertEqual(raised.exception.code, "usb_required")

    def test_transport_preserves_current_partial_proof_before_later_recovery(self):
        transport = compat.Transport()
        source = "fixed bounded payload"
        proof = {"worker_code_sha256": hashlib.sha256(source.encode()).hexdigest(),
                 "validation_stage": "startup_properties", "error": "PRIVATE worker failure"}

        def worker(*args, **kwargs):
            compat._save_private(self.directory / "last-worker-result.json", proof, replace=True)
            raise RuntimeError("PRIVATE transport exception")

        template = ("import os,sys,json,socket\n"
                    "os.setresgid(1000,0,0);os.setresuid(1000,0,0);lib.setfsgid(1000);lib.setfsuid(1000)\n"
                    "b'u:r:system_app:s0'")
        transport.module = types.SimpleNamespace(
            AGENTBUS_WORKER_TEMPLATE=template, STATE=self.directory,
            runtime_lock=contextlib.nullcontext, shell=lambda *args: "123",
            run_bounded_android_worker=worker)
        with mock.patch.object(compat, "worker_source", return_value=source):
            with self.assertRaises(RuntimeError):
                transport.run("gms", "status")
        self.assertEqual(transport.last_proof, proof)
        self.assertEqual(transport.module.AGENTBUS_WORKER_TEMPLATE, template)

    def test_invalid_scope_or_action_cannot_generate_arbitrary_worker(self):
        for scope, action in (("other", "apply"), ("gms", "execute-code")):
            with self.assertRaises(compat.RepairError):
                compat.worker_source(scope, action)

    def test_play_committed_account_ignores_regenerated_null_account_cache(self):
        import sys
        namespace = {"scope": "play", "sys": sys}
        exec(compat._module_loader("play") + compat.WORKER_COMMON, namespace)
        storem = namespace["storem"]
        filesm = namespace["filesm"]
        raw_non_empty = b"\x01\x00\x01a\x00\x01b\x00\x01c\x06\x00=GmscoreCompliance__activities_to_block_on_uncertified_devices\x00\x00\x00\x03abc\x00"
        raw_empty = storem.patch_cache(raw_non_empty)
        entry = lambda name, raw: {
            "path": "/data/user/0/com.android.vending/files/" + name,
            "sha256": hashlib.sha256(raw).hexdigest(),
            "data_zlib_b64": filesm._compressed(raw),
        }
        baseline = {"entries": [
            entry("experiment-flags-regular-account", raw_empty),
            entry("experiment-flags-regular-null-account", raw_non_empty),
        ]}
        self.assertTrue(namespace["inspect_files"](baseline, {"state": {"account_id": 1}}))
        self.assertFalse(namespace["inspect_files"](baseline, {"state": {"account_id": 0}}))


if __name__ == "__main__":
    unittest.main()
