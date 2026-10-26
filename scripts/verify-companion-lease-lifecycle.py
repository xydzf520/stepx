#!/usr/bin/env python3
"""Real-phone lease lifecycle acceptance. Written for root coordination; never auto-run.

Only ZYPZ261A01M692000LP, no chats, models, account data or arbitrary intents.
Default requires the phone already noninteractive, and leaves it noninteractive.
--screen-epochs adds true wake/sleep transitions. It never unlocks/dismisses the
keyguard or changes lock/security settings; all checks work behind the keyguard.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
ADB = os.environ.get('STEPX_ADB', str(Path.home() / 'Android/Sdk/platform-tools/adb'))
SERIAL = os.environ.get('STEPX_DEVICE_SERIAL') or os.environ.get('STEPX_SERIAL') or ''
SERVICE = 'com.stepx.companion.app/.AmooConnectionService'
STATUS = '/sdcard/Android/data/com.stepx.companion.app/files/amoo-connection.json'
REPORT = ROOT / 'diagnostics/companion-background-20261022/lease-lifecycle.json'
FINGERPRINT = 'StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys'
FIELDS = ('protocol', 'requestId', 'supported', 'enabled', 'active', 'leaseCount',
          'leaseExpiresElapsedMs', 'uptimeMs', 'screenInteractive', 'wakeLockHeld',
          'wakeRequested', 'wakeRequestedElapsedMs', 'lastWakeRequestElapsedMs',
          'nextWakeInMs', 'nativeWindowRemainingMs', 'idleGraceRemainingMs', 'observedConnected',
          'observedConnectedElapsedMs', 'observedConnectionSource', 'message', 'lastError')


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def shell(*args):
    result = subprocess.run([ADB, '-s', SERIAL, 'shell', shlex.join(args)],
                            capture_output=True, text=True, timeout=20)
    if result.returncode:
        # These fixed diagnostics have no QR password or account payload.
        raise RuntimeError('adb failed: ' + shlex.join(args[:3]) + ': ' +
                           (result.stderr or result.stdout).strip()[:250])
    return result.stdout.strip()


def identify():
    check(shell('getprop', 'ro.serialno') == SERIAL, 'Connected transport is not the authorized phone')
    check(shell('getprop', 'ro.build.fingerprint') == FINGERPRINT, 'Unreviewed phone firmware')


def state():
    raw = shell('cat', STATUS)
    check(len(raw) <= 65536, 'Oversized companion state')
    data = json.loads(raw)
    return {key: data[key] for key in FIELDS if key in data}


def power():
    dump = shell('dumpsys', 'power', '--brief')
    wakefulness = re.search(r'^\s*mWakefulness=(\w+)\s*$', dump, re.M)
    hal = re.search(r'^\s*mHalInteractiveModeEnabled=(true|false)\s*$', dump, re.M)
    check(wakefulness is not None, 'Global wakefulness missing from native power dump')
    value = wakefulness.group(1)
    check(value in ('Awake', 'Dreaming', 'Asleep', 'Dozing'), 'Unrecognized native wakefulness')
    return {'wakefulness': value, 'interactive': value in ('Awake', 'Dreaming'),
            'halInteractive': None if hal is None else hal.group(1) == 'true',
            'phoneUptimeMs': int(float(shell('cat', '/proc/uptime').split()[0]) * 1000)}


class Run:
    def __init__(self, epochs):
        self.started = time.monotonic()
        self.original_enabled = None
        self.original_power = None
        self.own = set()
        self.renew_at = {}
        self.events = []
        self.samples = []
        self.report = {'version': 1, 'serial': SERIAL, 'startedAt': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                       'status': 'running', 'screenEpochTest': epochs, 'noChatRequests': True,
                       'noLockSettingsChanged': True, 'noNativeInjection': True,
                       'events': self.events, 'samples': self.samples, 'checks': {}, 'cleanup': []}

    def save(self):
        REPORT.parent.mkdir(parents=True, exist_ok=True)
        temporary = REPORT.with_suffix('.json.tmp')
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, 'w') as output:
            json.dump(self.report, output, indent=2, ensure_ascii=False)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, REPORT)

    def event(self, kind, **values):
        item = {'elapsedSeconds': round(time.monotonic()-self.started, 3), 'event': kind, **values}
        self.events.append(item)
        print(json.dumps(item, ensure_ascii=False), flush=True)
        self.save()

    def operation(self, op, lease=None, expected_error=False):
        identify()
        request = 'lifecycle-' + uuid.uuid4().hex
        command = ['am', 'start-foreground-service', '--user', '0', '-n', SERVICE,
                   '--es', 'op', op, '--es', 'request_id', request]
        if lease is not None:
            command += ['--es', 'lease_id', lease]
        output = shell(*command)
        check('Error:' not in output and 'Exception' not in output, 'Android did not accept fixed service operation ' + op)
        deadline = time.monotonic() + 15
        last = None
        while time.monotonic() < deadline:
            try:
                last = state()
            except (RuntimeError, ValueError):
                time.sleep(.2)
                continue
            if last.get('requestId') == request:
                error = last.get('lastError', '')
                self.event('operation', operation=op, lease=lease, state=last, expectedError=expected_error)
                check(bool(error) == expected_error, 'Unexpected service result for ' + op + ': ' + error)
                return last
            time.sleep(.2)
        raise RuntimeError('No matching companion acknowledgement for ' + op)

    def new_lease(self, label):
        lease = 'lifecycle-' + label + '-' + uuid.uuid4().hex
        # Track before the mutation so an interrupted acknowledgement still cleans up.
        self.own.add(lease)
        result = self.operation('lease', lease)
        check(result.get('active') is True and result.get('wakeLockHeld') is True, 'Lease did not hold CPU lock')
        self.renew_at[lease] = time.monotonic() + 25
        return lease, result

    def renew_due(self, leases):
        for lease in leases:
            if time.monotonic() >= self.renew_at.get(lease, 0):
                result = self.operation('renew', lease)
                check(result.get('enabled') is True and result.get('active') is True, 'Renewal did not keep the lease active')
                self.renew_at[lease] = time.monotonic() + 25

    def sample(self, phase, asleep=True, active=None, lease_count=None):
        actual = power()
        current = state()
        item = {'elapsedSeconds': round(time.monotonic()-self.started, 3), 'phase': phase,
                'power': actual, 'state': current}
        self.samples.append(item)
        if asleep:
            check(actual['interactive'] is False and actual['halInteractive'] is False,
                  'Phone became interactive during ' + phase)
            check(current.get('screenInteractive') is False, 'Companion screen state is stale during ' + phase)
        if active is not None:
            check(current.get('active') is active, 'Unexpected active state during ' + phase)
            check(current.get('wakeLockHeld') is active, 'Unexpected CPU lock state during ' + phase)
        if lease_count is not None:
            check(current.get('leaseCount') == lease_count, 'Unexpected/foreign lease count during ' + phase)
        if current.get('active'):
            age = actual['phoneUptimeMs'] - current.get('uptimeMs', 0)
            check(-1000 <= age <= 7000, 'Active service status is stale')
        self.save()
        return actual, current

    def hold(self, phase, seconds, leases=(), count=None):
        until = time.monotonic() + seconds
        next_sample = time.monotonic()
        while time.monotonic() < until:
            self.renew_due(leases)
            self.sample(phase, active=bool(leases) if leases else None, lease_count=count)
            next_sample += 2
            time.sleep(max(0, min(next_sample, until)-time.monotonic()))

    def real_screen(self, awake):
        identify()
        # These are real Android input wake/sleep events, never broadcast spoofing,
        # lockNow, keyguard dismissal, swipe, PIN entry or lock-setting changes.
        shell('input', 'keyevent', '224' if awake else '223')
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            result = power()
            if result['interactive'] is awake and result['halInteractive'] is awake:
                self.event('real_screen', requestedInteractive=awake, power=result)
                return result
            time.sleep(.25)
        raise RuntimeError('Android did not reach requested real screen state')

    def screen_epochs(self):
        lease, _ = self.new_lease('epochs')
        previous = -1
        for epoch in (1, 2):
            self.real_screen(True)
            # Let the service observe the actual SCREEN_ON on its worker queue.
            time.sleep(2)
            _, awake = self.sample('epoch-' + str(epoch) + '-on', asleep=False, active=True, lease_count=1)
            check(awake.get('screenInteractive') is True, 'Service missed real screen-on')
            check(awake.get('wakeRequested') is False and awake.get('nativeWindowRemainingMs') == 0,
                  'Old native window survived real screen-on')
            asleep = self.real_screen(False)
            off_at = asleep['phoneUptimeMs']
            time.sleep(2)  # Allow the real screen event to reach the service worker.
            deadline = time.monotonic() + 75
            observed = None
            while time.monotonic() < deadline:
                self.renew_due((lease,))
                actual, current = self.sample('epoch-' + str(epoch) + '-off', active=True, lease_count=1)
                wake = current.get('wakeRequestedElapsedMs', -1)
                if current.get('wakeRequested'):
                    check(wake >= off_at + 14000, 'Native request preceded true 15-second screen settling interval')
                    check(wake > previous, 'An old screen-epoch request was reused')
                    if previous >= 0:
                        check(wake - previous >= 60000, 'Native request rate limit was bypassed by a new screen epoch')
                    observed = wake
                    break
                check(wake == -1, 'Invalidated epoch still advertises a request timestamp')
                time.sleep(2)
            check(observed is not None, 'No new native request within epoch deadline')
            previous = observed
            self.event('epoch_pass', epoch=epoch, wakeRequestedElapsedMs=observed)
        self.operation('release', lease)
        self.report['checks']['screen_epoch_invalidation'] = True

    def sustained(self):
        first, _ = self.new_lease('first')
        second, initial = self.new_lease('second')
        start = time.monotonic()
        phone_start = initial['uptimeMs']
        wake_times = set()
        next_sample = time.monotonic()
        while True:
            self.renew_due((first, second))
            _, current = self.sample('sustained-two-leases', active=True, lease_count=2)
            wake = current.get('wakeRequestedElapsedMs', -1)
            if current.get('wakeRequested') and wake >= phone_start:
                wake_times.add(wake)
            elapsed = time.monotonic()-start
            if elapsed >= 190 and len(wake_times) >= 3:
                break
            check(elapsed < 250, 'Fewer than 3 real native requests across sustained >180-second leases')
            next_sample += 2
            time.sleep(max(0, next_sample-time.monotonic()))
        ordered = sorted(wake_times)
        check(all(b-a >= 60000 for a, b in zip(ordered, ordered[1:])), 'CloudWake request interval shorter than 60 seconds')
        self.report['checks']['sustained_seconds'] = round(time.monotonic()-start, 3)
        self.report['checks']['sustained_wake_times_ms'] = ordered
        self.event('sustained_pass', wakeTimesMs=ordered)
        observer = subprocess.run([str(ROOT / 'desktop/target/debug/examples/verify_amoo_background_observe'), SERIAL], capture_output=True, text=True, timeout=35)
        self.report['checks']['sustained_real_cloud_response'] = observer.returncode == 0
        self.event('sustained_cloud_response', success=observer.returncode == 0)
        check(observer.returncode == 0, 'Real cloud response after sustained lease failed')
        released = self.operation('release', first)
        check(released.get('active') is True and released.get('leaseCount') == 1 and released.get('wakeLockHeld') is True,
              'Releasing one connection revoked the other connection')
        self.hold('one-lease-after-release', 8, (second,), 1)
        self.report['checks']['independent_release'] = True
        stopped = self.operation('disable')
        check(stopped.get('enabled') is False and stopped.get('active') is False and stopped.get('leaseCount') == 0 and stopped.get('wakeLockHeld') is False,
              'User disable did not clear all task leases/CPU lock')
        rejected = self.operation('renew', second, expected_error=True)
        check(rejected.get('enabled') is False and rejected.get('active') is False and rejected.get('leaseCount') == 0 and rejected.get('wakeLockHeld') is False,
              'Renewal resurrected the user-disabled service')
        self.hold('disabled-renew-rejected', 4, count=0)
        self.report['checks']['disable_and_no_resurrection'] = True
        self.operation('enable')

    def expiry(self):
        lease, initial = self.new_lease('unrenewed')
        expiry = initial['leaseExpiresElapsedMs']
        self.event('expiry_begin', lease=lease, leaseExpiresElapsedMs=expiry)
        deadline = time.monotonic() + 105
        next_sample = time.monotonic()
        expired = None
        while time.monotonic() < deadline:
            actual, current = self.sample('unrenewed-lease-expiry')
            check(current.get('leaseCount') in (0, 1), 'Foreign lease interfered with expiry test')
            if actual['phoneUptimeMs'] < expiry-1000:
                check(current.get('active') is True and current.get('wakeLockHeld') is True,
                      'Unrenewed lease expired before its 90-second deadline')
            if current.get('active') is False:
                check(actual['phoneUptimeMs'] >= expiry-1000, 'Lease expired prematurely')
                check(current.get('leaseCount') == 0 and current.get('wakeLockHeld') is False,
                      'Expired lease retained its CPU lock')
                expired = actual['phoneUptimeMs']
                break
            check(actual['phoneUptimeMs'] <= expiry+5000, 'Expired lease was not released promptly')
            next_sample += 2
            time.sleep(max(0, next_sample-time.monotonic()))
        check(expired is not None, 'Unrenewed lease did not expire')
        rejected = self.operation('renew', lease, expected_error=True)
        check(rejected.get('active') is False and rejected.get('leaseCount') == 0 and rejected.get('wakeLockHeld') is False,
              'An expired lease was recreated by renewal')
        self.report['checks']['unrenewed_expiry'] = {'expiresElapsedMs': expiry, 'observedElapsedMs': expired}

    def cleanup(self):
        # Restore only owned task IDs and the original user preference. No native
        # shutdown, force-stop, global Doze change, unlock action or chat deletion.
        for lease in sorted(self.own):
            try:
                self.operation('release', lease)
                self.report['cleanup'].append({'operation': 'release', 'lease': lease, 'ok': True})
            except Exception as error:
                self.report['cleanup'].append({'operation': 'release', 'lease': lease, 'ok': False, 'error': str(error)})
        if self.original_enabled is not None:
            try:
                result = self.operation('enable' if self.original_enabled else 'disable')
                check(result.get('enabled') is self.original_enabled, 'Original user preference was not restored')
                self.report['cleanup'].append({'operation': 'restore_enabled', 'value': self.original_enabled, 'ok': True})
            except Exception as error:
                self.report['cleanup'].append({'operation': 'restore_enabled', 'ok': False, 'error': str(error)})
        if self.original_power is not None:
            try:
                now = power()
                if now['interactive'] != self.original_power['interactive']:
                    self.real_screen(self.original_power['interactive'])
                self.report['finalPower'] = power()
                self.report['cleanup'].append({'operation': 'restore_interactivity', 'ok': True})
            except Exception as error:
                self.report['cleanup'].append({'operation': 'restore_interactivity', 'ok': False, 'error': str(error)})
        self.report['cleanupComplete'] = all(item['ok'] for item in self.report['cleanup'])
        self.report['finishedAt'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        self.report['elapsedSeconds'] = round(time.monotonic()-self.started, 3)
        self.save()

    def run(self):
        try:
            identify()
            self.original_power = power()
            self.report['initialPower'] = self.original_power
            check(self.original_power['interactive'] is False and self.original_power['halInteractive'] is False,
                  'Start with the phone already asleep; this script will not unexpectedly lock an unlocked phone')
            original = self.operation('status')
            self.original_enabled = original.get('enabled')
            check(type(self.original_enabled) is bool, 'Original enabled preference is missing')
            check(original.get('protocol') == 2 and original.get('supported') is True,
                  'The reviewed CloudWake service is not installed/ready')
            check(original.get('leaseCount') == 0 and original.get('active') is False,
                  'An existing task owns a lease; stop that test/task before running this lifecycle test')
            self.report['initialState'] = original
            self.operation('enable')
            if self.report['screenEpochTest']:
                self.screen_epochs()
            self.sustained()
            self.expiry()
            self.report['status'] = 'passed'
        except BaseException as error:
            self.report['status'] = 'interrupted' if isinstance(error, (KeyboardInterrupt, SystemExit)) else 'failed'
            self.report['error'] = type(error).__name__ + ': ' + str(error)
        finally:
            self.cleanup()
        if not self.report['cleanupComplete']:
            self.report['status'] = 'cleanup_failed'
            self.save()
        print(json.dumps({'status': self.report['status'], 'report': str(REPORT),
                          'checks': self.report['checks'], 'cleanupComplete': self.report['cleanupComplete']}, ensure_ascii=False), flush=True)
        return 0 if self.report['status'] == 'passed' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--screen-epochs', action='store_true', help='Add real on/off/on/off epoch checks; phone must initially be asleep')
    args = parser.parse_args()
    def interrupted(signum, frame):
        raise KeyboardInterrupt('Diagnostic interrupted by signal ' + str(signum))
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    return Run(args.screen_epochs).run()


if __name__ == '__main__':
    sys.exit(main())
