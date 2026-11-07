"""Bounded, identity-pinned boot restoration. No socket, arbitrary command or root daemon."""
import os,json,time,signal,hashlib,base64,ctypes,struct,mmap,fcntl,subprocess,re
BASE='/usr/local/lib/stepx-companion'
STATE='/var/lib/stepx-companion'
lockfd=os.open(STATE+'/lock',os.O_RDWR|os.O_CREAT,0o600)
try:fcntl.flock(lockfd,fcntl.LOCK_EX|fcntl.LOCK_NB)
except BlockingIOError:os._exit(0)
config=json.load(open(BASE+'/config.json'))
host='/proc/self/fd/'+str(rootfd)
boot=open(host+'/proc/sys/kernel/random/boot_id').read().strip()
identity=boot
def save(value):
    value.update(identity=identity,boot_id=boot,unix_time=int(time.time()),version=2)
    p=STATE+'/last.new'
    fd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
    with os.fdopen(fd,'w') as stream:json.dump(value,stream);stream.flush();os.fsync(stream.fileno())
    os.replace(p,STATE+'/last.json')
try:old=json.load(open(STATE+'/last.json'))
except (OSError,ValueError):old={}
# A native failure is never automatically retried in the same boot.
if old.get('boot_id')==boot and old.get('status') in ('started','failed'):os._exit(0)
payload=open(BASE+'/grant.py').read()
if hashlib.sha256(payload.encode()).hexdigest()!=config['grant_sha256']:raise RuntimeError('Boot payload integrity mismatch')
save({'status':'started','scope':'companion_boot_v1','target_pid':0})
shared=mmap.mmap(-1,256*1024,flags=mmap.MAP_SHARED|mmap.MAP_ANONYMOUS)
lib=ctypes.CDLL(None,use_errno=True)
json.dumps({});hashlib.sha256(b'');base64.b64decode('AA==')
child=os.fork()
if child==0:
    out={}
    def expired(a,b):raise TimeoutError('Companion boot deadline')
    signal.signal(signal.SIGALRM,expired);signal.alarm(35)
    try:
        os.fchdir(rootfd);os.chroot('.');os.chdir('/')
        os.setresgid(1000,0,0);os.setresuid(1000,0,0);lib.setfsgid(1000);lib.setfsuid(1000)
        for domain in ('u:r:system_server_startup:s0','u:r:system_server:s0'):
            with open('/proc/self/attr/current','w') as stream:stream.write(domain)
        lib.setfsuid(0)
        candidates=[]
        for pid in os.listdir('/proc'):
            if not pid.isdigit():continue
            try:
                with open('/proc/'+pid+'/comm') as f:
                    if f.read().strip()=='system_server':candidates.append(int(pid))
            except OSError:pass
        if len(candidates)!=1:raise RuntimeError('No unique Android service after entering host context')
        target=candidates[0];out['target_pid']=target
        # Resolve inside Android instead of depending on a Google boot receipt.
        if old.get('boot_id')==boot and old.get('status')=='success' and old.get('target_pid')==target:
            out.update(old);out['already_restored']=True
            raise SystemExit()
        build=subprocess.check_output(['/system/bin/getprop','ro.build.fingerprint'],text=True,timeout=3).strip()
        if build!=config['fingerprint']:raise RuntimeError('Firmware changed; adapter must be re-evaluated')
        os.environ['STEPX_SYSTEM_SERVER_PID']=str(target)
        exec(compile(payload,'companion-fixed-boot-grant','exec'),globals())
        result=json.loads(out.get('java_result','{}'))
        monitor=result.get('companion_update',{})
        if result.get('status')!='success' or not result.get('all_granted') or not result.get('boot_pairing_started') or not out.get('detached') or monitor.get('update_monitor') is not True or monitor.get('package')!=config['package'] or monitor.get('uid')!=config['uid']:raise RuntimeError(result.get('error','Boot grant read-back failed'))
        out['grant']=result
        out['package_update_recovery']=True
        out['background_exempt']=result.get('runtime',{}).get('background_exempt') is True
        if not out['background_exempt']:raise RuntimeError('Background exemption read-back failed')
        # Check from Android's system domain: Ubuntu cannot stat Android tasks.
        for _ in range(20):
            time.sleep(1)
            if not os.path.exists('/proc/'+str(target)+'/comm') or open('/proc/'+str(target)+'/comm').read().strip()!='system_server':
                raise RuntimeError('Android service restarted after restoration')
        out['stability_verified']=True
        out['status']='success'
    except SystemExit:pass
    except BaseException as error:out.update(status='failed',error=repr(error))
    finally:
        signal.alarm(0)
        data=json.dumps(out).encode()
        if len(data)>len(shared)-8:data=json.dumps({'status':'failed','error':'Oversized boot result'}).encode()
        shared[4:4+len(data)]=data;shared[:4]=len(data).to_bytes(4,'little');os._exit(0)
deadline=time.monotonic()+42
while True:
    done,status=os.waitpid(child,os.WNOHANG)
    if done:break
    if time.monotonic()>deadline:
        os.kill(child,signal.SIGKILL);os.waitpid(child,0);save({'status':'failed','error':'Boot worker hard deadline'});os._exit(1)
    time.sleep(.05)
length=int.from_bytes(shared[:4],'little')
result=json.loads(shared[4:4+length]) if 0<length<len(shared)-8 else {'status':'failed','error':'No boot result','child_status':status}
save(result)
