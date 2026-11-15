from contextlib import contextmanager
import hashlib
import hmac
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request


def _subprocess_opts(common_mod):
    fn = getattr(common_mod, 'subprocess_options', None)
    if callable(fn):
        return fn()
    return {'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}


def _check_output_with_failover(common_mod, suffix, timeout):
    opts = _subprocess_opts(common_mod)
    try:
        return subprocess.check_output(
            common_mod.ADB + suffix, text=True, timeout=timeout, **opts
        ).strip()
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
        is_drop = getattr(common_mod, '_is_transport_drop', None)
        recover = getattr(common_mod, 'recover_active_transport', None)
        if callable(is_drop) and callable(recover) and is_drop(exc) and recover():
            return subprocess.check_output(
                common_mod.ADB + suffix, text=True, timeout=timeout, **opts
            ).strip()
        raise


@contextmanager
def gateway_session():
    import common as common_mod
    common_mod.guard()
    opts = _subprocess_opts(common_mod)
    android_id = _check_output_with_failover(
        common_mod, ['shell', 'settings', 'get', 'secure', 'android_id'], 20
    )
    assert android_id and android_id != 'null'
    token = hmac.new(
        b'stepclaw-gateway-token-salt-v1',
        ('stepclaw-gateway-v1|' + android_id).encode(),
        hashlib.sha256,
    ).hexdigest()
    port = _check_output_with_failover(common_mod, ['forward', 'tcp:0', 'tcp:18789'], 15)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        def send_payload(payload, http_timeout=40):
            body = dict(payload)
            route = body.pop('_route', '/plugins/stepclaw-exec/run')
            assert route.startswith('/')
            req = urllib.request.Request(
                'http://127.0.0.1:' + port + route,
                data=json.dumps(body).encode(),
                headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'},
            )
            try:
                with opener.open(req, timeout=http_timeout) as r:
                    return 200, r.read(2000000).decode().replace(token, '[REDACTED]')
            except urllib.error.HTTPError as e:
                return e.code, e.read(3000).decode().replace(token, '[REDACTED]')

        yield send_payload
    finally:
        try:
            subprocess.run(
                common_mod.ADB + ['forward', '--remove', 'tcp:' + port],
                capture_output=True, text=True, timeout=10, **opts,
            )
        except Exception:
            pass


def main():
    payload = json.load(sys.stdin)
    with gateway_session() as send_payload:
        status_code, data = send_payload(payload)
        if status_code == 200:
            print(data)
        else:
            print('HTTP', status_code, data)


if __name__ == '__main__':
    main()
