"""System activation proof and bounded gateway staging; no device access."""
import contextlib,importlib.util,json,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('companion_boot_test',Path(__file__).resolve().parents[2]/'scripts/companion-boot.py')
b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
class BootAdapter(unittest.TestCase):
    def test_staging_large_artifact_roundtrip_and_argv_bound(self):
        with tempfile.TemporaryDirectory() as root:
            data=bytes(range(256))*1000;target=Path(root)/'artifact';calls=[]
            def gateway(argv):
                self.assertEqual(argv[:2],['python3','-c']);self.assertLess(len(argv[2]),16000)
                calls.append(argv);exec(argv[2],{})
            with patch.object(b,'gateway_command',side_effect=gateway):b.stage_file(str(target),data)
            self.assertEqual(target.read_bytes(),data);self.assertGreater(len(calls),10)
    def test_existing_staging_file_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as root:
            target=Path(root)/'artifact';target.write_bytes(b'keep')
            with patch.object(b,'gateway_command',side_effect=lambda argv:exec(argv[2],{})):
                with self.assertRaises(FileExistsError):b.stage_file(str(target),b'replace')
            self.assertEqual(target.read_bytes(),b'keep')
    def test_wrong_identity_does_not_persist_system_payload(self):
        answer={'package':b.p.PACKAGE,'uid':99999,'signature_verified':True,'scope':'companion_boot_v1','all_granted':True,'boot_pairing_started':True}
        proof={'java_result':json.dumps(answer),'system_server_unchanged':True,'system_server_stable':True,'detached':True,'worker_exit_code':0}
        with patch.object(b.p,'inspect',return_value={'signature_verified':True,'uid':10215}),patch.object(b.p,'guard'),patch.object(b.p,'signer',return_value='a'*64),patch.object(b,'baseline_for',return_value={}),patch.object(b.p,'shell',return_value='123'),patch.object(b,'payload',return_value=b''),patch.object(b.p,'runtime_lock',return_value=contextlib.nullcontext()),patch.object(b.p,'run_bounded_android_worker',return_value=proof),patch.object(b,'private'),patch.object(b,'install') as persist:
            with self.assertRaisesRegex(RuntimeError,'readback'):b.activate(Path('companion.apk'))
            persist.assert_not_called()
    def test_failed_or_unmonitored_activation_cannot_claim_success(self):
        valid={'status':'success','operation':'grant','package':b.p.PACKAGE,'uid':10215,'signature_verified':True,'scope':'companion_boot_v1','all_granted':True,'boot_pairing_started':True,
               'companion_update':{'update_monitor':True,'package':b.p.PACKAGE,'uid':10215}}
        for changes in ({'status':'failed'},{'operation':'inspect'},{'companion_update':{}},
                        {'companion_update':{'update_monitor':True,'package':'another.app','uid':10215}},
                        {'companion_update':{'update_monitor':True,'package':b.p.PACKAGE,'uid':99999}}):
            answer={**valid,**changes}
            proof={'java_result':json.dumps(answer),'system_server_unchanged':True,'system_server_stable':True,'detached':True,'worker_exit_code':0}
            with self.subTest(changes=changes),patch.object(b.p,'inspect',return_value={'signature_verified':True,'uid':10215}),patch.object(b.p,'guard'),patch.object(b.p,'signer',return_value='a'*64),patch.object(b,'baseline_for',return_value={}),patch.object(b.p,'shell',return_value='123'),patch.object(b,'payload',return_value=b''),patch.object(b.p,'runtime_lock',return_value=contextlib.nullcontext()),patch.object(b.p,'run_bounded_android_worker',return_value=proof),patch.object(b,'private'),patch.object(b,'install') as persist:
                with self.assertRaisesRegex(RuntimeError,'readback'):b.activate(Path('companion.apk'))
                persist.assert_not_called()
    def test_manual_activation_is_ready_but_not_reboot_proof(self):
        initial={k:True for k in ('installed','signature_verified','granted','connection_permissions_granted','wake_permissions_granted')};initial['uid']=10215
        installed={'installed':True,'hook_exists':True,'config':{'signer':'a'*64,'uid':10215,'payload_installed_boot':'current-boot'},'last':{'boot_id':'current-boot','target_pid':123,'status':'success','source':'verified_manual_activation'}}
        with patch.object(b.p,'inspect',return_value=initial),patch.object(b.p,'signer',return_value='a'*64),patch.object(b.p,'root_json',return_value=installed),patch.object(b.p,'shell',side_effect=lambda *args:'123' if args[0]=='pidof' else 'current-boot'):
            state=b.status(Path('companion.apk'))
        self.assertTrue(state['current_boot_ready']);self.assertFalse(state['reboot_verified'])
    def test_missing_boot_hook_is_not_reported_as_reboot_ready(self):
        initial={k:True for k in ('installed','signature_verified','granted','connection_permissions_granted','wake_permissions_granted')};initial['uid']=10215
        with patch.object(b.p,'inspect',return_value=initial),patch.object(b.p,'signer',return_value='a'*64),patch.object(b.p,'root_json',return_value={'installed':False,'hook_exists':False}),patch.object(b.p,'shell',return_value='current-boot'):
            state=b.status(Path('companion.apk'))
        self.assertFalse(state['current_boot_ready']);self.assertFalse(state['reboot_verified']);self.assertTrue(state['current_session_ready'])
        self.assertEqual(state['status'],'needs_bootstrap')
    def test_changed_system_process_cannot_record_old_activation(self):
        with patch.object(b.p,'shell',return_value='456'),patch.object(b.p,'root_json') as writer:
            with self.assertRaisesRegex(RuntimeError,'changed'):b.record_activation({},123,{})
            writer.assert_not_called()
if __name__=='__main__':unittest.main()
