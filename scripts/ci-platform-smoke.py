"""Exercise new cross-platform helpers without accounts or a connected phone."""
import os, shutil, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[1]
def run(*args): subprocess.run([sys.executable,'-X','utf8',*args],cwd=root,check=True,timeout=240)
run('-m','unittest','discover','-s','desktop/tests','-p','test_desktop_update.py')
run('scripts/test-host-runtime.py')
run('-m','unittest','discover','-s','desktop/tests','-p','test_store_download.py')
run('-m','unittest','discover','-s','desktop/tests','-p','test_companion_system.py')
for suite in ['test_root_bridge.py','test_google_install_flow.py','test_google_runtime_status.py','test_google_autonomous.py','test_companion_boot.py']:
 run('-m','unittest','discover','-s','desktop/tests','-p',suite)
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
if not browser: raise RuntimeError('CI requires Chrome or Edge for the real local-app browser test')
run('scripts/test-studio-lab.py','--browser',browser)
print('Platform updater and browser interactions: PASS')
