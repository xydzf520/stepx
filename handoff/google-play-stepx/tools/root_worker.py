#!/usr/bin/env python3
"""Bounded independent diagnostic child, restores factory launcher before work."""
import pathlib,subprocess,json,sys,time,base64,uuid,os
from common import ROOT, ADB, guard
import argparse
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('operation');parser.add_argument('output')
args=parser.parse_args()
guard(native=True)
code=pathlib.Path(args.operation).read_text();output=pathlib.Path(args.output);tag=uuid.uuid4().hex[:10];wrapper='/tmp/stepx-gms-worker-'+tag+'.py';result='/data/system/stepx-gms-worker-'+tag+'.json'
def sh(s):return subprocess.check_output(ADB+['shell',s],text=True).strip()
def gw(c):
 p=subprocess.run([sys.executable,str(ROOT/'tools/gateway.py')],input=json.dumps({'command':['python3','-c',c],'timeoutMs':15000}),capture_output=True,text=True,timeout=45)
 if p.returncode:raise RuntimeError(p.stderr[-500:])
 d=json.loads(p.stdout)
 if d.get('exitCode',0)!=0:raise RuntimeError(str(d)[:500])
 return d
assert sh('getprop persist.openclaw.zygote.path')==''
src='''#!/usr/bin/python3
import os,sys,json,socket,struct,ctypes,mmap,time,signal,base64,hashlib,binascii,stat,errno,re,subprocess,select,sqlite3,fcntl
lib=ctypes.CDLL(None,use_errno=True)
host='/proc/'+str(os.getppid())+'/root'
def prop(name,value):
 s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(3)
 try:
  s.connect(host+'/dev/socket/property_service');n=name.encode();v=value.encode();s.sendall(struct.pack('<II',0x20001,len(n))+n+struct.pack('<I',len(v))+v)
  r=s.recv(4)
  if len(r)!=4 or struct.unpack('<I',r)[0]!=0:raise RuntimeError('property restore failed')
 finally:s.close()
prop('persist.openclaw.zygote.path','');prop('sys.openclaw.env.start','')
# Keep the existing boot adapters out of this diagnostic operation. File locks
# survive fork in the worker; the factory launcher closes its copies on exec.
boot_locks=[];lock_error=None
try:
 for path in ('/var/lib/stepx-gms/lock','/var/lib/stepx-companion/lock'):
  if not os.path.isdir(os.path.dirname(path)):continue
  fd=os.open(path,os.O_RDWR|os.O_CREAT,0o600);boot_locks.append(fd)
  fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
except OSError:lock_error='Existing boot restoration is active; no diagnostic executed'
child=os.fork()
if child:
 os.execv('/usr/local/bin/ubuntu_zygote',['/usr/local/bin/ubuntu_zygote'])
# Independent short-lived worker. Leave service cgroup/session before parent replacement.
os.setsid()
fd=os.open(host+'/sys/fs/cgroup/cgroup.procs',os.O_WRONLY);os.write(fd,str(os.getpid()).encode());os.close(fd)
os.chroot(host);os.chdir('/')
os.setresgid(1000,0,0);os.setresuid(1000,0,0);lib.setfsgid(1000);lib.setfsuid(1000)
for domain in ['u:r:system_server_startup:s0','u:r:system_server:s0']:
 fd=os.open('/proc/self/attr/current',os.O_WRONLY);os.write(fd,domain.encode());os.close(fd)
def expired(a,b):raise TimeoutError('worker deadline')
signal.signal(signal.SIGALRM,expired);signal.alarm(25)
out={'configuration_restored':True,'uid':os.getuid(),'euid':os.geteuid(),'context':open('/proc/self/attr/current').read().strip('\\x00\\n')}
try:
 if lock_error:raise RuntimeError(lock_error)
 exec(CODE,globals(),locals())
except BaseException as e:out['error']=repr(e)
finally:
 signal.alarm(0);lib.setfsuid(1000)
 fd=os.open(RESULT,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
 try:os.write(fd,json.dumps(out).encode());os.fsync(fd)
 finally:os.close(fd)
 os._exit(0)
'''.replace('signal.alarm(25)', 'signal.alarm('+str(min(60,int(os.environ.get('STEPX_WORKER_TIMEOUT','25'))))+')').replace('CODE',repr(code)).replace('RESULT',repr(result))
gw('import os,base64;p='+repr(wrapper)+';open(p,"wb").write(base64.b64decode('+repr(base64.b64encode(src.encode()).decode())+'));os.chmod(p,0o755)')
def root(c):
 p=subprocess.run([sys.executable,str(ROOT/'tools/root_bridge.py')],input=c,capture_output=True,text=True,timeout=40);return p.stdout
parent=next(int(l.split()[0]) for l in sh('ps -A -o PID,PPID,NAME').splitlines() if l.split()[-1]=='openclaw_env' and l.split()[1]=='1')
trigger="import socket,struct\np='/proc/"+str(parent)+"/root/dev/socket/property_service'\nfor name,value in [('persist.openclaw.zygote.path',"+repr(wrapper)+"),('sys.openclaw.env.start','1')]:\n s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.connect(p);n=name.encode();v=value.encode();s.sendall(struct.pack('<II',0x20001,len(n))+n+struct.pack('<I',len(v))+v);r=s.recv(4);s.close();assert len(r)==4 and struct.unpack('<I',r)[0]==0\n"
root(trigger);print('Bounded worker dispatched.',flush=True)
for attempt in range(18):
 try:
  parent=next(int(l.split()[0]) for l in sh('ps -A -o PID,PPID,NAME').splitlines() if l.split()[-1]=='openclaw_env' and l.split()[1]=='1')
  s=root('print(open('+repr('/proc/'+str(parent)+'/root'+result)+').read())');d=json.JSONDecoder().raw_decode(s.lstrip())[0];assert isinstance(d,dict) and 'configuration_restored' in d
  output.write_text(json.dumps(d,indent=2));output.chmod(0o600);gw('import os;os.unlink('+repr(wrapper)+')');print('Saved',output,'error',d.get('error'));print('Device result',result);break
 except Exception as e:
  if attempt==17:raise
  time.sleep(2)
