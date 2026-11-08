"""Automatic maintenance must require proof, wait for native recovery, and stop."""
from contextlib import nullcontext
from contextlib import redirect_stdout
import ast
import importlib.util
import inspect
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

TOOLS = Path(__file__).resolve().parents[2] / 'handoff/google-play-stepx/tools'
sys.path.insert(0, str(TOOLS))
spec = importlib.util.spec_from_file_location('google_auto_manage', TOOLS / 'manage.py')
manage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manage)
import common


class AutomaticPolicyTests(unittest.TestCase):
    def fixture(self):
        return {'status': 'needs_repair', 'needs_repair': True,
                'component_versions': {p: {'versionCode': '123', 'lastUpdateTime': '2026-11-08 12:00:00'} for p in manage.PACKAGES},
                'permission_coverage_unknown': [], 'permission_definitions_checked': True,
                'missing_permissions': ['com.android.vending:android.permission.MANAGE_USERS'],
                'phone_recovery': {'status': 'needs_repair', 'signatures_verified': True, 'recheck_supported':True},
                'phone_recovery_current_session': True, 'phone_recovery_current_components': True}

    def test_requires_our_artifact_and_complete_current_evidence(self):
        summary = self.fixture()
        for field, value in [('needs_repair', None), ('permission_definitions_checked', False),
                             ('permission_coverage_unknown', ['com.android.vending']), ('component_versions', {})]:
            result, _ = manage.autonomous_plan({**summary, field: value}, {}, True, 100)
            self.assertFalse(result['repair_allowed'])
        result, _ = manage.autonomous_plan(summary, {}, False, 100)
        self.assertEqual(result['state'], 'blocked')
        for field in ['phone_recovery_current_session', 'phone_recovery_current_components']:
            result, _ = manage.autonomous_plan({**summary, field: False}, {}, True, 100)
            self.assertFalse(result['repair_allowed'])
        summary['phone_recovery']['signatures_verified'] = False
        self.assertFalse(manage.autonomous_plan(summary, {}, True, 100)[0]['repair_allowed'])

    def test_unavailable_phone_receipt_never_authorizes_upgrade(self):
        # Execute the exact shipped root read probe with failed/missing reads.
        tree = ast.parse(inspect.getsource(manage.status))
        code = next(n.args[0].value for n in ast.walk(tree) if isinstance(n, ast.Call)
                    and isinstance(n.func, ast.Name) and n.func.id == 'root_json')
        for condition, expected in [('missing', 'missing'), ('denied', 'unavailable'),
                                    ('malformed', 'unavailable'), ('host_missing', 'unavailable')]:
            def read(path):
                if path.endswith('/boot_id'): return io.StringIO('boot')
                if path.endswith('/comm'): return io.StringIO('openclaw_env')
                if condition == 'missing': raise FileNotFoundError()
                if condition == 'denied': raise PermissionError()
                return io.StringIO('{')
            out = io.StringIO()
            with self.subTest(condition=condition), patch('builtins.open', side_effect=read), \
                 patch('os.listdir', return_value=[] if condition=='host_missing' else ['1']), \
                 patch('os.path.exists', side_effect=lambda path:path.endswith('/comm')), redirect_stdout(out):
                exec(code, {})
            result = json.loads(out.getvalue())
            self.assertEqual(result['phone_recovery_read_state'], expected)
            if expected == 'unavailable':
                summary = {**self.fixture(), 'phone_recovery_read_state':expected, 'monitor_upgrade_needed':True}
                self.assertFalse(manage.autonomous_plan(summary, {}, True, 100)[0]['repair_allowed'])

    def test_phone_retry_waits_for_relative_deadline(self):
        summary = self.fixture()
        summary['phone_recovery'].update(recovery_pending=True, checked_at=100_000, retry_after_ms=45_000)
        result, state = manage.autonomous_plan(summary, {}, True, 110)
        self.assertEqual((result['state'], result['retry_after_seconds'], state), ('waiting', 40, {}))
        self.assertIn('手机正在', result['message'])

    def test_native_negative_evidence_precedes_old_adapter_success(self):
        summary = {**self.fixture(), 'status': 'success', 'needs_repair': False}
        for status in ['blocked', 'disabled', 'not_installed', 'stopped']:
            summary['phone_recovery']['status'] = status
            result, state = manage.autonomous_plan(summary, {'attempts': 2}, True, 100)
            self.assertEqual(result['state'], 'blocked')
            self.assertEqual(state, {'attempts': 2})
        summary['phone_recovery'].update(status='retrying', recovery_pending=True, checked_at=100_000, retry_after_ms=45_000)
        self.assertEqual(manage.autonomous_plan(summary, {}, True, 110)[0]['state'], 'waiting')

    def test_initial_grace_and_budget_survive_process_restarts_and_reboots(self):
        summary = self.fixture()
        result, state = manage.autonomous_plan(summary, {}, True, 100)
        self.assertEqual(result['retry_after_seconds'], 45)
        saved = json.loads(json.dumps(state))
        self.assertFalse(manage.autonomous_plan(summary, saved, True, 144)[0]['repair_allowed'])
        self.assertTrue(manage.autonomous_plan(summary, saved, True, 145)[0]['repair_allowed'])
        saved.update(attempts=3)
        summary['boot_id'] = 'different-boot'
        result, same = manage.autonomous_plan(summary, saved, True, 99999)
        self.assertEqual(result['state'], 'needs_attention')
        self.assertEqual(same['attempts'], 3)
        summary['component_versions']['com.android.vending']['versionCode'] = '124'
        result, fresh = manage.autonomous_plan(summary, same, True, 100000)
        self.assertEqual(fresh['attempts'], 0)
        self.assertFalse(result['repair_allowed'])

    def test_verified_healthy_resets_prior_fault(self):
        summary = {**self.fixture(), 'status': 'success', 'needs_repair': False}
        self.assertEqual(manage.autonomous_plan(summary, {'attempts': 3}, True, 100)[1], {})

    def test_owned_old_monitor_upgrades_automatically_even_if_grants_healthy(self):
        summary = {**self.fixture(), 'status': 'success', 'needs_repair': False,
                   'monitor_upgrade_needed': True, 'phone_recovery': {},
                   'phone_recovery_current_components': False}
        decision, history = manage.autonomous_plan(summary, {}, True, 100)
        self.assertFalse(decision['repair_allowed'])
        decision, history = manage.autonomous_plan(summary, history, True, 145)
        self.assertTrue(decision['repair_allowed'])
        self.assertEqual(decision['operation'], 'upgrade_monitor')
        self.assertFalse(manage.autonomous_plan(summary, history, False, 145)[0]['repair_allowed'])

    def test_unknown_future_monitor_never_downgrades_or_resets_budget(self):
        summary = {**self.fixture(), 'status': 'success', 'needs_repair': False,
                   'monitor_schema_unsupported': True}
        decision, prior = manage.autonomous_plan(summary, {'attempts': 3}, True, 99999)
        self.assertEqual(decision['state'], 'blocked')
        self.assertEqual(prior, {'attempts': 3})

    def test_failed_monitor_upgrade_runs_once_until_new_reviewed_artifact(self):
        summary = {**self.fixture(), 'monitor_upgrade_needed':True, 'monitor_artifact_sha256':'first-artifact'}
        _, history = manage.autonomous_plan(summary, {}, True, 100)
        history['attempts'] = 1
        self.assertEqual(manage.autonomous_plan(summary, history, True, 999)[0]['state'], 'needs_attention')
        summary['component_versions']['com.android.vending']['versionCode'] = 'new-package'
        self.assertEqual(manage.autonomous_plan(summary, history, True, 999)[0]['state'], 'needs_attention')
        summary['monitor_artifact_sha256'] = 'fixed-artifact'
        decision, history = manage.autonomous_plan(summary, history, True, 1000)
        self.assertEqual(history['attempts'], 0)
        self.assertEqual(decision['state'], 'waiting')

    def test_call_or_unknown_telephony_state_defers_mutation(self):
        for output in ['', 'mCallState=1', 'mCallState=2', 'mCallState=0\nmCallState=1']:
            with patch.object(manage, 'shell', return_value=output):
                self.assertFalse(manage.phone_idle_for_recovery())
        with patch.object(manage, 'shell', side_effect=['mCallState=0\nmCallState=0', 'RUNNING_UNLOCKED']):
            self.assertTrue(manage.phone_idle_for_recovery())

    def test_native_request_targets_only_existing_system_observer(self):
        with patch.object(manage, 'shell', return_value='Broadcast completed: result=0') as shell, \
             patch.object(manage, 'root_json') as root:
            manage.request_native_recheck()
            shell.assert_called_once_with('am','broadcast','--user','0','-a',
                'com.stepx.companion.action.RECHECK_GOOGLE_RUNTIME','-p','android')
            root.assert_not_called()

    def test_attempt_is_saved_before_native_request_without_claiming_recovery(self):
        for success in [True, False]:
            with self.subTest(success=success), tempfile.TemporaryDirectory() as tmp:
                directory = Path(tmp)
                (directory / 'auto-recovery.json').write_text(json.dumps({'fault': 'same', 'attempts': 0}))
                def save(name, value):
                    (directory / name).write_text(json.dumps(value))
                def mutate():
                    self.assertEqual(json.loads((directory / 'auto-recovery.json').read_text())['attempts'], 1)
                    if not success: raise RuntimeError('native request interrupted')
                with patch.object(manage, 'STATE', directory), patch.object(manage, 'save_json', side_effect=save), \
                     patch.object(manage, 'operation_lock', return_value=nullcontext()), \
                     patch.object(manage, '_autonomous_status', return_value={'repair_allowed': True, 'fault': 'same'}), \
                     patch.object(manage, 'phone_idle_for_recovery', return_value=True), patch.object(manage, 'guard'), \
                     patch.object(manage, 'request_native_recheck', side_effect=mutate) as action, \
                     patch.object(manage, '_repair_runtime') as restart, patch.object(manage, 'status') as verify:
                    result = manage.repair_autonomous()
                    self.assertEqual(action.call_count, 1)
                    self.assertEqual(result['state'], 'waiting' if success else 'retrying')
                    restart.assert_not_called(); verify.assert_not_called()
                    persisted = json.loads((directory / 'auto-recovery.json').read_text())
                    self.assertEqual(persisted.get('attempts', 0), 1)

    def test_read_failure_or_busy_never_calls_repair(self):
        for failure in [RuntimeError('partial read'), common.OperationBusy('manual operation')]:
            with patch.object(manage, 'operation_lock', return_value=nullcontext()), \
                 patch.object(manage, '_autonomous_status', side_effect=failure), \
                 patch.object(manage, 'request_native_recheck') as action:
                if isinstance(failure, common.OperationBusy):
                    self.assertEqual(manage.repair_autonomous()['state'], 'waiting')
                else:
                    with self.assertRaises(RuntimeError): manage.repair_autonomous()
                action.assert_not_called()

    def test_companion_manual_operation_does_not_consume_attempt(self):
        with patch.object(manage, 'operation_lock', return_value=nullcontext()), \
             patch.object(manage, 'companion_operation_lock', side_effect=common.OperationBusy('manual companion operation')), \
             patch.object(manage, '_autonomous_status') as check, patch.object(manage, 'save_json') as save:
            self.assertEqual(manage.repair_autonomous()['state'], 'waiting')
            check.assert_not_called(); save.assert_not_called()

    def test_monitor_upgrade_does_not_add_runtime_repair_and_waits_for_native_result(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / 'auto-recovery.json').write_text(json.dumps({'fault': 'same', 'attempts': 0}))
            with patch.object(manage, 'STATE', directory), \
                 patch.object(manage, 'save_json', side_effect=lambda name,value:(directory/name).write_text(json.dumps(value))), \
                 patch.object(manage, 'operation_lock', return_value=nullcontext()), patch.object(manage, 'guard'), \
                 patch.object(manage, '_autonomous_status', return_value={'repair_allowed':True,'fault':'same','operation':'upgrade_monitor'}), \
                 patch.object(manage, 'phone_idle_for_recovery', return_value=True), patch.object(manage.time, 'sleep'), \
                 patch.object(manage, 'upgrade_owned_monitor') as upgrade, patch.object(manage, '_repair_runtime') as restart, \
                 patch.object(manage, 'status', return_value={'needs_repair':True,'monitor_upgrade_needed':False,'phone_recovery_current_session':True}):
                self.assertEqual(manage.repair_autonomous()['state'], 'waiting')
                upgrade.assert_called_once(); restart.assert_not_called()


class PhoneReceiptTests(unittest.TestCase):
    def fixture(self):
        permission = 'android.permission.MANAGE_USERS'
        dumps = {p: '  appId=10123\n  versionCode=123\n  versionName=1.2.3\n  lastUpdateTime=2026-11-08 12:00:00\n  requested permissions:\n    '+permission+'\n  install permissions:\n    '+permission+': granted=true\n' for p in manage.PACKAGES}
        reviewed = {'com.google.android.gms': (permission,), 'com.android.vending': (permission,)}
        proof = '\n'.join('check '+p+' '+permission+'=0' for p in reviewed)
        phone = {'schema': 2, 'status': 'ready', 'boot_id': 'boot', 'system_server_pid': 123,
                 'update_monitor': True, 'checked_at': 999_000, 'registered': True, 'domain_ready': True,
                 'signatures_verified': True, 'permissions_verified': True, 'permission_result': proof}
        for prefix in ['gms', 'play', 'gsf']:
            phone[prefix+'_version'] = 123
            phone[prefix+'_uid'] = 10123
            phone[prefix+'_updated_local'] = '2026-11-08 12:00:00'
        return phone, dumps, reviewed, {permission}

    def test_complete_small_permission_coverage_can_be_healthy(self):
        phone, dumps, reviewed, registered = self.fixture()
        with patch.dict(manage.REVIEWED_PERMISSIONS, reviewed, clear=True):
            result = manage.phone_recovery_summary(phone, 'boot', '123', dumps, registered)
        self.assertEqual(result['permission_checks'], 2)
        self.assertTrue(result['permission_coverage_verified'])
        self.assertFalse(result['needs_repair'])
        self.assertEqual(result['recovery_source'], 'phone_update_monitor')

    def test_readback_and_identity_required_despite_ready_flag(self):
        phone, dumps, reviewed, registered = self.fixture()
        for field, value in [('schema', 1), ('schema', 2.0), ('schema', True), ('boot_id', 'stale'), ('system_server_pid', 124),
                             ('gms_version', 124), ('gsf_version', 124), ('play_uid', 10124), ('play_updated_local', 'different-install'),
                             ('registered', False), ('domain_ready', False), ('signatures_verified', False),
                             ('permissions_verified', False), ('permission_result', '')]:
            with self.subTest(field=field), patch.dict(manage.REVIEWED_PERMISSIONS, reviewed, clear=True):
                self.assertIsNone(manage.phone_recovery_summary({**phone, field:value}, 'boot', '123', dumps, registered))
        dumps['com.android.vending'] = dumps['com.android.vending'].replace('granted=true', 'granted=false')
        with patch.dict(manage.REVIEWED_PERMISSIONS, reviewed, clear=True):
            self.assertIsNone(manage.phone_recovery_summary(phone, 'boot', '123', dumps, registered))

    def test_timezone_change_uses_epoch_and_still_detects_reinstallation(self):
        phone, dumps, reviewed, registered = self.fixture()
        from datetime import datetime, timezone
        for prefix in ['gms','play','gsf']:
            phone[prefix+'_updated_at'] = int(datetime(2026,11,8,12,tzinfo=timezone.utc).timestamp())*1000
            phone[prefix+'_updated_local'] = '2026-11-08 20:00:00'
        with patch.dict(manage.REVIEWED_PERMISSIONS, reviewed, clear=True):
            self.assertIsNotNone(manage.phone_recovery_summary(phone, 'boot', '123', dumps, registered, 'UTC'))
            dumps['com.android.vending'] = dumps['com.android.vending'].replace('12:00:00', '12:00:01')
            self.assertIsNone(manage.phone_recovery_summary(phone, 'boot', '123', dumps, registered, 'UTC'))


class OperationLockTests(unittest.TestCase):
    def test_nested_same_operation_reentrant_but_other_thread_defers(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(common, 'STATE', Path(tmp)), patch.object(common, 'init_state'):
            errors = []
            def competing():
                try:
                    with common.operation_lock(): errors.append('unexpected acquisition')
                except common.OperationBusy: errors.append('busy')
            with common.operation_lock():
                with common.operation_lock(): pass
                contender = threading.Thread(target=competing)
                contender.start(); contender.join(timeout=2)
                self.assertFalse(contender.is_alive(), 'operation lock must be nonblocking')
            self.assertEqual(errors, ['busy'])
            with common.operation_lock(): pass


class ManagedMonitorUpgradeTests(unittest.TestCase):
    def test_upgrade_requires_owned_bytes_identity_and_idle_before_activation(self):
        for failure in [None, 'identity', 'artifacts', 'call']:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as tmp:
                config = {'package':'com.stepx.companion.app', 'serial':'fixture-phone',
                          'boot_mode':'chained', 'fingerprint':'fixture-firmware'}
                if failure == 'identity': config['serial'] = 'different-phone'
                boot = SimpleNamespace(STATE_LOCAL=Path(tmp),
                    p=SimpleNamespace(PACKAGE='com.stepx.companion.app', device_key=lambda:'fixture-phone',
                                      ALLOWED_FINGERPRINTS=('fixture-firmware',)),
                    status=Mock(return_value={'installed':True,'config':config}),
                    activate=Mock(return_value={'status':'activated','system_server_unchanged':True,
                                              'google_login':{'status':'scheduled','update_monitor':True}}))
                with patch.object(manage, 'companion_boot_module', return_value=boot), \
                     patch.object(manage, 'operation_lock', return_value=nullcontext()), \
                     patch.object(manage, 'root_json', return_value={'owned':failure!='artifacts'}), \
                     patch.object(manage, 'phone_idle_for_recovery', return_value=failure!='call'):
                    if failure:
                        with self.assertRaises(RuntimeError): manage.upgrade_owned_monitor()
                        boot.activate.assert_not_called()
                    else:
                        manage.upgrade_owned_monitor()
                        boot.activate.assert_called_once()
                        self.assertIs(boot.activate.call_args.kwargs['rebind'], False)


if __name__ == '__main__':
    unittest.main()
