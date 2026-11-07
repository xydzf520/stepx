#!/usr/bin/python3
"""Start the factory container immediately, then the bounded companion boot worker."""
import os
import hashlib
import json

STOCK = '/usr/local/bin/ubuntu_zygote.stepx-stock-20261021'
BASE = '/usr/local/lib/stepx-companion'
try:
    host = '/proc/' + str(os.getppid()) + '/root'
    rootfd = os.open(host, os.O_RDONLY | os.O_DIRECTORY)
    child = os.fork()
except BaseException:
    os.execv(STOCK, ['/usr/local/bin/ubuntu_zygote'])
if child:
    os.close(rootfd)
    os.execv(STOCK, ['/usr/local/bin/ubuntu_zygote'])
try:
    os.setsid()
    with open(host + '/sys/fs/cgroup/cgroup.procs', 'w') as stream:
        stream.write(str(os.getpid()))
    with open('/dev/null', 'r+b') as stream:
        for n in (0, 1, 2):
            os.dup2(stream.fileno(), n)
    config = json.load(open(BASE + '/config.json'))
    code = open(BASE + '/boot.py', 'rb').read()
    if hashlib.sha256(code).hexdigest() != config['boot_sha256']:
        raise RuntimeError('Companion boot artifact changed')
    exec(compile(code, BASE + '/boot.py', 'exec'), globals())
except BaseException as error:
    try:
        with open('/var/lib/stepx-companion/launcher-error.txt', 'w') as stream:
            stream.write(repr(error))
    except BaseException:
        pass
os._exit(0)
