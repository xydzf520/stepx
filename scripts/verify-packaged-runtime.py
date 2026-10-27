"""Exercise the shipped runtime with PATH/SDK cleared, including non-ASCII paths."""
import importlib.util, os, shutil, struct, subprocess, sys, tempfile
from pathlib import Path
root=Path(sys.argv[1]).resolve()

def run_checked(args, env, timeout):
    # GUI-subsystem EXEs need explicit redirected handles for CI diagnostics.
    result = subprocess.run(args, env=env, capture_output=True, encoding='utf-8', errors='replace', timeout=timeout)
    if result.stdout: print(result.stdout, end='', flush=True)
    if result.stderr: print(result.stderr, end='', file=sys.stderr, flush=True)
    result.check_returncode()

with tempfile.TemporaryDirectory(prefix='StepX 中文 runtime ') as td:
    copy=Path(td)/'完整安装 包';copy.mkdir()
    for folder in ['runtime','scripts','handoff','tools']:
        shutil.copytree(root/folder,copy/folder,symlinks=True)
    (copy/'desktop/assets').mkdir(parents=True)
    apk=copy/'desktop/assets/stepx-companion.apk';shutil.copy2(root/'desktop/assets/stepx-companion.apk',apk)
    env={k:v for k,v in os.environ.items() if k not in ['ANDROID_HOME','ANDROID_SDK_ROOT','JAVA_HOME','STEPX_JDK','PYTHONPATH','STEPX_RESOURCE_DIR','STEPX_PYTHON']}
    env['PATH']=str(Path(os.environ.get('SystemRoot','/usr'))/'System32') if os.name=='nt' else '/usr/bin:/bin'
    env['PYTHONUTF8']='1';env['PYTHONIOENCODING']='utf-8'
    env['STEPX_STATE_DIR']=str(Path(td)/'user data');env['PYTHONDONTWRITEBYTECODE']='1'
    python=copy/'runtime/python/python.exe' if os.name=='nt' else copy/'runtime/python/bin/python3'
    code="""import importlib.util,sys
from pathlib import Path
root=Path(sys.argv[1]);sys.path.insert(0,str(root/'scripts'))
for name in ['companion-permissions','companion-power','companion-system']:
 spec=importlib.util.spec_from_file_location(name,root/'scripts'/(name+'.py'));m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
 if name=='companion-permissions':
  signer=m.signer(root/'desktop/assets/stepx-companion.apk');assert len(signer)==64
# Verify that signature checking rejects modified APK bytes, not just a known filename.
import zipfile
apk=root/'desktop/assets/stepx-companion.apk';bad=root/'tampered.apk';data=bytearray(apk.read_bytes())
with zipfile.ZipFile(apk) as archive:
 item=archive.getinfo('classes.dex');offset=item.header_offset
 name=int.from_bytes(data[offset+26:offset+28],'little');extra=int.from_bytes(data[offset+28:offset+30],'little')
 data[offset+30+name+extra+10]^=1
bad.write_bytes(data)
spec=importlib.util.spec_from_file_location('verification',root/'scripts/companion-permissions.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
try:m.signer(bad)
except RuntimeError:pass
else:raise AssertionError('Modified APK accepted')
# A different package must fail identity validation before signature validation.
wrong=root/'wrong-package.apk'
with zipfile.ZipFile(apk) as source, zipfile.ZipFile(wrong,'w') as target:
 for item in source.infolist():
  content=source.read(item.filename)
  if item.filename=='AndroidManifest.xml':
   original=content
   for encoding in ['utf-8','utf-16le']:
    content=content.replace('com.stepx.companion.app'.encode(encoding),'com.stepx.companion.bad'.encode(encoding))
   assert content!=original,'Manifest test fixture did not change'
  target.writestr(item,content)
try:m.signer(wrong)
except RuntimeError as error:assert 'APK is not StepX' in str(error),str(error)
else:raise AssertionError('Wrong APK package accepted')
from host_runtime import file_lock
with file_lock(root/'lock'):pass
with file_lock(root/'lock'):pass
print('Packaged signer, Unicode paths, imports and file lock: PASS')
"""
    binary_root=root.parent/'MacOS' if root.name=='Resources' else root
    binary_name='stepx-companion.exe' if os.name=='nt' else 'stepx-companion'
    # Exercise actual Contents/MacOS -> Contents/Resources resolution as shipped.
    run_checked([str(binary_root/binary_name),'--check-runtime'],env,45)
    shutil.copy2(binary_root/binary_name,copy/binary_name)
    run_checked([str(copy/binary_name),'--check-runtime'],env,45)
    adb=copy/'tools/bin'/('adb.exe' if os.name=='nt' else 'adb')
    run_checked([str(adb),'version'],env,15)
    run_checked([str(python),'-X','utf8','-c',code,str(copy)],env,45)
    # Use the client command, including the argv used by store APK validation.
    run_checked([str(copy/binary_name),'--check-apk',str(apk)],env,45)
    result = subprocess.run([str(copy/binary_name),'--check-apk',str(copy/'tampered.apk')],
                            env=env,capture_output=True,encoding='utf-8',errors='replace',timeout=45)
    assert result.returncode != 0, 'Client accepted a modified APK signature'
    assert 'EOF while parsing' not in result.stderr, 'Client masked APK failure with JSON EOF'
    print('Client APK verification and modified-signature rejection: PASS')
    run_checked([str(python),'-B','-X','utf8',str(copy/'scripts/test-host-runtime.py')],env,30)
if os.name=='nt':
    binary=(root/'stepx-companion.exe').read_bytes();pe=struct.unpack_from('<I',binary,0x3c)[0]
    assert struct.unpack_from('<H',binary,pe+24+68)[0]==2,'EXE must use Windows GUI subsystem'
    print('Windows GUI subsystem: PASS')
