"""Exercise new cross-platform helpers without accounts or a connected phone."""
import ast, os, shutil, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(root/'scripts'))
from source_integrity import python_source_sha256, read_python_source

compat_dir=root/'handoff/google-play-stepx/compat/scripts'
for compat_py in sorted(compat_dir.glob('*.py')):
 canonical=root/'scripts'/compat_py.name
 if not canonical.is_file() or read_python_source(compat_py)!=read_python_source(canonical):
  raise RuntimeError(f'Handoff compat script drift detected: {compat_py.name}')
cp_tree=ast.parse((root/'scripts/companion-permissions.py').read_text(encoding='utf-8'))
worker_pin=next(ast.literal_eval(n.value) for n in cp_tree.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='WORKER_SHA256' for t in n.targets))
if worker_pin!=python_source_sha256(root/'handoff/google-play-stepx/tools/root_worker.py'):
 raise RuntimeError('WORKER_SHA256 drift between scripts/companion-permissions.py and tools/root_worker.py')

failures=[]

def run(*args):
 try:
  subprocess.run([sys.executable,'-X','utf8',*args],cwd=root,check=True,timeout=240)
 except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
  failures.append(' '.join(args))
  print(error, file=sys.stderr, flush=True)
run('-m','unittest','discover','-s','desktop/tests','-p','test_desktop_update.py')
run('scripts/test-host-runtime.py')
run('-m','unittest','discover','-s','desktop/tests','-p','test_store_download.py')
run('-m','unittest','discover','-s','desktop/tests','-p','test_companion_system.py')
for suite in ['test_root_bridge.py','test_google_install_flow.py','test_google_recovery_install.py','test_google_runtime_status.py','test_google_autonomous.py','test_google_play_compat_cli.py','test_companion_boot.py']:
 run('-m','unittest','discover','-s','desktop/tests','-p',suite)
run('-m','unittest','discover','-s','scripts/tests','-p','test_*.py')
run('-m','unittest','discover','-s','desktop/tests','-p','test_google_login_runtime.py')
if os.name=='posix':
 # These tests execute the Android/Linux filesystem payload with fcntl and
 # O_NOFOLLOW in an isolated host directory. Host-only boot tests run above.
 run('scripts/test-companion-boot.py')
else:
 print('Android boot filesystem payload: POSIX-only; host boot checks still ran')
paths=[shutil.which(x) for x in ['google-chrome','chromium','msedge','chrome']]
paths += ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome','/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge']
for key in ['PROGRAMFILES','PROGRAMFILES(X86)','LOCALAPPDATA']:
 if os.environ.get(key):
  paths += [str(Path(os.environ[key])/p) for p in ['Microsoft/Edge/Application/msedge.exe','Google/Chrome/Application/chrome.exe']]
browser=next((p for p in paths if p and Path(p).is_file()),None)
if browser:
 run('scripts/test-studio-lab.py','--browser',browser)
else:
 failures.append('CI requires Chrome or Edge for the real local-app browser test')
if failures:
 raise SystemExit('Platform smoke checks failed:\n'+'\n'.join(failures))
print('Platform updater and browser interactions: PASS')
