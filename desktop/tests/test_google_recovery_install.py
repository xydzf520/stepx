"""Recovery scenarios run against fake phones and local archives, never ADB."""
import contextlib
import hashlib
import importlib.util
import io
import itertools
import json
import os
from pathlib import Path
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch
import zipfile

from test_google_install_flow import InstallGateFixture, manage, install_recovery, WorkflowError
import common
import root_bridge
import workflow_errors

_COMPAT_SPEC = importlib.util.spec_from_file_location(
    'google_play_compat_recovery_test',
    Path(__file__).resolve().parents[2] / 'scripts/google-play-compat.py',
)
compat = importlib.util.module_from_spec(_COMPAT_SPEC)
_COMPAT_SPEC.loader.exec_module(compat)


class RecoveryInstallTests(InstallGateFixture, unittest.TestCase):
    def run_scenario(self, present, fail_at=None):
        # A previous computer's original baseline must never be replaced.
        original = b'{"private_original_baseline":"must stay unchanged"}'
        (self.root / 'baseline.json').write_bytes(original)
        baseline = dict(kind='recovery_snapshot', serial='fixture-device',
                        fingerprint='reviewed-12', packages=list(present))
        (self.root / 'recovery-baseline.json').write_text(json.dumps(baseline))
        progress = dict(serial='fixture-device', fingerprint='reviewed-12', completed=[])
        (self.root / 'recovery-progress.json').write_text(json.dumps(progress))
        installed, installs, checked = set(present), [], []
        metadata = {entry['package']: entry for entry in manage.CONFIG['apks']}
        original_shell, original_gate = self.shell, manage.temporary_install_gate

        def shell(*args):
            if args[:4] == ('pm', 'list', 'packages', '--user'):
                selected = installed if len(args) == 5 else installed & {args[-1]}
                return ('package:android\n' if len(args) == 5 else '') + '\n'.join('package:' + p for p in selected)
            if args[:2] == ('pm', 'path'):
                return 'package:/data/app/' + args[-1] + '/base.apk'
            if args[0] == 'sha256sum':
                return metadata[args[1].split('/')[3]]['sha256'] + '  ' + args[1]
            if args[:2] in (('pm', 'grant'), ('cmd', 'appops')) or args[:3] == ('settings', 'put', 'global'):
                self.assertEqual(installed, set(manage.PACKAGES), 'no grants until all packages verified')
                return ''
            return original_shell(*args)

        def verify(package):
            checked.append(package)
            return {'verified': True}

        def install(args, **kwargs):
            package = next(p for p in manage.PACKAGES if metadata[p]['filename'] == Path(args[-1]).name)
            self.assertTrue(set(present) <= set(checked), 'all preexisting packages verified before first install')
            self.assertNotIn(package, present)
            self.assertNotIn('-r', args)
            self.assertNotIn('-d', args)
            if package == fail_at:
                raise subprocess.CalledProcessError(1, ['fixture-adb'], stderr='INSTALL_FAILED_INSUFFICIENT_STORAGE')
            installed.add(package)
            installs.append(package)
            return types.SimpleNamespace(stdout='Success\n')

        def gate(package):
            self.package = package
            return original_gate(package)

        with patch.object(manage, 'shell', side_effect=shell), patch.object(manage, 'run', side_effect=install), \
                patch.object(manage, 'check_apks') as cache, \
                patch.object(install_recovery, 'verify_installed_package', side_effect=verify), \
                patch.object(manage, 'temporary_install_gate', side_effect=gate), contextlib.redirect_stdout(io.StringIO()):
            if fail_at:
                with self.assertRaises(subprocess.CalledProcessError):
                    manage._install_apks(recovery=True)
                self.assertEqual(self.settings, {'usb_install_item_switch': 'switch:1'})
                self.assertFalse(list(self.root.glob('usb-install-*.json')))
                fail_at = None
            manage._install_apks(recovery=True)
            # Repeating a completed recovery never installs again or needs an archive.
            cache.reset_mock()
            manage._install_apks(recovery=True)
            cache.assert_not_called()
        self.assertEqual(installed, set(manage.PACKAGES))
        self.assertEqual(installs, [p for p in manage.PACKAGES if p not in present])
        self.assertEqual((self.root / 'baseline.json').read_bytes(), original)
        self.assertEqual(json.loads((self.root / 'recovery-progress.json').read_text())['completed'], installs)
        self.assertEqual(self.settings, {'usb_install_item_switch': 'switch:1'})

    def test_every_partial_inventory_and_complete_unmanaged_install(self):
        for mask in itertools.product((False, True), repeat=3):
            with self.subTest(mask=mask):
                self.run_scenario([p for p, exists in zip(manage.PACKAGES, mask) if exists])

    def test_interruption_at_each_package_resumes_without_reinstalling(self):
        for package in manage.PACKAGES:
            with self.subTest(package=package):
                self.run_scenario([], fail_at=package)

    def test_foreign_progress_stops_before_installs_and_grants(self):
        baseline = dict(kind='recovery_snapshot', serial='fixture-device', fingerprint='reviewed-12', packages=[])
        (self.root / 'recovery-baseline.json').write_text(json.dumps(baseline))
        (self.root / 'recovery-progress.json').write_text(json.dumps(dict(baseline, serial='different-phone', completed=[])))
        with patch.object(manage, 'run') as run, self.assertRaises(WorkflowError):
            manage._install_apks(recovery=True)
        run.assert_not_called()
        self.assertEqual(self.writes, [])

    def test_snapshot_progress_crash_gap_is_recoverable_without_new_snapshot(self):
        baseline = dict(kind='recovery_snapshot', serial='fixture-device', fingerprint='reviewed-12', packages=[])
        (self.root / 'recovery-baseline.json').write_text(json.dumps(baseline))
        observed = dict(fingerprint='reviewed-12', user_unlocked=True, google_packages=dict.fromkeys(manage.PACKAGES, False))
        with patch.object(manage, 'preflight', return_value=observed), patch.object(manage, 'owned_runtime', return_value=True), \
                patch.object(manage, 'sha256', return_value=manage.CONFIG['stock_sha256']), patch.object(manage, 'root_json') as root, \
                contextlib.redirect_stdout(io.StringIO()):
            manage._prepare(recovery=True)
        root.assert_not_called()
        self.assertEqual(json.loads((self.root / 'recovery-progress.json').read_text())['completed'], [])
        self.assertFalse((self.root / 'baseline.json').exists())

    def test_fresh_snapshot_crash_gap_before_any_package_is_installed(self):
        baseline = dict(serial='fixture-device', fingerprint='reviewed-12', packages=['android'])
        (self.root / 'baseline.json').write_text(json.dumps(baseline))
        observed = dict(fingerprint='reviewed-12', user_unlocked=True, google_packages=dict.fromkeys(manage.PACKAGES, False))
        with patch.object(manage, 'preflight', return_value=observed), \
                patch.object(manage, 'sha256', return_value=manage.CONFIG['stock_sha256']), \
                contextlib.redirect_stdout(io.StringIO()):
            manage._prepare()
        self.assertEqual(json.loads((self.root / 'install-progress.json').read_text())['completed'], [])
        self.assertEqual(json.loads((self.root / 'baseline.json').read_text()), baseline)


class AdoptionTests(unittest.TestCase):
    def test_empty_or_failed_inventory_never_means_fresh_install(self):
        for raw in ('', 'Error: package manager unavailable', 'package:android\ntruncated output'):
            with patch.object(install_recovery, 'shell', return_value=raw), self.assertRaises(WorkflowError) as error:
                install_recovery.package_inventory()
            self.assertTrue(error.exception.retryable)
    def test_plans_distinguish_owned_partial_and_retained_state(self):
        for mask in itertools.product((False, True), repeat=3):
            for owned in (False, True):
                value = dict(google_packages=dict(zip(install_recovery.PACKAGES, mask)))
                result = install_recovery.plan(value, owned)
                expected = 'existing' if all(mask) and owned else 'fresh' if not any(mask) and not owned else 'recovery'
                self.assertEqual(result['mode'], expected)
                self.assertEqual(result['requires_install_switch'], not all(mask))
        value = dict(google_packages=dict.fromkeys(install_recovery.PACKAGES, False), retained_packages=['com.google.android.gms'])
        self.assertEqual(install_recovery.plan(value, False)['mode'], 'recovery')

    def verify_fixture(self, split=False, bad_signer=False, changing=False, pinned=False):
        meta = install_recovery.CONFIG['apks'][0]
        files = ['/data/app/fixture/base.apk'] + (['/data/app/fixture/split.apk'] if split else [])
        data = {f: f.encode() for f in files}
        digest = {f: hashlib.sha256(b).hexdigest() for f, b in data.items()}
        if pinned: digest[files[0]] = meta['sha256']
        seen = {}
        def shell(*args):
            if args[0] == 'pm': return '\n'.join('package:' + f for f in files)
            f = args[1]
            seen[f] = seen.get(f, 0) + 1
            return ('f' * 64 if changing and seen[f] > 1 else digest[f]) + '  ' + f
        def run(args, **kwargs):
            if 'pull' in args:
                Path(args[-1]).write_bytes(data[args[-2]])
                return types.SimpleNamespace(stdout='')
            signer = '0' * 64 if bad_signer else meta['signer_sha256']
            return types.SimpleNamespace(stdout='Signer #1 certificate SHA-256 digest: ' + signer + '\n')
        with tempfile.TemporaryDirectory() as temp, patch.object(install_recovery, 'STATE', Path(temp)), \
                patch.object(install_recovery, 'shell', side_effect=shell), patch.object(install_recovery, 'run', side_effect=run), \
                patch.object(install_recovery, 'sdk_command', return_value=['fixture-apksigner']):
            result = install_recovery.verify_installed_package(meta['package'])
            self.assertEqual(list(Path(temp).iterdir()), [], 'pulled APKs must be removed')
            return result

    def test_legitimate_newer_and_split_apks_are_preserved(self):
        for split in (False, True):
            self.assertTrue(self.verify_fixture(split=split)['verified'])

    def test_unknown_signer_is_rejected(self):
        with self.assertRaisesRegex(WorkflowError, '签名'):
            self.verify_fixture(bad_signer=True)

    def test_package_changed_during_verification_is_retryable(self):
        for pinned in (False, True):
            with self.assertRaises(WorkflowError) as error:
                self.verify_fixture(changing=True, pinned=pinned)
            self.assertTrue(error.exception.retryable)

    def test_error_categories_do_not_leak_private_tracebacks(self):
        for cause, code, retry in [(AssertionError('Unsupported firmware: private-id'), 'unsupported_firmware', False),
                                  (subprocess.CalledProcessError(1, [], stderr='INSTALL_FAILED_INSUFFICIENT_STORAGE private-id'), 'storage_full', False),
                                  (TimeoutError('private-id'), 'timeout', True),
                                  (RuntimeError('/home/stepx-user/account'), 'needs_inspection', False)]:
            result = workflow_errors.describe(cause)
            self.assertEqual((result['code'], result['retryable']), (code, retry))
            self.assertNotIn('private-id', result['message'])


class DurableCacheTests(unittest.TestCase):
    def test_legacy_removal_rejects_missing_corrupt_foreign_and_recovery_records(self):
        baseline = dict(serial='phone', fingerprint='fp', packages=[])
        with tempfile.TemporaryDirectory() as temp, patch.object(install_recovery, 'STATE', Path(temp)), \
                patch.object(install_recovery, 'shell') as shell, patch.object(install_recovery, 'verify_existing') as verify:
            progress = Path(temp) / 'install-progress.json'
            for data in (None, '{', json.dumps(dict(serial='other', fingerprint='fp', completed=[]))):
                progress.unlink(missing_ok=True)
                if data: progress.write_text(data)
                with self.assertRaises(WorkflowError):
                    install_recovery.require_fresh_rollback(baseline, 'fp', 'phone')
            progress.write_text(json.dumps(dict(serial='phone', fingerprint='fp', completed=[])))
            (Path(temp) / 'recovery-baseline.json').write_text('corrupt is still evidence')
            with self.assertRaises(WorkflowError):
                install_recovery.require_fresh_rollback(baseline, 'fp', 'phone')
            verify.assert_not_called()
            shell.assert_not_called()

    def test_updated_packages_and_existing_or_unknown_accounts_block_legacy_removal(self):
        package = install_recovery.PACKAGES[0]
        baseline = dict(serial='phone', fingerprint='fp', packages=[])
        with tempfile.TemporaryDirectory() as temp, patch.object(install_recovery, 'STATE', Path(temp)), \
                patch.object(install_recovery, 'shell') as shell, patch.object(install_recovery, 'verify_existing') as verify:
            (Path(temp) / 'install-progress.json').write_text(json.dumps(dict(serial='phone', fingerprint='fp', completed=[package])))
            verify.return_value = {package: {'reviewed_archive': False}}
            with self.assertRaises(WorkflowError):
                install_recovery.require_fresh_rollback(baseline, 'fp', 'phone')
            shell.assert_not_called()
            verify.return_value = {package: {'reviewed_archive': True}}
            for raw in ('Permission denied', 'Accounts: 1\n', 'Accounts: 1\n Account {name=fixture, type=com.google}\n'):
                shell.return_value = raw
                with self.assertRaises(WorkflowError):
                    install_recovery.require_fresh_rollback(baseline, 'fp', 'phone')
            shell.return_value = 'Accounts: 0\n'
            install_recovery.require_fresh_rollback(baseline, 'fp', 'phone')

    def test_failed_atomic_receipt_write_retains_old_record(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'receipt.json'
            path.write_bytes(b'original')
            with patch.object(common.os, 'replace', side_effect=OSError('disk failure')), self.assertRaises(OSError):
                common.private_write(path, b'next')
            self.assertEqual(path.read_bytes(), b'original')
            self.assertEqual(list(Path(temp).iterdir()), [path])

    def test_bad_cached_archive_is_replaced_and_download_is_bounded(self):
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, 'w') as archive: archive.writestr('test.apk', b'apk fixture')
        payload = stream.getvalue()
        config = dict(archive=dict(name='fixture.zip', url='https://example.invalid/fixture', sha256=hashlib.sha256(payload).hexdigest()),
                      apks=[dict(filename='test.apk', zip_path='test.apk', sha256=hashlib.sha256(b'apk fixture').hexdigest())])
        with tempfile.TemporaryDirectory() as temp, patch.object(manage, 'DOWNLOADS', Path(temp)), \
                patch.object(manage, 'CONFIG', config), patch.object(manage, 'check_apks'), \
                patch.object(manage.urllib.request, 'urlopen', side_effect=[TimeoutError('interrupted'), io.BytesIO(payload)]) as download:
            cached = Path(temp) / 'fixture.zip'
            cached.write_bytes(b'broken old cache')
            manage.fetch()
            self.assertEqual(cached.read_bytes(), payload)
            self.assertEqual(download.call_count, 2)
            self.assertFalse((Path(temp) / 'fixture.partial').exists())
            manage.fetch()
            self.assertEqual(download.call_count, 2)

    def test_bundled_runtime_google_apks_bypass_network_download(self):
        apk_bytes = b'bundled-apk-fixture'
        apk_sha = hashlib.sha256(apk_bytes).hexdigest()
        config = dict(
            archive=dict(name='fixture.zip', url='https://github.com/example/fixture.zip', sha256='0' * 64),
            apks=[dict(filename='test.apk', zip_path='test.apk', sha256=apk_sha)],
        )
        with tempfile.TemporaryDirectory() as temp:
            fake_root = Path(temp) / 'repo/handoff/google-play-stepx'
            bundled_dir = Path(temp) / 'repo/runtime/google-play'
            downloads_dir = Path(temp) / 'downloads'
            fake_root.mkdir(parents=True)
            bundled_dir.mkdir(parents=True)
            downloads_dir.mkdir(parents=True)
            (bundled_dir / 'test.apk').write_bytes(apk_bytes)
            (downloads_dir / 'fixture.partial').write_bytes(b'stale-5mb-partial')
            with patch.object(manage, 'ROOT', fake_root), \
                    patch.object(manage, 'DOWNLOADS', downloads_dir), \
                    patch.object(manage, 'CONFIG', config), \
                    patch.object(manage, 'check_apks') as check_apks, \
                    patch.object(manage.urllib.request, 'urlopen', side_effect=AssertionError('network should not be called')):
                manage.fetch()
                self.assertEqual((downloads_dir / 'test.apk').read_bytes(), apk_bytes)
                self.assertFalse((downloads_dir / 'fixture.partial').exists())
                check_apks.assert_called_once()

    def test_github_archive_mirror_fallback_and_range_resume(self):
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, 'w') as archive:
            archive.writestr('test.apk', b'apk fixture')
        payload = stream.getvalue()
        config = dict(
            archive=dict(
                name='fixture.zip',
                url='https://github.com/example/fixture.zip',
                sha256=hashlib.sha256(payload).hexdigest(),
            ),
            apks=[dict(filename='test.apk', zip_path='test.apk', sha256=hashlib.sha256(b'apk fixture').hexdigest())],
        )
        calls = []

        class PartialThenTimeout(io.BytesIO):
            def __init__(self, prefix):
                super().__init__(prefix)
                self._done = False

            def read(self, size=-1):
                if not self._done:
                    self._done = True
                    return super().read(size)
                raise TimeoutError('Download stalled below minimum throughput')

        class RangeResponse(io.BytesIO):
            status = 206

        def fake_urlopen(req, timeout=60):
            url = req.full_url if hasattr(req, 'full_url') else req
            range_hdr = req.get_header('Range') if hasattr(req, 'get_header') else None
            calls.append((url, range_hdr))
            if len(calls) == 1:
                return PartialThenTimeout(payload[:10])
            return RangeResponse(payload[10:])

        with tempfile.TemporaryDirectory() as temp:
            fake_root = Path(temp) / 'repo/handoff/google-play-stepx'
            downloads_dir = Path(temp) / 'downloads'
            fake_root.mkdir(parents=True)
            with patch.object(manage, 'ROOT', fake_root), \
                    patch.object(manage, 'DOWNLOADS', downloads_dir), \
                    patch.object(manage, 'CONFIG', config), \
                    patch.object(manage, 'check_apks') as check_apks, \
                    patch.object(manage.urllib.request, 'urlopen', side_effect=fake_urlopen):
                manage.fetch()
                self.assertEqual((downloads_dir / 'fixture.zip').read_bytes(), payload)
                self.assertEqual((downloads_dir / 'test.apk').read_bytes(), b'apk fixture')
                self.assertFalse((downloads_dir / 'fixture.partial').exists())
                self.assertEqual(calls, [
                    ('https://github.com/example/fixture.zip', None),
                    ('https://ghfast.top/https://github.com/example/fixture.zip', 'bytes=10-'),
                ])
                check_apks.assert_called_once()


class WirelessToUsbFailoverTests(unittest.TestCase):
    @contextlib.contextmanager
    def wireless_context(self, wireless='192.0.2.10:42123', hw_serial='PHONE123'):
        with tempfile.TemporaryDirectory() as temp:
            state_dir = Path(temp) / 'state'
            env = {
                'STEPX_ADB': 'fixture-adb',
                'STEPX_SERIAL': wireless,
                'STEPX_DEVICE_SERIAL': hw_serial,
            }
            with patch.dict(os.environ, env, clear=False), \
                    patch.object(common, 'SERIAL', wireless), \
                    patch.object(common, '_INITIAL_SERIAL', wireless), \
                    patch.object(common, 'DEVICE_SERIAL', hw_serial), \
                    patch.object(common, 'EXPECTED_HW_SERIAL', hw_serial), \
                    patch.object(common, 'DEVICE_KEY', hw_serial), \
                    patch.object(common, '_active_transport', wireless), \
                    patch.object(common, '_failover_from', None), \
                    patch.object(common, 'STATE', state_dir):
                yield state_dir

    def test_wireless_drop_mid_run_switches_to_verified_usb_and_keeps_pin_across_guard(self):
        wireless = '192.0.2.10:42123'
        calls = []

        def fake_subprocess_run(cmd, *args, **kwargs):
            calls.append(list(cmd))
            if cmd == ['fixture-adb', 'devices']:
                return types.SimpleNamespace(
                    returncode=0,
                    stdout=(
                        'List of devices attached\n'
                        f'{wireless}\toffline\n'
                        'OTHER999\tdevice\n'
                        'USB_PORT_A\tdevice\n'
                    ),
                    stderr='',
                )
            if len(cmd) >= 3 and cmd[:2] == ['fixture-adb', '-s']:
                target = cmd[2]
                sub = cmd[3:]
                if target == wireless:
                    raise subprocess.CalledProcessError(
                        1, cmd, stderr=f"error: device '{wireless}' offline",
                    )
                if target == 'OTHER999' and sub == ['shell', 'getprop', 'ro.serialno']:
                    return types.SimpleNamespace(returncode=0, stdout='OTHER999\n', stderr='')
                if target == 'USB_PORT_A':
                    if sub == ['shell', 'getprop', 'ro.serialno']:
                        return types.SimpleNamespace(returncode=0, stdout='PHONE123\n', stderr='')
                    if sub == ['get-state']:
                        return types.SimpleNamespace(returncode=0, stdout='device\n', stderr='')
                    if len(sub) == 2 and sub[0] == 'shell':
                        script = sub[1]
                        if script == 'getprop sys.boot_completed':
                            return types.SimpleNamespace(returncode=0, stdout='1\n', stderr='')
                        if script == 'getprop ro.serialno':
                            return types.SimpleNamespace(returncode=0, stdout='PHONE123\n', stderr='')
                        if script == 'getprop ro.build.fingerprint':
                            return types.SimpleNamespace(returncode=0, stdout=common.CONFIG['fingerprint'] + '\n', stderr='')
                        if script == 'getprop ro.build.version.sdk':
                            return types.SimpleNamespace(returncode=0, stdout=common.CONFIG['sdk'] + '\n', stderr='')
                        if script == 'getprop ro.product.cpu.abilist':
                            return types.SimpleNamespace(returncode=0, stdout=common.CONFIG['abi'] + '\n', stderr='')
                        if script == 'getenforce':
                            return types.SimpleNamespace(returncode=0, stdout='Enforcing\n', stderr='')
            raise AssertionError(f'Unexpected subprocess call: {cmd}')

        with self.wireless_context(wireless=wireless, hw_serial='PHONE123'), \
                patch.object(common.subprocess, 'run', side_effect=fake_subprocess_run):
            self.assertEqual(list(common.ADB), ['fixture-adb', '-s', wireless])
            self.assertEqual(common.shell('getprop', 'sys.boot_completed'), '1')
            self.assertEqual(list(common.ADB), ['fixture-adb', '-s', 'USB_PORT_A'])
            self.assertEqual(os.environ['STEPX_SERIAL'], 'USB_PORT_A')
            self.assertEqual(os.environ['STEPX_DEVICE_SERIAL'], 'PHONE123')
            # Subsequent guard() must keep the verified USB failover rather than reverting to wireless.
            common.guard()
            self.assertEqual(list(common.ADB), ['fixture-adb', '-s', 'USB_PORT_A'])

    def test_wireless_drop_rejects_unrelated_devices_with_different_ro_serialno(self):
        wireless = '192.0.2.10:42123'
        probed_serials = []

        def fake_subprocess_run(cmd, *args, **kwargs):
            if cmd == ['fixture-adb', 'devices']:
                return types.SimpleNamespace(
                    returncode=0,
                    stdout=(
                        'List of devices attached\n'
                        'PHONE123\tdevice\n'
                        'OTHER999\tdevice\n'
                    ),
                    stderr='',
                )
            if len(cmd) >= 3 and cmd[:2] == ['fixture-adb', '-s']:
                target = cmd[2]
                sub = cmd[3:]
                if target == wireless:
                    raise subprocess.CalledProcessError(
                        1, cmd, stderr=f"error: device '{wireless}' not found",
                    )
                if sub == ['shell', 'getprop', 'ro.serialno']:
                    probed_serials.append(target)
                    # Even a transport named PHONE123 must be rejected if hardware ro.serialno differs.
                    actual = 'SPOOFED999' if target == 'PHONE123' else 'OTHER999'
                    return types.SimpleNamespace(returncode=0, stdout=actual + '\n', stderr='')
            raise AssertionError(f'Foreign device must never receive workload commands: {cmd}')

        with self.wireless_context(wireless=wireless, hw_serial='PHONE123'), \
                patch.object(common.subprocess, 'run', side_effect=fake_subprocess_run):
            with self.assertRaises(subprocess.CalledProcessError):
                common.shell('getprop', 'sys.boot_completed')
            self.assertEqual(probed_serials, ['PHONE123', 'OTHER999'])
            self.assertEqual(list(common.ADB), ['fixture-adb', '-s', wireless])
            self.assertEqual(os.environ['STEPX_SERIAL'], wireless)
            with self.assertRaises(subprocess.CalledProcessError):
                common.guard()
            self.assertEqual(list(common.ADB), ['fixture-adb', '-s', wireless])

    def test_mutating_adb_command_switches_transport_for_caller_without_blind_replay(self):
        wireless = '192.0.2.10:42123'
        install_targets = []

        def fake_subprocess_run(cmd, *args, **kwargs):
            if cmd == ['fixture-adb', 'devices']:
                return types.SimpleNamespace(
                    returncode=0,
                    stdout='List of devices attached\nPHONE123\tdevice\n',
                    stderr='',
                )
            if cmd[:3] == ['fixture-adb', '-s', 'PHONE123'] and cmd[3:] == ['shell', 'getprop', 'ro.serialno']:
                return types.SimpleNamespace(returncode=0, stdout='PHONE123\n', stderr='')
            if len(cmd) >= 4 and cmd[:2] == ['fixture-adb', '-s'] and cmd[3] == 'install':
                install_targets.append(cmd[2])
                if cmd[2] == wireless:
                    raise subprocess.CalledProcessError(1, cmd, stderr='error: closed')
                return types.SimpleNamespace(returncode=0, stdout='Success\n', stderr='')
            raise AssertionError(f'Unexpected subprocess call: {cmd}')

        with self.wireless_context(wireless=wireless, hw_serial='PHONE123'), \
                patch.object(common.subprocess, 'run', side_effect=fake_subprocess_run):
            with self.assertRaises(subprocess.CalledProcessError):
                common.run(common.ADB + ['install', '--user', '0', '/tmp/base.apk'])
            self.assertEqual(install_targets, [wireless])
            self.assertEqual(list(common.ADB), ['fixture-adb', '-s', 'PHONE123'])


class StaleZygotePropHealingTests(unittest.TestCase):
    def test_stale_worker_prop_clears_via_shell_when_absent_or_zombie(self):
        for ps_extra in ('', '\n701 520 Z python3 [python3] /data/local/tmp/stepx-gms-w9a8b.py'):
            with self.subTest(ps_extra=ps_extra):
                props = {
                    'persist.openclaw.zygote.path': '/data/local/tmp/stepx-gms-w9a8b.py',
                    'sys.openclaw.env.start': '',
                }
                actions = []

                def fake_shell(*args):
                    if args[:1] == ('getprop',):
                        return props.get(args[1], '')
                    if args == ('ps', '-A', '-o', 'PID,PPID,S,NAME,ARGS'):
                        return 'PID PPID S NAME ARGS\n520 1 S openclaw_env /system/bin/openclaw_env' + ps_extra
                    if args[:2] == ('sh', '-c') and args[2].startswith('test -f '):
                        return '1'
                    if args == ('setprop', 'persist.openclaw.zygote.path', ''):
                        actions.append('setprop_clear')
                        props['persist.openclaw.zygote.path'] = ''
                        return ''
                    if args == ('sh', '-c', 'rm -f /data/local/tmp/stepx-gms-w*'):
                        actions.append('rm_worker_globs')
                        return ''
                    raise AssertionError(args)

                with patch.object(manage, 'shell', side_effect=fake_shell), \
                        patch.object(manage, 'root_json') as root_mock:
                    self.assertEqual(manage.assert_clean_props(), ('', ''))
                self.assertEqual(actions, ['setprop_clear', 'rm_worker_globs'])
                root_mock.assert_not_called()

    def test_stale_worker_prop_falls_back_to_root_property_service_when_shell_setprop_denied(self):
        props = {
            'persist.openclaw.zygote.path': '/data/local/tmp/stepx-gms-wdeadbeef.py',
            'sys.openclaw.env.start': '',
        }
        root_payloads = []

        def fake_shell(*args):
            if args[:1] == ('getprop',):
                return props.get(args[1], '')
            if args == ('ps', '-A', '-o', 'PID,PPID,S,NAME,ARGS'):
                return 'PID PPID S NAME ARGS\n612 1 S openclaw_env /system/bin/openclaw_env'
            if args[:2] == ('sh', '-c') and args[2].startswith('test -f '):
                return '0'
            if args == ('setprop', 'persist.openclaw.zygote.path', ''):
                raise RuntimeError('SELinux shell setprop denied')
            if args == ('sh', '-c', 'rm -f /data/local/tmp/stepx-gms-w*'):
                return ''
            raise AssertionError(args)

        def fake_root_json(code):
            root_payloads.append(code)
            props['persist.openclaw.zygote.path'] = ''
            return {'cleared': True}

        with patch.object(manage, 'shell', side_effect=fake_shell), \
                patch.object(manage, 'root_json', side_effect=fake_root_json):
            self.assertEqual(manage.assert_clean_props(), ('', ''))
        self.assertEqual(len(root_payloads), 1)
        self.assertIn("'/proc/612/root/dev/socket/property_service'", root_payloads[0])
        self.assertIn('/proc/612/root/data/local/tmp/stepx-gms-w*', root_payloads[0])

    def test_active_worker_and_foreign_zygote_prop_are_preserved(self):
        def active_shell(*args):
            if args == ('getprop', 'persist.openclaw.zygote.path'):
                return '/data/local/tmp/stepx-gms-wlive01.py'
            if args == ('getprop', 'sys.openclaw.env.start'):
                return ''
            if args == ('ps', '-A', '-o', 'PID,PPID,S,NAME,ARGS'):
                return (
                    'PID PPID S NAME ARGS\n'
                    '612 1 S openclaw_env /system/bin/openclaw_env\n'
                    '888 612 S python3 python3 /data/local/tmp/stepx-gms-wlive01.py'
                )
            if args[:2] == ('sh', '-c') and args[2].startswith('test -f '):
                return '1'
            raise AssertionError(f'Active worker must not be mutated: {args}')

        with patch.object(manage, 'shell', side_effect=active_shell), \
                patch.object(manage, 'root_json') as root_mock:
            with self.assertRaisesRegex(AssertionError, 'Another startup operation is active'):
                manage.assert_clean_props()
            root_mock.assert_not_called()

        with patch.object(manage, 'shell', side_effect=lambda *a: '/data/local/tmp/custom-hook.py' if a == ('getprop', 'persist.openclaw.zygote.path') else ''), \
                patch.object(manage, 'root_json') as root_mock:
            with self.assertRaises(AssertionError):
                manage.assert_clean_props()
            root_mock.assert_not_called()

    def test_run_root_worker_heals_stale_zygote_prop_around_worker_timeout(self):
        events = []
        with patch.object(common, 'heal_stale_zygote_prop', side_effect=lambda: events.append('heal')) as heal, \
                patch.object(root_bridge.subprocess, 'run', side_effect=lambda *a, **kw: (events.append('run'), (_ for _ in ()).throw(subprocess.TimeoutExpired(a[0], 130)))):
            with self.assertRaises(subprocess.TimeoutExpired):
                root_bridge.run_root_worker('/tmp/op.py', '/tmp/out.json', timeout=130)
        self.assertEqual(events, ['heal', 'run', 'heal'])
        self.assertEqual(heal.call_count, 2)


class CompatDynamicIdentityTests(unittest.TestCase):
    def _fake_companion_module(self, state_dir, serial, shell_fn, pull_fn, worker_fn):
        mod = types.ModuleNamespace() if hasattr(types, 'ModuleNamespace') else types.SimpleNamespace()
        mod.ADB = ['fixture-adb', '-s', serial]
        mod.STATE = state_dir
        mod.AGENTBUS_WORKER_TEMPLATE = (
            "import os,sys,json,socket\n"
            "ctx=b'u:r:system_app:s0'\n"
            "os.setresgid(1000,0,0);os.setresuid(1000,0,0);lib.setfsgid(1000);lib.setfsuid(1000)\n"
        )
        mod.guard = lambda native=False: None
        mod.shell = shell_fn
        mod.run = pull_fn

        @contextlib.contextmanager
        def runtime_lock():
            yield

        mod.runtime_lock = runtime_lock
        mod.run_bounded_android_worker = worker_fn
        return mod

    def test_dynamic_non_default_10xxx_uids_and_updated_split_apks_pass_cert_check_and_roundtrip(self):
        serial = 'PHONE999'
        gms_base = b'updated-gms-base-apk'
        gms_split = b'updated-gms-arm64-split-apk'
        play_base = b'updated-phonesky-base-apk'
        apk_blobs = {
            '/data/app/~~gms/base.apk': gms_base,
            '/data/app/~~gms/split_config.arm64_v8a.apk': gms_split,
            '/data/app/~~vending/base.apk': play_base,
        }
        apk_digests = {path: hashlib.sha256(raw).hexdigest() for path, raw in apk_blobs.items()}
        cert_by_digest = {
            apk_digests['/data/app/~~gms/base.apk']: compat.SCOPES['gms']['cert_sha256'],
            apk_digests['/data/app/~~gms/split_config.arm64_v8a.apk']: compat.SCOPES['gms']['cert_sha256'],
            apk_digests['/data/app/~~vending/base.apk']: compat.SCOPES['play']['cert_sha256'],
        }
        # Non-default 10xxx UIDs: 10241 (gms) and 10268 (play; offset 268 = 0x10c -> c12,c257).
        expected_gms_domain = 'u:r:gmscore_app:s0:c512,c768'
        expected_play_domain = 'u:r:untrusted_app:s0:c12,c257,c512,c768'
        active_state = {'gms': False, 'play': False}
        seen_templates = []
        apksigner_calls = []

        def shell_fn(*args):
            if args == ('getprop', 'ro.serialno'):
                return serial
            if args == ('ps', '-AZ'):
                return f'{expected_gms_domain} u0_a241 1200 900 com.google.android.gms:persistent'
            if args == ('dumpsys', 'package', 'com.google.android.gms'):
                return 'Packages:\n  Package [com.google.android.gms]\n    appId=10241\n    versionName=26.42.31\n    seInfo=gmscore\n'
            if args == ('dumpsys', 'package', 'com.android.vending'):
                return 'Packages:\n  Package [com.android.vending]\n    userId=10268\n    versionName=45.2.19\n    seInfo=default:targetSdkVersion=35:complete\n'
            if args == ('ls', '-Zd', '/data/user/0/com.android.vending'):
                raise RuntimeError('ls -Zd unavailable; compute MLS from dynamic 10xxx uid')
            if args == ('pm', 'path', '--user', '0', 'com.google.android.gms'):
                return 'package:/data/app/~~gms/base.apk\npackage:/data/app/~~gms/split_config.arm64_v8a.apk'
            if args == ('pm', 'path', '--user', '0', 'com.android.vending'):
                return 'package:/data/app/~~vending/base.apk'
            if args[:1] == ('sha256sum',):
                return f'{apk_digests[args[1]]}  {args[1]}'
            if args[:3] == ('am', 'force-stop', '--user'):
                return ''
            if args == ('pidof', 'system_server'):
                return '1500'
            raise AssertionError(args)

        def pull_fn(args, timeout=180):
            self.assertEqual(args[:4], ['fixture-adb', '-s', serial, 'pull'])
            Path(args[5]).write_bytes(apk_blobs[args[4]])
            return types.SimpleNamespace(returncode=0, stdout='')

        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            state_dir = temp_path / 'compat-state'
            state_dir.mkdir(mode=0o700)
            perm_state = temp_path / 'perm-state'
            perm_state.mkdir(mode=0o700)

            holder = {}

            def worker_fn(source, pid, worker_context='agentbus', preserve_verified_result=None):
                self.assertEqual(pid, 1500)
                self.assertEqual(worker_context, 'agentbus')
                tpl = holder['mod'].AGENTBUS_WORKER_TEMPLATE
                seen_templates.append(tpl)
                scope = 'gms' if "scope='gms'" in source else 'play'
                uid = 10241 if scope == 'gms' else 10268
                domain = expected_gms_domain if scope == 'gms' else expected_play_domain
                self.assertIn(f'os.setresuid({uid},0,0)', tpl)
                self.assertIn(repr(domain.encode()), tpl)
                if 'prepared=' in source:
                    return {
                        'uid': uid,
                        'euid': 0,
                        'context': domain + '\n',
                        'summary': {'cache_active': False, 'db_active': False},
                        'file_baseline': {'format': 1, 'scope': scope, 'entries': []},
                        'db_baseline': {'format': 1, 'scope': scope, 'override': None},
                    }
                if 'rollback_owned()' in source and 'rolled_back=True' in source:
                    active_state[scope] = False
                    return {
                        'uid': uid,
                        'euid': 0,
                        'context': domain + '\n',
                        'summary': {'rolled_back': True, 'cache_active': False, 'db_active': False},
                    }
                if 'out[\'cache_result\']=' in source:
                    active_state[scope] = True
                    return {
                        'uid': uid,
                        'euid': 0,
                        'context': domain + '\n',
                        'summary': {'cache_active': True, 'db_active': True},
                    }
                return {
                    'uid': uid,
                    'euid': 0,
                    'context': domain + '\n',
                    'summary': {'cache_active': active_state[scope], 'db_active': active_state[scope]},
                }

            fake_mod = self._fake_companion_module(perm_state, serial, shell_fn, pull_fn, worker_fn)
            holder['mod'] = fake_mod
            original_template = fake_mod.AGENTBUS_WORKER_TEMPLATE

            class FakeSpec:
                loader = types.SimpleNamespace(exec_module=lambda m: m.__dict__.update(fake_mod.__dict__))

            def fake_apksigner_run(cmd, **kwargs):
                apksigner_calls.append(cmd)
                pulled_digest = hashlib.sha256(Path(cmd[-1]).read_bytes()).hexdigest()
                cert = cert_by_digest[pulled_digest]
                return types.SimpleNamespace(
                    returncode=0,
                    stdout=f'Signer #1 certificate SHA-256 digest: {cert}\n',
                    stderr='',
                )

            with patch.dict(os.environ, {'STEPX_SERIAL': serial, 'STEPX_DEVICE_SERIAL': serial}, clear=False), \
                    patch.object(compat, 'SERIAL', serial), \
                    patch.object(compat.importlib.util, 'spec_from_file_location', return_value=FakeSpec()), \
                    patch.object(compat.importlib.util, 'module_from_spec', side_effect=lambda spec: holder.setdefault('loaded', fake_mod)), \
                    patch.object(compat.host_runtime, 'sdk_command', return_value=['apksigner-cross-platform']), \
                    patch.object(compat.host_runtime, 'java_environment', return_value={}), \
                    patch.object(compat.host_runtime, 'subprocess_options', return_value={}), \
                    patch.object(compat.subprocess, 'run', side_effect=fake_apksigner_run):
                transport = compat.Transport()
                applied = compat.execute('apply', state_dir, transport)
                self.assertTrue(applied['ok'])
                self.assertTrue(applied['active'])
                self.assertEqual(transport.scopes['gms']['uid'], 10241)
                self.assertEqual(transport.scopes['play']['uid'], 10268)
                self.assertEqual(len(apksigner_calls), 3)
                receipt = compat.current_receipt(state_dir)
                self.assertEqual(receipt['pins']['gms']['uid'], 10241)
                self.assertEqual(receipt['pins']['play']['uid'], 10268)

                status_res = compat.execute('status', state_dir, compat.Transport())
                self.assertTrue(status_res['active'])
                self.assertTrue(status_res['can_rollback'])

                # Rollback succeeds without re-running apksigner even on dynamic 10xxx UIDs.
                apksigner_calls.clear()
                rolled = compat.execute('rollback', state_dir, compat.Transport())
                self.assertTrue(rolled['ok'])
                self.assertFalse(rolled['active'])
                self.assertFalse(rolled['can_rollback'])
                self.assertEqual(apksigner_calls, [])
                self.assertEqual(fake_mod.AGENTBUS_WORKER_TEMPLATE, original_template)

    def test_compat_rejects_foreign_certificate_and_non_app_uids(self):
        serial = 'PHONE999'
        gms_bytes = b'foreign-signed-gms-apk'
        gms_sha = hashlib.sha256(gms_bytes).hexdigest()
        uid_box = {'gms': 10241}

        def shell_fn(*args):
            if args == ('getprop', 'ro.serialno'):
                return serial
            if args == ('ps', '-AZ'):
                return ''
            if args == ('dumpsys', 'package', 'com.google.android.gms'):
                return f'Packages:\n  Package [com.google.android.gms]\n    appId={uid_box["gms"]}\n'
            if args == ('ls', '-Zd', '/data/user/0/com.google.android.gms'):
                raise RuntimeError('ls unavailable')
            if args == ('pm', 'path', '--user', '0', 'com.google.android.gms'):
                return 'package:/data/app/~~gms/base.apk'
            if args == ('sha256sum', '/data/app/~~gms/base.apk'):
                return f'{gms_sha}  /data/app/~~gms/base.apk'
            raise AssertionError(args)

        def pull_fn(args, timeout=180):
            Path(args[5]).write_bytes(gms_bytes)
            return types.SimpleNamespace(returncode=0, stdout='')

        with tempfile.TemporaryDirectory() as temp:
            fake_mod = self._fake_companion_module(Path(temp), serial, shell_fn, pull_fn, lambda *a, **kw: {})

            class FakeSpec:
                loader = types.SimpleNamespace(exec_module=lambda m: None)

            with patch.dict(os.environ, {'STEPX_SERIAL': serial, 'STEPX_DEVICE_SERIAL': serial}, clear=False), \
                    patch.object(compat, 'SERIAL', serial), \
                    patch.object(compat.importlib.util, 'spec_from_file_location', return_value=FakeSpec()), \
                    patch.object(compat.importlib.util, 'module_from_spec', return_value=fake_mod), \
                    patch.object(compat.host_runtime, 'sdk_command', return_value=['apksigner-cross-platform']), \
                    patch.object(compat.host_runtime, 'java_environment', return_value={}), \
                    patch.object(compat.host_runtime, 'subprocess_options', return_value={}), \
                    patch.object(compat.subprocess, 'run', return_value=types.SimpleNamespace(
                        returncode=0,
                        stdout='Signer #1 certificate SHA-256 digest: ' + ('f' * 64) + '\n',
                        stderr='',
                    )):
                with self.assertRaises(compat.RepairError) as bad_cert:
                    compat.Transport().check('status')
                self.assertEqual(bad_cert.exception.code, 'unsupported_apk')

                for invalid_uid in (1000, 9999, 1010241):
                    uid_box['gms'] = invalid_uid
                    with self.assertRaises(compat.RepairError) as bad_uid:
                        compat.Transport().check('status')
                    self.assertEqual(bad_uid.exception.code, 'unsupported_package_identity')
                    self.assertFalse(compat._valid_receipt_pins({
                        'gms': {'package': 'com.google.android.gms', 'uid': invalid_uid, 'domain': 'u:r:gmscore_app:s0:c512,c768'},
                        'play': {'package': 'com.android.vending', 'uid': 10268, 'domain': 'u:r:untrusted_app:s0:c12,c257,c512,c768'},
                    }))


if __name__ == '__main__': unittest.main()
