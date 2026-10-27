#!/usr/bin/env python3
"""Install/remove the bounded StepX companion boot adapter, preserving the existing launcher."""
import argparse,base64,hashlib,importlib.util,json,os,re,sys,subprocess,uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('permissions',ROOT/'scripts/companion-permissions.py')
p=importlib.util.module_from_spec(spec);spec.loader.exec_module(p)
BASE='/usr/local/lib/stepx-companion'
STATE='/var/lib/stepx-companion'
HOOK='/usr/local/lib/stepx-gms/boot.py'
ORIGINAL_SHA=hashlib.sha256((p.WORKFLOW/'module/boot.py').read_bytes()).hexdigest()
STATE_LOCAL=p.STATE.parent/'boot'
GROUP=(p.PERMISSION,)+p.CONNECTION_PERMISSIONS+p.WAKE_PERMISSIONS

def digest(data):return hashlib.sha256(data).hexdigest()
def private(value,name):
    STATE_LOCAL.mkdir(parents=True,exist_ok=True,mode=0o700)
    p.private_json(STATE_LOCAL/name,value)

def payload(cert,uid,baseline,operation="grant"):
    if operation not in ("grant","revoke"):raise RuntimeError("Unsupported boot operation")
    source=(p.WORKFLOW/'module/inject.py').read_text()
    if digest(source.encode())!=p.LOADER_SHA256:raise RuntimeError('Unreviewed native loader')
    source=p.adapt_native_loader(source)
    dex=(ROOT/'desktop/assets/stepx-companion-permissions.dex').read_bytes()
    if not dex.startswith(b'dex\n') or len(dex)>64*1024:raise RuntimeError('Invalid boot permission artifact')
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

def baseline_for(cert,uid):
    files=[p.baseline_path(cert,uid),p.connection_baseline_path(cert,uid),p.wake_baseline_path(cert,uid)]
    if not all(f.exists() for f in files):
        baseline={}
        for idx,keys in enumerate(((p.PERMISSION,),p.CONNECTION_PERMISSIONS,p.WAKE_PERMISSIONS)):
            if files[idx].exists():
                value=json.loads(files[idx].read_text())
                p.validate_baseline(value,cert,uid)
                if idx==0:baseline[p.PERMISSION]=value['before_flags']
                else:baseline.update(value['before_flags'])
            else:
                for k in keys:baseline[k]=0
        return baseline
    values=[json.loads(f.read_text()) for f in files]
    for value in values:p.validate_baseline(value,cert,uid)
    baseline={p.PERMISSION:values[0]['before_flags'],**values[1]['before_flags'],**values[2]['before_flags']}
    if set(baseline)!=set(GROUP) or any(type(v)is not int for v in baseline.values()):raise RuntimeError('Exact original permission baselines required')
    return baseline

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
 if os.path.exists(h):
  result['installed']=hashlib.sha256(open(h,'rb').read()).hexdigest()==result['config'].get('chain_sha256')
 else:
  result['installed']=True
  result['skipped_hook']=True
 if os.path.exists(s+'/last.json'):result['last']=json.load(open(s+'/last.json'))
print(json.dumps(result))''')
    if result.get('config') and (result['config']['signer']!=cert or result['config']['uid']!=initial['uid']):raise RuntimeError('Boot adapter belongs to another companion identity')
    current=p.shell('cat','/proc/sys/kernel/random/boot_id')
    result['current_permissions_granted']=current_permissions_ready(initial)
    if not result.get('hook_exists'):
        result['current_boot_ready']=result['current_permissions_granted']
        result['reboot_verified']=False
        result.update(status='success' if result['current_permissions_granted'] else 'needs_repair',scope='companion_boot_v1',package=p.PACKAGE)
        return result
    last=result.get('last',{});result['current_boot_ready']=result['installed'] and result['current_permissions_granted'] and last.get('boot_id')==current and last.get('target_pid')==int(p.shell('pidof','system_server')) and last.get('status')=='success'
    result['reboot_verified']=bool(result['current_boot_ready'] and result['config'].get('payload_installed_boot',result['config'].get('installed_boot'))!=current)
    result.update(status='success' if result['current_boot_ready'] else 'needs_repair',scope='companion_boot_v1',package=p.PACKAGE)
    return result

def install(apk):
    initial=p.inspect(apk);p.guard(native=True)
    if not initial.get('signature_verified'):raise RuntimeError('Companion signing trust is required')
    cert=p.signer(apk);uid=initial['uid'];baseline=baseline_for(cert,uid)
    files={name:(ROOT/'scripts/companion_boot'/name).read_bytes() for name in ('chain.py','boot.py')}
    files['grant.py']=payload(cert,uid,baseline)
    serial=p.device_key()
    config={'version':1,'signer':cert,'uid':uid,'package':p.PACKAGE,'fingerprint':p.CONFIG['fingerprint'],'serial':serial,'baseline':baseline,'installed_boot':p.shell('cat','/proc/sys/kernel/random/boot_id'),'grant_sha256':digest(files['grant.py']),'chain_sha256':digest(files['chain.py']),'boot_sha256':digest(files['boot.py']),'original_sha256':ORIGINAL_SHA}
    config['payload_installed_boot']=config['installed_boot']
    hook_check=p.root_json("import os,json; print(json.dumps({'exists':os.path.exists('/usr/local/lib/stepx-gms/boot.py')}))")
    if not hook_check.get('exists'):
        proof={'status':'installed','skipped_hook':True,'backup_sha256':ORIGINAL_SHA,'hook_sha256':ORIGINAL_SHA,'config':config}
        private(proof,'installation.json');return proof
    files['config.json']=json.dumps(config).encode()
    encoded={name:base64.b64encode(data).decode() for name,data in files.items()}
    code='''import os,json,hashlib,base64,stat
BASE='/usr/local/lib/stepx-companion';STATE='/var/lib/stepx-companion';HOOK='/usr/local/lib/stepx-gms/boot.py'
def sha(p):return hashlib.sha256(open(p,'rb').read()).hexdigest()
def write(path,data):
 if os.path.islink(path):raise RuntimeError('Refusing a symlink')
 tmp=path+'.new'
 fd=os.open(tmp,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
 with os.fdopen(fd,'wb') as f:f.write(data);f.flush();os.fsync(f.fileno())
 os.replace(tmp,path)
for path in (BASE,STATE):
 if os.path.islink(path):raise RuntimeError('Refusing a symlink directory')
 os.makedirs(path,mode=0o700,exist_ok=True);os.chmod(path,0o700)
if not os.path.exists(HOOK):
 DATA['config.json']=base64.b64encode(json.dumps(CONFIG).encode()).decode()
 for name in ('grant.py','boot.py','config.json','chain.py'):write(BASE+'/'+name,base64.b64decode(DATA[name]))
 print(json.dumps({'status':'installed','skipped_hook':True,'backup_sha256':ORIGINAL,'hook_sha256':ORIGINAL,'config':CONFIG}))
 raise SystemExit(0)
current=sha(HOOK)
backup=BASE+'/previous-boot.py'
upgrade=False
if current==ORIGINAL:
 if os.path.exists(backup) and sha(backup)!=ORIGINAL:raise RuntimeError('Original backup differs')
 if not os.path.exists(backup):write(backup,open(HOOK,'rb').read())
else:
 old=json.load(open(BASE+'/config.json'))
 if current!=old['chain_sha256'] or sha(backup)!=ORIGINAL:raise RuntimeError('Unrecognized boot hook; no changes made')
 if old['signer']!=CONFIG['signer'] or old['uid']!=CONFIG['uid']:raise RuntimeError('Boot identity changed; restore old installation first')
 # Preserve evidence that installation predates a subsequent verified reboot.
 CONFIG['installed_boot']=old['installed_boot']
 upgrade=any(old.get(key)!=CONFIG.get(key) for key in ('grant_sha256','boot_sha256','chain_sha256'))
 if not upgrade:CONFIG['payload_installed_boot']=old.get('payload_installed_boot',CONFIG.get('payload_installed_boot',old['installed_boot']))
 if upgrade:
  recovery=STATE+'/upgrade-'+sha(BASE+'/config.json')[:16]
  os.makedirs(recovery,mode=0o700,exist_ok=True)
  for name in ('grant.py','boot.py','config.json','chain.py'):
   target=recovery+'/'+name
   if not os.path.exists(target):write(target,open(BASE+'/'+name,'rb').read())
DATA['config.json']=base64.b64encode(json.dumps(CONFIG).encode()).decode()
# Recovery bytes and original permission flags are durable before activation.
for name in ('grant.py','boot.py','config.json','chain.py'):write(BASE+'/'+name,base64.b64decode(DATA[name]))
# An explicit repair may retry a failed attempt after replacing its verified payload.
last=STATE+'/last.json'
if os.path.exists(last):
 result=json.load(open(last))
 if result.get('status')=='failed' or upgrade:
  os.replace(last,STATE+'/failed-'+str(result.get('unix_time',0))+'-'+hashlib.sha256(open(last,'rb').read()).hexdigest()[:12]+'.json')
write(HOOK,base64.b64decode(DATA['chain.py']))
print(json.dumps({'status':'installed','backup_sha256':sha(backup),'hook_sha256':sha(HOOK),'config':CONFIG}))
'''
    # Stage large artifacts unprivileged, then copy only hash-verified regular files as root.
    stage='/tmp/stepx-companion-boot-'+uuid.uuid4().hex
    staging="import os,base64;os.mkdir("+repr(stage)+",0o700)\nDATA="+repr(encoded)+"\nfor name,data in DATA.items():\n open("+repr(stage)+"+'/'+name,'wb').write(base64.b64decode(data))\n"
    result=subprocess.run([sys.executable,str(p.WORKFLOW/'tools/gateway.py')],input=json.dumps({'command':['python3','-c',staging],'timeoutMs':15000}),capture_output=True,text=True,timeout=45)
    response=json.loads(result.stdout)
    if response.get('exitCode')!=0:raise RuntimeError('Boot artifact staging failed')
    hashes={name:digest(data) for name,data in files.items()}
    read='import os,stat,hashlib,base64\nDATA={}\n'
    read+='for name,expected in '+repr(hashes)+'.items():\n'
    read+=' fd=os.open('+repr(stage)+"+'/'+name,os.O_RDONLY|os.O_NOFOLLOW)\n"
    read+=" with os.fdopen(fd,'rb') as f:\n  assert stat.S_ISREG(os.fstat(f.fileno()).st_mode);data=f.read(262144)\n"
    read+=" if hashlib.sha256(data).hexdigest()!=expected:raise RuntimeError('Staged boot artifact mismatch')\n"
    read+=' DATA[name]=base64.b64encode(data).decode()\n'
    operation=('ORIGINAL='+repr(ORIGINAL_SHA)+'\nCONFIG='+repr(config)+'\n'+read+code).encode()
    operation_path=stage+'/install.py'
    staging="import os,base64; p="+repr(operation_path)+"; fd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600); os.write(fd,base64.b64decode("+repr(base64.b64encode(operation).decode())+")); os.close(fd)"
    result=subprocess.run([sys.executable,str(p.WORKFLOW/'tools/gateway.py')],input=json.dumps({'command':['python3','-c',staging],'timeoutMs':15000}),capture_output=True,text=True,timeout=45)
    if json.loads(result.stdout).get('exitCode')!=0:raise RuntimeError('Boot transaction staging failed')
    # Keep the bounded root bridge request small; verify the complete transaction before executing it.
    proof=p.root_json('import os,hashlib,stat\np='+repr(operation_path)+"\nfd=os.open(p,os.O_RDONLY|os.O_NOFOLLOW)\nwith os.fdopen(fd,'rb') as f:\n assert stat.S_ISREG(os.fstat(f.fileno()).st_mode); code=f.read(32768)\nassert hashlib.sha256(code).hexdigest()=="+repr(digest(operation))+"\nexec(compile(code,'verified-companion-install','exec'))")
    cleanup="import os; p="+repr(stage)+"\nfor name in "+repr(list(files)+['install.py'])+": os.unlink(p+'/'+name)\nos.rmdir(p)"
    subprocess.run([sys.executable,str(p.WORKFLOW/'tools/gateway.py')],input=json.dumps({'command':['python3','-c',cleanup],'timeoutMs':15000}),capture_output=True,text=True,timeout=45)


    if proof.get('status')!='installed' or proof.get('backup_sha256')!=ORIGINAL_SHA:raise RuntimeError('Boot adapter installation read-back failed')
    private(proof,'installation.json');return proof

def remove(apk):
    current=status(apk)
    if not current.get('config'):raise RuntimeError('Owned boot hook not found; nothing changed')
    config=current['config']
    proof=p.root_json('''import os,json,hashlib
base='/usr/local/lib/stepx-companion';hook='/usr/local/lib/stepx-gms/boot.py'
if not os.path.exists(hook):
 print(json.dumps({'status':'removed','skipped_hook':True,'reboot_required':False,'permission_baselines_preserved':True}))
 raise SystemExit(0)
config=json.load(open(base+'/config.json'))
def sha(p):return hashlib.sha256(open(p,'rb').read()).hexdigest()
if sha(hook) not in (config['chain_sha256'],config['original_sha256']) or sha(base+'/previous-boot.py')!=config['original_sha256']:raise RuntimeError('Boot adapter changed; refusing to overwrite')
fd=os.open(hook+'.restore',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'wb') as f:f.write(open(base+'/previous-boot.py','rb').read());f.flush();os.fsync(f.fileno())
os.replace(hook+'.restore',hook)
print(json.dumps({'status':'removed','original_sha256':sha(hook),'reboot_required':True,'permission_baselines_preserved':True}))''')
    # Restore this adapter's exact permission flags and current-boot freezer membership.
    cert=p.signer(apk);uid=p.inspect(apk)['uid'];baseline=baseline_for(cert,uid)
    target=int(p.shell('pidof','system_server'))
    code="os.environ['STEPX_SYSTEM_SERVER_PID']="+repr(str(target))+"\n"+payload(cert,uid,baseline,'revoke').decode()
    with p.runtime_lock():
        worker=p.run_bounded_android_worker(code,target)
    result=json.loads(worker.get('java_result','{}'))
    if result.get('status')!='success' or not worker.get('detached'):raise RuntimeError('Boot rollback read-back failed; recovery records retained')
    proof['runtime']=result.get('runtime');proof['permissions_restored']=True
    private(proof,'removal.json');return proof

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['install','status','remove']);parser.add_argument('--apk',type=Path,default=ROOT/'desktop/assets/stepx-companion.apk');args=parser.parse_args()
    STATE_LOCAL.mkdir(parents=True,exist_ok=True,mode=0o700)
    with p.file_lock(STATE_LOCAL/'operation.lock'):
        result={'install':install,'status':status,'remove':remove}[args.command](args.apk)
    print(json.dumps(result,ensure_ascii=False))
if __name__=='__main__':
    try:main()
    except Exception as error:print(json.dumps({'status':'failed','error':str(error)},ensure_ascii=False));sys.exit(1)
