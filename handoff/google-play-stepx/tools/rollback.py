#!/usr/bin/env python3
"""恢复原厂 StepClaw 启动器并移除本次 GMS 安装；会删除本次 Google 本地数据。"""
import argparse
import json
import pathlib
import subprocess
import sys
import time

from common import (
    ADB,
    CONFIG,
    DEVICE_KEY,
    ROOT,
    STATE as HERE,
    baseline_matches,
    guard,
    hold_operation_lock,
    recover_active_transport,
    shell,
    subprocess_options,
)
from source_integrity import python_source_sha256

_DEFAULT_COMPANION_LAUNCHER_SHA = 'ebf2eafabe29dc78f3c3289e59c658c024ca3aeb8d873b469a8e822b72a9e2a2'
_companion_launcher_src = ROOT.parents[1] / 'scripts/companion_boot/launcher.py'
COMPANION_LAUNCHER_SHA = (
    python_source_sha256(_companion_launcher_src)
    if _companion_launcher_src.is_file()
    else _DEFAULT_COMPANION_LAUNCHER_SHA
)


def build_rollback_code(stock_sha: str, wrapper_sha: str, companion_launcher_sha: str = COMPANION_LAUNCHER_SHA) -> str:
    return (
        '''import fcntl,hashlib,json,os,shutil,stat
launcher='/usr/local/bin/ubuntu_zygote';stock=launcher+'.stepx-stock-20261021';state='/var/lib/stepx-gms';base='/usr/local/lib/stepx-gms'
companion='/usr/local/lib/stepx-companion';companion_state='/var/lib/stepx-companion'
def regular_read(p):
 fd=os.open(p,os.O_RDONLY|os.O_NOFOLLOW)
 with os.fdopen(fd,'rb') as f:
  assert stat.S_ISREG(os.fstat(fd).st_mode), 'Expected regular boot artifact'
  return f.read()
def regular_digest(p):return hashlib.sha256(regular_read(p)).hexdigest()
def atomic_write(p,data,mode):
 if os.path.islink(p):raise RuntimeError('Refusing symlink boot artifact')
 temp=p+'.stepx-rollback'
 if os.path.lexists(temp):os.unlink(temp)
 fd=os.open(temp,os.O_WRONLY|os.O_CREAT|os.O_EXCL,mode)
 try:
  with os.fdopen(fd,'wb') as f:f.write(data);f.flush();os.fsync(f.fileno())
  os.chmod(temp,mode);os.chown(temp,0,0);os.replace(temp,p)
 finally:
  if os.path.lexists(temp):os.unlink(temp)
def sync_dir(d):
 fd=os.open(d,os.O_RDONLY|os.O_DIRECTORY)
 try:os.fsync(fd)
 finally:os.close(fd)
current=regular_digest(launcher)
if current==STOCK_SHA or current==COMPANION_LAUNCHER_SHA:
 standalone_companion=False
 if os.path.isfile(companion+'/config.json'):
  try:
   cfg=json.loads(regular_read(companion+'/config.json').decode('utf-8'))
   standalone_companion=cfg.get('boot_mode')=='standalone' and current==COMPANION_LAUNCHER_SHA
  except Exception:standalone_companion=False
 if os.path.lexists(stock) and not standalone_companion:
  assert regular_digest(stock)==STOCK_SHA, 'Stock backup digest mismatch'
  os.unlink(stock);sync_dir(os.path.dirname(launcher))
 if os.path.lexists(base):shutil.rmtree(base)
 if os.path.lexists(state):shutil.rmtree(state)
 print(json.dumps({'restored':True,'already_restored':True,'companion_preserved':standalone_companion,'sha256':current}))
else:
 assert current==WRAPPER_SHA, 'Launcher changed since installation; refusing overwrite'
 locks=[]
 for lock_path in (state+'/lock',companion_state+'/lock'):
  if os.path.isdir(os.path.dirname(lock_path)):
   lfd=os.open(lock_path,os.O_RDWR|os.O_CREAT,0o600);locks.append(lfd)
   fcntl.flock(lfd,fcntl.LOCK_EX|fcntl.LOCK_NB)
 stock_data=regular_read(stock);assert hashlib.sha256(stock_data).hexdigest()==STOCK_SHA
 chained_cfg=None;companion_launcher=None
 if os.path.isfile(companion+'/config.json') and os.path.isfile(companion+'/launcher.py'):
  cfg=json.loads(regular_read(companion+'/config.json').decode('utf-8'))
  lbytes=regular_read(companion+'/launcher.py')
  ldigest=hashlib.sha256(lbytes).hexdigest()
  if cfg.get('boot_mode')=='chained' and ldigest==COMPANION_LAUNCHER_SHA and cfg.get('launcher_sha256') in (None,COMPANION_LAUNCHER_SHA):
   chained_cfg=cfg;companion_launcher=lbytes
 if chained_cfg is not None:
  atomic_write(launcher,companion_launcher,0o755)
  chained_cfg['boot_mode']='standalone'
  chained_cfg['hook_path']=launcher
  chained_cfg['hook_sha256']=COMPANION_LAUNCHER_SHA
  chained_cfg['launcher_sha256']=COMPANION_LAUNCHER_SHA
  chained_cfg['original_sha256']=STOCK_SHA
  atomic_write(companion+'/config.json',json.dumps(chained_cfg).encode('utf-8'),0o600)
  for stale in (companion+'/previous-boot.py',companion+'/standalone-config-before-google.json'):
   if os.path.lexists(stale):
    regular_read(stale);os.unlink(stale)
  sync_dir(companion);sync_dir(os.path.dirname(launcher))
  if os.path.lexists(base):shutil.rmtree(base)
  if os.path.lexists(state):shutil.rmtree(state)
  print(json.dumps({'restored':True,'companion_preserved':True,'boot_mode':'standalone','sha256':regular_digest(launcher)}))
 else:
  stock_mode=stat.S_IMODE(os.stat(stock).st_mode)|0o755
  atomic_write(launcher,stock_data,stock_mode)
  os.unlink(stock)
  sync_dir(os.path.dirname(launcher))
  if os.path.lexists(base):shutil.rmtree(base)
  if os.path.lexists(state):shutil.rmtree(state)
  print(json.dumps({'restored':True,'companion_preserved':False,'sha256':regular_digest(launcher)}))
'''
        .replace('STOCK_SHA', repr(stock_sha))
        .replace('WRAPPER_SHA', repr(wrapper_sha))
        .replace('COMPANION_LAUNCHER_SHA', repr(companion_launcher_sha))
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--keep-apps', action='store_true')
    args = parser.parse_args()
    guard()
    _operation_lease = hold_operation_lock()
    baseline = json.loads((HERE / 'baseline.json').read_text(encoding='utf-8'))
    assert baseline_matches(baseline), 'Wrong device baseline'
    if not args.keep_apps:
        from install_recovery import require_fresh_rollback
        from common import verified_fingerprint
        require_fresh_rollback(baseline, verified_fingerprint(), DEVICE_KEY)
    install = json.loads((HERE / 'boot-install.json').read_text(encoding='utf-8'))
    assert subprocess.check_output(
        ADB + ['shell', 'getprop', 'ro.build.fingerprint'],
        text=True, encoding='utf-8', errors='replace', timeout=30, **subprocess_options(),
    ).strip() == baseline['fingerprint'], '设备固件已变化，停止自动回退'
    code = build_rollback_code(install['original_sha256'], install['launcher_sha256'])
    result = subprocess.run(
        [sys.executable, str(ROOT / 'tools/root_bridge.py')],
        input=code, text=True, encoding='utf-8', errors='replace', capture_output=True, timeout=150, **subprocess_options(),
    )
    raw_stdout = (result.stdout or '').lstrip()
    if not raw_stdout:
        raise RuntimeError('Root bridge returned empty output during rollback')
    proof = json.JSONDecoder().raw_decode(raw_stdout)[0]
    assert proof.get('restored'), '启动器恢复未得到确认；停止后续操作'
    print('原厂启动器已恢复。正在重启以撤销运行时权限和应用身份适配。', flush=True)
    subprocess.run(ADB + ['reboot'], check=True, capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=30, **subprocess_options())
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        try:
            p = subprocess.run(
                ADB + ['shell', 'getprop', 'sys.boot_completed'],
                capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=10, **subprocess_options(),
            )
        except subprocess.SubprocessError:
            p = None
        if p is not None and p.returncode == 0 and p.stdout.strip() == '1':
            break
        recover_active_transport()
        time.sleep(3)
    else:
        raise RuntimeError('等待 Android 启动超时；原厂启动器已恢复，但尚未卸载应用')
    if args.keep_apps:
        print('已停用启动适配并重启；保留 Google 应用及账号数据，Google 功能可能不再正常。')
        sys.exit(0)
    for package in ['com.google.android.deskclock', 'com.google.android.calculator', 'com.android.vending', 'com.google.android.gms', 'com.google.android.gsf']:
        if package in baseline['packages']:
            continue  # Preserve any test app that predated this installation.
        installed = subprocess.run(
            ADB + ['shell', 'pm', 'path', package],
            capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=30, **subprocess_options(),
        )
        if installed.returncode == 0 and installed.stdout.strip():
            result = subprocess.run(
                ADB + ['uninstall', package],
                capture_output=True, text=True, encoding='utf-8', errors='replace', check=True, timeout=120, **subprocess_options(),
            )
            assert 'Success' in result.stdout, package + ': ' + result.stdout
    for table, key, value in [('global', 'cn_gms_core', baseline['google_core']), ('secure', 'autofill_service', baseline['autofill'])]:
        shell('settings', *(['delete', table, key] if value == 'null' else ['put', table, key, value]))
    print('本轮 Google 应用与测试时钟、计算器已卸载，原始设置已恢复。私有备份保留在本目录。')


if __name__ == '__main__':
    main()
