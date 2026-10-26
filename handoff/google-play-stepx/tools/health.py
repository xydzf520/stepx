#!/usr/bin/env python3
"""Common Google dependency health. Inspection never changes device/account state."""
import argparse
import importlib.util
import json
import re
import time
from pathlib import Path
from common import ROOT, guard, root_json, shell

GOOGLE = ('com.google.android.gsf', 'com.google.android.gms', 'com.android.vending')


def current_login(receipt, boot, pid, versions):
    return bool(receipt.get('status') == 'ready' and receipt.get('registered') is True
                and receipt.get('update_monitor') is True and receipt.get('boot_id') == boot
                and receipt.get('system_server_pid') == pid
                and receipt.get('gms_version') == versions.get('com.google.android.gms')
                and receipt.get('play_version') == versions.get('com.android.vending'))


def inspect():
    guard()
    packages = {}
    versions = {}
    for name in GOOGLE:
        value = shell('dumpsys', 'package', name)
        version = re.search(r'^\s*versionCode=(\d+)', value, re.M)
        user = re.search(r'^\s*User 0:([^\n]*)', value, re.M)
        state = user.group(1) if user else ''
        enabled = re.search(r'\benabled=(\d+)', state)
        packages[name] = bool(version and 'installed=true' in state and 'hidden=false' in state
                              and 'suspended=false' in state and enabled and enabled[1] in ('0', '1'))
        versions[name] = int(version[1]) if version else 0
    pid = int(shell('pidof', 'system_server'))
    boot = shell('cat', '/proc/sys/kernel/random/boot_id')
    receipt = {}
    read_error = False
    try:
        receipt = root_json('''import os,json
host='/proc/'+next(p for p in os.listdir('/proc') if p.isdigit() and os.path.exists('/proc/'+p+'/comm') and open('/proc/'+p+'/comm').read().strip()=='openclaw_env')+'/root'
p=host+'/data/system/stepx-google-login-runtime.json'
print(json.dumps(json.load(open(p)) if os.path.exists(p) else {}))''')
    except Exception:
        read_error = True
    ready = current_login(receipt, boot, pid, versions)
    domain_ready = ready and receipt.get('domain_ready') is True
    accounts = shell('dumpsys', 'account')
    google_accounts = len(re.findall(r'Account \{[^\n]*type=com\.google\}', accounts))
    checks = [
        {'id': 'components', 'title': 'Google 基础组件', 'state': 'pass' if all(packages.values()) else 'fail',
         'detail': '服务框架、Play 服务与商店已启用' if all(packages.values()) else '组件缺失、已停用或被暂停；保留数据，请先检查应用设置'},
        {'id': 'login', 'title': 'Google 登录入口', 'state': 'pass' if ready else 'unknown' if read_error else 'fail',
         'detail': '系统凭据提供者已登记' if ready else '登录适配未就绪或组件版本已变化，请修复后重试'},
        {'id': 'updates', 'title': '组件更新后恢复', 'state': 'pass' if ready else 'unknown',
         'detail': '已监听 Play 服务和商店更新，更新后重新校验权限与登录入口' if ready else '尚未确认恢复监听生效'},
        {'id': 'domains', 'title': '应用链接验证', 'state': 'pass' if domain_ready else 'unknown',
         'detail': '系统已配置链接验证服务；由 Android 正常验证网站归属' if domain_ready else '链接验证服务未确认；部分登录回跳可能受影响'},
        {'id': 'account', 'title': 'Google 账号', 'state': 'pass' if google_accounts else 'unknown',
         'detail': f'系统中有 {google_accounts} 个 Google 账号；不读取账号内容' if google_accounts else '尚未读到 Google 账号，请在手机登录'},
    ]
    return {'schema': 1, 'login_ready': ready, 'needs_repair': not (ready and domain_ready and all(packages.values())), 'checks': checks, 'versions': versions,
            'system_server_pid': pid, 'account_count': google_accounts,
            'certification': 'not_asserted', 'third_party_login': 'requires_app_validation'}


def repair():
    if not inspect()['needs_repair']:
        return {'status': 'already_ready'}
    # The same companion boot installation is used by the combined Play installer.
    # It keeps the previous payload, signatures, baseline and original boot hook.
    script = ROOT.parents[1] / 'scripts/companion-boot.py'
    spec = importlib.util.spec_from_file_location('companion_boot_install', script)
    boot = importlib.util.module_from_spec(spec); spec.loader.exec_module(boot)
    boot.install(ROOT.parents[1] / 'desktop/assets/stepx-companion.apk')
    # A current companion receipt may predate the Google installation, or its
    # login registration may have been lost without changing the system PID.
    root_json('''import os,json,fcntl,time,hashlib
b='/usr/local/lib/stepx-companion';s='/var/lib/stepx-companion'
c=json.load(open(b+'/config.json'))
assert hashlib.sha256(open('/usr/local/lib/stepx-gms/boot.py','rb').read()).hexdigest()==c['chain_sha256']
fd=os.open(s+'/lock',os.O_RDWR);fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
if os.path.exists(s+'/last.json'):os.replace(s+'/last.json',s+'/login-repair-'+str(time.time_ns())+'.json')
os.close(fd);print(json.dumps({'prepared':True}))''')
    import manage
    manage.repair_runtime()
    until = time.monotonic() + 75
    while time.monotonic() < until:
        report = inspect()
        status = boot.status(ROOT.parents[1] / 'desktop/assets/stepx-companion.apk')
        if not report['needs_repair'] and status.get('current_boot_ready') and status.get('last', {}).get('stability_verified'):
            return {'status': 'ready', 'health': report}
        time.sleep(3)
    raise RuntimeError('Google 登录恢复未通过回读，请查看诊断；已有账号和数据未清除')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['status', 'repair'], default='status', nargs='?')
    args = p.parse_args()
    if args.action == 'status':
        print(json.dumps(inspect(), ensure_ascii=False))
    else:
        print(json.dumps(repair(), ensure_ascii=False))
