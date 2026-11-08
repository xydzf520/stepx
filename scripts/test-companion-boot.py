"""Exercise real boot-adapter filesystem transactions in an isolated temporary root."""
import ast,base64,contextlib,hashlib,io,json,tempfile,unittest,fcntl,types,importlib.util
from pathlib import Path
SOURCE=Path(__file__).with_name('companion-boot.py').read_text()
TREE=ast.parse(SOURCE)
def hashof(data):return hashlib.sha256(data).hexdigest()
def install_code():
    function=next(x for x in TREE.body if isinstance(x,ast.FunctionDef) and x.name=='install')
    return next(x.value.value for x in function.body if isinstance(x,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='code' for t in x.targets))
def remove_code():
    function=next(x for x in TREE.body if isinstance(x,ast.FunctionDef) and x.name=='remove')
    return next(x.value.args[0].value for x in function.body if isinstance(x,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='proof' for t in x.targets))
class BootFilesystemTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.base=self.root/'usr/local/lib/stepx-companion';self.hook=self.root/'usr/local/lib/stepx-gms/boot.py'
        self.hook.parent.mkdir(parents=True);self.original=b'# existing factory adapter\n';self.hook.write_bytes(self.original)
        self.chain=b'# companion chain\n'
        self.config={'version':2,'package':'com.stepx.companion.app','serial':'test-device','fingerprint':'reviewed-fw','signer':'a'*64,'uid':10215,'installed_boot':'old','payload_installed_boot':'old','chain_sha256':hashof(self.chain),'grant_sha256':hashof(b'# grant'),'boot_sha256':hashof(b'# boot'),'launcher_sha256':hashof(b'# launcher'),'original_sha256':hashof(self.original),'baseline':{'permission':7}}
        self.data={name:base64.b64encode(data).decode() for name,data in {'chain.py':self.chain,'grant.py':b'# grant','boot.py':b'# boot','config.json':json.dumps(self.config).encode(),'launcher.py':b'# launcher'}.items()}
    def tearDown(self):self.temp.cleanup()
    def run_code(self,code,**extra):
        code=code.replace("'/usr/local/bin/",repr(str(self.root/'usr/local/bin')+'/')[:-1]).replace("'/usr/local/lib/",repr(str(self.root/'usr/local/lib')+'/')[:-1]).replace("'/var/lib/",repr(str(self.root/'var/lib')+'/')[:-1])
        output=io.StringIO()
        with contextlib.redirect_stdout(output):exec(code,{'ORIGINAL':hashof(self.original),'CONFIG':dict(self.config),'DATA':dict(self.data),'ALLOW_REBIND':False,'ALLOWED_FINGERPRINTS':('reviewed-fw',),**extra})
        return json.loads(output.getvalue())
    def install(self,rebind=False):return self.run_code(install_code(),ALLOW_REBIND=rebind)
    def test_install_keeps_exact_backup_and_can_restore(self):
        self.install();self.assertEqual((self.base/'previous-boot.py').read_bytes(),self.original)
        self.assertEqual(self.hook.read_bytes(),self.chain)
        self.run_code(remove_code());self.assertEqual(self.hook.read_bytes(),self.original)
    def test_foreign_hook_is_never_overwritten(self):
        self.install();self.hook.write_bytes(b'# another administrator')
        with self.assertRaisesRegex(RuntimeError,'Unrecognized boot hook'):self.install()
        self.assertEqual(self.hook.read_bytes(),b'# another administrator')
    def test_changed_backup_blocks_restore(self):
        self.install();(self.base/'previous-boot.py').write_bytes(b'# modified')
        with self.assertRaisesRegex(RuntimeError,'refusing to overwrite'):self.run_code(remove_code())
        self.assertEqual(self.hook.read_bytes(),self.chain)
    def test_companion_identity_change_blocks_upgrade(self):
        self.install();self.config['uid']=10216
        before=(self.base/'config.json').read_bytes()
        with self.assertRaisesRegex(RuntimeError,'Boot identity changed'):self.install()
        self.assertEqual((self.base/'config.json').read_bytes(),before)
    def test_reinstall_preserves_first_install_boot_and_original(self):
        self.install();self.config['installed_boot']='new';self.install()
        self.assertEqual(json.loads((self.base/'config.json').read_text())['installed_boot'],'old')
        self.assertEqual((self.base/'previous-boot.py').read_bytes(),self.original)

    def test_same_identity_metadata_refresh_preserves_receipt_and_baseline(self):
        self.install();state=self.root/'var/lib/stepx-companion';receipt=b'{"status":"success","boot_id":"old"}'
        (state/'last.json').write_bytes(receipt);self.config['fingerprint']='actual-reviewed-12';self.install()
        current=json.loads((self.base/'config.json').read_text())
        self.assertEqual(current['fingerprint'],'actual-reviewed-12');self.assertEqual(current['baseline'],{'permission':7})
        self.assertEqual(current['payload_installed_boot'],'old');self.assertEqual((state/'last.json').read_bytes(),receipt)

    def new_identity(self):
        self.config.update(signer='b'*64,uid=10216,installed_boot='new',payload_installed_boot='new',baseline={'permission':19})
        grant=b'# new identity grant';self.data['grant.py']=base64.b64encode(grant).decode();self.config['grant_sha256']=hashof(grant)

    def test_explicit_rebind_archives_exact_owned_state_and_keeps_new_baseline(self):
        self.install();state=self.root/'var/lib/stepx-companion'
        previous={'status':'success','unix_time':123,'boot_id':'old'};(state/'last.json').write_text(json.dumps(previous))
        old={name:(self.base/name).read_bytes() for name in ('config.json','grant.py','boot.py','chain.py','launcher.py')}
        self.new_identity();self.install(rebind=True)
        config=json.loads((self.base/'config.json').read_text());self.assertEqual(config['uid'],10216);self.assertEqual(config['signer'],'b'*64)
        self.assertEqual(config['baseline'],{'permission':19});self.assertEqual(config['installed_boot'],'new');self.assertEqual(config['payload_installed_boot'],'new')
        recovery=Path(config['rebind_from']['archive']);manifest=json.loads((recovery/'archive.json').read_text())
        for name,data in old.items():self.assertEqual((recovery/name).read_bytes(),data);self.assertEqual(manifest['files'][name],hashof(data))
        self.assertEqual(json.loads((recovery/'last.json').read_text()),previous);self.assertFalse((state/'last.json').exists())
        self.assertEqual((recovery/'original-boot.py').read_bytes(),self.original);self.assertEqual((recovery/'active-hook.py').read_bytes(),self.chain)
        self.assertEqual((self.base/'previous-boot.py').read_bytes(),self.original);self.assertEqual(recovery.stat().st_mode&0o777,0o700)
        self.install(rebind=True);self.assertEqual(len(list(state.glob('rebind-*'))),1)

    def test_explicit_rebind_still_rejects_foreign_hook_or_damaged_backup(self):
        self.install();self.new_identity();old=(self.base/'config.json').read_bytes();self.hook.write_bytes(b'# foreign hook')
        with self.assertRaisesRegex(RuntimeError,'Unrecognized boot hook'):self.install(rebind=True)
        self.assertEqual((self.base/'config.json').read_bytes(),old)
        self.hook.write_bytes(self.chain);(self.base/'previous-boot.py').write_bytes(b'# wrong backup')
        with self.assertRaisesRegex(RuntimeError,'Unrecognized boot hook'):self.install(rebind=True)
        self.assertEqual((self.base/'config.json').read_bytes(),old)

    def test_rebind_rejects_different_package_device_or_unreviewed_firmware(self):
        self.install();old=json.loads((self.base/'config.json').read_text());self.new_identity()
        for field,value,error in [('package','com.other.app','package'),('serial','another-device','another device'),('fingerprint','unknown-fw','firmware')]:
            damaged={**old,field:value};(self.base/'config.json').write_text(json.dumps(damaged))
            with self.assertRaisesRegex(RuntimeError,error):self.install(rebind=True)
            self.assertEqual(json.loads((self.base/'config.json').read_text()),damaged)

    def test_rebind_checks_old_payload_and_never_follows_payload_symlinks(self):
        self.install();self.new_identity();old=(self.base/'config.json').read_bytes();grant=self.base/'grant.py'
        grant.write_bytes(b'# changed old grant')
        with self.assertRaisesRegex(RuntimeError,'payload mismatch'):self.install(rebind=True)
        self.assertEqual((self.base/'config.json').read_bytes(),old)
        grant.unlink();elsewhere=self.root/'foreign';elsewhere.write_bytes(b'# grant');grant.symlink_to(elsewhere)
        with self.assertRaises(OSError):self.install(rebind=True)
        self.assertEqual((self.base/'config.json').read_bytes(),old);self.assertEqual(elsewhere.read_bytes(),b'# grant')

    def test_archive_failure_prevents_replacement_of_active_payload(self):
        from unittest.mock import patch
        import os
        self.install();old=(self.base/'config.json').read_bytes();old_grant=(self.base/'grant.py').read_bytes();self.new_identity();replace=os.replace
        def fail_archive(source,target):
            if str(target).endswith('/archive.json'):raise OSError('injected archive fsync/rename failure')
            return replace(source,target)
        with patch('os.replace',side_effect=fail_archive):
            with self.assertRaisesRegex(OSError,'injected archive'):self.install(rebind=True)
        self.assertEqual((self.base/'config.json').read_bytes(),old);self.assertEqual((self.base/'grant.py').read_bytes(),old_grant);self.assertEqual(self.hook.read_bytes(),self.chain)

    def test_running_boot_restore_blocks_rebinding(self):
        self.install();self.new_identity();old=(self.base/'config.json').read_bytes()
        with (self.root/'var/lib/stepx-companion/lock').open('r+b') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            with self.assertRaisesRegex(RuntimeError,'restoration is active'):self.install(rebind=True)
        self.assertEqual((self.base/'config.json').read_bytes(),old)

    def test_legacy_launcher_without_saved_digest_must_match_reviewed_bytes(self):
        self.install();old=json.loads((self.base/'config.json').read_text());old.pop('launcher_sha256')
        (self.base/'config.json').write_text(json.dumps(old));self.new_identity();launcher=self.base/'launcher.py'
        launcher.write_bytes(b'# unknown executable')
        with self.assertRaisesRegex(RuntimeError,'payload mismatch: launcher'):self.install(rebind=True)
        self.assertEqual(json.loads((self.base/'config.json').read_text()),old)
        launcher.write_bytes(b'# launcher');self.install(rebind=True)
        self.assertEqual(json.loads((self.base/'config.json').read_text())['uid'],10216)

    def assert_active_write_failure_restores_complete_state(self,failed_name):
        from unittest.mock import patch
        import os
        self.install();state=self.root/'var/lib/stepx-companion';(state/'last.json').write_text(json.dumps({'status':'success','unix_time':123}))
        active=[self.base/name for name in ('grant.py','boot.py','config.json','chain.py','launcher.py')]+[self.hook,state/'last.json']
        before={path:(path.read_bytes(),path.stat().st_mode&0o777) for path in active}
        self.new_identity();replace=os.replace;failed=False
        def fail_active(source,target):
            nonlocal failed
            if str(target)==str(self.base/failed_name) and not failed:failed=True;raise OSError('injected active replacement failure')
            return replace(source,target)
        with patch('os.replace',side_effect=fail_active):
            result=self.install(rebind=True)
        self.assertEqual(result['status'],'install_failed');self.assertTrue(result['rolled_back']);self.assertFalse(result['identity_rebound'])
        self.assertEqual({path:(path.read_bytes(),path.stat().st_mode&0o777) for path in active},before)
        recovery=Path(result['recovery_path']);self.assertTrue((recovery/'archive.json').is_file());self.assertEqual((recovery/'config.json').read_bytes(),before[self.base/'config.json'][0])
        self.assertFalse(list(self.base.glob('*.new-*')));self.assertEqual(self.install(rebind=True)['status'],'installed')
    def test_first_active_write_failure_restores_complete_state(self):self.assert_active_write_failure_restores_complete_state('grant.py')
    def test_second_active_write_failure_restores_complete_state(self):self.assert_active_write_failure_restores_complete_state('boot.py')
    def test_third_active_write_failure_restores_complete_state(self):self.assert_active_write_failure_restores_complete_state('config.json')
    def test_failed_rollback_reports_recovery_and_never_keeps_ready_receipt(self):
        from unittest.mock import patch
        import os
        self.install();state=self.root/'var/lib/stepx-companion';(state/'last.json').write_text(json.dumps({'status':'success','unix_time':123}))
        before=(self.base/'grant.py').read_bytes();self.new_identity();replace=os.replace;failed=False
        def fail_write_and_rollback(source,target):
            nonlocal failed
            if str(target)==str(self.base/'boot.py') and not failed:failed=True;raise OSError('injected second write failure')
            if str(target)==str(self.base/'grant.py') and failed:raise OSError('injected rollback failure')
            return replace(source,target)
        with patch('os.replace',side_effect=fail_write_and_rollback):result=self.install(rebind=True)
        self.assertEqual(result['status'],'install_failed');self.assertFalse(result['rolled_back']);self.assertTrue(result['rollback_errors'])
        self.assertFalse((state/'last.json').exists());self.assertEqual((Path(result['recovery_path'])/'grant.py').read_bytes(),before)
        self.assertEqual(json.loads((state/'installation-failure.json').read_text()),result)
    def test_symlink_hook_target_is_not_written(self):
        self.hook.unlink();other=self.root/'other';other.write_bytes(self.original);self.hook.symlink_to(other)
        with self.assertRaisesRegex(RuntimeError,'symlink'):self.install()
        self.assertEqual(other.read_bytes(),self.original)

    def test_standalone_without_google_installs_and_restores_executable(self):
        self.hook.unlink()
        self.hook=self.root/'usr/local/bin/ubuntu_zygote'
        self.hook.parent.mkdir(parents=True);self.hook.write_bytes(self.original);self.hook.chmod(0o755)
        launcher=b'# launcher'
        self.config.update(boot_mode='standalone',hook_path=str(self.hook),hook_sha256=hashof(launcher))
        self.install()
        self.assertEqual(self.hook.read_bytes(),launcher)
        self.assertEqual(self.hook.stat().st_mode & 0o777,0o755)
        self.assertEqual(Path(str(self.hook)+'.stepx-stock-20261021').read_bytes(),self.original)
        self.install()  # idempotent upgrade on a Google-free installation
        self.run_code(remove_code());self.assertEqual(self.hook.read_bytes(),self.original)
        self.assertEqual(self.hook.stat().st_mode & 0o777,0o755)

    def test_current_permissions_must_survive_package_update(self):
        function=next(x for x in TREE.body if isinstance(x,ast.FunctionDef) and x.name=='current_permissions_ready')
        scope={};exec(compile(ast.Module(body=[function],type_ignores=[]),'current-permissions','exec'),scope)
        ready=scope['current_permissions_ready']
        original={name:True for name in ('installed','signature_verified','granted','connection_permissions_granted','wake_permissions_granted')}
        self.assertTrue(ready(original))
        for key in original:
            for value in (False,None,'true'):
                self.assertFalse(ready({**original,key:value}),key)
            self.assertFalse(ready({k:v for k,v in original.items() if k!=key}),key)
class RebindBaselineTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);self.paths=[self.root/name for name in ('backscreen','connection','wake')]
        def validate(value,cert,uid):
            if value.get('signer')!=cert or value.get('uid')!=uid:raise RuntimeError('identity mismatch')
        self.adapter=types.SimpleNamespace(PERMISSION='back',CONNECTION_PERMISSIONS=('connection',),WAKE_PERMISSIONS=('wake',),
            baseline_path=lambda *_:self.paths[0],connection_baseline_path=lambda *_:self.paths[1],wake_baseline_path=lambda *_:self.paths[2],
            validate_baseline=validate,device_key=lambda:'test-device')
        function=next(x for x in TREE.body if isinstance(x,ast.FunctionDef) and x.name=='baseline_for')
        scope={'p':self.adapter,'json':json,'GROUP':('back','connection','wake'),'MissingPermissionBaselines':RuntimeError,'validate_baseline_evidence':lambda *_:None};exec(compile(ast.Module(body=[function],type_ignores=[]),'baseline','exec'),scope);self.baseline=scope['baseline_for']
        for i,path in enumerate(self.paths):path.write_text(json.dumps({'signer':'new','uid':10216,'serial':'test-device','before_flags':7 if i==0 else {('connection','wake')[i-1]:i+7}}))
    def tearDown(self):self.temp.cleanup()
    def test_rebind_requires_every_new_identity_baseline(self):
        self.assertEqual(self.baseline('new',10216,require_exact=True),{'back':7,'connection':8,'wake':9})
        self.paths[2].unlink()
        with self.assertRaisesRegex(RuntimeError,'exact saved'):self.baseline('new',10216,require_exact=True)
    def test_rebind_refuses_old_identity_or_another_device(self):
        value=json.loads(self.paths[0].read_text());value.update(signer='old',uid=10215);self.paths[0].write_text(json.dumps(value))
        with self.assertRaisesRegex(RuntimeError,'identity'):self.baseline('new',10216,require_exact=True)
        value.update(signer='new',uid=10216,serial='another-device');self.paths[0].write_text(json.dumps(value))
        with self.assertRaisesRegex(RuntimeError,'another device'):self.baseline('new',10216,require_exact=True)

class BaselineCaptureTests(unittest.TestCase):
    def setUp(self):
        from unittest.mock import patch
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);self.stack=contextlib.ExitStack()
        spec=importlib.util.spec_from_file_location('boot_capture_under_test',Path(__file__).with_name('companion-boot.py'))
        self.b=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.b);b=self.b
        self.cert='b'*64;self.uid=10218;self.events=[];self.bad_result=None;self.flags=4
        self.actual_fingerprint='StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys'
        self.stack.enter_context(patch.object(b.p,'STATE',self.root/'permissions'))
        self.stack.enter_context(patch.object(b,'STATE_LOCAL',self.root/'boot'))
        self.stack.enter_context(patch.object(b.p,'inspect',return_value={'installed':True,'signature_verified':True,'uid':self.uid}))
        self.stack.enter_context(patch.object(b.p,'guard'))
        self.stack.enter_context(patch.object(b.p,'signer',return_value=self.cert))
        self.stack.enter_context(patch.object(b.p,'device_key',return_value='test-device'))
        self.stack.enter_context(patch.object(b.p,'shell',side_effect=lambda *args:self.actual_fingerprint if args==('getprop','ro.build.fingerprint') else '3171' if args[0]=='pidof' else 'test-boot'))
        self.stack.enter_context(patch.object(b.p,'runtime_lock',side_effect=lambda:contextlib.nullcontext()))
        self.stack.enter_context(patch.object(b,'payload',side_effect=lambda cert,uid,baseline,operation='grant':json.dumps({'operation':operation,'baseline':baseline}).encode()))
        self.worker=self.stack.enter_context(patch.object(b.p,'run_bounded_android_worker',side_effect=self.run_worker))
        self.stack.enter_context(patch.object(b.p,'private_json',side_effect=self.persist))
    def tearDown(self):self.stack.close();self.temp.cleanup()
    def paths(self):return [self.b.p.baseline_path(self.cert,self.uid),self.b.p.connection_baseline_path(self.cert,self.uid),self.b.p.wake_baseline_path(self.cert,self.uid)]
    def persist(self,path,value):
        path=Path(path);self.events.append('persist:'+path.name);path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(value))
    def run_worker(self,code,target):
        operation='inspect' if '"operation": "inspect"' in code else 'grant';self.events.append(operation)
        if operation=='grant':
            self.assertTrue(all(path.is_file() for path in self.paths()),'grant began before every baseline was durable')
        b=self.b
        result={'status':'success','operation':operation,'scope':'companion_boot_v1','package':b.p.PACKAGE,'uid':self.uid,'signature_verified':True,'all_granted':True,
            'permissions':{name:{'before_flags':self.flags,'after_flags':self.flags,'declared':True,'signature_permission':True,'owner':b.p.WAKE_OWNER if name in b.p.WAKE_PERMISSIONS else 'android'} for name in b.GROUP}}
        if operation=='grant':
            result['boot_pairing_started']=True
            result['companion_update']={'update_monitor':True,'package':b.p.PACKAGE,'uid':self.uid}
        if self.bad_result:self.bad_result(result)
        return {'worker_exit_code':0,'system_server_unchanged':True,'system_server_stable':True,'detached':True,'java_result':json.dumps(result)}
    def test_rebind_snapshot_preserves_already_granted_flags_and_never_inherits_old_uid(self):
        b=self.b;old=b.p.baseline_path('a'*64,10215);self.persist(old,{'signer':'a'*64,'uid':10215,'before_flags':0})
        before=old.read_bytes();result=b.capture_baselines(Path('test.apk'))
        self.assertEqual(result['origin'],'before_rebind_snapshot');self.assertFalse(result['permission_changes']);self.assertEqual(old.read_bytes(),before)
        values=[json.loads(path.read_text()) for path in self.paths()]
        self.assertEqual(values[0]['before_flags'],4);self.assertTrue(all(value['uid']==self.uid and value['origin']=='before_rebind_snapshot' for value in values))
        self.assertTrue(all(flags==4 for flags in b.baseline_for(self.cert,self.uid).values()));self.assertEqual(self.events.count('inspect'),1);self.assertNotIn('grant',self.events)
        self.assertIn('not a claim about permissions before the first installation',result['restoration_boundary'])
    def test_first_activation_captures_every_group_before_any_grant(self):
        from unittest.mock import patch
        with patch.object(self.b,'install',return_value={'status':'installed','skipped_hook':True}):self.b.activate(Path('test.apk'))
        self.assertEqual([event for event in self.events if event in ('inspect','grant')],['inspect','grant'])
        self.assertTrue(all(json.loads(path.read_text())['origin']=='before_activation' for path in self.paths()))
    def test_google_monitor_failure_is_visible_in_activation_result(self):
        from unittest.mock import patch
        self.bad_result=lambda result:result.update(google_login={'status':'needs_google_repair','error':'SecurityException'})
        with patch.object(self.b,'install',return_value={'status':'installed','skipped_hook':True}):
            result=self.b.activate(Path('test.apk'))
        self.assertEqual(result['status'],'activated')
        self.assertEqual(result['google_login'],{'status':'needs_google_repair','error':'SecurityException'})
    def test_existing_exact_baseline_wins_over_later_snapshot(self):
        b=self.b;self.flags=0;b.capture_baselines(Path('test.apk'));self.flags=4
        path=self.paths()[0]
        for other in self.paths()[1:]:other.unlink()
        before=path.read_bytes();b.capture_baselines(Path('test.apk'));self.assertEqual(path.read_bytes(),before)
        self.assertEqual(b.baseline_for(self.cert,self.uid)[b.p.PERMISSION],0)
        self.worker.reset_mock();self.assertEqual(b.capture_baselines(Path('test.apk'))['status'],'already_saved');self.worker.assert_not_called()
    def test_unproven_legacy_zero_never_authorizes_use_capture_or_removal(self):
        from unittest.mock import patch
        b=self.b;b.capture_baselines(Path('test.apk'));path=self.paths()[0]
        legacy={'signer':self.cert,'fingerprint':b.p.CONFIG['fingerprint'],'before_flags':0,'uid':self.uid}
        self.persist(path,legacy);before=path.read_bytes();self.worker.reset_mock()
        for action in (lambda:b.baseline_for(self.cert,self.uid),lambda:b.capture_baselines(Path('test.apk'))):
            with self.assertRaisesRegex(RuntimeError,'trustworthy native'):action()
        with patch.object(b,'status',return_value={'config':{'uid':self.uid}}),patch.object(b.p,'root_json') as root:
            with self.assertRaisesRegex(RuntimeError,'trustworthy native'):b.remove(Path('test.apk'))
            root.assert_not_called()
        self.worker.assert_not_called();self.assertEqual(path.read_bytes(),before)
    def test_legacy_grant_before_activation_is_remeasured_and_archived(self):
        from unittest.mock import patch
        b=self.b;b.capture_baselines(Path('test.apk'));path=self.paths()[0]
        legacy={'signer':self.cert,'fingerprint':b.p.CONFIG['fingerprint'],'before_flags':0,'uid':self.uid}
        self.persist(path,legacy);before=path.read_bytes();self.events.clear()
        with patch.object(b,'install',return_value={'status':'installed','skipped_hook':True}):
            result=b.activate(Path('test.apk'))
        self.assertEqual(result['status'],'activated')
        self.assertEqual([event for event in self.events if event in ('inspect','grant')],['inspect','grant'])
        self.assertEqual(b.baseline_for(self.cert,self.uid)[b.p.PERMISSION],4)
        archives=list(path.parent.glob(path.stem+'-before-capture-*.json'))
        self.assertEqual(len(archives),1);self.assertEqual(archives[0].read_bytes(),before)
        self.assertEqual(json.loads(path.read_text())['origin'],'before_activation')
    def test_refresh_cannot_replace_wrong_uid_or_tampered_evidence(self):
        b=self.b;b.capture_baselines(Path('test.apk'));path=self.paths()[0];original=path.read_bytes()
        for defect in ('uid','serial','flags','proof'):
            value=json.loads(original)
            if defect=='uid':value['uid']=10001
            elif defect=='serial':value['serial']='another-device'
            elif defect=='flags':value['before_flags']=1234
            else:value['snapshot_proof']=str(self.root/'missing.json')
            self.persist(path,value);before=path.read_bytes();self.worker.reset_mock()
            with self.subTest(defect=defect),self.assertRaises(RuntimeError):
                b.capture_baselines(Path('test.apk'),origin='before_activation',force_refresh=True)
            self.worker.assert_not_called();self.assertEqual(path.read_bytes(),before)
        path.write_bytes(original)
    def test_explicit_refresh_archives_valid_baseline_before_new_boundary(self):
        b=self.b;self.flags=0;b.capture_baselines(Path('test.apk'));before={p:p.read_bytes() for p in self.paths()}
        self.flags=4;b.capture_baselines(Path('test.apk'),origin='before_activation',force_refresh=True)
        self.assertEqual(set(b.baseline_for(self.cert,self.uid).values()),{4})
        for path,data in before.items():
            archives=list(path.parent.glob(path.stem+'-before-capture-*.json'))
            self.assertEqual(len(archives),1);self.assertEqual(archives[0].read_bytes(),data)
    def test_direct_install_can_capture_legacy_grants_before_persistence(self):
        from unittest.mock import patch
        b=self.b;b.capture_baselines(Path('test.apk'));path=self.paths()[0]
        self.persist(path,{'signer':self.cert,'fingerprint':b.p.CONFIG['fingerprint'],'before_flags':0,'uid':self.uid})
        staged={};self.events.clear()
        def root_result(code):
            if "'exists':os.path.exists" in code:return {'exists':True}
            config=json.loads(staged['config.json'])
            return {'status':'installed','backup_sha256':config['original_sha256'],'config':config}
        with patch.object(b,'stage_file',side_effect=lambda path,data:staged.update({Path(path).name:data})),patch.object(b,'gateway_command'),patch.object(b.p,'root_json',side_effect=root_result),patch.object(b.subprocess,'run'):
            result=b.install(Path('test.apk'))
        self.assertEqual(result['config']['baseline'][b.p.PERMISSION],4)
        self.assertEqual([e for e in self.events if e in ('inspect','grant')],['inspect'])
    def test_snapshot_reference_must_prove_the_exact_flags_and_identity(self):
        b=self.b;b.capture_baselines(Path('test.apk'));path=self.paths()[0];original=path.read_text();value=json.loads(original)
        evidence=Path(value['snapshot_proof']);proof=evidence.read_text()
        for defect in ('flags','uid','operation','missing','foreign_path','changed_flags'):
            path.write_text(original);evidence.write_text(proof)
            if defect=='flags':value2=json.loads(original);value2['before_flags']=0;path.write_text(json.dumps(value2))
            elif defect=='missing':evidence.unlink()
            elif defect=='foreign_path':
                outside=self.root/'foreign.json';outside.write_text(proof);value2=json.loads(original);value2['snapshot_proof']=str(outside);path.write_text(json.dumps(value2))
            else:
                record=json.loads(proof);answer=json.loads(record['proof']['java_result'])
                if defect=='uid':record['uid']=10215
                elif defect=='operation':answer['operation']='grant'
                else:answer['permissions'][b.p.PERMISSION]['after_flags']=0
                record['proof']['java_result']=json.dumps(answer);evidence.write_text(json.dumps(record))
            with self.subTest(defect=defect),self.assertRaisesRegex(RuntimeError,'trustworthy native'):b.baseline_for(self.cert,self.uid)
    def test_actual_firmware_is_saved_in_new_snapshot_and_install_metadata(self):
        from unittest.mock import patch
        b=self.b;self.assertIn('/172:',b.p.CONFIG['fingerprint']);self.assertIn(self.actual_fingerprint,b.p.ALLOWED_FINGERPRINTS)
        result=b.capture_baselines(Path('test.apk'))
        self.assertEqual(json.loads(Path(result['snapshot_proof']).read_text())['fingerprint'],self.actual_fingerprint)
        self.assertTrue(all(json.loads(path.read_text())['fingerprint']==self.actual_fingerprint for path in self.paths()))
        before={path:path.read_bytes() for path in self.paths()};staged={};self.worker.reset_mock()
        def root_result(code):
            if "'exists':os.path.exists" in code:return {'exists':True}
            config=json.loads(staged['config.json'])
            return {'status':'installed','backup_sha256':config['original_sha256'],'config':config}
        with patch.object(b,'stage_file',side_effect=lambda path,data:staged.update({Path(path).name:data})),patch.object(b,'gateway_command'),patch.object(b.p,'root_json',side_effect=root_result),patch.object(b.subprocess,'run'):
            installed=b.install(Path('test.apk'))
        self.assertEqual(installed['config']['fingerprint'],self.actual_fingerprint);self.worker.assert_not_called()
        self.assertEqual({path:path.read_bytes() for path in self.paths()},before)
    def test_unreviewed_actual_firmware_cannot_be_saved_or_staged(self):
        from unittest.mock import patch
        self.b.capture_baselines(Path('test.apk'));self.worker.reset_mock();self.actual_fingerprint='unreviewed-firmware'
        with self.assertRaisesRegex(RuntimeError,'firmware'):self.b.capture_baselines(Path('test.apk'))
        with patch.object(self.b,'stage_file') as stage:
            with self.assertRaisesRegex(RuntimeError,'firmware'):self.b.install(Path('test.apk'))
            stage.assert_not_called()
        self.worker.assert_not_called()
    def test_host_reports_failed_install_with_recovery_path_even_after_rollback(self):
        from unittest.mock import patch
        b=self.b;b.capture_baselines(Path('test.apk'));self.worker.reset_mock()
        for restored in (True,False):
            failure={'status':'install_failed','rolled_back':restored,'recovery_path':'/var/lib/stepx-companion/transaction-test','error':'write failed','rollback_errors':[] if restored else ['grant.py: failed']}
            with self.subTest(restored=restored),patch.object(b,'stage_file'),patch.object(b,'gateway_command'),patch.object(b.p,'root_json',side_effect=[{'exists':True},failure]),patch.object(b.subprocess,'run'):
                with self.assertRaisesRegex(RuntimeError,'recovery: /var/lib/stepx-companion/transaction-test'):b.install(Path('test.apk'))
            self.assertEqual(json.loads((b.STATE_LOCAL/'installation-failure.json').read_text()),failure)
            self.assertFalse((b.STATE_LOCAL/'installation.json').exists())
        self.worker.assert_not_called()
    def test_invalid_read_proof_cannot_become_recovery_baseline(self):
        for invalid in ['uid','operation','changed','owner']:
            def damage(result):
                if invalid=='uid':result['uid']=10215
                elif invalid=='operation':result['operation']='grant'
                elif invalid=='changed':result['permissions'][self.b.p.PERMISSION]['after_flags']=0
                else:result['permissions'][self.b.p.PERMISSION]['owner']='untrusted.app'
            self.bad_result=damage
            with self.subTest(invalid=invalid),self.assertRaises(RuntimeError):self.b.capture_baselines(Path('test.apk'))
            self.assertFalse(any(path.exists() for path in self.paths()))
    def test_rebind_without_snapshot_stops_before_native_worker(self):
        with self.assertRaises(self.b.MissingPermissionBaselines):self.b.activate(Path('test.apk'),rebind=True)
        self.worker.assert_not_called()

class GoogleAdoptionTests(unittest.TestCase):
    def test_google_can_adopt_independent_companion_without_replacing_stock(self):
        from unittest.mock import patch
        tree=ast.parse((Path(__file__).parents[1]/'handoff/google-play-stepx/tools/install_adapter.py').read_text())
        code=next(n.value for n in ast.walk(tree) if isinstance(n,ast.Constant) and isinstance(n.value,str) and n.value.startswith('import os,stat,hashlib'))
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);base=root/'usr/local/lib/stepx-companion';base.mkdir(parents=True)
            launcher=root/'usr/local/bin/ubuntu_zygote';launcher.parent.mkdir(parents=True)
            original=b'# factory\n';chain=b'# companion chain\n';boot=b'# companion worker\n';standalone=b'# companion launcher\n'
            launcher.write_bytes(standalone);launcher.chmod(0o755)
            stock=Path(str(launcher)+'.stepx-stock-20261021');stock.write_bytes(original)
            (base/'chain.py').write_bytes(chain);(base/'boot.py').write_bytes(boot)
            config={'boot_mode':'standalone','hook_path':str(launcher),'hook_sha256':hashof(standalone),'chain_sha256':hashof(chain),'boot_sha256':hashof(boot)}
            (base/'config.json').write_text(json.dumps(config));(root/'var/lib').mkdir(parents=True)
            files={'launcher.py':b'# google launcher\n','boot.py':b'# google worker\n','inject.py':b'# fixed grant\n'}
            manifest={k:hashof(v) for k,v in files.items()}
            def stage():
                p=root/'stage';p.mkdir()
                for k,v in files.items():(p/k).write_bytes(v)
                return str(p)
            code=code.replace("'/usr/local/",repr(str(root/'usr/local')+'/')[:-1]).replace("'/var/lib/",repr(str(root/'var/lib')+'/')[:-1])
            env={'STAGE':stage(),'MANIFEST':manifest,'STOCK_SHA':hashof(original)}
            with patch('os.chown'),contextlib.redirect_stdout(io.StringIO()):exec(code,dict(env))
            self.assertEqual(stock.read_bytes(),original)
            self.assertEqual(launcher.read_bytes(),files['launcher.py'])
            self.assertEqual((base/'previous-boot.py').read_bytes(),files['boot.py'])
            self.assertEqual((root/'usr/local/lib/stepx-gms/boot.py').read_bytes(),chain)
            self.assertEqual(json.loads((base/'config.json').read_text())['boot_mode'],'chained')
            self.assertEqual(json.loads((base/'standalone-config-before-google.json').read_text()),config)
            env['STAGE']=stage()
            with patch('os.chown'),contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit) as done:exec(code,dict(env))
                self.assertEqual(done.exception.code,0)
            # A foreign hook must never be adopted or silently overwritten.
            launcher.write_bytes(b'# foreign\n');env['STAGE']=stage()
            with patch('os.chown'),contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(AssertionError):exec(code,dict(env))
            self.assertEqual(launcher.read_bytes(),b'# foreign\n')

if __name__=='__main__':unittest.main()
