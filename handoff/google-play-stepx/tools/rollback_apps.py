#!/usr/bin/env python3
"""Remove a partial fresh APK installation, only while the factory launcher is intact."""
import argparse
import json
from common import ADB, CONFIG, DEVICE_KEY, STATE, baseline_matches, guard, hold_operation_lock, root_json, run, shell, verified_fingerprint
from install_recovery import require_fresh_rollback

argparse.ArgumentParser(description=__doc__).parse_args()
guard()
operation_lease = hold_operation_lock()
baseline = json.loads((STATE / 'baseline.json').read_text(encoding='utf-8'))
assert baseline_matches(baseline), 'Wrong device baseline'
require_fresh_rollback(baseline, verified_fingerprint(), DEVICE_KEY)
proof = root_json("import hashlib,json;print(json.dumps({'sha':hashlib.sha256(open('/usr/local/bin/ubuntu_zygote','rb').read()).hexdigest()}))")
assert proof['sha'] == CONFIG['stock_sha256'], 'Use full rollback.py while adapter is installed'
for package in ['com.android.vending', 'com.google.android.gms', 'com.google.android.gsf']:
    assert package not in baseline['packages'], 'Would remove a pre-existing package'
    if shell('pm', 'list', 'packages', '--user', '0', package):
        result = run(ADB + ['uninstall', package])
        assert 'Success' in result.stdout
for table, key, value in [('global', 'cn_gms_core', baseline['google_core']),
                          ('secure', 'autofill_service', baseline['autofill'])]:
    shell('settings', *(['delete', table, key] if value == 'null' else ['put', table, key, value]))
print('Fresh Google APK installation removed and original settings restored.')
