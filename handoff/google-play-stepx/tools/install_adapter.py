from common import baseline_matches, DEVICE_KEY
import sys
#!/usr/bin/env python3
"""Install only the reviewed, fixed-device GMS boot adapter, preserving stock."""
import pathlib, subprocess, json, base64, uuid, hashlib, shutil

from common import ROOT, STATE as BACKUP, ADB, CONFIG, guard
from source_integrity import read_python_source
import argparse
argparse.ArgumentParser(description=__doc__).parse_args()
guard(native=True)
STOCK_SHA = '5cfe5a88a97eca424bd4922fe813e0cc553adad0e8623105311ec1514022152f'
baseline = json.loads((BACKUP / 'baseline.json').read_text())
assert baseline_matches(baseline), 'Wrong device baseline'
assert subprocess.check_output(ADB + ['shell', 'getprop', 'ro.build.fingerprint'], text=True).strip() == baseline['fingerprint']
assert subprocess.check_output(ADB + ['shell', 'getprop', 'persist.openclaw.zygote.path'], text=True).strip() == ''
files = {p.name: read_python_source(p) for p in (ROOT / 'module').glob('*.py')}
assert set(files) == {'launcher.py', 'boot.py', 'inject.py'}
for name, data in files.items():
    compile(data, name, 'exec')
manifest = {name: hashlib.sha256(data).hexdigest() for name, data in files.items()}
stage = '/tmp/stepx-gms-stage-' + uuid.uuid4().hex[:10]
remote = 'import os,base64,json;d=' + repr({n: base64.b64encode(b).decode() for n, b in files.items()}) + ';os.mkdir(' + repr(stage) + ',0o700)\nfor n,b in d.items():\n with open(' + repr(stage) + '+"/"+n,"wb") as f: f.write(base64.b64decode(b))\nprint("staged")'
result = subprocess.run([sys.executable, str(ROOT / 'tools/gateway.py')], input=json.dumps({'command': ['python3', '-c', remote], 'timeoutMs': 15000}), capture_output=True, text=True, timeout=45)
response = json.loads(result.stdout)
assert response.get('exitCode') == 0, 'Staging failed'
code = '''import os,stat,hashlib,json,shutil
stage=STAGE;manifest=MANIFEST
base='/usr/local/lib/stepx-gms';state='/var/lib/stepx-gms';launcher='/usr/local/bin/ubuntu_zygote';stock=launcher+'.stepx-stock-20261021'
if os.path.isdir(base) and os.path.isdir(state) and os.path.isfile(stock):
 def digest(p):return hashlib.sha256(open(p,'rb').read()).hexdigest()
 actual={n:digest(launcher if n=='launcher.py' else base+'/'+n) for n in manifest}
 companion='/usr/local/lib/stepx-companion'
 if actual.get('boot.py')!=manifest['boot.py'] and os.path.isfile(companion+'/config.json'):
  config=json.load(open(companion+'/config.json'))
  assert actual['boot.py']==config['chain_sha256'] and digest(companion+'/previous-boot.py')==manifest['boot.py'], 'Unknown companion chain'
  actual['boot.py']=manifest['boot.py']
 assert actual==manifest and digest(stock)==STOCK_SHA, '启动文件已变化，保留现状；请检测/修复'
 shutil.rmtree(stage)
 print(json.dumps({'installed':True,'already_installed':True,'original_sha256':digest(stock),'launcher_sha256':digest(launcher),'manifest':manifest}))
 raise SystemExit(0)
companion='/usr/local/lib/stepx-companion'
adopt=False
if not os.path.exists(base) and not os.path.exists(state) and os.path.isfile(stock) and os.path.isfile(companion+'/config.json'):
 config=json.load(open(companion+'/config.json'))
 def digest(p):return hashlib.sha256(open(p,'rb').read()).hexdigest()
 assert config.get('boot_mode')=='standalone' and config.get('hook_path')==launcher, 'Unknown companion bootstrap'
 assert digest(launcher)==config['hook_sha256'] and digest(stock)==STOCK_SHA, 'Companion launcher changed'
 assert digest(companion+'/chain.py')==config['chain_sha256'] and digest(companion+'/boot.py')==config['boot_sha256'], 'Companion artifact changed'
 adopt=True
assert not os.path.lexists(base) and not os.path.lexists(state) and (adopt or not os.path.lexists(stock)), '启动模块安装中断，已保留备份；不能覆盖未核对的启动文件'
original=open(stock if adopt else launcher,'rb').read();assert hashlib.sha256(original).hexdigest()==STOCK_SHA
metadata=os.stat(launcher);data={}
for n,h in manifest.items():
 fd=os.open(stage+'/'+n,os.O_RDONLY|os.O_NOFOLLOW)
 with os.fdopen(fd,'rb') as f:
  assert stat.S_ISREG(os.fstat(f.fileno()).st_mode);data[n]=f.read()
 assert hashlib.sha256(data[n]).hexdigest()==h
 compile(data[n],n,'exec')
def write(p,b,mode):
 fd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_EXCL,mode)
 with os.fdopen(fd,'wb') as f:f.write(b);f.flush();os.fsync(f.fileno())
 os.chmod(p,mode);os.chown(p,0,0)
if not adopt:write(stock,original,stat.S_IMODE(metadata.st_mode))
os.mkdir(base,0o700);os.mkdir(state,0o700)
for n in ['boot.py','inject.py']:write(base+'/'+n,data[n],0o600)
write(base+'/manifest.json',json.dumps(manifest).encode(),0o600)
if adopt:
 # Preserve both old companion configuration and the exact Google hook before switching.
 write(companion+'/standalone-config-before-google.json',open(companion+'/config.json','rb').read(),0o600)
 write(companion+'/previous-boot.py',data['boot.py'],0o600)
 config.update(boot_mode='chained',hook_path=base+'/boot.py',hook_sha256=config['chain_sha256'],original_sha256=manifest['boot.py'])
 write(companion+'/config.google-new',json.dumps(config).encode(),0o600)
 write(base+'/boot.chain-new',open(companion+'/chain.py','rb').read(),0o600)
 os.replace(base+'/boot.chain-new',base+'/boot.py')
 os.replace(companion+'/config.google-new',companion+'/config.json')
write(launcher+'.stepx-new',data['launcher.py'],stat.S_IMODE(metadata.st_mode))
os.replace(launcher+'.stepx-new',launcher)
for directory in [base,state,os.path.dirname(launcher)]:
 fd=os.open(directory,os.O_RDONLY|os.O_DIRECTORY);os.fsync(fd);os.close(fd)
shutil.rmtree(stage)
print(json.dumps({'installed':True,'original_sha256':hashlib.sha256(open(stock,'rb').read()).hexdigest(),'launcher_sha256':hashlib.sha256(open(launcher,'rb').read()).hexdigest(),'manifest':manifest}))
'''.replace('STAGE', repr(stage)).replace('MANIFEST', repr(manifest)).replace('STOCK_SHA', repr(STOCK_SHA))
result = subprocess.run([sys.executable, str(ROOT / 'tools/root_bridge.py')], input=code, capture_output=True, text=True, timeout=45)
proof = json.JSONDecoder().raw_decode(result.stdout.lstrip())[0]
assert proof.get('installed') and proof['original_sha256'] == STOCK_SHA
(BACKUP / 'boot-install.json').write_text(json.dumps(proof, indent=2))
(BACKUP / 'boot-install.json').chmod(0o600)
# Save the exact bytes staged and verified above, not checkout-native CRLF.
(BACKUP / 'boot-module').mkdir(exist_ok=True, mode=0o700)
(BACKUP / 'boot-module').chmod(0o700)
for name, data in files.items():
    path = BACKUP / 'boot-module' / name
    path.write_bytes(data)
    path.chmod(0o600)
print('Boot adapter installed; original launcher preserved and hashes verified.')
