"""Recovery scenarios run against fake phones and local archives, never ADB."""
import contextlib
import hashlib
import io
import itertools
import json
from pathlib import Path
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch
import zipfile

from test_google_install_flow import InstallGateFixture, manage, install_recovery, WorkflowError
import common
import workflow_errors


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


if __name__ == '__main__': unittest.main()
