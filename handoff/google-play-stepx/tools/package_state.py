import sys
import subprocess,pathlib,json,base64,xml.etree.ElementTree as ET
from common import ROOT, ADB as A, STATE as B, guard, init_state
import argparse
parser=argparse.ArgumentParser(description='Read persistent GMS package state without changing package settings.');parser.add_argument('--save-first',action='store_true');args=parser.parse_args()
guard();init_state()
lines=subprocess.check_output(A+['shell','ps','-A','-o','PID,PPID,NAME'],text=True).splitlines();parent=next(int(l.split()[0]) for l in lines if l.split()[-1]=='openclaw_env' and l.split()[1]=='1')
code='import base64,json,os\nh='+repr('/proc/'+str(parent)+'/root')+';p=h+"/data/user/0/com.google.android.gms";s=os.stat(p);print(json.dumps({"inode":s.st_ino,"uid":s.st_uid,"label":os.getxattr(p,"security.selinux").decode().strip(chr(0)),"abx":base64.b64encode(open(h+"/data/system/packages.xml","rb").read()).decode()}))'
p=subprocess.run([sys.executable,str(ROOT/'tools/root_bridge.py')],input=code,text=True,capture_output=True,timeout=40);d=json.JSONDecoder().raw_decode(p.stdout.lstrip())[0];blob=base64.b64decode(d.pop('abx'));f=B/'package-state-current.abx';f.write_bytes(blob);f.chmod(0o600)
subprocess.run(A+['push',str(f),'/data/local/tmp/stepx-gms-current.abx'],check=True,capture_output=True)
subprocess.run(A+['shell','abx2xml','/data/local/tmp/stepx-gms-current.abx','/data/local/tmp/stepx-gms-current.xml'],check=True,capture_output=True)
x=subprocess.check_output(A+['exec-out','cat','/data/local/tmp/stepx-gms-current.xml']);root=ET.fromstring(x);d['persistent_package']=next(n.attrib for n in root.findall('package') if n.attrib['name']=='com.google.android.gms');subprocess.run(A+['shell','rm','-f','/data/local/tmp/stepx-gms-current.abx','/data/local/tmp/stepx-gms-current.xml'],check=True)
(B/'package-state-current.json').write_text(json.dumps(d,indent=2));(B/'package-state-current.json').chmod(0o600);print(json.dumps(d,indent=2))
assert int(d['persistent_package']['publicFlags'])&1==0,'Persistent SYSTEM flag must stay clear'
assert int(d['persistent_package']['privateFlags'])&8==0,'Persistent PRIVILEGED flag must stay clear'

if args.save_first:
 first=B/'package-before-first-reboot.json'
 if not first.exists():
  with first.open('x',encoding='utf-8') as f:json.dump(d,f,indent=2)
  first.chmod(0o600)
