"""A cached boot receipt must not mask permissions lost during a GMS update."""
import importlib.util
import ast
import base64
import re
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[2] / 'handoff/google-play-stepx/tools'
sys.path.insert(0, str(TOOLS))
spec = importlib.util.spec_from_file_location('google_manage', TOOLS / 'manage.py')
manage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manage)


class GoogleRuntimeStatusTests(unittest.TestCase):
    def fixture(self):
        checks = ['check com.google.android.gms android.permission.FIXTURE_' + str(i) + '=0' for i in range(120)]
        dumps = {package: '  versionCode=123 minSdk=26\n  versionName=1.2.3\n  lastUpdateTime=2026-11-08 12:00:00\n  requested permissions:\n  install permissions:\n' for package in manage.PACKAGES}
        dumps['com.google.android.gms'] += '\n'.join('    android.permission.FIXTURE_' + str(i) + ': granted=true' for i in range(120))
        adapter = {'identity': 'boot', 'target_pid': 123, 'unix_time': 100,
                   'status': 'success', 'detached': True, 'java_result': '\n'.join(checks)}
        return adapter, dumps

    def test_reads_current_grants_not_old_receipt(self):
        receipt = ['check com.google.android.gms android.permission.MANAGE_USERS=0']
        with patch.object(manage, 'shell', return_value='  android.permission.MANAGE_USERS: granted=false'):
            self.assertEqual(manage.current_permission_checks(receipt),
                             ['com.google.android.gms:android.permission.MANAGE_USERS'])
        with patch.object(manage, 'shell', return_value='  android.permission.MANAGE_USERS: granted=true, flags=[ GRANTED_BY_DEFAULT ]'):
            self.assertEqual(manage.current_permission_checks(receipt), [])

    def test_other_package_or_similar_permission_cannot_satisfy_check(self):
        receipt = ['check com.google.android.gms android.permission.MANAGE_USERS=0']
        def output(*args):
            return ('  android.permission.MANAGE_USERS: granted=true' if args[-1] == 'com.android.vending'
                    else '  vendor.android.permission.MANAGE_USERS: granted=true')
        with patch.object(manage, 'shell', side_effect=output):
            self.assertEqual(len(manage.current_permission_checks(receipt)), 1)

    def test_malformed_or_failed_receipt_is_rejected(self):
        with patch.object(manage, 'shell', return_value='  android.permission.MANAGE_USERS: granted=true'):
            for receipt in ['check unknown android.permission.MANAGE_USERS=0',
                            'check com.google.android.gms android.permission.MANAGE_USERS=-1']:
                self.assertEqual(manage.current_permission_checks([receipt]), ['unrecognized_receipt'])

    def test_newly_declared_manage_users_cannot_hide_behind_old_success(self):
        adapter, dumps = self.fixture()
        adapter['java_result'] += '\nSKIP undeclared com.android.vending android.permission.MANAGE_USERS'
        initial = manage.runtime_summary(adapter, 'boot', '123', dumps)
        self.assertFalse(initial['needs_repair'])
        dumps['com.android.vending'] = dumps['com.android.vending'].replace('  requested permissions:\n', '  requested permissions:\n    android.permission.MANAGE_USERS\n')
        result = manage.runtime_summary(adapter, 'boot', '123', dumps)
        self.assertTrue(result['needs_repair'])
        self.assertEqual(result['status'], 'needs_repair')
        self.assertIn('com.android.vending:android.permission.MANAGE_USERS', result['newly_declared_permissions'])
        self.assertEqual(result['repair_command'], 'manage.py repair-runtime')

    def test_legacy_receipt_without_skip_lines_still_detects_uncovered_permission(self):
        adapter, dumps = self.fixture()
        dumps['com.android.vending'] = dumps['com.android.vending'].replace('  requested permissions:\n', '  requested permissions:\n    android.permission.MANAGE_USERS\n')
        result = manage.runtime_summary(adapter, 'boot', '123', dumps)
        self.assertTrue(result['needs_repair'])
        self.assertIn('com.android.vending:android.permission.MANAGE_USERS', result['newly_declared_permissions'])

    def test_unreadable_requested_permissions_never_claims_coverage(self):
        adapter, dumps = self.fixture()
        dumps['com.android.vending'] = dumps['com.android.vending'].replace('requested permissions:', 'unrecognized field:')
        result = manage.runtime_summary(adapter, 'boot', '123', dumps)
        self.assertTrue(result['needs_repair'])
        self.assertEqual(result['permission_coverage_unknown'], ['com.android.vending'])

    def test_missing_platform_permissions_are_excluded_only_with_current_inventory(self):
        adapter, dumps = self.fixture()
        permission = 'android.permission.GET_ROLE_HOLDERS'
        dumps['com.google.android.gms'] = dumps['com.google.android.gms'].replace('  requested permissions:\n', '  requested permissions:\n    ' + permission + '\n')
        adapter['java_result'] += '\nSKIP unavailable ' + permission
        self.assertTrue(manage.runtime_summary(adapter, 'boot', '123', dumps)['needs_repair'])
        self.assertFalse(manage.runtime_summary(adapter, 'boot', '123', dumps, registered={'android.permission.INTERNET'})['needs_repair'])
        # If an updated firmware now exposes the permission, an old SKIP cannot
        # suppress the new requirement even with the same app version.
        self.assertTrue(manage.runtime_summary(adapter, 'boot', '123', dumps, registered={permission})['needs_repair'])
        self.assertTrue(manage.runtime_summary(adapter, 'boot', '456', dumps, registered=set())['needs_repair'])

    def test_partial_platform_permission_output_is_not_treated_as_empty_inventory(self):
        for raw in ('', 'permission:android.permission.INTERNET', 'All Permissions:\npermission:android.permission.INTERNET'):
            with self.assertRaises(RuntimeError):
                manage.registered_permissions(raw)

    def test_component_version_change_requires_new_receipt_not_overwriting_evidence(self):
        adapter, dumps = self.fixture()
        before = manage.runtime_summary(adapter, 'boot', '123', dumps)
        prior = {'receipt_sha256': before['receipt_sha256'], 'versions': before['component_versions']}
        dumps['com.android.vending'] = dumps['com.android.vending'].replace('versionCode=123', 'versionCode=124')
        result = manage.runtime_summary(adapter, 'boot', '123', dumps, prior)
        self.assertTrue(result['needs_repair'])
        self.assertEqual(result['updated_components'], ['com.android.vending'])
        adapter['unix_time'] = 200
        repaired = manage.runtime_summary(adapter, 'boot', '123', dumps, prior)
        self.assertFalse(repaired['needs_repair'])

    def test_same_version_reinstallation_is_detected_from_update_time(self):
        adapter, dumps = self.fixture()
        before = manage.runtime_summary(adapter, 'boot', '123', dumps)
        prior = {'receipt_sha256': before['receipt_sha256'], 'versions': before['component_versions']}
        dumps['com.google.android.gms'] = dumps['com.google.android.gms'].replace('12:00:00', '12:01:00')
        result = manage.runtime_summary(adapter, 'boot', '123', dumps, prior)
        self.assertTrue(result['needs_repair'])
        self.assertEqual(result['updated_components'], ['com.google.android.gms'])

    def test_absent_version_or_changed_process_never_claims_ready(self):
        adapter, dumps = self.fixture()
        self.assertTrue(manage.runtime_summary(adapter, 'boot', '456', dumps)['needs_repair'])
        dumps['com.google.android.gsf'] = ''
        self.assertTrue(manage.runtime_summary(adapter, 'boot', '123', dumps)['needs_repair'])

    def test_reviewed_status_permission_list_matches_pinned_native_dex(self):
        loader = TOOLS.parent / 'module/inject.py'
        self.assertEqual(manage.python_source_sha256(loader), '279305289749acada3f9951c816d7e9f00367dce4331d4396c04554e4685fbf8')
        tree = ast.parse(loader.read_text())
        expression = next(node for node in ast.walk(tree) if isinstance(node, ast.Call)
                          and isinstance(node.func, ast.Attribute) and node.func.attr == 'b64decode'
                          and isinstance(node.args[0], ast.Constant))
        dex = base64.b64decode(expression.args[0].value)
        embedded = {value.decode() for value in re.findall(rb'android\.permission\.[A-Z0-9_]+', dex)}
        reviewed = set().union(*map(set, manage.REVIEWED_PERMISSIONS.values()))
        self.assertEqual(reviewed, embedded)
        self.assertEqual([len(v) for v in manage.REVIEWED_PERMISSIONS.values()], [120, 36])


if __name__ == '__main__':
    unittest.main()
