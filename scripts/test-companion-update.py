#!/usr/bin/env python3
"""Offline regression tests: a normal update must retain an installed GUI engine."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch
import companion_apk as apk

spec = importlib.util.spec_from_file_location('companion_permissions', Path(__file__).with_name('companion-permissions.py'))
permissions = importlib.util.module_from_spec(spec); spec.loader.exec_module(permissions)
plan_spec = importlib.util.spec_from_file_location('companion_install_plan', Path(__file__).with_name('companion-install-plan.py'))
install_plan = importlib.util.module_from_spec(plan_spec); plan_spec.loader.exec_module(install_plan)


def manifest(embedded=False):
    value = {'package':apk.PACKAGE,'components':[{'type':'activity','name':'.MainActivity'}]}
    if embedded:
        value['components'] += [
            {'type':'service','name':apk.NEO_NAMESPACE+'.ControlService','permission':'android.permission.BIND_ACCESSIBILITY_SERVICE'},
            {'type':'service','name':apk.NEO_NAMESPACE+'.AgentInputMethodService','permission':'android.permission.BIND_INPUT_METHOD'},
            {'type':'provider','name':apk.NEO_NAMESPACE+'.CompanionBridgeProvider','authorities':apk.AUTHORITY},
            {'type':'activity','name':'.NeoSettingsActivity'}]
    return value


class ReplacementFirmwareTests(unittest.TestCase):
    def device_patches(self, serial='NEW_PHONE', fingerprint='StepX/pandora/new-firmware', model='PZ261A', sdk='36'):
        values = {'ro.serialno': serial, 'ro.product.model': model,
                  'ro.product.device': 'pandora' if model == 'PZ261A' else 'other',
                  'ro.build.version.sdk': sdk, 'ro.build.fingerprint': fingerprint}
        def shell(*args):
            self.assertEqual(args[0], 'getprop')
            return values[args[1]]
        for mock in (patch.object(install_plan.p, 'ADB', ['adb', '-s', '192.0.2.8:43001']),
                     patch.dict(install_plan.os.environ, {'STEPX_DEVICE_SERIAL': 'NEW_PHONE'}),
                     patch.object(install_plan.p, 'run', return_value='device'),
                     patch.object(install_plan.p, 'shell', side_effect=shell)):
            mock.start(); self.addCleanup(mock.stop)

    def test_new_firmware_first_install_does_not_use_privileged_adapter(self):
        self.device_patches()
        with tempfile.TemporaryDirectory() as folder:
            candidate = Path(folder)/'candidate.apk'; candidate.write_bytes(b'signed fixture')
            trusted = Path(folder)/'trusted.apk'; trusted.write_bytes(b'signed fixture')
            selected = Path(folder)/'selected.apk'
            with patch.object(install_plan.p, 'guard', side_effect=AssertionError('must not adapt firmware')), \
                 patch.object(install_plan.p, 'root_json', side_effect=AssertionError('must not use root')), \
                 patch.object(install_plan, 'apk_identity', return_value=(apk.PACKAGE, 'signer')), \
                 patch.object(install_plan, 'apk_manifest', return_value=manifest()), \
                 patch.object(install_plan, 'package_paths', return_value=[]):
                result = install_plan.plan(candidate, trusted, selected)
            self.assertTrue(result['needs_install'])
            self.assertFalse(result['system_adaptation_supported'])
            self.assertEqual(selected.read_bytes(), candidate.read_bytes())

    def test_known_firmware_still_selects_guarded_system_adaptation(self):
        self.device_patches(fingerprint=install_plan.p.ALLOWED_FINGERPRINTS[0])
        self.assertTrue(install_plan.installation_device()['system_adaptation_supported'])

    def test_reused_wireless_address_for_another_phone_is_rejected(self):
        self.device_patches(serial='OLD_PHONE')
        with self.assertRaisesRegex(RuntimeError, '不一致'):
            install_plan.installation_device()

    def test_unknown_firmware_does_not_bypass_apk_signer(self):
        self.device_patches()
        with patch.object(install_plan, 'apk_identity', side_effect=[(apk.PACKAGE,'bad'),(apk.PACKAGE,'good')]), \
             patch.object(install_plan, 'package_paths') as paths:
            with self.assertRaisesRegex(RuntimeError, '签名不一致'):
                install_plan.plan(Path('candidate'), Path('trusted'), Path('selected'))
            paths.assert_not_called()

    def test_non_stepx_phone_is_rejected(self):
        self.device_patches(model='Other')
        with self.assertRaisesRegex(RuntimeError, '不是已支持'):
            install_plan.installation_device()

    def test_old_android_is_rejected(self):
        self.device_patches(sdk='32')
        with self.assertRaisesRegex(RuntimeError, 'Android 13'):
            install_plan.installation_device()


class PreservationTests(unittest.TestCase):
    def test_absent_android_package_is_a_first_install(self):
        with patch.object(install_plan.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', '')):
            self.assertEqual(install_plan.package_paths(), [])
    def test_transport_failure_is_not_treated_as_missing_package(self):
        with patch.object(install_plan.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'device offline')):
            with self.assertRaisesRegex(RuntimeError, '无法核对'):
                install_plan.package_paths()
    def test_generic_package_reader_does_not_apply_companion_size_limit(self):
        with tempfile.TemporaryDirectory() as temp:
            candidate = Path(temp) / 'large-app.apk'
            with candidate.open('wb') as stream: stream.truncate(201 * 1024 * 1024)
            with patch.object(apk, 'ROOT', Path(temp)), patch.object(apk, 'sdk_command', return_value=['aapt']), patch.object(apk, 'run', return_value="package: name='com.example.largegame' versionCode='1'"):
                self.assertEqual(apk.apk_package(candidate), 'com.example.largegame')
    def test_generic_package_reader_fails_when_identity_is_unknown(self):
        with tempfile.TemporaryDirectory() as temp:
            candidate = Path(temp) / 'broken.apk'; candidate.write_bytes(b'broken')
            with patch.object(apk, 'ROOT', Path(temp)), patch.object(apk, 'sdk_command', return_value=['aapt']), patch.object(apk, 'run', return_value=''):
                with self.assertRaisesRegex(RuntimeError, '无法核对 APK 包名'):
                    apk.apk_package(candidate)
    def test_first_install_requires_compiled_anchor(self):
        with patch.object(permissions,'signer',return_value='a'*64):
            with self.assertRaisesRegex(RuntimeError,'缺少客户端内置签名基线'):
                permissions.update_signer(Path('resource.apk'),False)
    def test_first_install_rejects_resource_with_other_signer(self):
        with patch.object(permissions,'signer',side_effect=['b'*64,'a'*64]):
            with self.assertRaisesRegex(RuntimeError,'内置签名基线不一致'):
                permissions.update_signer(Path('resource.apk'),False,Path('compiled.apk'))
    def test_first_install_allows_same_signer(self):
        with patch.object(permissions,'signer',return_value='a'*64):
            self.assertEqual(permissions.update_signer(Path('resource.apk'),False,Path('compiled.apk')),'a'*64)
    def test_existing_install_still_checks_supplied_anchor(self):
        with patch.object(permissions,'signer',side_effect=['b'*64,'a'*64]):
            with self.assertRaisesRegex(RuntimeError,'内置签名基线不一致'):
                permissions.update_signer(Path('resource.apk'),True,Path('compiled.apk'))
    def test_normal_to_normal_allowed(self):
        self.assertFalse(apk.validate_update_manifests(manifest(),manifest())['candidate_embedded_gui'])
    def test_normal_to_embedded_allowed(self):
        self.assertTrue(apk.validate_update_manifests(manifest(True),manifest())['candidate_embedded_gui'])
    def test_embedded_to_embedded_allowed(self):
        self.assertTrue(apk.validate_update_manifests(manifest(True),manifest(True))['installed_embedded_gui'])
    def test_normal_update_cannot_remove_embedded(self):
        with self.assertRaisesRegex(RuntimeError,'已阻止覆盖'):
            apk.validate_update_manifests(manifest(),manifest(True))
    def test_disabled_or_partial_old_engine_still_protected(self):
        old = manifest(True);old['components']=old['components'][1:2]
        old['components'][0]['enabled']=False
        with self.assertRaisesRegex(RuntimeError,'已阻止覆盖'):
            apk.validate_update_manifests(manifest(),old)
    def test_partial_new_engine_is_not_a_complete_update(self):
        new = manifest(True);new['components'].pop()
        with self.assertRaisesRegex(RuntimeError,'已阻止覆盖'):
            apk.validate_update_manifests(new,manifest(True))
    def test_package_mismatch_rejected(self):
        new=manifest(True);new['package']='other.app'
        with self.assertRaisesRegex(RuntimeError,'包名不符'):
            apk.validate_update_manifests(new,manifest(True))
    def test_file_check_reads_both_manifests(self):
        with patch.object(apk,'apk_manifest',side_effect=[manifest(True),manifest(True)]) as read:
            self.assertTrue(apk.ensure_update_preserves_runtime(Path('candidate.apk'),Path('installed.apk'))['candidate_embedded_gui'])
        self.assertEqual(read.call_count,2)
    def test_preflight_blocks_before_native_check_or_install_gate_write(self):
        # Stub only read-only transport calls; any unexpected shell call fails the test.
        def shell(*args):
            if args==('dumpsys','package',apk.PACKAGE):return '  Package ['+apk.PACKAGE+'] (x):\n    userId=10215\n'
            if args==('pm','path',apk.PACKAGE):return 'package:/data/app/test/base.apk'
            raise AssertionError('Unexpected device command '+repr(args))
        def metadata(path):return manifest(path.name=='installed.apk')
        with tempfile.TemporaryDirectory() as temp:
            candidate=Path(temp)/'candidate.apk';candidate.write_bytes(b'candidate')
            with patch.object(permissions,'AUTHORIZED_SERIAL',permissions.ADB[-1]), patch.object(permissions,'guard') as guard, patch.object(permissions,'shell',side_effect=shell), patch.object(permissions,'signer',return_value='a'*64), patch.object(permissions,'run',return_value=''), patch.object(permissions,'root_json') as root, patch.object(apk,'apk_manifest',side_effect=metadata):
                with self.assertRaisesRegex(RuntimeError,'已阻止覆盖'):permissions.preflight(candidate)
                with self.assertRaisesRegex(RuntimeError,'已阻止覆盖'):permissions.prepare_install(candidate)
                self.assertEqual(guard.call_count,2)
                self.assertTrue(all(not call.kwargs for call in guard.call_args_list))
                root.assert_not_called()
    def test_permission_inspection_is_read_only_without_update_policy(self):
        def shell(*args):
            if args==('dumpsys','package',apk.PACKAGE):return 'Package ['+apk.PACKAGE+'] (x):\n  userId=10215\n'
            if args==('pm','path',apk.PACKAGE):return 'package:/data/app/test/base.apk'
            raise AssertionError(args)
        with patch.object(permissions,'AUTHORIZED_SERIAL',permissions.ADB[-1]),patch.object(permissions,'guard'),patch.object(permissions,'shell',side_effect=shell),patch.object(permissions,'signer',return_value='a'*64),patch.object(permissions,'run',return_value=''),patch.object(permissions,'ensure_update_preserves_runtime') as preservation:
            result=permissions.inspect(Path('ordinary.apk'))
            self.assertTrue(result['signature_verified']);preservation.assert_not_called()

if __name__=='__main__':unittest.main()
