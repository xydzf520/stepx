"""Permission adapter guards; all Android/RootBridge operations are mocked."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[2] / 'scripts/companion-permissions.py'
spec = importlib.util.spec_from_file_location('companion_permissions', SCRIPT)
adapter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapter)


class Guards(unittest.TestCase):
    def test_android_tools_receive_explicit_java_runtime(self):
        with tempfile.TemporaryDirectory() as directory:
            jdk = Path(directory); (jdk / 'bin').mkdir(); (jdk / 'bin/java').touch()
            with patch.dict(adapter.os.environ, {'STEPX_JDK':str(jdk)}), patch.object(adapter.subprocess, 'run', return_value=SimpleNamespace(returncode=0,stdout='verified')) as run:
                self.assertEqual(adapter.run(['apksigner','verify','app.apk']),'verified')
                env = run.call_args.kwargs['env']
                self.assertEqual(env['JAVA_HOME'],str(jdk))
                self.assertTrue(env['PATH'].startswith(str(jdk / 'bin') + adapter.os.pathsep))

    def test_wrong_device_rejected_before_any_io(self):
        with patch.object(adapter, 'ADB', ['adb', '-s', 'different-device']), patch.object(adapter, 'guard') as guard:
            with self.assertRaisesRegex(RuntimeError, 'authorized'):
                adapter.inspect()
            guard.assert_not_called()

    def test_other_user_selected_stepx_serial_is_accepted(self):
        dump = '  Package [com.stepx.companion.app] (abcd):\n    appId=10234\n    com.stepos.permission.MANAGE_BACK_SCREEN: granted=true\n'
        for transport, physical in [
            ('ZYPZ261A01M692888AB', 'ZYPZ261A01M692888AB'),
            ('192.168.1.88:5555', 'ZYPZ261A01M692888AB'),
        ]:
            with self.subTest(transport=transport), patch.dict(
                adapter.os.environ,
                {'STEPX_SERIAL': transport, 'STEPX_DEVICE_SERIAL': physical},
            ):
                fresh_spec = importlib.util.spec_from_file_location('companion_permissions_user', SCRIPT)
                fresh = importlib.util.module_from_spec(fresh_spec)
                fresh_spec.loader.exec_module(fresh)
                self.assertEqual(fresh.AUTHORIZED_SERIAL, transport)
                with patch.object(fresh, 'ADB', ['adb', '-s', transport]), \
                     patch.object(fresh, 'guard') as guard, \
                     patch.object(fresh, 'shell', return_value=dump):
                    result = fresh.inspect()
                    self.assertTrue(result['granted'])
                    self.assertEqual(result['uid'], 10234)
                    guard.assert_called_once_with()

    def test_preflight_requires_apk(self):
        with patch.object(adapter, 'inspect') as inspect:
            with self.assertRaisesRegex(RuntimeError, 'APK'):
                adapter.preflight(None)
            inspect.assert_not_called()

    def test_wrong_package_and_multiple_signers_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'app.apk'; path.write_bytes(b'fixture')
            with patch.object(adapter, 'run', return_value="package: name='com.other.app'"):
                with self.assertRaisesRegex(RuntimeError, 'not StepX'):
                    adapter.signer(path)
            with patch.object(adapter, 'run', side_effect=["package: name='com.stepx.companion.app'", 'Signer #1 certificate SHA-256 digest: '+('a'*64)+'\nSigner #2 certificate SHA-256 digest: '+('b'*64)]):
                with self.assertRaisesRegex(RuntimeError, 'one verified'):
                    adapter.signer(path)

    def test_inspect_has_no_root_or_app_start(self):
        dump = '  Package [com.stepx.companion.app] (abcd):\n    appId=10234\n    com.stepos.permission.MANAGE_BACK_SCREEN: granted=true\n'
        with patch.object(adapter, 'ADB', ['adb','-s',adapter.AUTHORIZED_SERIAL]), patch.object(adapter, 'guard'), patch.object(adapter, 'root_json') as root, patch.object(adapter, 'shell', return_value=dump) as shell:
            result=adapter.inspect()
            self.assertTrue(result['granted']); self.assertEqual(result['uid'],10234)
            root.assert_not_called()
            shell.assert_called_once_with('dumpsys','package',adapter.PACKAGE)

    def test_reinstall_never_reuses_another_uid_baseline(self):
        self.assertNotEqual(adapter.baseline_path('a'*64,10234),adapter.baseline_path('a'*64,10235))
        baseline={'signer':'a'*64,'fingerprint':adapter.CONFIG['fingerprint'],'uid':10234,'before_flags':0}
        adapter.validate_baseline(baseline,'a'*64,10234)
        with self.assertRaisesRegex(RuntimeError, 'UID mismatch'):
            adapter.validate_baseline(baseline,'a'*64,10235)

    def test_preflight_does_not_inject_or_install(self):
        with patch.object(adapter,'inspect',return_value={'installed':False}), patch.object(adapter,'guard'), patch.object(adapter,'signer',return_value='a'*64), patch.object(adapter,'shell',side_effect=['1','RUNNING_UNLOCKED','switch:1']), patch.object(adapter,'root_json',return_value={'root_available':True}) as root, patch.object(adapter,'inject') as inject:
            result=adapter.preflight(Path('app.apk'),trusted_apk=Path('trusted.apk'))
            self.assertEqual(result['status'],'ready')
            self.assertFalse(result['boot_hook_installed'])
            inject.assert_not_called()
            self.assertEqual(root.call_count,1)
            self.assertIn('os.geteuid()==0',root.call_args.args[0])

    def test_usb_gate_restores_exact_single_package_value(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter,'STATE',Path(directory)), patch.object(adapter,'preflight',return_value={'usb_install_master_enabled':True}), patch.object(adapter,'signer',return_value='a'*64), patch.object(adapter,'ADB',['adb','-s',adapter.AUTHORIZED_SERIAL]), patch.object(adapter,'guard'):
            current={'value':'StepX伴侣:0'}; commands=[]
            def shell(*args):
                commands.append(args)
                self.assertEqual(args[0],'settings')
                self.assertEqual(args[2],'secure')
                self.assertEqual(args[3],adapter.INSTALL_KEY)
                if args[1]=='put':
                    self.assertTrue(adapter.install_gate_path().exists(),'baseline must precede mutation')
                    current['value']=args[4]; return ''
                if args[1]=='get':return current['value']
                raise AssertionError(args)
            with patch.object(adapter,'shell',side_effect=shell):
                self.assertTrue(adapter.prepare_install(Path('app.apk'))['restore_required'])
                self.assertEqual(current['value'],'StepX伴侣:1')
                self.assertEqual(adapter.finish_install()['status'],'restored')
                self.assertEqual(current['value'],'StepX伴侣:0')
                self.assertFalse(adapter.install_gate_path().exists())
                self.assertFalse(any('usb_install_item_switch' in c for c in commands))

    def test_master_gate_disabled_never_changes_settings(self):
        with patch.object(adapter,'preflight',return_value={'usb_install_master_enabled':False}), patch.object(adapter,'shell') as shell:
            with self.assertRaisesRegex(RuntimeError,'总开关'):
                adapter.prepare_install(Path('app.apk'))
            shell.assert_not_called()

    def test_pending_install_baseline_not_overwritten(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter,'STATE',Path(directory)), patch.object(adapter,'preflight',return_value={'usb_install_master_enabled':True}), patch.object(adapter,'shell') as shell:
            adapter.install_gate_path().write_text('previous recovery data')
            with self.assertRaisesRegex(RuntimeError,'finish-install'):
                adapter.prepare_install(Path('app.apk'))
            self.assertEqual(adapter.install_gate_path().read_text(),'previous recovery data')
            shell.assert_not_called()

    def test_runtime_exempts_only_companion_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory); path = state / 'runtime.json'
            identity = {'uid':10215, 'signer':'a'*64}
            initial = {'installed':True,'signature_verified':True,'uid':10215}
            lists = {'cloud':['existing.cloud'],'user':['existing.user'],'local':['existing.local']}
            calls=[]; frozen={'value':True}
            def shell(*args):
                calls.append(args)
                if args == ('dumpsys','activity','freeze','whiteAdd',adapter.PACKAGE):
                    self.assertTrue(path.exists(),'save recovery before changing any whitelist')
                    for section in ('cloud','local'):lists[section].append(adapter.PACKAGE)
                elif args == ('dumpsys','activity','freeze','doUnfreeze','10215'):
                    frozen['value']=False
                elif args == ('dumpsys','activity','freeze','whiteRemove',adapter.PACKAGE):
                    for section in ('cloud','local'):lists[section].remove(adapter.PACKAGE)
                else:raise AssertionError(args)
                return ''
            with patch.object(adapter,'STATE',state), patch.object(adapter,'runtime_identity',return_value=(identity,path)), patch.object(adapter,'signer',return_value='a'*64), patch.object(adapter,'freeze_lists',side_effect=lambda: {k:v[:] for k,v in lists.items()}), patch.object(adapter,'frozen_uid',side_effect=lambda uid: frozen['value']), patch.object(adapter,'shell',side_effect=shell):
                result=adapter.ensure_runtime(Path('app.apk'),initial)
                self.assertTrue(result['background_added']);self.assertTrue(result['background_thawed'])
                self.assertFalse(adapter.ensure_runtime(Path('app.apk'),initial)['background_added'])
                self.assertEqual(len(calls),2,'already exempt/unfrozen must cause no writes')
                self.assertTrue(adapter.revoke_runtime(Path('app.apk'),initial)['background_reverted'])

    def test_runtime_preserves_preexisting_exemption(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'runtime.json'
            initial={'installed':True,'signature_verified':True,'uid':10215}
            lists={'cloud':[adapter.PACKAGE],'user':[],'local':[]}
            with patch.object(adapter,'STATE',Path(directory)), patch.object(adapter,'runtime_identity',return_value=({},path)), patch.object(adapter,'signer',return_value='a'*64), patch.object(adapter,'freeze_lists',return_value=lists), patch.object(adapter,'frozen_uid',return_value=False), patch.object(adapter,'shell') as shell:
                self.assertFalse(adapter.ensure_runtime(Path('app.apk'),initial)['background_added'])
                self.assertFalse(adapter.revoke_runtime(Path('app.apk'),initial)['background_owned'])
                shell.assert_not_called()

    def test_runtime_rejects_shared_uid_before_any_mutation(self):
        text='UidStats {uid: 10215, pkgs: {com.stepx.companion.app, com.other.app}, currentState: frozen,'
        with patch.object(adapter,'shell',return_value=text):
            with self.assertRaisesRegex(RuntimeError,'shared'):
                adapter.frozen_uid(10215)

    def test_runtime_requires_verified_signer(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter,'STATE',Path(directory)), patch.object(adapter,'runtime_identity') as identity:
            with self.assertRaisesRegex(RuntimeError,'verified'):
                adapter.ensure_runtime(Path('app.apk'),{'installed':True,'signature_verified':False})
            identity.assert_not_called()

    def test_connection_baseline_is_separate_and_exact(self):
        self.assertNotEqual(adapter.connection_baseline_path('a'*64,10215), adapter.baseline_path('a'*64,10215))
        adapter.validate_connection_flags({name: 0 for name in adapter.CONNECTION_PERMISSIONS})
        for invalid in ({adapter.PERMISSION: 0}, {name: True for name in adapter.CONNECTION_PERMISSIONS},
                        {name: 2**31 for name in adapter.CONNECTION_PERMISSIONS}):
            with self.assertRaisesRegex(RuntimeError, 'baseline'):
                adapter.validate_connection_flags(invalid)

    def test_connection_revoke_requires_own_existing_baseline(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter,'STATE',Path(directory)), \
                patch.object(adapter,'inspect',return_value={'installed':True,'signature_verified':True,'uid':10215}), \
                patch.object(adapter,'guard'), patch.object(adapter,'signer',return_value='a'*64), \
                patch.object(adapter,'inject_connection') as inject:
            # An existing old backscreen baseline must not authorize connection restoration.
            adapter.baseline_path('a'*64,10215).write_text('{"before_flags":0}')
            with self.assertRaisesRegex(RuntimeError,'No connection permission baseline'):
                adapter.connection_permissions(Path('app.apk'),'revoke')
            inject.assert_not_called()

    def test_connection_grant_saves_flags_before_mutation_and_revoke_restores_them(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter,'STATE',Path(directory)), \
                patch.object(adapter,'inspect',return_value={'installed':True,'signature_verified':True,'uid':10215}), \
                patch.object(adapter,'guard'), patch.object(adapter,'signer',return_value='a'*64):
            # One permission was already granted before install: restore exactly, not blanket revoke.
            before = {adapter.CONNECTION_PERMISSIONS[0]: 4, adapter.CONNECTION_PERMISSIONS[1]: 2}
            baseline_path=adapter.connection_baseline_path('a'*64,10215)
            calls=[]
            def inject(cert,operation,flags,uid):
                self.assertEqual(cert,'a'*64); self.assertEqual(uid,10215)
                calls.append(operation)
                if operation=='inspect':
                    self.assertFalse(baseline_path.exists())
                    return {'permissions': {name:{'before_flags':value} for name,value in before.items()}}
                self.assertTrue(baseline_path.exists(),'native recovery flags must precede mutation')
                self.assertEqual(flags,before)
                return {'status':'success','operation':operation}
            with patch.object(adapter,'inject_connection',side_effect=inject):
                adapter.connection_permissions(Path('app.apk'),'grant')
                adapter.connection_permissions(Path('app.apk'),'grant')
                adapter.connection_permissions(Path('app.apk'),'revoke')
            self.assertEqual(calls,['inspect','grant','grant','revoke'])
            self.assertFalse(adapter.baseline_path('a'*64,10215).exists())

    def test_connection_inspection_does_not_create_baseline_or_grant(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter,'STATE',Path(directory)), \
                patch.object(adapter,'inspect',return_value={'installed':True,'signature_verified':True,'uid':10215}), \
                patch.object(adapter,'guard'), patch.object(adapter,'signer',return_value='a'*64), \
                patch.object(adapter,'inject_connection',return_value={'status':'success'}) as inject:
            adapter.connection_permissions(Path('app.apk'),'inspect')
            inject.assert_called_once_with('a'*64,'inspect',{},10215)
            self.assertEqual(list(Path(directory).iterdir()),[])

    def test_already_granted_connection_permissions_keep_valid_baseline_without_worker(self):
        import json
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter,'STATE',Path(directory)), \
                patch.object(adapter,'inspect',return_value={'installed':True,'signature_verified':True,
                    'uid':10215,'connection_permissions_granted':True}), \
                patch.object(adapter,'guard'), patch.object(adapter,'signer',return_value='a'*64), \
                patch.object(adapter,'inject_connection') as inject:
            baseline={'signer':'a'*64,'fingerprint':adapter.CONFIG['fingerprint'],'uid':10215,
                      'serial':adapter.ADB[-1],'scope':adapter.CONNECTION_SCOPE,
                      'before_flags':{name:0 for name in adapter.CONNECTION_PERMISSIONS}}
            path=adapter.connection_baseline_path('a'*64,10215)
            original=json.dumps(baseline);path.write_text(original)
            result=adapter.connection_permissions(Path('app.apk'),'grant')
            self.assertEqual(result['status'],'already_granted')
            self.assertFalse(result['injected']);inject.assert_not_called()
            self.assertEqual(path.read_text(),original)
            baseline['uid']=10216;path.write_text(json.dumps(baseline))
            with self.assertRaisesRegex(RuntimeError,'UID mismatch'):
                adapter.connection_permissions(Path('app.apk'),'grant')
            inject.assert_not_called()

    def test_wake_scope_is_fixed_and_separate_from_other_baselines(self):
        self.assertEqual(adapter.WAKE_PERMISSIONS, ('com.stepos.cloud.push.permission.MESSAGE',))
        self.assertEqual(adapter.WAKE_OWNER, 'com.stepos.pushservice')
        paths = {adapter.baseline_path('a'*64, 10215),
                 adapter.connection_baseline_path('a'*64, 10215),
                 adapter.wake_baseline_path('a'*64, 10215)}
        self.assertEqual(len(paths), 3)
        self.assertNotEqual(adapter.wake_baseline_path('a'*64, 10215),
                            adapter.wake_baseline_path('a'*64, 10216))
        for flags in ({}, {adapter.PERMISSION: 0},
                      {adapter.WAKE_PERMISSIONS[0]: True},
                      {adapter.WAKE_PERMISSIONS[0]: 2**31}):
            with self.assertRaises(RuntimeError):
                adapter.validate_wake_flags(flags)
        with patch.object(adapter, 'inspect') as inspect:
            with self.assertRaisesRegex(RuntimeError, 'Unsupported'):
                adapter.wake_permissions(Path('app.apk'), 'arbitrary')
            inspect.assert_not_called()

    def test_wake_revoke_requires_own_baseline_and_inspection_never_creates_one(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter, 'STATE', Path(directory)), \
                patch.object(adapter, 'inspect', return_value={'installed':True, 'signature_verified':True, 'uid':10215}), \
                patch.object(adapter, 'guard'), patch.object(adapter, 'signer', return_value='a'*64), \
                patch.object(adapter, 'inject_wake', return_value={'status':'success'}) as inject:
            adapter.wake_permissions(Path('app.apk'), 'inspect')
            inject.assert_called_once_with('a'*64, 'inspect', {}, 10215)
            self.assertEqual(list(Path(directory).iterdir()), [])
            adapter.baseline_path('a'*64, 10215).write_text('{}')
            adapter.connection_baseline_path('a'*64, 10215).write_text('{}')
            inject.reset_mock()
            with self.assertRaisesRegex(RuntimeError, 'No wake permission baseline'):
                adapter.wake_permissions(Path('app.apk'), 'revoke')
            inject.assert_not_called()

    def test_wake_grant_saves_original_flags_and_revoke_preserves_preexisting_grant(self):
        import json
        for original_flags in (2, 4, 36):
            with self.subTest(original_flags=original_flags), tempfile.TemporaryDirectory() as directory, \
                    patch.object(adapter, 'STATE', Path(directory)), \
                    patch.object(adapter, 'inspect', return_value={'installed':True, 'signature_verified':True, 'uid':10215}), \
                    patch.object(adapter, 'guard'), patch.object(adapter, 'signer', return_value='a'*64):
                before = {adapter.WAKE_PERMISSIONS[0]: original_flags}
                path = adapter.wake_baseline_path('a'*64, 10215)
                calls = []
                def inject(cert, operation, flags, uid):
                    calls.append(operation)
                    self.assertEqual((cert, uid), ('a'*64, 10215))
                    if operation == 'inspect':
                        self.assertFalse(path.exists())
                        return {'permissions': {name:{'before_flags':value} for name,value in before.items()}}
                    saved = json.loads(path.read_text())
                    self.assertEqual(saved['owner'], adapter.WAKE_OWNER)
                    self.assertEqual(saved['scope'], adapter.WAKE_SCOPE)
                    self.assertEqual(saved['before_flags'], before)
                    self.assertEqual(flags, before)
                    self.assertEqual(path.stat().st_mode & 0o777, 0o600)
                    return {'status':'success', 'operation':operation}
                with patch.object(adapter, 'inject_wake', side_effect=inject):
                    adapter.wake_permissions(Path('app.apk'), 'grant')
                    adapter.wake_permissions(Path('app.apk'), 'revoke')
                self.assertEqual(calls, ['inspect', 'grant', 'revoke'])
                self.assertFalse(adapter.baseline_path('a'*64, 10215).exists())
                self.assertFalse(adapter.connection_baseline_path('a'*64, 10215).exists())

    def test_existing_wake_grant_requires_same_identity_and_keeps_recovery(self):
        import json
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter, 'STATE', Path(directory)), \
                patch.object(adapter, 'inspect', return_value={'installed':True, 'signature_verified':True,
                    'uid':10215, 'wake_permissions_granted':True}), \
                patch.object(adapter, 'guard'), patch.object(adapter, 'signer', return_value='a'*64), \
                patch.object(adapter, 'inject_wake') as inject:
            baseline = {'signer':'a'*64, 'fingerprint':adapter.CONFIG['fingerprint'], 'uid':10215,
                        'serial':adapter.ADB[-1], 'scope':adapter.WAKE_SCOPE, 'owner':adapter.WAKE_OWNER,
                        'before_flags':{adapter.WAKE_PERMISSIONS[0]:2}}
            path = adapter.wake_baseline_path('a'*64, 10215)
            original = json.dumps(baseline); path.write_text(original)
            result = adapter.wake_permissions(Path('app.apk'), 'grant')
            self.assertEqual(result['status'], 'already_granted')
            self.assertFalse(result['injected'])
            self.assertEqual(path.read_text(), original)
            for key, value in [('uid',10216), ('signer','b'*64), ('owner','android'),
                               ('scope',adapter.CONNECTION_SCOPE), ('serial','other')]:
                path.write_text(json.dumps({**baseline, key:value}))
                with self.assertRaises(RuntimeError):
                    adapter.wake_permissions(Path('app.apk'), 'grant')
            inject.assert_not_called()

    def test_wake_response_requires_exact_owner_protection_flags_and_permission_readback(self):
        import copy
        name = adapter.WAKE_PERMISSIONS[0]
        before = {name:2}
        result = {'scope':adapter.WAKE_SCOPE, 'operation':'grant', 'package':adapter.PACKAGE,
                  'uid':10215, 'signature_verified':True, 'all_granted':True,
                  'permissions':{name:{'owner':adapter.WAKE_OWNER, 'declared':True,
                     'signature_permission':True, 'before_flags':2, 'after_flags':4, 'granted':True}}}
        adapter.validate_wake_response(result, 'grant', before, 10215)
        for key, value in [('owner','android'), ('declared',False), ('signature_permission',False),
                           ('after_flags',0), ('granted',False)]:
            modified = copy.deepcopy(result); modified['permissions'][name][key] = value
            with self.assertRaises(RuntimeError):
                adapter.validate_wake_response(modified, 'grant', before, 10215)
        for key, value in [('scope',adapter.CONNECTION_SCOPE), ('package','other'), ('uid',10216),
                           ('signature_verified',False), ('all_granted',False)]:
            with self.assertRaises(RuntimeError):
                adapter.validate_wake_response({**result, key:value}, 'grant', before, 10215)
        modified = copy.deepcopy(result)
        modified['permissions'][adapter.PERMISSION] = modified['permissions'][name]
        with self.assertRaises(RuntimeError):
            adapter.validate_wake_response(modified, 'grant', before, 10215)

    def test_agentbus_domain_failure_is_returned_by_parent_without_payload(self):
        import ast,json,subprocess,sys
        # Execute the actual generated supervisor on the host, replacing only
        # Android transport/namespace operations. A failed identity transition
        # must still return a proof without executing the payload.
        source=adapter.agentbus_worker_source((adapter.WORKFLOW/'tools/root_worker.py').read_text())
        node=next(n for n in ast.parse(source).body if isinstance(n,ast.Assign) and
                  any(isinstance(t,ast.Name) and t.id=='src' for t in n.targets))
        value=node.value
        while isinstance(value,ast.Call):value=value.func.value
        template=ast.literal_eval(value)
        with tempfile.TemporaryDirectory() as directory:
            result=Path(directory)/'result.json'
            template=template.replace("prop('persist.openclaw.zygote.path','');prop('sys.openclaw.env.start','')",'pass')
            template=template.replace("child=os.fork()\nif child:os.execv('/usr/local/bin/ubuntu_zygote',['/usr/local/bin/ubuntu_zygote'])",'pass')
            template=template.replace("fd=os.open(host+'/sys/fs/cgroup/cgroup.procs',os.O_WRONLY);os.write(fd,str(os.getpid()).encode());os.close(fd)",'pass')
            template=template.replace('rootfd=os.open(host,os.O_RDONLY|os.O_DIRECTORY)',"rootfd=os.open('/',os.O_RDONLY|os.O_DIRECTORY)")
            template=template.replace("os.fchdir(rootfd);os.chroot('.');os.chdir('/')",'pass')
            template=template.replace('os.setresgid(1000,0,0);os.setresuid(1000,0,0);lib.setfsgid(1000);lib.setfsuid(1000)',"raise PermissionError('identity denied')")
            template=template.replace('CODE',repr("out['payload_executed']=True")).replace('RESULT',repr(str(result)))
            process=subprocess.run([sys.executable,'-c',template],capture_output=True,text=True,timeout=5)
            self.assertEqual(process.returncode,0,process.stderr)
            proof=json.loads(result.read_text())
            self.assertEqual(proof['phase'],'worker_identity')
            self.assertIn('identity denied',proof['error'])
            self.assertNotIn('payload_executed',proof)
            self.assertEqual(proof['child_status'],0)
            self.assertEqual(result.stat().st_mode&0o777,0o600)

    def test_agentbus_transport_requires_audited_source_and_keeps_parent_result_outside_app(self):
        with self.assertRaisesRegex(RuntimeError,'changed'):
            adapter.agentbus_worker_source('unreviewed transport')
        source=adapter.agentbus_worker_source((adapter.WORKFLOW/'tools/root_worker.py').read_text())
        self.assertIn("result='/tmp/stepx-background-worker-'",source)
        self.assertNotIn("repr('/proc/'+str(parent)+'/root'+result)",source)
        self.assertNotIn('/data/user_de/0/com.stepos.agentbus',source)

    def test_google_concurrent_repair_never_loses_successful_worker_proof(self):
        import json
        with tempfile.TemporaryDirectory() as directory, patch.object(adapter,'STATE',Path(directory)):
            counts={}; preserved=[]
            answer={'before_flags':0,'after_flags':4,'status':'success'}
            def shell(*args):
                if args[0]=='getprop': return ''
                if args==('pidof','system_server'): return '2859'
                if args==('date','+%s'): return '1000'
                if args[:2]==('dumpsys','package'):
                    package=args[2]; counts[package]=counts.get(package,0)+1
                    granted=package=='com.android.vending' or counts[package]>1
                    return 'android.permission.MANAGE_USERS: granted='+str(granted).lower()
                raise AssertionError(args)
            def run(args,**kwargs):
                Path(args[-1]).write_text(json.dumps({'detached':True,'java_result':json.dumps(answer)}))
                return SimpleNamespace(returncode=0)
            def preserve(proof):
                stored=json.loads((Path(directory)/'last-worker-result.json').read_text())
                self.assertEqual(stored['java_result'],json.dumps(answer))
                preserved.append(json.loads(proof['java_result'])['before_flags'])
            identity={'launcher':adapter.hashlib.sha256((adapter.WORKFLOW/'module/launcher.py').read_bytes()).hexdigest(),'root_available':True}
            repair={'status':'success','target_pid':2859,'unix_time':1001,'observed_epoch':1002}
            with patch.object(adapter,'shell',side_effect=shell), patch.object(adapter.subprocess,'run',side_effect=run), \
                    patch.object(adapter,'root_json',side_effect=[identity,identity,repair]):
                with self.assertRaisesRegex(RuntimeError,'原有 Google'):
                    adapter.run_bounded_android_worker('fixed payload',2859,preserve)
            proof=json.loads((Path(directory)/'last-worker-result.json').read_text())
            self.assertEqual(preserved,[0])
            self.assertFalse(proof['google_permission_before']['com.google.android.gms'])
            self.assertTrue(proof['google_permission_after']['com.google.android.gms'])
            self.assertFalse(proof['google_permission_baseline_unchanged'])
            self.assertTrue(proof['startup_properties_restored'])
            self.assertEqual(proof['java_result'],json.dumps(answer))


if __name__ == '__main__':
    unittest.main()
