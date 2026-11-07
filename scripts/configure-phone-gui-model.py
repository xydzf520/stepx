#!/usr/bin/env python3
"""Seal an existing local Step Plan key to this companion's Android Keystore key.
No plaintext credential goes into adb arguments, phone files, or diagnostic output.
"""
import base64
import json
import os
import shutil
from pathlib import Path
import re
import subprocess
import tempfile
from cryptography.hazmat.primitives import serialization, hashes
from cryptography.hazmat.primitives.asymmetric import padding

def adb_command():
    serial = os.environ.get('STEPX_SERIAL', '').strip()
    if not serial or serial.startswith('-') or any(c.isspace() for c in serial):
        raise RuntimeError('Set STEPX_SERIAL to the explicitly selected phone transport')
    return [os.environ.get('STEPX_ADB') or shutil.which('adb') or 'adb', '-s', serial]
def call(q):
    raw=subprocess.check_output(adb_command()+['shell','content','call','--uri','content://com.stepx.companion.app.gui','--method','request','--arg',base64.b64encode(json.dumps(q).encode()).decode()],text=True)
    match=re.search(r'result=([A-Za-z0-9+/=]+)',raw)
    if not match:raise RuntimeError('Phone did not return a configuration response')
    value=json.loads(base64.b64decode(match[1]))
    if not value.get('ok'):raise RuntimeError(value.get('error','Configuration failed'))
    return value
def main():
    info=call({'op':'agent','agentOp':'config_info'})
    public=serialization.load_der_public_key(base64.b64decode(info['publicKey']))
    keypath=Path.home()/'.config/stepx/gui-agent/stepfun.key'
    if keypath.is_symlink() or keypath.stat().st_mode & 0o077:raise RuntimeError('Local key must have private permissions')
    sealed=public.encrypt(keypath.read_bytes().strip(),padding.OAEP(mgf=padding.MGF1(hashes.SHA1()),algorithm=hashes.SHA256(),label=None))
    with tempfile.TemporaryDirectory(prefix='stepx-model-import-') as directory:
        file=Path(directory)/'key.sealed';file.write_bytes(sealed)
        subprocess.run(adb_command()+['push',str(file),'/sdcard/Android/data/com.stepx.companion.app/files/gui-key.sealed'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    result=call({'op':'agent','agentOp':'install_key'})
    print(json.dumps({k:result.get(k) for k in ('ready','modelConfigured','model','requiresDesktop')},ensure_ascii=False))
if __name__=='__main__':main()
