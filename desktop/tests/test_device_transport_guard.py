"""Identity and firmware guards for USB and paired wireless companion calls."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

path = Path(__file__).resolve().parents[2] / 'handoff/google-play-stepx/tools/common.py'
spec = importlib.util.spec_from_file_location('device_transport_common', path)
common = importlib.util.module_from_spec(spec)
spec.loader.exec_module(common)


class TransportGuardTests(unittest.TestCase):
    def check(self, transport, expected='', physical='PHONE123', state='device', firmware=None, native=False):
        def shell(*args):
            if args == ('getprop', 'ro.serialno'): return physical
            if args == ('getprop', 'ro.build.fingerprint'): return firmware or common.CONFIG['fingerprint']
            if args == ('getprop', 'ro.build.version.sdk'): return common.CONFIG['sdk']
            if args == ('getprop', 'ro.product.cpu.abilist'): return common.CONFIG['abi']
            if args == ('getenforce',): return 'Enforcing'
            if args[0] == 'sha256sum': return common.CONFIG['native_hashes'][args[1]] + '  ' + args[1]
            raise AssertionError(args)
        with patch.object(common, 'SERIAL', transport), patch.object(common, 'DEVICE_SERIAL', expected), \
             patch.object(common, 'ADB', ['adb', '-s', transport]), \
             patch.object(common, 'run', return_value=SimpleNamespace(stdout=state)) as run, \
             patch.object(common, 'shell', side_effect=shell) as read:
            common.guard(native=native)
            run.assert_called_once_with(['adb', '-s', transport, 'get-state'])
            if native:
                for p in common.CONFIG['native_hashes']: self.assertIn((('sha256sum', p),), read.call_args_list)

    def test_usb_identity(self): self.check('PHONE123')
    def test_wireless_ip_with_selected_physical_identity(self): self.check('192.0.2.10:42123', 'PHONE123')
    def test_wireless_mdns_with_selected_physical_identity(self): self.check('adb-PHONE123-test._adb-tls-connect._tcp', 'PHONE123')
    def test_wireless_requires_explicit_physical_identity(self):
        for transport in ['192.0.2.10:42123', 'adb-PHONE123-test._adb-tls-connect._tcp']:
            with self.subTest(transport=transport), self.assertRaisesRegex(RuntimeError, '缺少设备身份'): self.check(transport)
    def test_endpoint_reassigned_to_other_phone_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, '不一致'): self.check('192.0.2.10:42123', 'PHONE123', physical='OTHER')
    def test_usb_other_phone_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, '不一致'): self.check('PHONE123', physical='OTHER')
    def test_unauthorized_device_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, '未授权'): self.check('PHONE123', state='unauthorized')
    def test_invalid_transport_is_rejected(self):
        for transport in ['', '-d', 'phone other']:
            with self.subTest(transport=transport), self.assertRaisesRegex(RuntimeError, '先选择'): self.check(transport)
    def test_wireless_retains_firmware_guard(self):
        with self.assertRaisesRegex(AssertionError, 'Unsupported firmware'): self.check('192.0.2.10:42123', 'PHONE123', firmware='other-build')
    def test_unsupported_firmware_includes_actual_fingerprint(self):
        with self.assertRaisesRegex(AssertionError, r'Unsupported firmware:.*other-build'):
            self.check('PHONE123', firmware='other-build')
    def test_wireless_retains_native_hash_guard(self): self.check('192.0.2.10:42123', 'PHONE123', native=True)


if __name__ == '__main__': unittest.main()
