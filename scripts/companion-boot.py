#!/usr/bin/env python3
"""Install/remove the bounded StepX companion boot adapter, preserving the existing launcher."""
import argparse,base64,hashlib,importlib.util,json,os,re,sys,subprocess,uuid,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('permissions',ROOT/'scripts/companion-permissions.py')
p=importlib.util.module_from_spec(spec);spec.loader.exec_module(p)
from source_integrity import read_python_source, python_source_sha256
BASE='/usr/local/lib/stepx-companion'
STATE='/var/lib/stepx-companion'
HOOK='/usr/local/lib/stepx-gms/boot.py'
STOCK_SHA='5cfe5a88a97eca424bd4922fe813e0cc553adad0e8623105311ec1514022152f'
ORIGINAL_SHA=python_source_sha256(p.WORKFLOW/'module/boot.py')
STATE_LOCAL=p.STATE.parent/'boot'
GROUP=(p.PERMISSION,)+p.CONNECTION_PERMISSIONS+p.WAKE_PERMISSIONS

class MissingPermissionBaselines(RuntimeError):
    pass

class LegacyPermissionBaseline(RuntimeError):
    """Identity is checked separately; this record predates native read proofs."""
    pass

def digest(data):return hashlib.sha256(data).hexdigest()
def verified_fingerprint():
    actual=p.shell('getprop','ro.build.fingerprint')
    if actual not in p.ALLOWED_FINGERPRINTS:raise RuntimeError('Current firmware was not reviewed')
    return actual

def private(value,name):
    STATE_LOCAL.mkdir(parents=True,exist_ok=True,mode=0o700)
    p.private_json(STATE_LOCAL/name,value)

def sync_parent(path):
    if os.name=='posix':
        fd=os.open(path.parent,os.O_RDONLY|os.O_DIRECTORY)
        try:os.fsync(fd)
        finally:os.close(fd)

def payload(cert,uid,baseline,operation="grant"):
    if operation not in ("inspect","grant","revoke"):raise RuntimeError("Unsupported boot operation")
    source=read_python_source(p.WORKFLOW/'module/inject.py').decode('utf-8')
    if digest(source.encode())!=p.LOADER_SHA256:raise RuntimeError('Unreviewed native loader')
    source=p.adapt_native_loader(source)
    dex=(ROOT/'desktop/assets/stepx-companion-permissions.dex').read_bytes()
    if not dex.startswith(b'dex\n') or len(dex)>128*1024:raise RuntimeError('Invalid boot permission artifact')
    source,count=re.subn(r'dex=base64\.b64decode\([^\n]+?\);dexp=','dex=base64.b64decode(BOOT_DEX);dexp=',source)
    if count!=1:raise RuntimeError('Native template changed')
    source=source.replace("cstr('stepx.GmsProbe')","cstr('stepx.CompanionGrant')")
    old="run=method(clazz,'run','()Ljava/lang/String;',True);answer=jni(116,clazz,run,0);"
    new="run=method(clazz,'runBoot','(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;I)Ljava/lang/String;',True);signer=jni(167,cstr(BOOT_SIGNER));op=jni(167,cstr('grant'));baseline=jni(167,cstr(BOOT_BASELINE));answer=jni(116,clazz,run,argsv(signer,op,baseline,BOOT_UID));"
    if source.count(old)!=1:raise RuntimeError('Native entry template changed')
    new=new.replace("cstr('grant')", "cstr("+repr(operation)+")")
    source=source.replace(old,new)
    if 'stepx.GmsProbe' in source or 'ZGV4C' in source:raise RuntimeError('Non-companion payload remained')
    result=f'BOOT_DEX={base64.b64encode(dex).decode()!r}\nBOOT_SIGNER={cert!r}\nBOOT_UID={uid!r}\nBOOT_BASELINE={json.dumps(baseline)!r}\n'+source
    compile(result,'companion-boot-payload','exec');return result.encode()

def validate_baseline_evidence(value,cert,uid,names):
    """Only a durable, identity-bound native read proves original flags.

    Legacy records did not distinguish observed flags from an assumed zero.
    Keep those files untouched, but never treat them as exact recovery proof.
    """
    failure='Permission baseline lacks trustworthy native before-state evidence; preserving existing permissions and recovery files'
    if not any(k in value for k in ('origin','snapshot_proof','permission_changes','boot_id','captured_at')):
        raise LegacyPermissionBaseline(failure+'; run companion-boot.py capture-baselines --origin before_activation to measure current flags without revoking permissions')
    if value.get('origin') not in ('before_rebind_snapshot','before_activation') or value.get('permission_changes') is not False:raise RuntimeError(failure)
    scope='backscreen_permission_v1' if names==(p.PERMISSION,) else p.WAKE_SCOPE if names==p.WAKE_PERMISSIONS else p.CONNECTION_SCOPE
    if value.get('serial')!=p.device_key() or value.get('scope')!=scope or (names==p.WAKE_PERMISSIONS and value.get('owner')!=p.WAKE_OWNER):raise RuntimeError(failure)
    evidence=Path(value.get('snapshot_proof',''))
    if evidence.parent.resolve()!=p.STATE.resolve() or evidence.is_symlink() or not evidence.is_file() or evidence.stat().st_size>1048576:raise RuntimeError(failure)
    record=json.loads(evidence.read_text())
    if (record.get('signer')!=cert or record.get('uid')!=uid or record.get('serial')!=p.device_key()
            or record.get('fingerprint')!=value.get('fingerprint') or record.get('fingerprint') not in p.ALLOWED_FINGERPRINTS
            or record.get('origin')!=value.get('origin') or record.get('permission_changes') is not False
            or not record.get('boot_id') or record.get('boot_id')!=value.get('boot_id')
            or type(record.get('captured_at')) is not int or record['captured_at']<=0 or record.get('captured_at')!=value.get('captured_at')):raise RuntimeError(failure)
    proof=record.get('proof',{});answer=json.loads(proof.get('java_result','{}'))
    if not (proof.get('system_server_unchanged') is True and proof.get('system_server_stable') is True and proof.get('detached') is True and proof.get('worker_exit_code')==0
            and answer.get('status')=='success' and answer.get('operation')=='inspect' and answer.get('scope')=='companion_boot_v1'
            and answer.get('signature_verified') is True and answer.get('package')==p.PACKAGE and answer.get('uid')==uid):raise RuntimeError(failure)
    details=answer.get('permissions',{})
    if set(details)!=set(GROUP):raise RuntimeError(failure)
    for name in GROUP:
        detail=details[name];flags=detail.get('before_flags');owner=p.WAKE_OWNER if name in p.WAKE_PERMISSIONS else 'android'
        if type(flags)is not int or not -(2**31)<=flags<2**31 or type(detail.get('after_flags'))is not int or detail['after_flags']!=flags or detail.get('declared') is not True or detail.get('signature_permission') is not True or detail.get('owner')!=owner:raise RuntimeError(failure)
    expected={name:details[name]['before_flags'] for name in names}
    saved={p.PERMISSION:value.get('before_flags')} if names==(p.PERMISSION,) else value.get('before_flags')
    if saved!=expected:raise RuntimeError(failure)

def baseline_for(cert,uid,require_exact=True):
    # No default zero flags: absence of a recovery record is not evidence that a
    # permission was previously denied. All callers now require measured state.
    files=[p.baseline_path(cert,uid),p.connection_baseline_path(cert,uid),p.wake_baseline_path(cert,uid)]
    if not all(f.is_file() and not f.is_symlink() for f in files):
        raise MissingPermissionBaselines('Rebinding requires exact saved permission baselines for the new signer and UID; run capture-baselines first')
    values=[json.loads(f.read_text()) for f in files]
    for value,names in zip(values,[(p.PERMISSION,),p.CONNECTION_PERMISSIONS,p.WAKE_PERMISSIONS]):
        p.validate_baseline(value,cert,uid)
        if value.get('serial',p.device_key())!=p.device_key():raise RuntimeError('New permission baseline belongs to another device')
        validate_baseline_evidence(value,cert,uid,names)
    baseline={p.PERMISSION:values[0]['before_flags'],**values[1]['before_flags'],**values[2]['before_flags']}
    if set(baseline)!=set(GROUP) or any(type(v)is not int or not -(2**31)<=v<2**31 for v in baseline.values()):raise RuntimeError('Exact original permission baselines required')
    return baseline

def capture_baselines(apk,origin='before_rebind_snapshot',force_refresh=False):
    """Read fixed permissions, save identity-bound recovery, never grant/revoke.

    A reinstall snapshot describes the state immediately before this rebind;
    it deliberately makes no claim about permissions before the first install.
    Earlier valid baselines for the same identity always win over a later read
    unless force_refresh is requested after an APK update resets permission flags.
    """
    if origin not in ('before_rebind_snapshot','before_activation'):raise RuntimeError('Unsupported baseline observation boundary')
    if force_refresh and origin!='before_activation':raise RuntimeError('--force-refresh requires --origin before_activation')
    initial=p.inspect(apk);p.guard(native=True)
    if not initial.get('installed') or initial.get('signature_verified') is not True:raise RuntimeError('Verified installed companion is required for baseline capture')
    cert=p.signer(apk);uid=initial['uid'];serial=p.device_key();fingerprint=verified_fingerprint()
    paths=[p.baseline_path(cert,uid),p.connection_baseline_path(cert,uid),p.wake_baseline_path(cert,uid)]
    groups=[(p.PERMISSION,),p.CONNECTION_PERMISSIONS,p.WAKE_PERMISSIONS]
    scopes=['backscreen_permission_v1',p.CONNECTION_SCOPE,p.WAKE_SCOPE]
    missing=[];previous={}
    for index,path in enumerate(paths):
        if not path.exists():missing.append(index);continue
        if path.is_symlink() or not path.is_file():raise RuntimeError('Invalid existing permission baseline file')
        previous[index]=path.read_bytes()
        try:
            prior=json.loads(path.read_text());p.validate_baseline(prior,cert,uid)
            if prior.get('serial',serial)!=serial:raise RuntimeError('Existing permission baseline belongs to another device')
            flags={p.PERMISSION:prior.get('before_flags')} if index==0 else prior.get('before_flags')
            if not isinstance(flags,dict) or set(flags)!=set(groups[index]) or any(type(v)is not int or not -(2**31)<=v<2**31 for v in flags.values()):raise RuntimeError('Existing permission baseline is invalid; no overwrite')
            if index>0 and prior.get('scope')!=scopes[index]:raise RuntimeError('Existing permission baseline scope differs')
            if index==2 and prior.get('owner')!=p.WAKE_OWNER:raise RuntimeError('Existing wake permission owner differs')
            validate_baseline_evidence(prior,cert,uid,groups[index])
        except LegacyPermissionBaseline:
            if origin=='before_activation':
                missing.append(index);continue
            raise
        if force_refresh:missing.append(index)
    if not missing:
        baseline_for(cert,uid)
        return {'status':'already_saved','package':p.PACKAGE,'uid':uid,'permission_changes':False,'existing_baselines_preserved':True}
    target=int(p.shell('pidof','system_server'));boot_id=p.shell('cat','/proc/sys/kernel/random/boot_id')
    code="os.environ['STEPX_SYSTEM_SERVER_PID']="+repr(str(target))+"\n"+payload(cert,uid,{},operation='inspect').decode()
    with p.runtime_lock():
        proof=p.run_bounded_android_worker(code,target)
        answer=json.loads(proof.get('java_result','{}'))
        if not (proof.get('system_server_unchanged') is True and proof.get('system_server_stable') is True and proof.get('detached') is True and proof.get('worker_exit_code')==0
                and answer.get('status')=='success' and answer.get('operation')=='inspect' and answer.get('scope')=='companion_boot_v1'
                and answer.get('signature_verified') is True and answer.get('package')==p.PACKAGE and answer.get('uid')==uid):
            raise RuntimeError('Read-only permission snapshot did not pass identity/process verification')
        if p.shell('pidof','system_server')!=str(target) or p.shell('cat','/proc/sys/kernel/random/boot_id')!=boot_id:raise RuntimeError('Phone restarted during permission snapshot')
        details=answer.get('permissions',{})
        if set(details)!=set(GROUP):raise RuntimeError('Permission snapshot did not cover the exact fixed group')
        measured={}
        for name in GROUP:
            value=details[name];flags=value.get('before_flags');owner=p.WAKE_OWNER if name in p.WAKE_PERMISSIONS else 'android'
            if type(flags)is not int or not -(2**31)<=flags<2**31 or value.get('after_flags')!=flags or value.get('declared') is not True or value.get('signature_permission') is not True or value.get('owner')!=owner:
                raise RuntimeError('Permission snapshot was incomplete or changed flags')
            measured[name]=flags
        # The full read proof is durable before any group-specific recovery file.
        evidence=p.STATE/('baseline-snapshot-'+uuid.uuid4().hex+'.json')
        record={'version':1,'origin':origin,'serial':serial,'signer':cert,'uid':uid,'fingerprint':fingerprint,'boot_id':boot_id,'captured_at':int(time.time()),'permission_changes':False,'proof':proof}
        p.private_json(evidence,record)
        sync_parent(evidence)
        for index in missing:
            path=paths[index]
            if path.exists() and not (origin=='before_activation'):raise RuntimeError('Permission baseline appeared during capture; preserving it and stopping')
            if index in previous:
                if path.read_bytes()!=previous[index]:raise RuntimeError('Permission baseline changed during capture; no overwrite')
                archive=path.with_name(path.stem+'-before-capture-'+uuid.uuid4().hex+'.json')
                # Keep the exact legacy recovery file before replacing it with a
                # newly measured activation boundary (never an assumed zero).
                fd=os.open(archive,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
                with os.fdopen(fd,'wb') as f:f.write(previous[index]);f.flush();os.fsync(f.fileno())
                sync_parent(archive)
            elif path.exists():raise RuntimeError('Permission baseline appeared during capture; no overwrite')
            flags=measured[p.PERMISSION] if index==0 else {name:measured[name] for name in groups[index]}
            value={'signer':cert,'fingerprint':fingerprint,'uid':uid,'serial':serial,'scope':scopes[index],'before_flags':flags,'origin':origin,'captured_at':record['captured_at'],'boot_id':boot_id,'snapshot_proof':str(evidence),'permission_changes':False}
            if index==2:value['owner']=p.WAKE_OWNER
            p.private_json(path,value)
            sync_parent(path)
    baseline_for(cert,uid)
    return {'status':'captured','package':p.PACKAGE,'uid':uid,'origin':origin,'permission_changes':False,'groups_saved':len(missing),'existing_baselines_preserved':not force_refresh,
            'restoration_boundary':'Exact state before this rebind; not a claim about permissions before the first installation' if origin=='before_rebind_snapshot' else 'Exact observed state before activation grants',
            'snapshot_proof':str(evidence)}

def activation_baseline(apk,cert,uid,rebind=False):
    try:return baseline_for(cert,uid,require_exact=True)
    except (MissingPermissionBaselines,LegacyPermissionBaseline):
        if rebind:raise
        capture_baselines(apk,origin='before_activation')
        return baseline_for(cert,uid,require_exact=True)

def current_permissions_ready(initial):
    # Updating an APK can invalidate grants without changing the boot ID/PID.
    return all(initial.get(key) is True for key in ('installed','signature_verified','granted','connection_permissions_granted','wake_permissions_granted'))

def status(apk):
    initial=p.inspect(apk);cert=p.signer(apk)
    result=p.root_json('''import json,os,hashlib
b='/usr/local/lib/stepx-companion';s='/var/lib/stepx-companion';h='/usr/local/lib/stepx-gms/boot.py'
result={'installed':False,'hook_exists':os.path.exists(h)}
if os.path.exists(b+'/config.json'):
 result['config']=json.load(open(b+'/config.json'))
 h=result['config'].get('hook_path',h);result['hook_exists']=os.path.exists(h)
 if os.path.exists(h):
  result['installed']=hashlib.sha256(open(h,'rb').read()).hexdigest()==result['config'].get('hook_sha256',result['config'].get('chain_sha256'))
 else:
  result['installed']=False
  result['skipped_hook']=True
 if os.path.exists(s+'/last.json'):result['last']=json.load(open(s+'/last.json'))
print(json.dumps(result))''')
    if result.get('config') and (result['config']['signer']!=cert or result['config']['uid']!=initial['uid']):raise RuntimeError('Boot adapter belongs to another companion identity')
    current=p.shell('cat','/proc/sys/kernel/random/boot_id')
    result['current_permissions_granted']=current_permissions_ready(initial)
    if not result.get('hook_exists'):
        result['current_boot_ready']=False
        result['current_session_ready']=result['current_permissions_granted']
        result['reboot_verified']=False
        result.update(status='needs_bootstrap',scope='companion_boot_v1',package=p.PACKAGE)
        return result
    last=result.get('last',{});result['current_boot_ready']=result['installed'] and result['current_permissions_granted'] and last.get('boot_id')==current and last.get('target_pid')==int(p.shell('pidof','system_server')) and last.get('status')=='success'
    result['reboot_verified']=bool(result['current_boot_ready'] and result['config'].get('payload_installed_boot',result['config'].get('installed_boot'))!=current)
    result.update(status='success' if result['current_boot_ready'] else 'needs_repair',scope='companion_boot_v1',package=p.PACKAGE)
    return result

def gateway_command(command):
    result=subprocess.run([sys.executable,str(p.WORKFLOW/'tools/gateway.py')],
        input=json.dumps({'command':command,'timeoutMs':15000}),capture_output=True,text=True,timeout=45,
        **p.subprocess_options())
    try:answer=json.loads(result.stdout)
    except ValueError:raise RuntimeError('Boot staging gateway returned no valid result') from None
    if result.returncode or answer.get('exitCode')!=0:raise RuntimeError('Boot staging command failed; system files were not activated')
    return answer

def stage_file(path,data):
    # Bound argv and gateway body size on every OS; a DEX encoded inside a Python
    # payload previously exceeded the gateway's request/exec limits.
    gateway_command(['python3','-c',"import os;fd=os.open("+repr(path)+",os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.close(fd)"])
    for start in range(0,len(data),8192):
        chunk=base64.b64encode(data[start:start+8192]).decode()
        gateway_command(['python3','-c',"import os,base64;fd=os.open("+repr(path)+",os.O_WRONLY|os.O_APPEND|os.O_NOFOLLOW);f=os.fdopen(fd,'ab');f.write(base64.b64decode("+repr(chunk)+"));f.close()"])

def install(apk,rebind=False):
    initial=p.inspect(apk);p.guard(native=True)
    if not initial.get('signature_verified'):raise RuntimeError('Companion signing trust is required')
    cert=p.signer(apk);uid=initial['uid'];fingerprint=verified_fingerprint();baseline=activation_baseline(apk,cert,uid,rebind)
    files={name:read_python_source(ROOT/'scripts/companion_boot'/name) for name in ('chain.py','boot.py','launcher.py')}
    files['grant.py']=payload(cert,uid,baseline)
    serial=p.device_key()
    config={'version':2,'signer':cert,'uid':uid,'package':p.PACKAGE,'fingerprint':fingerprint,'serial':serial,'baseline':baseline,'installed_boot':p.shell('cat','/proc/sys/kernel/random/boot_id'),'grant_sha256':digest(files['grant.py']),'chain_sha256':digest(files['chain.py']),'boot_sha256':digest(files['boot.py']),'launcher_sha256':digest(files['launcher.py']),'original_sha256':ORIGINAL_SHA}
    config['payload_installed_boot']=config['installed_boot']
    hook_check=p.root_json("import os,json; print(json.dumps({'exists':os.path.exists('/usr/local/lib/stepx-gms/boot.py')}))")
    standalone=not hook_check.get('exists')
    config.update(boot_mode='standalone' if standalone else 'chained',
        hook_path='/usr/local/bin/ubuntu_zygote' if standalone else HOOK,
        hook_sha256=digest(files['launcher.py'] if standalone else files['chain.py']),
        original_sha256=STOCK_SHA if standalone else ORIGINAL_SHA)
    files['config.json']=json.dumps(config).encode()
    code='''import os,json,hashlib,base64,stat,fcntl,uuid
BASE='/usr/local/lib/stepx-companion';STATE='/var/lib/stepx-companion';HOOK=CONFIG.get('hook_path','/usr/local/lib/stepx-gms/boot.py')
def read_regular(path):
 fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW)
 with os.fdopen(fd,'rb') as f:
  if not stat.S_ISREG(os.fstat(f.fileno()).st_mode):raise RuntimeError('Expected a regular owned boot file')
  data=f.read(1048577)
 if len(data)>1048576:raise RuntimeError('Owned boot file exceeds recovery bound')
 return data
def sha(p):return hashlib.sha256(read_regular(p)).hexdigest()
def write(path,data,mode=0o600):
 if os.path.islink(path):raise RuntimeError('Refusing a symlink')
 tmp=path+'.new-'+uuid.uuid4().hex
 fd=os.open(tmp,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
 try:
  with os.fdopen(fd,'wb') as f:f.write(data);f.flush();os.fsync(f.fileno())
  os.chmod(tmp,mode);os.replace(tmp,path)
  sync_parent(path)
 finally:
  if os.path.lexists(tmp):os.unlink(tmp)
def sync_parent(path):
 fd=os.open(os.path.dirname(path),os.O_RDONLY|os.O_DIRECTORY);os.fsync(fd);os.close(fd)
def discard(path):
 if os.path.lexists(path):read_regular(path);os.unlink(path);sync_parent(path)
def archive(directory,files,metadata):
 if os.path.islink(directory):raise RuntimeError('Refusing a symlink recovery directory')
 os.makedirs(directory,mode=0o700,exist_ok=True);os.chmod(directory,0o700)
 for name,data in files.items():
  path=directory+'/'+name
  if os.path.lexists(path):
   if read_regular(path)!=data:raise RuntimeError('Existing recovery bytes differ; no overwrite')
  else:write(path,data)
  if read_regular(path)!=data:raise RuntimeError('Recovery read-back failed')
 manifest={'version':1,'status':'archived','files':{name:hashlib.sha256(data).hexdigest() for name,data in files.items()},**metadata}
 data=json.dumps(manifest,sort_keys=True).encode();path=directory+'/archive.json'
 if os.path.lexists(path):
  if read_regular(path)!=data:raise RuntimeError('Existing recovery manifest differs')
 else:write(path,data)
 return directory
for path in (BASE,STATE):
 if os.path.islink(path):raise RuntimeError('Refusing a symlink directory')
 os.makedirs(path,mode=0o700,exist_ok=True);os.chmod(path,0o700)
lockfd=os.open(STATE+'/lock',os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW,0o600)
with os.fdopen(lockfd,'r+b') as lock:
 if not stat.S_ISREG(os.fstat(lock.fileno()).st_mode):raise RuntimeError('Invalid companion boot lock')
 try:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
 except BlockingIOError:raise RuntimeError('Companion boot restoration is active; retry after it finishes')
 if not os.path.isfile(HOOK) or os.path.islink(HOOK):raise RuntimeError('Missing or symlink boot entry')
 current=sha(HOOK)
 standalone=CONFIG.get('boot_mode')=='standalone'
 backup=HOOK+'.stepx-stock-20261021' if standalone else BASE+'/previous-boot.py'
 mode=stat.S_IMODE(os.stat(HOOK).st_mode)
 upgrade=False;identity_rebound=False;recovery=None
 if current==ORIGINAL:
  if os.path.lexists(backup) and sha(backup)!=ORIGINAL:raise RuntimeError('Original backup differs')
  if not os.path.exists(backup):write(backup,read_regular(HOOK),mode)
 else:
  old_bytes=read_regular(BASE+'/config.json');old=json.loads(old_bytes)
  if current!=old.get('hook_sha256',old.get('chain_sha256')) or sha(backup)!=ORIGINAL:raise RuntimeError('Unrecognized boot hook; no changes made')
  changed_identity=old.get('signer')!=CONFIG['signer'] or old.get('uid')!=CONFIG['uid']
  if changed_identity and not ALLOW_REBIND:raise RuntimeError('Boot identity changed; explicit --rebind is required after verified reinstall')
  # Every upgrade/rebind recovery image must match the currently owned config.
  prior={'config.json':old_bytes,'active-hook.py':read_regular(HOOK),'original-boot.py':read_regular(backup)}
  for name,key in [('grant.py','grant_sha256'),('boot.py','boot_sha256'),('chain.py','chain_sha256'),('launcher.py','launcher_sha256')]:
   data=read_regular(BASE+'/'+name);expected=old.get(key)
   if name=='launcher.py' and not expected:expected=old.get('hook_sha256') if standalone else CONFIG.get('launcher_sha256')
   if not expected or hashlib.sha256(data).hexdigest()!=expected:raise RuntimeError('Old boot payload mismatch: '+name)
   prior[name]=data
  if changed_identity:
   if old.get('package')!=CONFIG.get('package') or old.get('package')!='com.stepx.companion.app':raise RuntimeError('Old boot adapter does not belong to this companion package')
   if not CONFIG.get('serial') or old.get('serial')!=CONFIG['serial']:raise RuntimeError('Old boot adapter belongs to another device')
   if old.get('fingerprint') not in ALLOWED_FINGERPRINTS:raise RuntimeError('Old boot firmware was not reviewed')
   if old.get('boot_mode','chained')!=CONFIG.get('boot_mode','chained') or old.get('hook_path',HOOK)!=HOOK:raise RuntimeError('Old boot entry mode differs; refusing implicit relocation')
   if old.get('original_sha256')!=ORIGINAL:raise RuntimeError('Old original backup identity differs')
   if type(old.get('uid')) is not int or not 10000<=old['uid']<100000 or not isinstance(old.get('signer'),str) or len(old['signer'])!=64:raise RuntimeError('Old companion identity is incomplete')
   if os.path.lexists(STATE+'/last.json'):prior['last.json']=read_regular(STATE+'/last.json')
   old_sha=hashlib.sha256(old_bytes).hexdigest();new_sha=hashlib.sha256(json.dumps(CONFIG,sort_keys=True).encode()).hexdigest()
   recovery=archive(STATE+'/rebind-'+old_sha[:16]+'-'+new_sha[:16],prior,{'old_config_sha256':old_sha,'old_uid':old['uid'],'old_signer':old['signer'],'new_uid':CONFIG['uid'],'new_signer':CONFIG['signer'],'new_baseline_is_identity_specific':True})
   # The new identity starts a new boot-verification history. Old permission
   # baselines stay exclusively in recovery; CONFIG already contains new ones.
   CONFIG['rebind_from']={'config_sha256':old_sha,'uid':old['uid'],'signer':old['signer'],'archive':recovery}
   identity_rebound=True;upgrade=True
  else:
   CONFIG['installed_boot']=old['installed_boot']
   upgrade=any(old.get(key)!=CONFIG.get(key) for key in ('grant_sha256','boot_sha256','chain_sha256','hook_sha256','launcher_sha256'))
   if not upgrade:CONFIG['payload_installed_boot']=old.get('payload_installed_boot',CONFIG.get('payload_installed_boot',old['installed_boot']))
   if old.get('rebind_from'):CONFIG['rebind_from']=old['rebind_from']
   if upgrade:
    recovery=STATE+'/upgrade-'+sha(BASE+'/config.json')[:16]
    prior={name:read_regular(BASE+'/'+name) for name in ('grant.py','boot.py','config.json','chain.py','launcher.py') if os.path.exists(BASE+'/'+name)}
    archive(recovery,prior,{'old_config_sha256':sha(BASE+'/config.json')})
 DATA['config.json']=base64.b64encode(json.dumps(CONFIG).encode()).decode()
 # A durable complete image precedes every active write, including metadata-only
 # updates. Restore all files on failure, then restore readiness only when every
 # active byte and mode has been verified. Failed rollback never claims ready.
 last=STATE+'/last.json'
 targets=[(name,BASE+'/'+name) for name in ('grant.py','boot.py','config.json','chain.py','launcher.py')]+[('active-hook.py',HOOK)]
 saved={};entries={}
 for name,path in targets+[('last.json',last)]:
  if os.path.lexists(path):
   saved[name]=read_regular(path);entries[name]={'path':path,'mode':stat.S_IMODE(os.stat(path).st_mode),'sha256':hashlib.sha256(saved[name]).hexdigest()}
  else:entries[name]={'path':path,'absent':True}
 rollback=archive(STATE+'/transaction-'+uuid.uuid4().hex,saved,{'kind':'complete_active_rollback','targets':entries})
 def restore(name):
  entry=entries[name];path=entry['path']
  if entry.get('absent'):
   discard(path)
   if os.path.lexists(path):raise RuntimeError('Absent-file rollback verification failed')
  else:
   data=read_regular(rollback+'/'+name)
   if hashlib.sha256(data).hexdigest()!=entry['sha256']:raise RuntimeError('Rollback snapshot hash mismatch')
   write(path,data,entry['mode'])
   if sha(path)!=entry['sha256'] or stat.S_IMODE(os.stat(path).st_mode)!=entry['mode']:raise RuntimeError('Rollback read-back failed')
 try:
  discard(last)
  for name,path in targets:
   data=base64.b64decode(DATA['launcher.py' if standalone else 'chain.py'] if name=='active-hook.py' else DATA[name])
   write(path,data,mode if name=='active-hook.py' else 0o600)
   if read_regular(path)!=data:raise RuntimeError('Active payload read-back failed: '+name)
  if not upgrade and 'last.json' in saved and json.loads(saved['last.json']).get('status')!='failed':restore('last.json')
  result={'status':'installed','backup_sha256':sha(backup),'hook_sha256':sha(HOOK),'config':CONFIG,'identity_rebound':identity_rebound,'recovery_path':recovery,'rollback_path':rollback}
 except Exception as error:
  errors=[]
  for name,path in targets:
   try:restore(name)
   except Exception as rollback_error:errors.append(name+': '+str(rollback_error))
  if not errors:
   try:restore('last.json')
   except Exception as rollback_error:errors.append('last.json: '+str(rollback_error))
  if errors:
   try:discard(last)
   except Exception as rollback_error:errors.append('invalidate readiness: '+str(rollback_error))
  result={'status':'install_failed','rolled_back':not errors,'error':str(error),'rollback_errors':errors,'recovery_path':rollback,'identity_rebound':False}
  try:write(STATE+'/installation-failure.json',json.dumps(result).encode())
  except Exception as diagnostic_error:result['diagnostic_error']=str(diagnostic_error)
 print(json.dumps(result))
'''

    # Stage large artifacts unprivileged, then copy only hash-verified regular files as root.
    stage='/tmp/stepx-companion-boot-'+uuid.uuid4().hex
    gateway_command(['python3','-c',"import os;os.mkdir("+repr(stage)+",0o700)"])
    for name,data in files.items():stage_file(stage+'/'+name,data)
    hashes={name:digest(data) for name,data in files.items()}
    read='import os,stat,hashlib,base64\nDATA={}\n'
    read+='for name,expected in '+repr(hashes)+'.items():\n'
    read+=' fd=os.open('+repr(stage)+"+'/'+name,os.O_RDONLY|os.O_NOFOLLOW)\n"
    read+=" with os.fdopen(fd,'rb') as f:\n  assert stat.S_ISREG(os.fstat(f.fileno()).st_mode);data=f.read(262144)\n"
    read+=" if hashlib.sha256(data).hexdigest()!=expected:raise RuntimeError('Staged boot artifact mismatch')\n"
    read+=' DATA[name]=base64.b64encode(data).decode()\n'
    operation=('ORIGINAL='+repr(config['original_sha256'])+'\nCONFIG='+repr(config)+'\nALLOW_REBIND='+repr(bool(rebind))+'\nALLOWED_FINGERPRINTS='+repr(tuple(p.ALLOWED_FINGERPRINTS))+'\n'+read+code).encode()
    if len(operation)>32768:raise RuntimeError('Boot transaction exceeds verified size bound')
    operation_path=stage+'/install.py'
    stage_file(operation_path,operation)
    # Keep the bounded root bridge request small; verify the complete transaction before executing it.
    proof=p.root_json('import os,hashlib,stat\np='+repr(operation_path)+"\nfd=os.open(p,os.O_RDONLY|os.O_NOFOLLOW)\nwith os.fdopen(fd,'rb') as f:\n assert stat.S_ISREG(os.fstat(f.fileno()).st_mode); code=f.read(32768)\nassert hashlib.sha256(code).hexdigest()=="+repr(digest(operation))+"\nexec(compile(code,'verified-companion-install','exec'))")
    cleanup="import os; p="+repr(stage)+"\nfor name in "+repr(list(files)+['install.py'])+": os.unlink(p+'/'+name)\nos.rmdir(p)"
    subprocess.run([sys.executable,str(p.WORKFLOW/'tools/gateway.py')],input=json.dumps({'command':['python3','-c',cleanup],'timeoutMs':15000}),capture_output=True,text=True,timeout=45)


    if proof.get('status')=='install_failed':
        private(proof,'installation-failure.json')
        raise RuntimeError('Boot install failed; '+('previous active state restored' if proof.get('rolled_back') is True else 'rollback incomplete; readiness invalidated')+'; recovery: '+str(proof.get('recovery_path')))
    if proof.get('status')!='installed' or proof.get('backup_sha256')!=config['original_sha256'] or proof.get('config',{}).get('signer')!=cert or proof.get('config',{}).get('uid')!=uid:raise RuntimeError('Boot adapter installation read-back failed')
    private(proof,'installation.json');return proof

def remove(apk):
    current=status(apk)
    if not current.get('config'):raise RuntimeError('Owned boot hook not found; nothing changed')
    config=current['config']
    # Refuse an unproven legacy baseline before changing either hook or grants.
    cert=p.signer(apk);uid=p.inspect(apk)['uid'];baseline=baseline_for(cert,uid)
    proof=p.root_json('''import os,json,hashlib
base='/usr/local/lib/stepx-companion';config=json.load(open(base+'/config.json'));hook=config.get('hook_path','/usr/local/lib/stepx-gms/boot.py')
backup=hook+'.stepx-stock-20261021' if config.get('boot_mode')=='standalone' else base+'/previous-boot.py'
if not os.path.exists(hook):
 print(json.dumps({'status':'removed','skipped_hook':True,'reboot_required':False,'permission_baselines_preserved':True}))
 raise SystemExit(0)
config=json.load(open(base+'/config.json'))
def sha(p):return hashlib.sha256(open(p,'rb').read()).hexdigest()
if sha(hook) not in (config.get('hook_sha256',config['chain_sha256']),config['original_sha256']) or sha(backup)!=config['original_sha256']:raise RuntimeError('Boot adapter changed; refusing to overwrite')
fd=os.open(hook+'.restore',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'wb') as f:f.write(open(backup,'rb').read());f.flush();os.fsync(f.fileno())
os.chmod(hook+'.restore',os.stat(hook).st_mode & 0o777);os.replace(hook+'.restore',hook)
print(json.dumps({'status':'removed','original_sha256':sha(hook),'reboot_required':True,'permission_baselines_preserved':True}))''')
    # Restore this adapter's exact permission flags and current-boot freezer membership.
    target=int(p.shell('pidof','system_server'))
    code="os.environ['STEPX_SYSTEM_SERVER_PID']="+repr(str(target))+"\n"+payload(cert,uid,baseline,'revoke').decode()
    with p.runtime_lock():
        worker=p.run_bounded_android_worker(code,target)
    result=json.loads(worker.get('java_result','{}'))
    if result.get('status')!='success' or not worker.get('detached'):raise RuntimeError('Boot rollback read-back failed; recovery records retained')
    proof['runtime']=result.get('runtime');proof['permissions_restored']=True
    private(proof,'removal.json');return proof

def activate(apk,rebind=False):
    """Activate the fixed signed-companion service now, then persist its reviewed boot payload."""
    initial=p.inspect(apk);p.guard(native=True)
    if not initial.get('signature_verified'):raise RuntimeError('Companion signing trust is required')
    cert=p.signer(apk);uid=initial['uid']
    baseline=activation_baseline(apk,cert,uid,rebind)
    target=int(p.shell('pidof','system_server'))
    code="os.environ['STEPX_SYSTEM_SERVER_PID']="+repr(str(target))+"\n"+payload(cert,uid,baseline).decode()
    with p.runtime_lock():proof=p.run_bounded_android_worker(code,target)
    private(proof,'activation.json')
    result=json.loads(proof.get('java_result','{}'))
    if not rebind and result.get('status')=='failed' and 'Permission changed since baseline' in str(result.get('error','')):
        capture_baselines(apk,origin='before_activation',force_refresh=True)
        baseline=baseline_for(cert,uid,require_exact=True)
        target=int(p.shell('pidof','system_server'))
        code="os.environ['STEPX_SYSTEM_SERVER_PID']="+repr(str(target))+"\n"+payload(cert,uid,baseline).decode()
        with p.runtime_lock():proof=p.run_bounded_android_worker(code,target)
        private(proof,'activation.json')
        result=json.loads(proof.get('java_result','{}'))
    monitor=result.get('companion_update',{})
    if not (proof.get('system_server_unchanged') and proof.get('system_server_stable') and proof.get('detached') and proof.get('worker_exit_code')==0 and result.get('status')=='success' and result.get('operation')=='grant' and result.get('signature_verified') and result.get('package')==p.PACKAGE and result.get('uid')==uid and result.get('scope')=='companion_boot_v1' and result.get('all_granted') and result.get('boot_pairing_started') and monitor.get('update_monitor') is True and monitor.get('package')==p.PACKAGE and monitor.get('uid')==uid):
        raise RuntimeError('Companion system activation did not pass readback; recovery evidence retained')
    persistence=install(apk,rebind=rebind)
    if not persistence.get('skipped_hook'):
        record_activation(persistence['config'],target,result)
    return {'status':'activated','package':p.PACKAGE,'service_uid':1000,'signature_verified':True,
            'system_server_unchanged':True,'boot_installed':persistence.get('status')=='installed' and not persistence.get('skipped_hook'),
            'package_update_recovery':True,
            'reboot_verified':False,'identity_rebound':persistence.get('identity_rebound',False),'recovery_path':persistence.get('recovery_path')}

def record_activation(config,target,result):
    """Keep runtime readiness truthful after an upgrade archives the old boot receipt."""
    if int(p.shell('pidof','system_server'))!=target:raise RuntimeError('Android service changed after activation')
    receipt={'status':'success','scope':'companion_boot_v1','source':'verified_manual_activation',
             'target_pid':target,'boot_id':p.shell('cat','/proc/sys/kernel/random/boot_id'),
             'background_exempt':result.get('runtime',{}).get('background_exempt') is True,
             'package_update_recovery':result.get('companion_update',{}).get('update_monitor') is True,
             'stability_verified':True,'version':2}
    code="import os,json,time\nbase="+repr(BASE)+"\nstate="+repr(STATE)+"\nexpected="+repr(config)+"\nreceipt="+repr(receipt)+'''\ncurrent=json.load(open(base+'/config.json'))
if any(current.get(k)!=expected.get(k) for k in ('signer','uid','grant_sha256','payload_installed_boot')):raise RuntimeError('Installed service changed before receipt')
path=state+'/last.json'
if not os.path.exists(path):
 receipt.update(identity=receipt['boot_id'],unix_time=int(time.time()))
 fd=os.open(path+'.new',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
 with os.fdopen(fd,'w') as f:json.dump(receipt,f);f.flush();os.fsync(f.fileno())
 os.replace(path+'.new',path)
print(json.dumps({'recorded':True}))
'''
    if not p.root_json(code).get('recorded'):raise RuntimeError('Runtime readiness receipt was not saved')

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['install','status','remove','activate','capture-baselines']);parser.add_argument('--apk',type=Path,default=ROOT/'desktop/assets/stepx-companion.apk');parser.add_argument('--rebind',action='store_true',help='After an explicitly authorized reinstall, archive and rebind only a fully verified owned companion boot adapter')
    parser.add_argument('--origin',choices=['before_activation','before_rebind_snapshot'],default=None)
    parser.add_argument('--force-refresh',action='store_true',help='Archive prior valid evidence and capture current flags; never revoke permissions')
    args=parser.parse_args()
    if args.rebind and args.command not in ('install','activate'):parser.error('--rebind is only valid with install or activate')
    if (args.origin or args.force_refresh) and args.command!='capture-baselines':parser.error('--origin/--force-refresh require capture-baselines')
    if args.force_refresh and args.origin!='before_activation':parser.error('--force-refresh requires --origin before_activation')
    STATE_LOCAL.mkdir(parents=True,exist_ok=True,mode=0o700)
    with p.file_lock(STATE_LOCAL/'operation.lock'):
        if args.command in ('install','activate'):result={'install':install,'activate':activate}[args.command](args.apk,rebind=args.rebind)
        elif args.command=='capture-baselines':result=capture_baselines(args.apk,origin=args.origin or 'before_rebind_snapshot',force_refresh=args.force_refresh)
        else:result={'status':status,'remove':remove}[args.command](args.apk)
    print(json.dumps(result,ensure_ascii=False))
if __name__=='__main__':
    try:main()
    except Exception as error:print(json.dumps({'status':'failed','error':str(error)},ensure_ascii=False));sys.exit(1)
