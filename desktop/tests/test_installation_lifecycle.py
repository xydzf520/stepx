"""Version retention and interrupted GMS installation, without phone writes."""
import importlib.util,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
sys.path.insert(0,str(ROOT/'handoff/google-play-stepx/tools'))
from companion_apk import installation_choice,PACKAGE
import common
spec=importlib.util.spec_from_file_location('lifecycle_manage',ROOT/'handoff/google-play-stepx/tools/manage.py')
manage=importlib.util.module_from_spec(spec);spec.loader.exec_module(manage)

class InstallLifecycle(unittest.TestCase):
    def test_update_not_skipped_when_permissions_already_granted(self):
        self.assertEqual(installation_choice({'package':PACKAGE,'versionCode':36,'components':[]},{'package':PACKAGE,'versionCode':35,'components':[]})[0],'candidate')
    def test_downgrade_is_retained(self):
        self.assertEqual(installation_choice({'package':PACKAGE,'versionCode':34,'components':[]},{'package':PACKAGE,'versionCode':35,'components':[]})[0],'installed')
    def test_embedded_engine_is_never_removed_by_base_repair(self):
        with patch('companion_apk.has_embedded_engine',return_value=True),patch('companion_apk.embedded_runtime',return_value=False):
            self.assertEqual(installation_choice({'package':PACKAGE,'versionCode':99},{'package':PACKAGE,'versionCode':35})[0],'installed')
    def test_invalid_version_and_identity_are_rejected(self):
        for bad in ({'package':PACKAGE},{'package':'untrusted','versionCode':35}):
            with self.assertRaises(RuntimeError):installation_choice(bad,{'package':PACKAGE,'versionCode':35})
    def test_baseline_tracks_hardware_not_network_port(self):
        with patch.object(common,'DEVICE_KEY','phone1'),patch.object(common,'SERIAL','10.0.0.2:12345'):
            self.assertTrue(common.baseline_matches({'serial':'phone1','fingerprint':common.CONFIG['fingerprint']}))
            self.assertFalse(common.baseline_matches({'serial':'phone2','fingerprint':common.CONFIG['fingerprint']}))
    def fixture(self,p):
        (p/'baseline.json').write_text(json.dumps({'serial':'phone1','fingerprint':manage.CONFIG['fingerprint'],'packages':[]}))
        (p/'install-progress.json').write_text(json.dumps({'serial':'phone1','fingerprint':manage.CONFIG['fingerprint'],'completed':[]}))
    def test_only_owned_incomplete_transaction_can_resume(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(manage,'DEVICE_KEY','phone1'),patch.object(manage,'baseline_matches',side_effect=lambda b:b['serial']=='phone1'):
            p=Path(tmp)
            with patch.object(manage,'STATE',p):
                self.assertFalse(manage.resumable_install());self.fixture(p);self.assertTrue(manage.resumable_install())
                (p/'boot-install.json').write_text('{}');self.assertFalse(manage.resumable_install())
    def test_prepare_retry_keeps_original_backup(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);self.fixture(p);original=(p/'baseline.json').read_bytes()
            with patch.object(manage,'STATE',p),patch.object(manage,'preflight',return_value={'user_unlocked':True}),patch.object(manage,'resumable_install',return_value=True),patch.object(manage,'sha256',return_value=manage.CONFIG['stock_sha256']),patch.object(manage,'root_json') as root:
                manage.prepare();self.assertEqual((p/'baseline.json').read_bytes(),original);root.assert_not_called()
    def test_partial_install_skips_exact_existing_apks(self):
        existing=manage.PACKAGES[0];installed=[]
        def shell(*a):
            if a[:3]==('pm','list','packages'):return 'package:'+existing if a[-1]==existing else ''
            if a[:2]==('pm','path'):return 'package:/data/app/'+a[-1]+'/base.apk'
            if a[0]=='sha256sum':return next(v['sha256'] for v in manage.CONFIG['apks'] if v['package'] in a[1])+'  base.apk'
            return ''
        def run(a,**kw):
            installed.append(a);return type('Result',(),{'stdout':'Success'})()
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);self.fixture(p)
            with patch.object(manage,'STATE',p),patch.object(manage,'guard'),patch.object(manage,'baseline_matches',return_value=True),patch.object(manage,'resumable_install',return_value=True),patch.object(manage,'check_apks'),patch.object(manage,'shell',side_effect=shell),patch.object(manage,'run',side_effect=run),patch.object(manage,'save_json'):
                manage.install_apks()
            self.assertEqual(len(installed),2)
            self.assertTrue(all('-r' not in a for a in installed))
    def test_foreign_partial_apk_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);self.fixture(p)
            def shell(*a):
                if a[0]=='sha256sum':return 'unknownhash base.apk'
                if a[:2]==('pm','path'):return 'package:/data/app/pkg/base.apk'
                return 'package:present'
            with patch.object(manage,'STATE',p),patch.object(manage,'guard'),patch.object(manage,'baseline_matches',return_value=True),patch.object(manage,'resumable_install',return_value=True),patch.object(manage,'check_apks'),patch.object(manage,'shell',side_effect=shell),patch.object(manage,'run') as run:
                with self.assertRaisesRegex(AssertionError,'版本不同'):manage.install_apks()
                run.assert_not_called()
if __name__=='__main__':unittest.main()
