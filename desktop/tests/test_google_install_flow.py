"""Google installation recovery fixtures; no ADB, root, network or account access."""
import ast
import base64
import builtins
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[2] / 'handoff/google-play-stepx/tools'
sys.path.insert(0, str(TOOLS))
spec = importlib.util.spec_from_file_location('google_install_flow', TOOLS / 'manage.py')
manage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manage)


def digest(data):
    return hashlib.sha256(data).hexdigest()


class InstallGateFixture:
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.stack = contextlib.ExitStack()
        self.package = manage.PACKAGES[0]
        self.key = 'usb_install_item_' + self.package
        self.settings = {'usb_install_item_switch': 'switch:1'}
        self.writes = []
        self.stack.enter_context(patch.object(manage, 'STATE', self.root))
        self.stack.enter_context(patch.object(manage, 'DEVICE_KEY', 'fixture-device'))
        self.stack.enter_context(patch.object(manage, 'verified_fingerprint', return_value='reviewed-12'))
        self.stack.enter_context(patch.object(manage, 'baseline_matches', side_effect=lambda x: x.get('serial') == 'fixture-device' and x.get('fingerprint') == 'reviewed-12'))
        self.stack.enter_context(patch.object(manage, 'save_json', side_effect=lambda name, value: (self.root / name).write_text(json.dumps(value))))
        self.stack.enter_context(patch.object(manage, 'shell', side_effect=self.shell))

    def tearDown(self):
        self.stack.close()
        self.temp.cleanup()

    def shell(self, *args):
        if args[:3] == ('settings', 'get', 'secure'):
            return self.settings.get(args[3], 'null')
        if args[:3] == ('settings', 'put', 'secure'):
            self.writes.append(args)
            self.settings[args[3]] = args[4]
            self.assertTrue(manage.install_gate_path(self.package).exists(), 'recovery must precede a setting write')
            return ''
        if args[:3] == ('settings', 'delete', 'secure'):
            self.writes.append(args)
            self.settings.pop(args[3], None)
            return ''
        raise AssertionError(args)

class InstallGateTests(InstallGateFixture, unittest.TestCase):
    def test_closed_master_switch_gives_guidance_without_mutations(self):
        self.settings['usb_install_item_switch'] = 'null'
        with self.assertRaisesRegex(RuntimeError, '开发者选项'):
            with manage.temporary_install_gate(self.package):
                self.fail('must not install')
        self.assertEqual(self.writes, [])
        self.assertFalse(list(self.root.iterdir()))

    def test_success_and_failed_install_restore_exact_prior_setting(self):
        for prior in (None, 'Google 服务框架:0', 'custom label:1'):
            for failed in (False, True):
                with self.subTest(prior=prior, failed=failed):
                    self.settings.pop(self.key, None)
                    if prior is not None:
                        self.settings[self.key] = prior
                    try:
                        with manage.temporary_install_gate(self.package):
                            self.assertEqual(self.settings[self.key], manage.INSTALL_LABELS[self.package] + ':1')
                            if failed:
                                raise subprocess.CalledProcessError(1, ['adb', 'install'])
                    except subprocess.CalledProcessError:
                        pass
                    self.assertEqual(self.settings.get(self.key), prior)
                    self.assertFalse(manage.install_gate_path(self.package).exists())
                    self.assertEqual(self.settings['usb_install_item_switch'], 'switch:1')
        self.assertTrue(all(call[3] == self.key for call in self.writes))

    def test_interrupted_gate_is_restored_before_next_attempt(self):
        path = manage.install_gate_path(self.package)
        temporary = manage.INSTALL_LABELS[self.package] + ':1'
        path.write_text(json.dumps({'serial': 'fixture-device', 'fingerprint': 'reviewed-12',
            'package': self.package, 'key': self.key, 'temporary': temporary, 'previous': 'original:0'}))
        self.settings[self.key] = temporary
        with manage.temporary_install_gate(self.package):
            self.assertEqual(json.loads(path.read_text())['previous'], 'original:0')
        self.assertEqual(self.settings[self.key], 'original:0')

    def test_concurrent_setting_change_is_preserved_with_recovery_record(self):
        with self.assertRaisesRegex(RuntimeError, '其他操作'):
            with manage.temporary_install_gate(self.package):
                self.settings[self.key] = 'phone user changed:0'
        self.assertEqual(self.settings[self.key], 'phone user changed:0')
        self.assertTrue(manage.install_gate_path(self.package).exists())

    def test_foreign_package_or_identity_cannot_be_authorized(self):
        with self.assertRaises(RuntimeError):
            with manage.temporary_install_gate('another.app'):
                pass
        path = manage.install_gate_path(self.package)
        path.write_text(json.dumps({'serial': 'another-device'}))
        with self.assertRaisesRegex(RuntimeError, '恢复记录'):
            manage.finish_install_gate(self.package)
        self.assertEqual(self.writes, [])

    def test_old_default_firmware_baseline_is_not_silently_reassigned(self):
        baseline = {'serial': 'fixture-device', 'fingerprint': 'reviewed-172', 'packages': []}
        (self.root / 'baseline.json').write_text(json.dumps(baseline))
        (self.root / 'install-progress.json').write_text(json.dumps({**baseline, 'completed': []}))
        self.assertFalse(manage.resumable_install('reviewed-12'))
        with self.assertRaises(RuntimeError):
            manage.require_current_baseline(baseline)
        self.assertEqual(json.loads((self.root / 'baseline.json').read_text()), baseline)


@unittest.skipUnless(os.name == 'posix', 'executes an Android/Linux no-follow payload in a local fixture')
class PreparationPayloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.launcher = self.root / 'usr/local/bin/ubuntu_zygote'
        self.launcher.parent.mkdir(parents=True)
        self.original = b'factory fixture executable'
        self.launcher.write_bytes(self.original)
        self.stock = Path(str(self.launcher) + '.stepx-stock-20261021')
        self.host = self.root / 'android'
        for relative in ('data/system/packages.xml', 'data/misc_de/0/apexdata/com.android.permission/access.abx'):
            path = self.host / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'metadata')
        tree = ast.parse((TOOLS / 'manage.py').read_text())
        self.code = next(n.value for n in ast.walk(tree) if isinstance(n, ast.Constant)
                         and isinstance(n.value, str) and n.value.startswith('import os,json,hashlib,base64,stat'))
        self.code = self.code.replace("'/usr/local/", repr(str(self.root / 'usr/local') + '/')[:-1])
        self.code = self.code.replace("'/var/lib/", repr(str(self.root / 'var/lib') + '/')[:-1])

    def tearDown(self):
        self.temp.cleanup()

    def execute(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            exec(self.code, {'HOST': str(self.host), 'STOCK_SHA': digest(self.original),
                             'DEVICE': 'fixture-device', 'FINGERPRINT': 'reviewed-12'})
        return json.loads(output.getvalue())

    def test_permission_denied_optional_metadata_does_not_lose_stock_proof(self):
        real_open = builtins.open
        def read(path, *args, **kwargs):
            if str(path).endswith('/access.abx'):
                raise PermissionError('SELinux fixture')
            return real_open(path, *args, **kwargs)
        with patch('builtins.open', side_effect=read):
            proof = self.execute()
        self.assertEqual(base64.b64decode(proof['stock']), self.original)
        self.assertEqual(proof['stock_sha256'], digest(self.original))
        self.assertEqual(proof['skipped_files'], {'/data/misc_de/0/apexdata/com.android.permission/access.abx': 'permission_denied'})
        self.assertIn('/data/system/packages.xml', proof['files'])

    def test_removed_companion_stock_backup_is_reused_without_deletion(self):
        self.stock.write_bytes(self.original)
        proof = self.execute()
        self.assertEqual(proof['stock_sha256'], digest(self.original))
        self.assertEqual(self.stock.read_bytes(), self.original)

    def test_missing_or_changed_factory_backup_is_not_optional(self):
        self.stock.write_bytes(b'changed')
        with self.assertRaisesRegex(AssertionError, 'backup differs'):
            self.execute()
        self.stock.unlink()
        self.launcher.unlink()
        with self.assertRaises(FileNotFoundError):
            self.execute()

    def test_symlink_stock_backup_is_rejected(self):
        self.stock.symlink_to(self.launcher)
        with self.assertRaises(OSError):
            self.execute()

    def test_active_companion_is_verified_before_stock_is_adopted(self):
        base = self.root / 'usr/local/lib/stepx-companion'
        base.mkdir(parents=True)
        self.stock.write_bytes(self.original)
        self.launcher.write_bytes(b'companion launcher')
        config = {'serial': 'fixture-device', 'fingerprint': 'reviewed-12', 'boot_mode': 'standalone',
                  'hook_path': str(self.launcher), 'hook_sha256': digest(self.launcher.read_bytes())}
        for name in ('chain', 'boot', 'grant'):
            data = (name + ' fixture').encode()
            (base / (name + '.py')).write_bytes(data)
            config[name + '_sha256'] = digest(data)
        (base / 'config.json').write_text(json.dumps(config))
        self.assertEqual(base64.b64decode(self.execute()['stock']), self.original)
        (base / 'grant.py').write_bytes(b'changed')
        with self.assertRaisesRegex(AssertionError, 'artifact changed'):
            self.execute()

    def test_prepare_saves_actual_firmware_in_both_recovery_records(self):
        state = self.root / 'host-state'
        state.mkdir()
        observed = {'fingerprint': 'actual-reviewed-12', 'user_unlocked': True,
                    'google_packages': dict.fromkeys(manage.PACKAGES, False)}
        def shell(*args):
            if args == ('pm', 'list', 'users'):
                return 'Users:\n UserInfo{0:Owner:13}'
            if args[:2] == ('ps', '-A'):
                return 'PID PPID NAME\n123 1 openclaw_env'
            return ''
        proof = {'stock': base64.b64encode(self.original).decode(), 'stock_sha256': digest(self.original),
                 'files': {}, 'skipped_files': {'/data/misc_de/0/apexdata/com.android.permission/access.abx': 'permission_denied'}}
        with patch.object(manage, 'STATE', state), patch.object(manage, 'DEVICE_KEY', 'fixture-device'), \
                patch.object(manage, 'CONFIG', {**manage.CONFIG, 'fingerprint': 'profile-default-172', 'stock_sha256': digest(self.original)}), \
                patch.object(manage, 'preflight', return_value=observed), patch.object(manage, 'init_state'), \
                patch.object(manage, 'shell', side_effect=shell), patch.object(manage, 'root_json', return_value=proof), \
                patch.object(manage, 'save_json', side_effect=lambda n,v: (state / n).write_text(json.dumps(v))), \
                contextlib.redirect_stdout(io.StringIO()):
            manage.prepare()
        for name in ('baseline.json', 'install-progress.json'):
            self.assertEqual(json.loads((state / name).read_text())['fingerprint'], observed['fingerprint'])
        self.assertEqual((state / 'ubuntu_zygote.stock').read_bytes(), self.original)
        self.assertIn('permission_denied', json.loads((state / 'baseline.json').read_text())['metadata_backups'].values())


class InstallTransactionTests(InstallGateFixture, unittest.TestCase):
    def pending_gates(self):
        baseline = {'serial': 'fixture-device', 'fingerprint': 'reviewed-12', 'packages': []}
        (self.root / 'baseline.json').write_text(json.dumps(baseline))
        (self.root / 'install-progress.json').write_text(json.dumps({**baseline, 'completed': []}))
        for package in manage.PACKAGES[1:]:
            key = 'usb_install_item_' + package
            temporary = manage.INSTALL_LABELS[package] + ':1'
            manage.install_gate_path(package).write_text(json.dumps({
                'serial': 'fixture-device', 'fingerprint': 'reviewed-12', 'package': package,
                'key': key, 'temporary': temporary, 'previous': None}))
            self.settings[key] = temporary

    def assert_gates_restored(self):
        self.assertEqual(self.settings, {'usb_install_item_switch': 'switch:1'})
        self.assertFalse(list(self.root.glob('usb-install-*.json')))

    def test_missing_apk_cache_does_not_prevent_all_pending_gate_recovery(self):
        self.pending_gates()
        def check():
            self.assert_gates_restored()
            raise FileNotFoundError('downloaded APK was removed')
        with patch.object(manage, 'check_apks', side_effect=check), patch.object(manage, 'run') as install:
            with self.assertRaises(FileNotFoundError):
                manage._install_apks()
            install.assert_not_called()
        self.assert_gates_restored()

    def test_updated_first_package_does_not_block_later_package_gate_recovery(self):
        self.pending_gates()
        original_shell = self.shell
        def shell(*args):
            if args[:4] == ('pm', 'list', 'packages', '--user'):
                self.assert_gates_restored()
                return 'package:' + manage.PACKAGES[0]
            if args[:2] == ('pm', 'path'):
                return 'package:/data/app/updated/base.apk'
            if args[0] == 'sha256sum':
                return '0' * 64 + '  ' + args[1]
            return original_shell(*args)
        with patch.object(manage, 'shell', side_effect=shell), patch.object(manage, 'check_apks'), \
                patch.object(manage, 'run') as install:
            with self.assertRaisesRegex(AssertionError, '版本不同'):
                manage._install_apks()
            install.assert_not_called()
        self.assert_gates_restored()

    def test_all_three_apks_are_verified_and_each_gate_restored(self):
        baseline = {'serial': 'fixture-device', 'fingerprint': 'reviewed-12', 'packages': []}
        (self.root / 'baseline.json').write_text(json.dumps(baseline))
        (self.root / 'install-progress.json').write_text(json.dumps({**baseline, 'completed': []}))
        installed = set()
        original_shell = self.shell
        metadata = {entry['package']: entry for entry in manage.CONFIG['apks']}
        def shell(*args):
            if args[:4] == ('pm', 'list', 'packages', '--user'):
                return 'package:' + args[-1] if args[-1] in installed else ''
            if args[:2] == ('pm', 'path'):
                return 'package:/data/app/' + args[-1] + '/base.apk'
            if args[0] == 'sha256sum':
                return metadata[args[1].split('/')[3]]['sha256'] + '  ' + args[1]
            if args[:2] in (('pm', 'grant'), ('cmd', 'appops')) or args[:3] == ('settings', 'put', 'global'):
                return ''
            return original_shell(*args)
        def install(args, **kwargs):
            package = next(p for p in manage.PACKAGES if metadata[p]['filename'] == Path(args[-1]).name)
            self.package = package  # Fixture asserts the per-package record exists before writes.
            self.assertEqual(self.settings['usb_install_item_' + package], manage.INSTALL_LABELS[package] + ':1')
            installed.add(package)
            return types.SimpleNamespace(stdout='Success\n')
        def package_gate(package):
            self.package = package
            return original_gate(package)
        original_gate = manage.temporary_install_gate
        with patch.object(manage, 'shell', side_effect=shell), patch.object(manage, 'check_apks'), \
                patch.object(manage, 'run', side_effect=install), patch.object(manage, 'temporary_install_gate', side_effect=package_gate), \
                contextlib.redirect_stdout(io.StringIO()):
            manage._install_apks()
        self.assertEqual(installed, set(manage.PACKAGES))
        self.assertEqual(self.settings, {'usb_install_item_switch': 'switch:1'})
        self.assertFalse(list(self.root.glob('usb-install-*.json')))
        self.assertEqual(json.loads((self.root / 'install-progress.json').read_text())['completed'], manage.PACKAGES)

    def test_failed_adb_install_restores_gate_and_keeps_progress_for_retry(self):
        baseline = {'serial': 'fixture-device', 'fingerprint': 'reviewed-12', 'packages': []}
        (self.root / 'baseline.json').write_text(json.dumps(baseline))
        receipt = {**baseline, 'completed': []}
        (self.root / 'install-progress.json').write_text(json.dumps(receipt))
        original_shell = self.shell
        def shell(*args):
            if args[:4] == ('pm', 'list', 'packages', '--user'):
                return ''
            return original_shell(*args)
        def install(*args, **kwargs):
            self.assertEqual(self.settings[self.key], manage.INSTALL_LABELS[self.package] + ':1')
            raise subprocess.CalledProcessError(1, ['adb', 'install'])
        with patch.object(manage, 'shell', side_effect=shell), patch.object(manage, 'check_apks'), \
                patch.object(manage, 'run', side_effect=install):
            with self.assertRaises(subprocess.CalledProcessError):
                manage._install_apks()
        self.assertNotIn(self.key, self.settings)
        self.assertFalse(manage.install_gate_path(self.package).exists())
        self.assertEqual(json.loads((self.root / 'install-progress.json').read_text()), receipt)


@unittest.skipUnless(os.name == 'posix', 'executes an Android/Linux ownership and directory-fsync payload')
class AdapterReuseTests(unittest.TestCase):
    def test_removed_companion_backup_installs_google_without_overwriting_stock(self):
        tree = ast.parse((TOOLS / 'install_adapter.py').read_text())
        code = next(n.value for n in ast.walk(tree) if isinstance(n, ast.Constant) and
                    isinstance(n.value, str) and n.value.startswith('import os,stat,hashlib'))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            launcher = root / 'usr/local/bin/ubuntu_zygote'
            launcher.parent.mkdir(parents=True)
            (root / 'usr/local/lib/stepx-companion').mkdir(parents=True)
            (root / 'var/lib').mkdir(parents=True)
            original = b'factory executable'
            launcher.write_bytes(original)
            launcher.chmod(0o755)
            stock = Path(str(launcher) + '.stepx-stock-20261021')
            stock.write_bytes(original)
            inode = stock.stat().st_ino
            files = {'launcher.py': b'# google launcher\n', 'boot.py': b'# google boot\n', 'inject.py': b'# google inject\n'}
            stage = root / 'stage'
            stage.mkdir()
            for name, data in files.items():
                (stage / name).write_bytes(data)
            code = code.replace("'/usr/local/", repr(str(root / 'usr/local') + '/')[:-1])
            code = code.replace("'/var/lib/", repr(str(root / 'var/lib') + '/')[:-1])
            with patch('os.chown'), contextlib.redirect_stdout(io.StringIO()):
                exec(code, {'STAGE': str(stage), 'MANIFEST': {k: digest(v) for k, v in files.items()}, 'STOCK_SHA': digest(original)})
            self.assertEqual(stock.read_bytes(), original)
            self.assertEqual(stock.stat().st_ino, inode)
            self.assertEqual(launcher.read_bytes(), files['launcher.py'])


if __name__ == '__main__':
    unittest.main()
