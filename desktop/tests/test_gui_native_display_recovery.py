import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('display_recovery_tested', ROOT / 'scripts/gui-native-display-recovery.py')
recovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recovery)


class RecoveryGuards(unittest.TestCase):
    def test_exact_failed_run_and_redirect(self):
        directory = ROOT / 'diagnostics/native-gui-backup-20261022-1910'
        if not directory.is_dir():
            self.skipTest('optional local diagnostics capture not present')
        owner = json.loads((directory / ('task-host-' + recovery.OWNER_ID + '.json')).read_text())
        redirect = json.loads((directory / 'redirect-after-task-d0cf1f25.json').read_text())
        self.assertEqual(recovery.validate_owner(owner, redirect), redirect['process'])
        for key, value in [('id', 'other'), ('boot_id', 'other'), ('system_server_pid', -1)]:
            altered = copy.deepcopy(owner); altered[key] = value
            with self.assertRaises(RuntimeError): recovery.validate_owner(altered, redirect)
        for key, value in [('mdi_task_count_before_release', 0), ('owned_task_cleared', False), ('test_terminal', 'finish')]:
            altered = copy.deepcopy(owner); altered['collect_proof']['final'][key] = value
            with self.assertRaises(RuntimeError): recovery.validate_owner(altered, redirect)

    def test_recovery_requires_complete_and_exact_readback(self):
        value = {key: True for key in recovery.SUCCESS_FIELDS}
        value.update(version=1, mode='native_gui_owned_display_recovery', pid=2859, uid=1000,
                     read_only=False, global_rollback_verified=False, owned_display_id=90000001,
                     target_uid=10124, target_package='com.android.launcher3', operation_count=1,
                     current_before=90000001, previous_before=90000000, current_after=-1, previous_after=90000001)
        self.assertEqual(recovery.validate_result(value, 2859), value)
        for key, bad in [('owned_display_id', 0), ('unrelated_map_unchanged', False),
                         ('global_rollback_verified', True), ('current_after', 0)]:
            altered = value.copy(); altered[key] = bad
            with self.assertRaises(RuntimeError): recovery.validate_result(altered, 2859)
        altered = value.copy(); altered['raw_details'] = 'unexpected'
        with self.assertRaises(RuntimeError): recovery.validate_result(altered, 2859)

    def test_worker_pins_reviewed_dex_and_has_no_old_payload(self):
        if not recovery.DEX.is_file():
            self.skipTest('optional local display recovery research DEX not built')
        identity = {'pid': 2859, 'start_ticks': '706', 'boot_id': 'c92e4fb8-5efd-4c7f-b448-0571f5080d79'}
        code = recovery.build_worker(recovery.DEX.read_bytes(), identity)
        self.assertIn('stepx.NativeGuiDisplayRecovery', code)
        self.assertNotIn('stepx.GmsProbe', code)
        self.assertIn('RECOVERY_START', code)
        with self.assertRaises(RuntimeError): recovery.build_worker(b'dex\nwrong', identity)


if __name__ == '__main__': unittest.main()
