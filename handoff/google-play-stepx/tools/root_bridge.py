"""Bound the sudo IPC request; execute only hash-verified staged source bytes."""
import base64
import hashlib
import json
import pathlib
import shlex
import subprocess
import sys
import time
import uuid


def gateway(command):
    from host_runtime import subprocess_options
    result = subprocess.run(
        [sys.executable, str(pathlib.Path(__file__).with_name('gateway.py'))],
        input=json.dumps({'command': command, 'timeoutMs': 20000}),
        text=True, encoding='utf-8', capture_output=True, timeout=30,
        **subprocess_options())
    try:
        value = json.loads(result.stdout)
    except ValueError:
        raise RuntimeError('Root bridge staging returned no structured response') from None
    if result.returncode or value.get('exitCode') != 0:
        raise RuntimeError('Root bridge transport failed; no operation proof was returned')
    return value


def root_command(code):
    # Always stop before apt update. Success remains the payload's JSON proof.
    hook = '/usr/bin/python3 -I -c ' + shlex.quote(code) + '; exit 73'
    argv = ['/usr/bin/sudo', '/usr/bin/apt-get', '-o', 'Debug::NoLocking=1',
            '-o', 'APT::Update::Pre-Invoke::=' + hook,
            '-o', 'Dir::Etc::sourcelist=/dev/null', '-o', 'Dir::Etc::sourceparts=-', 'update']
    if sum(len(a.encode()) + 1 for a in argv) > 1900:
        raise RuntimeError('Root bridge IPC request exceeds its reviewed bound')
    remote = ('import subprocess; p=subprocess.run(' + repr(argv) +
              ',capture_output=True,text=True,timeout=15); '
              'print(p.stdout.split("Reading package lists...")[0]); '
              'print(p.stderr); print("helper_exit",p.returncode)')
    return ['python3', '-c', remote]


def execute(code, send=gateway):
    # Leave room inside the host's 150s operation bound for the last gateway
    # request and cleanup (each <=30s), including slow wireless staging.
    deadline = time.monotonic() + 80
    transport = send
    def send(command):
        if time.monotonic() >= deadline:
            raise RuntimeError('Root bridge staging timed out before operation confirmation')
        return transport(command)
    data = code.encode('utf-8')
    if not data or len(data) > 1048576:
        raise RuntimeError('Root operation source exceeds the staging bound')
    compile(data, '<stepx-root-operation>', 'exec')
    if len(data) <= 600:
        try:
            direct = root_command(code)
        except RuntimeError:
            pass  # Shell escaping can make even short source exceed the IPC cap.
        else:
            return send(direct)
    path = '/tmp/stepx-root-' + uuid.uuid4().hex + '.py'
    created = False
    try:
        send(['python3', '-c', 'import os;fd=os.open(' + repr(path) +
              ',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.close(fd)'])
        created = True
        for start in range(0, len(data), 900):
            chunk = base64.b64encode(data[start:start + 900]).decode('ascii')
            send(['python3', '-c', 'import os,base64;fd=os.open(' + repr(path) +
                  ',os.O_WRONLY|os.O_APPEND|os.O_NOFOLLOW);f=os.fdopen(fd,"ab");'
                  'f.write(base64.b64decode(' + repr(chunk) + '));f.close()'])
        # Hash and compile the same bytes: no path re-open after checking.
        runner = ('import os,stat,hashlib\nfd=os.open(' + repr(path) +
                  ',os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)\nwith os.fdopen(fd,"rb") as f:\n'
                  ' if not stat.S_ISREG(os.fstat(fd).st_mode): raise RuntimeError("Invalid staged file")\n'
                  ' d=f.read(1048577)\n'
                  'if len(d)!=' + str(len(data)) + ' or hashlib.sha256(d).hexdigest()!=' + repr(hashlib.sha256(data).hexdigest()) +
                  ': raise RuntimeError("Staged source digest mismatch")\n'
                  'exec(compile(d,"<stepx-root-operation>","exec"))')
        return send(root_command(runner))
    finally:
        if created:
            try:
                transport(['python3', '-c', 'import os;os.unlink(' + repr(path) + ')'])
            except Exception:
                # Don't retry a completed operation just because cleanup failed.
                print('Root staging cleanup pending; private temporary source retained', file=sys.stderr)


if __name__ == '__main__':
    import common  # Adds the packaged scripts directory to sys.path.
    response = execute(sys.stdin.read())
    print(response.get('stdout', ''))
    print(response.get('stderr', ''))
