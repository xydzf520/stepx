"""Portable host dependencies; Android/remote commands are deliberately unchanged."""
from contextlib import contextmanager
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def user_state_root():
    """Mutable files must never live inside a signed/read-only application bundle."""
    if os.environ.get('STEPX_STATE_DIR'):
        return Path(os.environ['STEPX_STATE_DIR'])
    if os.environ.get('XDG_STATE_HOME'):
        return Path(os.environ['XDG_STATE_HOME']) / 'stepx-companion'
    if os.name == 'nt':
        return Path(os.environ.get('LOCALAPPDATA') or Path.home() / 'AppData/Local') / 'StepX Companion'
    if sys.platform == 'darwin':
        return Path.home() / 'Library/Application Support/StepX Companion'
    return Path.home() / '.local/state/stepx-companion'


def migrate_legacy_state(source, destination):
    """Copy old recovery receipts atomically; never delete or replace either baseline."""
    source, destination = Path(source), Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with file_lock(destination.parent / '.state-migration.lock'):
        if destination.exists() or not source.exists():
            return
        if source.is_symlink() or not source.is_dir() or any(p.is_symlink() for p in source.rglob('*')):
            raise RuntimeError('旧版恢复目录包含异常链接，请先检查原始备份')
        with tempfile.TemporaryDirectory(prefix='.state-migrate-', dir=destination.parent) as temp:
            staged = Path(temp) / 'state'
            shutil.copytree(source, staged)
            for path in staged.rglob('*'):
                path.chmod(0o700 if path.is_dir() else 0o600)
            staged.chmod(0o700)
            staged.rename(destination)


def subprocess_options():
    return {'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}


def optional_tool_home():
    """A user profile is optional when locating bundled or PATH-based tools."""
    try:
        return Path.home()
    except (RuntimeError, OSError):
        # Windows service/test environments may omit USERPROFILE and HOMEDRIVE.
        # Do not invent a writable user-data location; only skip SDK/JDK hints.
        return None


def adb_executable():
    if os.environ.get('STEPX_ADB'):
        return os.environ['STEPX_ADB']
    executable = 'adb.exe' if os.name == 'nt' else 'adb'
    candidates = [
        ROOT / 'tools/bin' / executable,
        ROOT / '.tools/bin' / executable,
    ]
    for key in ('ANDROID_SDK_ROOT', 'ANDROID_HOME'):
        if os.environ.get(key):
            candidates.append(Path(os.environ[key]) / 'platform-tools' / executable)
    for candidate in candidates:
        if candidate.is_file():
            return str(candidate)
    home = optional_tool_home()
    if home is not None:
        for sdk in ('Android/Sdk', 'Library/Android/sdk'):
            candidate = home / sdk / 'platform-tools' / executable
            if candidate.is_file():
                return str(candidate)
    return shutil.which('adb') or 'adb'


def java_environment():
    env = os.environ.copy()
    candidates = [Path(env['STEPX_JDK'])] if env.get('STEPX_JDK') else []
    candidates.append(ROOT / 'runtime/java')
    if env.get('JAVA_HOME'): candidates.append(Path(env['JAVA_HOME']))
    home = optional_tool_home()
    if home is not None:
        candidates.append(home / '.local/opt/android-studio/jbr')
    candidates += [Path('/opt/android-studio/jbr'), Path('/Applications/Android Studio.app/Contents/jbr/Contents/Home'),
                   Path('/usr/lib/jvm/default-java'), Path('/usr/lib/jvm/java-21-openjdk-amd64'),
                   Path('/usr/lib/jvm/java-17-openjdk-amd64'), Path('/opt/homebrew/opt/openjdk')]
    executable = 'java.exe' if os.name == 'nt' else 'java'
    for directory in candidates:
        if (directory / 'bin' / executable).is_file():
            env['JAVA_HOME'] = str(directory)
            env['PATH'] = str(directory / 'bin') + os.pathsep + env.get('PATH', '')
            break
    return env


def build_tools():
    """Do not guess a developer's home or a single installed SDK version."""
    yield ROOT / 'runtime/android-build-tools'
    yield ROOT / '.tools/android-build-tools'
    for key in ('ANDROID_SDK_ROOT', 'ANDROID_HOME'):
        if os.environ.get(key):
            base = Path(os.environ[key]) / 'build-tools'
            if base.is_dir():
                yield from sorted((p for p in base.iterdir() if p.is_dir()), reverse=True)
    home = optional_tool_home()
    bases = [home / 'Android/Sdk', home / 'Library/Android/sdk'] if home is not None else []
    if os.environ.get('LOCALAPPDATA'):
        bases.append(Path(os.environ['LOCALAPPDATA'])/'Android/Sdk')
    for sdk in bases:
        base = sdk / 'build-tools'
        if base.is_dir():
            yield from sorted((p for p in base.iterdir() if p.is_dir()), reverse=True)


def sdk_tool(name):
    suffix = '.exe' if os.name == 'nt' else ''
    for directory in build_tools():
        candidate = directory / (name + suffix)
        if candidate.is_file():
            return str(candidate)
    found = shutil.which(name)
    if found:
        return found
    raise RuntimeError('缺少 Android '+name+' 工具。请下载完整安装包并完整解压，勿单独运行 EXE。')


def sdk_command(name):
    # Avoid .bat/cmd quoting, Unicode and space-path failures on Windows.
    if name == 'apk-identity':
        directory = ROOT / 'runtime/android-build-tools/lib'
        java = shutil.which('java', path=java_environment().get('PATH'))
        if not java or not (directory / 'apk-identity.jar').is_file():
            raise RuntimeError('缺少 APK 身份校验运行时，请重新解压完整安装包。')
        return [java, '-cp', os.pathsep.join(str(directory / name) for name in ['apk-identity.jar', 'apksigner.jar']), 'ApkIdentity']
    if name == 'apksigner':
        for directory in build_tools():
            jar = directory / 'lib/apksigner.jar'
            if jar.is_file():
                env = java_environment()
                java = shutil.which('java', path=env.get('PATH'))
                if not java:
                    raise RuntimeError('缺少签名校验运行时。请重新解压完整安装包。')
                return [java, '-jar', str(jar)]
    return [sdk_tool(name)]


@contextmanager
def file_lock(path, timeout=30):
    """Per-operation lock with Windows byte locking and POSIX flock."""
    path = Path(path)
    if path.is_symlink():
        raise RuntimeError('锁文件不可为符号链接')
    fd = os.open(path, os.O_RDWR | os.O_CREAT | getattr(os, 'O_NOFOLLOW', 0), 0o600)
    with os.fdopen(fd, 'r+b') as stream:
        if os.name == 'nt':
            import msvcrt
            if os.fstat(fd).st_size == 0:
                stream.write(b'\0'); stream.flush()
            deadline = time.monotonic() + timeout
            while True:
                try:
                    stream.seek(0); msvcrt.locking(fd, msvcrt.LK_NBLCK, 1); break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise RuntimeError('另一个安装或修复操作正在运行，请稍后重试')
                    time.sleep(.1)
            try:
                yield
            finally:
                stream.seek(0); msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
        else:
            import fcntl
            fcntl.flock(fd, fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)
