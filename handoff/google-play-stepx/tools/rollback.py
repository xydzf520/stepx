import sys
#!/usr/bin/env python3
"""恢复原厂 StepClaw 启动器并移除本次 GMS 安装；会删除本次 Google 本地数据。"""
import pathlib, subprocess, json, time, sys

from common import ROOT, STATE as HERE, ADB, CONFIG, guard, shell
import argparse
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--keep-apps', action='store_true')
args = parser.parse_args()
guard()
BASELINE = json.loads((HERE / 'baseline.json').read_text())
assert BASELINE.get('serial', CONFIG.get('serial', '')) == ADB[-1], 'Wrong device baseline'
INSTALL = json.loads((HERE / 'boot-install.json').read_text())
assert subprocess.check_output(ADB + ['shell', 'getprop', 'ro.build.fingerprint'], text=True).strip() == BASELINE['fingerprint'], '设备固件已变化，停止自动回退'
code = '''import os,json,hashlib,shutil,fcntl,stat
launcher='/usr/local/bin/ubuntu_zygote';stock=launcher+'.stepx-stock-20261021';state='/var/lib/stepx-gms';base='/usr/local/lib/stepx-gms'
current=hashlib.sha256(open(launcher,'rb').read()).hexdigest()
if current==STOCK_SHA:
 print(json.dumps({'restored':True,'already_stock':True}))
else:
 assert current==WRAPPER_SHA, 'Launcher changed since installation; refusing overwrite'
 lock=os.open(state+'/lock',os.O_RDWR);fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
 data=open(stock,'rb').read();assert hashlib.sha256(data).hexdigest()==STOCK_SHA
 temp=launcher+'.stepx-rollback';fd=os.open(temp,os.O_WRONLY|os.O_CREAT|os.O_EXCL,stat.S_IMODE(os.stat(stock).st_mode))
 with os.fdopen(fd,'wb') as f:f.write(data);f.flush();os.fsync(f.fileno())
 os.chown(temp,0,0);os.replace(temp,launcher)
 fd=os.open(os.path.dirname(launcher),os.O_RDONLY|os.O_DIRECTORY);os.fsync(fd);os.close(fd)
 shutil.rmtree(base);shutil.rmtree(state)
 print(json.dumps({'restored':True,'sha256':hashlib.sha256(open(launcher,'rb').read()).hexdigest()}))
'''.replace('STOCK_SHA', repr(INSTALL['original_sha256'])).replace('WRAPPER_SHA', repr(INSTALL['launcher_sha256']))
result = subprocess.run([sys.executable, str(ROOT / 'tools/root_bridge.py')], input=code, text=True, capture_output=True, timeout=45)
proof = json.JSONDecoder().raw_decode(result.stdout.lstrip())[0]
assert proof.get('restored'), '启动器恢复未得到确认；停止后续操作'
print('原厂启动器已恢复。正在重启以撤销运行时权限和应用身份适配。', flush=True)
subprocess.run(ADB + ['reboot'], check=True)
deadline = time.monotonic() + 180
while time.monotonic() < deadline:
    p = subprocess.run(ADB + ['shell', 'getprop', 'sys.boot_completed'], capture_output=True, text=True, timeout=10)
    if p.returncode == 0 and p.stdout.strip() == '1':
        break
    time.sleep(3)
else:
    raise RuntimeError('等待 Android 启动超时；原厂启动器已恢复，但尚未卸载应用')
if args.keep_apps:
    print('已停用启动适配并重启；保留 Google 应用及账号数据，Google 功能可能不再正常。')
    sys.exit(0)
for package in ['com.google.android.deskclock', 'com.google.android.calculator', 'com.android.vending', 'com.google.android.gms', 'com.google.android.gsf']:
    if package in BASELINE['packages']:
        continue  # Preserve any test app that predated this installation.
    installed = subprocess.run(ADB + ['shell', 'pm', 'path', package], capture_output=True, text=True)
    if installed.returncode == 0 and installed.stdout.strip():
        result = subprocess.run(ADB + ['uninstall', package], capture_output=True, text=True, check=True)
        assert 'Success' in result.stdout, package + ': ' + result.stdout
for table, key, value in [('global', 'cn_gms_core', BASELINE['google_core']), ('secure', 'autofill_service', BASELINE['autofill'])]:
    shell('settings', *(['delete', table, key] if value == 'null' else ['put', table, key, value]))
print('本轮 Google 应用与测试时钟、计算器已卸载，原始设置已恢复。私有备份保留在本目录。')
