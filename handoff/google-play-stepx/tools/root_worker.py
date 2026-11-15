#!/usr/bin/env python3
"""Bounded independent diagnostic child, restores factory launcher before work."""
import pathlib,subprocess,json,sys,time,base64,uuid,os,hashlib
from common import ROOT, ADB, guard, heal_stale_zygote_prop, subprocess_options
import argparse
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('operation');parser.add_argument('output')
args=parser.parse_args()
guard(native=True)
code=pathlib.Path(args.operation).read_text(encoding='utf-8');output=pathlib.Path(args.output);tag=uuid.uuid4().hex[:10];wrapper='/tmp/stepx-gms-worker-'+tag+'.py';result='/data/system/stepx-gms-worker-'+tag+'.json'
DISPATCHER='/usr/local/lib/stepx-gms-worker-dispatch.py';ACTIVE_PTR='/tmp/stepx-gms-worker-active'
def result_path(parent):return repr('/proc/'+str(parent)+'/root'+result)
def sh(s):return subprocess.check_output(ADB+['shell',s],text=True,timeout=30,encoding='utf-8',errors='replace',**subprocess_options()).strip()
def gw(c):
 p=subprocess.run([sys.executable,str(ROOT/'tools/gateway.py')],input=json.dumps({'command':['python3','-c',c],'timeoutMs':15000}),capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=45,**subprocess_options())
 if p.returncode:raise RuntimeError(p.stderr[-500:])
 d=json.loads(p.stdout)
 if d.get('exitCode',0)!=0:raise RuntimeError(str(d)[:500])
 return d
heal_stale_zygote_prop()
assert sh('getprop persist.openclaw.zygote.path')==''
dispatcher_code=r"""#!/usr/bin/python3
import os,re,socket,stat,struct,sys
def clear_props():
 p='/proc/'+str(os.getppid())+'/root/dev/socket/property_service'
 for name in ('persist.openclaw.zygote.path','sys.openclaw.env.start'):
  s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(5)
  try:
   s.connect(p);n=name.encode();s.sendall(struct.pack('<II',0x20001,len(n))+n+struct.pack('<I',0));s.recv(4)
  except OSError:pass
  finally:s.close()
target=None
try:
 fd=os.open('/tmp/stepx-gms-worker-active',os.O_RDONLY|os.O_NOFOLLOW)
 try:
  st=os.fstat(fd)
  if stat.S_ISREG(st.st_mode) and st.st_uid==0 and (st.st_mode&0o777)==0o600 and 0<st.st_size<=256:
   raw=os.read(fd,256).decode('utf-8','strict').strip()
   if re.fullmatch(r'/tmp/stepx-gms-worker-[0-9a-f]{10}\.py',raw):target=raw
 finally:os.close(fd)
 try:os.unlink('/tmp/stepx-gms-worker-active')
 except OSError:pass
except OSError:pass
if target:
 try:
  st=os.lstat(target)
  if stat.S_ISREG(st.st_mode) and os.access(target,os.X_OK):
   os.execv(target,[target]+sys.argv[1:])
 except OSError:pass
clear_props()
os.execv('/usr/local/bin/ubuntu_zygote',['/usr/local/bin/ubuntu_zygote']+sys.argv[1:])
"""
src='''#!/usr/bin/python3
import os,sys,json,socket,struct,ctypes,mmap,time,signal,base64,hashlib,binascii,stat,errno,re,subprocess,select,sqlite3,fcntl
lib=ctypes.CDLL(None,use_errno=True)
host='/proc/'+str(os.getppid())+'/root'
def prop(name,value):
 s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(5)
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
 os.execv('/usr/local/bin/ubuntu_zygote',['/usr/local/bin/ubuntu_zygote']+sys.argv[1:])
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
def stage_source(path,source):
 data=source.encode()
 if not data or len(data)>512*1024:raise RuntimeError('Invalid bounded worker size')
 gw('import os;fd=os.open('+repr(path)+',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.close(fd)')
 try:
  for offset in range(0,len(data),8192):
   chunk=base64.b64encode(data[offset:offset+8192]).decode()
   gw('import os,base64;p='+repr(path)+';assert os.path.getsize(p)=='+str(offset)+';f=open(p,"ab");f.write(base64.b64decode('+repr(chunk)+'));f.close()')
  gw('import os,hashlib;p='+repr(path)+';f=open(p,"rb");digest=hashlib.sha256(f.read()).hexdigest();f.close();assert digest=='+repr(hashlib.sha256(data).hexdigest())+';os.chmod(p,0o755)')
 except Exception:
  try:gw('import os;os.unlink('+repr(path)+')')
  except Exception:pass
  raise
def root(c):
 p=subprocess.run([sys.executable,str(ROOT/'tools/root_bridge.py')],input=c,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=40,**subprocess_options());return p.stdout
def find_parent():
 return next(int(l.split()[0]) for l in sh('ps -A -o PID,PPID,NAME').splitlines() if l.split()[-1]=='openclaw_env' and l.split()[1]=='1')
stage_source(wrapper,src)
parent=find_parent()
dispatch_sha=hashlib.sha256(dispatcher_code.encode('utf-8')).hexdigest()
completed=False
try:
 trigger=(
  'import hashlib,os,socket,stat,struct,sys,tempfile,time\n'
  'disp='+repr(DISPATCHER)+';active='+repr(ACTIVE_PTR)+';wrap='+repr(wrapper)+';body='+repr(dispatcher_code)+'.encode("utf-8")\n'
  'st=os.lstat(wrap)\n'
  'assert stat.S_ISREG(st.st_mode)\n'
  'os.makedirs(os.path.dirname(disp),mode=0o755,exist_ok=True)\n'
  'need=True\n'
  'if os.path.lexists(disp):\n'
  ' dst=os.lstat(disp)\n'
  ' assert stat.S_ISREG(dst.st_mode)\n'
  ' if (dst.st_mode&0o777)==0o755 and dst.st_uid==0 and dst.st_gid==0:\n'
  '  with open(disp,"rb") as f:\n'
  '   need=hashlib.sha256(f.read()).hexdigest()!='+repr(dispatch_sha)+'\n'
  'if need:\n'
  ' fd,tmp=tempfile.mkstemp(prefix=".stepx-disp-",dir=os.path.dirname(disp))\n'
  ' try:\n'
  '  os.write(fd,body);os.fchmod(fd,0o755);os.fchown(fd,0,0);os.fsync(fd);os.close(fd);fd=-1\n'
  '  os.replace(tmp,disp)\n'
  ' finally:\n'
  '  if fd>=0:os.close(fd)\n'
  '  if os.path.exists(tmp):os.unlink(tmp)\n'
  'fd,atmp=tempfile.mkstemp(prefix=".stepx-active-",dir="/tmp")\n'
  'try:\n'
  ' os.write(fd,(wrap+"\\n").encode("utf-8"));os.fchmod(fd,0o600);os.fchown(fd,0,0);os.fsync(fd);os.close(fd);fd=-1\n'
  ' if os.path.islink(active):raise RuntimeError("Symlink active worker pointer")\n'
  ' os.replace(atmp,active)\n'
  'finally:\n'
  ' if fd>=0:os.close(fd)\n'
  ' if os.path.exists(atmp):os.unlink(atmp)\n'
  'p="/proc/'+str(parent)+'/root/dev/socket/property_service"\n'
  'def setp(name,value):\n'
  ' s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(5)\n'
  ' try:\n'
  '  s.connect(p);n=name.encode();v=value.encode();s.sendall(struct.pack("<II",0x20001,len(n))+n+struct.pack("<I",len(v))+v);r=s.recv(4)\n'
  '  assert len(r)==4 and struct.unpack("<I",r)[0]==0\n'
  ' finally:s.close()\n'
  'setp("persist.openclaw.zygote.path",disp)\n'
  'if os.fork()==0:\n'
  ' try:\n'
  '  os.setsid()\n'
  '  for fd in (0,1,2):\n'
  '   try:os.close(fd)\n'
  '   except OSError:pass\n'
  '  time.sleep(0.25)\n'
  '  setp("sys.openclaw.env.start","1")\n'
  ' except Exception:pass\n'
  ' os._exit(0)\n'
  'print("ok");sys.stdout.flush()\n'
 )
 if 'ok' not in root(trigger):raise RuntimeError('Failed to dispatch bounded worker')
 print('Bounded worker dispatched.',flush=True)
 time.sleep(1.5)
 for attempt in range(22):
  try:
   parent=find_parent()
   s=root('import os\np='+result_path(parent)+'\ndata=open(p,encoding="utf-8").read()\nfor item in (p,'+repr(ACTIVE_PTR)+','+repr(wrapper)+'):\n try:\n  if os.path.lexists(item) and not os.path.islink(item):os.unlink(item)\n except OSError:pass\nprint(data)')
   d=json.JSONDecoder().raw_decode(s.lstrip())[0];assert isinstance(d,dict) and 'configuration_restored' in d
   output.write_text(json.dumps(d,indent=2),encoding='utf-8');output.chmod(0o600);completed=True;print('Saved',output,'error',d.get('error'));print('Device result',result);break
  except Exception:
   if attempt==21:raise
   time.sleep(2)
finally:
 if not completed:
  try:gw('import os\nif os.path.lexists('+repr(wrapper)+'):os.unlink('+repr(wrapper)+')')
  except Exception:pass
 try:
  prop_left=sh('getprop persist.openclaw.zygote.path')!=''
 except Exception:
  prop_left=not completed
 if not completed or prop_left:
  try:
   parent=find_parent()
   cleanup=(
    'import os,socket,struct\n'
    'for item in ('+repr(ACTIVE_PTR)+','+repr(wrapper)+','+result_path(parent)+'):\n'
    ' try:\n'
    '  if os.path.lexists(item) and not os.path.islink(item):os.unlink(item)\n'
    ' except OSError:pass\n'
    'p="/proc/'+str(parent)+'/root/dev/socket/property_service"\n'
    'for name in ("persist.openclaw.zygote.path","sys.openclaw.env.start"):\n'
    ' s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(5)\n'
    ' try:\n'
    '  s.connect(p);n=name.encode();s.sendall(struct.pack("<II",0x20001,len(n))+n+struct.pack("<I",0));s.recv(4)\n'
    ' except OSError:pass\n'
    ' finally:s.close()\n'
    'print("cleaned")\n'
   )
   root(cleanup)
  except Exception:pass
