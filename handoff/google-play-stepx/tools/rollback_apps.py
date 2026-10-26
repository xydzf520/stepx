#!/usr/bin/env python3
"""Remove a partial fresh APK installation, only while the factory launcher is intact."""
import json
from common import ADB, CONFIG, STATE, guard, root_json, run, shell

import argparse
argparse.ArgumentParser(description=__doc__).parse_args()
guard()
baseline = json.loads((STATE / 'baseline.json').read_text())
assert baseline.get('serial', CONFIG.get('serial', '')) == ADB[-1], 'Wrong device baseline'
assert baseline['fingerprint'] == CONFIG['fingerprint']
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
