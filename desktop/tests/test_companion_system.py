"""The pairing daemon may be stopped/reused only with a matching installation identity."""
import importlib.util
import base64
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('companion_system',ROOT/'scripts/companion-system.py')
service=importlib.util.module_from_spec(spec);spec.loader.exec_module(service)


class PairingServiceGuards(unittest.TestCase):
    def setUp(self):
        self.identity={'service':'stepx-pairing-v1','package':service.permissions.PACKAGE,
            'serial':service.permissions.ADB[-1],'fingerprint':'fixed','boot_id':'boot',
            'signer':'a'*64,'companion_uid':10215,'dex_sha256':'b'*64}
        self.process={'pid':4321,'start_ticks':'123','uid':2000,'context':'u:r:shell:s0',
            'executable':service.APP_PROCESS,'argv':[service.MAIN]}
        probe=patch.object(service,'pairing_probe',return_value={'state':'unavailable','ready':False})
        probe.start();self.addCleanup(probe.stop)

    def test_unknown_existing_service_is_not_adopted_or_replaced(self):
        with tempfile.TemporaryDirectory() as directory,patch.object(service,'STATE',Path(directory)), \
                patch.object(service,'candidates',return_value=[4321]),patch.object(service,'process') as process:
            with self.assertRaisesRegex(RuntimeError,'without this installation record'):
                service.saved_service(self.identity)
            process.assert_not_called()

    def test_pid_reuse_never_triggers_stop_signal(self):
        prior={**self.identity,'process':self.process}
        changed={**self.process,'start_ticks':'124'}
        with patch.object(service,'preflight',return_value=self.identity), \
                patch.object(service,'saved_service',return_value=prior), \
                patch.object(service,'process',return_value=changed),patch.object(service.permissions,'shell') as shell:
            with self.assertRaisesRegex(RuntimeError,'changed before stop'):
                service.stop(Path('app.apk'))
            shell.assert_not_called()

    def test_new_artifact_requires_explicit_stop_of_verified_old_service(self):
        prior={**self.identity,'dex_sha256':'c'*64,'process':self.process}
        with patch.object(service,'preflight',return_value=self.identity), \
                patch.object(service,'saved_service',return_value=prior),patch.object(service.permissions,'shell') as shell:
            with self.assertRaisesRegex(RuntimeError,'Stop the verified old'):
                service.start(Path('app.apk'))
            shell.assert_not_called()

    def test_existing_current_service_start_is_read_only(self):
        prior={**self.identity,'process':self.process}
        with patch.object(service,'preflight',return_value=self.identity), \
                patch.object(service,'saved_service',return_value=prior),patch.object(service.permissions,'shell') as shell:
            result=service.start(Path('app.apk'))
            self.assertEqual(result['status'],'already_running');shell.assert_not_called()

    def test_exact_launch_arguments_are_required(self):
        valid={**self.process,'argv':[service.APP_PROCESS,'/system/bin',service.MAIN,'a'*64,'10215']}
        service.validate_launch_identity(valid,self.identity)
        for argv in ([service.MAIN,'--arbitrary'],[service.APP_PROCESS,'/system/bin',service.MAIN,'a'*64,'10216']):
            with self.assertRaisesRegex(RuntimeError,'fixed Java entry'):
                service.validate_launch_identity({**valid,'argv':argv},self.identity)

    def test_old_boot_record_does_not_authorize_touching_new_process(self):
        with tempfile.TemporaryDirectory() as directory,patch.object(service,'STATE',Path(directory)), \
                patch.object(service,'candidates',return_value=[4321]),patch.object(service,'process') as process:
            service.record_path().write_text(json.dumps({**self.identity,'boot_id':'old-boot','process':self.process}))
            with self.assertRaisesRegex(RuntimeError,'without this installation record'):
                service.saved_service(self.identity)
            process.assert_not_called()


class EffectivePairingStatus(unittest.TestCase):
    def setUp(self):
        self.identity={'service':'stepx-pairing-v1','package':service.permissions.PACKAGE,
            'companion_uid':10215,'boot_id':'test-boot','system_session':{'pid':222,'start_ticks':'42'},
            'scope':'current_device_session','reboot_verified':False,'native_injection':False,
            'google_launcher_changed':False}
        self.answer={'protocol':1,'ok':True,'service':'stepx-pairing-v1',
            'uid':10215,'processUid':1000,'pid':222,'supported':True}

    def read_probe(self, answer=None, *, session=None, boot='test-boot', raw=None):
        value=self.answer if answer is None else answer
        output=raw if raw is not None else 'Result: Bundle[{result='+base64.b64encode(json.dumps(value).encode()).decode()+'}]'
        def shell(*args):
            if args[0]=='content':return output
            if args==('cat','/proc/sys/kernel/random/boot_id'):return boot
            raise AssertionError('Unexpected device read: '+str(args))
        with patch.object(service,'system_session',side_effect=session if session is not None else None,
                          return_value=self.identity['system_session']),patch.object(service.permissions,'shell',side_effect=shell):
            return service.pairing_probe(self.identity)

    def test_system_status_uses_live_binder_and_current_system_process(self):
        probe=self.read_probe()
        self.assertTrue(probe['ready']);self.assertEqual(probe['source'],'system')
        with patch.object(service,'preflight',return_value=self.identity),patch.object(service,'saved_service',return_value=None),patch.object(service,'pairing_probe',return_value=probe):
            status=service.status(Path('app.apk'))
        self.assertEqual(status['status'],'running');self.assertTrue(status['running'])
        self.assertFalse(status['shell_running']);self.assertEqual(status['service_source'],'system')
        self.assertEqual(status['effective_scope'],'companion_boot_v1')
        self.assertEqual(status['scope'],'current_device_session');self.assertFalse(status['reboot_verified'])

    def test_system_ready_does_not_launch_or_overwrite_shell_artifacts(self):
        probe=self.read_probe()
        with patch.object(service,'preflight',return_value=self.identity),patch.object(service,'saved_service',return_value=None),patch.object(service,'pairing_probe',return_value=probe),patch.object(service.permissions,'shell') as shell,patch.object(service.permissions,'run') as run,patch.object(service,'save') as save:
            status=service.start(Path('app.apk'))
        self.assertEqual(status['status'],'already_running');self.assertTrue(status['running'])
        shell.assert_not_called();run.assert_not_called();save.assert_not_called()

    def test_stop_never_signals_system_server(self):
        probe=self.read_probe()
        with patch.object(service,'preflight',return_value=self.identity),patch.object(service,'saved_service',return_value=None),patch.object(service,'pairing_probe',return_value=probe),patch.object(service.permissions,'shell') as shell:
            status=service.stop(Path('app.apk'))
        self.assertFalse(status['running']);self.assertTrue(status['effective_running']);shell.assert_not_called()

    def test_stale_or_foreign_identity_cannot_claim_system_ready(self):
        for changed in ({'pid':221},{'uid':10216},{'processUid':0},{'ok':1},
                        {'service':'other'},{'protocol':2},{'uid':'10215'},
                        {'pid':222.0},{'processUid':1000.0}):
            with self.subTest(changed=changed):
                result=self.read_probe({**self.answer,**changed})
                self.assertEqual(result['state'],'unknown');self.assertFalse(result['ready'])
        for session in ([{'pid':223,'start_ticks':'42'}],
                        [self.identity['system_session'],{'pid':222,'start_ticks':'43'}]):
            result=self.read_probe(session=session);self.assertEqual(result['state'],'unknown')
        self.assertEqual(self.read_probe(boot='different-boot')['state'],'unknown')

    def test_legacy_apk_or_corrupt_reply_is_unknown_never_cached_ready(self):
        for raw in ('Error: unknown method','Result: Bundle[{result=notbase64}]','x'*8193):
            with self.subTest(raw=raw[:30]):self.assertEqual(self.read_probe(raw=raw)['state'],'unknown')
        unknown={'state':'unknown','ready':False}
        with patch.object(service,'preflight',return_value=self.identity),patch.object(service,'saved_service',return_value=None),patch.object(service,'pairing_probe',return_value=unknown),patch.object(service.permissions,'shell') as shell:
            self.assertEqual(service.status(Path('app.apk'))['status'],'unknown')
            with self.assertRaisesRegex(RuntimeError,'未启动重复服务'):service.start(Path('app.apk'))
        shell.assert_not_called()

    def test_signed_provider_reports_unavailable_without_claiming_ready(self):
        result=self.read_probe({**self.answer,'ok':False,'pid':0,'processUid':0})
        self.assertEqual(result,{'state':'unavailable','ready':False})
        result=self.read_probe({**self.answer,'supported':False})
        self.assertEqual(result['state'],'unsupported');self.assertFalse(result['ready'])

    def test_shell_binder_does_not_claim_system_provider(self):
        result=self.read_probe({**self.answer,'pid':4321,'processUid':2000})
        self.assertTrue(result['ready']);self.assertEqual(result['source'],'shell')


if __name__=='__main__':unittest.main()
