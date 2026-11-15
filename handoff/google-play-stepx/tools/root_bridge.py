"""Bound the sudo IPC request; execute only hash-verified staged source bytes."""
import base64
from contextlib import ExitStack
import hashlib
import json
import pathlib
import shlex
import subprocess
import sys
import time
import uuid

ROOT_WORKER = pathlib.Path(__file__).with_name('root_worker.py')
STAGE_CHUNK_SIZE = 8192


def _parse_gateway_response(status_code, raw_stdout, returncode=0):
    try:
        value = json.loads(raw_stdout) if status_code == 200 else None
    except ValueError:
        value = None
    if not isinstance(value, dict):
        raise RuntimeError('Root bridge staging returned no structured response')
    if returncode or status_code != 200 or value.get('exitCode') != 0:
        raise RuntimeError('Root bridge transport failed; no operation proof was returned')
    return value


def gateway(command):
    from host_runtime import subprocess_options
    result = subprocess.run(
        [sys.executable, str(pathlib.Path(__file__).with_name('gateway.py'))],
        input=json.dumps({'command': command, 'timeoutMs': 20000}),
        text=True, encoding='utf-8', capture_output=True, timeout=30,
        **subprocess_options())
    return _parse_gateway_response(200, result.stdout, result.returncode)


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


def stage_file(send, path, data, chunk_size=STAGE_CHUNK_SIZE, on_created=None):
    send(['python3', '-c', 'import os;fd=os.open(' + repr(path) +
          ',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.close(fd)'])
    if on_created is not None:
        on_created()
    for start in range(0, len(data), chunk_size):
        chunk = base64.b64encode(data[start:start + chunk_size]).decode('ascii')
        send(['python3', '-c', 'import os,base64;fd=os.open(' + repr(path) +
              ',os.O_WRONLY|os.O_APPEND|os.O_NOFOLLOW);f=os.fdopen(fd,"ab");'
              'f.write(base64.b64decode(' + repr(chunk) + '));f.close()'])


def run_root_worker(operation_path, output_path, timeout=130, env=None):
    """Run bounded ROOT_WORKER with pre/post self-healing for stale stepx-gms-w*.py props."""
    import common
    from host_runtime import subprocess_options
    common.heal_stale_zygote_prop()
    try:
        return subprocess.run(
            [sys.executable, str(ROOT_WORKER), str(operation_path), str(output_path)],
            text=True, encoding='utf-8', capture_output=True, timeout=timeout,
            env=env, **subprocess_options(),
        )
    finally:
        try:
            common.heal_stale_zygote_prop()
        except Exception:
            pass


def execute(code, send=gateway):
    # Leave room inside the host's 150s operation bound for the last gateway
    # request and cleanup (each <=30s), including slow wireless staging.
    data = code.encode('utf-8')
    if not data or len(data) > 1048576:
        raise RuntimeError('Root operation source exceeds the staging bound')
    compile(data, '<stepx-root-operation>', 'exec')
    use_default_gateway = send is gateway
    chunk_size = STAGE_CHUNK_SIZE if use_default_gateway else getattr(send, 'chunk_size', 900)
    deadline = time.monotonic() + 80
    with ExitStack() as stack:
        if use_default_gateway:
            import common  # Ensures tools/ and scripts/ are on sys.path.
            from gateway import gateway_session
            session_send = stack.enter_context(gateway_session())
            def transport(command):
                status_code, raw = session_send({'command': command, 'timeoutMs': 20000}, http_timeout=30)
                return _parse_gateway_response(status_code, raw)
        else:
            transport = send

        def bounded_send(command):
            if time.monotonic() >= deadline:
                raise RuntimeError('Root bridge staging timed out before operation confirmation')
            return transport(command)

        try:
            if len(data) <= 600:
                try:
                    direct = root_command(code)
                except RuntimeError:
                    pass  # Shell escaping can make even short source exceed the IPC cap.
                else:
                    return bounded_send(direct)
            path = '/tmp/stepx-root-' + uuid.uuid4().hex + '.py'
            created = False

            def mark_created():
                nonlocal created
                created = True

            try:
                stage_file(bounded_send, path, data, chunk_size=chunk_size, on_created=mark_created)
                # Hash and compile the same bytes: no path re-open after checking.
                runner = ('import os,stat,hashlib\nfd=os.open(' + repr(path) +
                          ',os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)\nwith os.fdopen(fd,"rb") as f:\n'
                          ' if not stat.S_ISREG(os.fstat(fd).st_mode): raise RuntimeError("Invalid staged file")\n'
                          ' d=f.read(1048577)\n'
                          'if len(d)!=' + str(len(data)) + ' or hashlib.sha256(d).hexdigest()!=' + repr(hashlib.sha256(data).hexdigest()) +
                          ': raise RuntimeError("Staged source digest mismatch")\n'
                          'exec(compile(d,"<stepx-root-operation>","exec"))')
                return bounded_send(root_command(runner))
            finally:
                if created:
                    try:
                        transport(['python3', '-c', 'import os;os.unlink(' + repr(path) + ')'])
                    except Exception:
                        # Don't retry a completed operation just because cleanup failed.
                        print('Root staging cleanup pending; private temporary source retained', file=sys.stderr)
        except Exception:
            if use_default_gateway and 'persist.openclaw.zygote.path' in code:
                try:
                    import common
                    common.heal_stale_zygote_prop()
                except Exception:
                    pass
            raise


if __name__ == '__main__':
    import common  # Adds the packaged scripts directory to sys.path.
    response = execute(sys.stdin.read())
    print(response.get('stdout', ''))
    print(response.get('stderr', ''))
