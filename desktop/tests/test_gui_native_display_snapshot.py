"""Offline guard and payload checks. These tests never invoke ADB or a worker."""
import copy
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('snapshot', ROOT / 'scripts/gui-native-display-snapshot.py')
snapshot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(snapshot)


def sample():
    return dict(version=1, mode='native_gui_display_snapshot', pid=1234, uid=1000, read_only=True,
                ok=True, initialized_wms_singleton=True, redirect_ref_count=0, redirect_component=None,
                stable_observation=True, atomic_snapshot=False, control_packages=['com.stepos.ability'],
                control_package_count=1, ability_control_allowed=True)


class DisplaySnapshotTests(unittest.TestCase):
    def test_valid_snapshot(self):
        self.assertEqual(snapshot.validate_snapshot(sample(), 1234), sample())

    def test_invalid_snapshot_rejected(self):
        for key, value in [('pid', 1235), ('uid', 0), ('read_only', False), ('ok', False),
                           ('stable_observation', False), ('atomic_snapshot', True),
                           ('redirect_ref_count', -1), ('redirect_ref_count', True),
                           ('redirect_component', 'token=secret'), ('control_package_count', 2),
                           ('control_packages', ['../outside']), ('ability_control_allowed', False)]:
            data = sample(); data[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(RuntimeError):
                snapshot.validate_snapshot(data, 1234)
        data = sample(); data['secret'] = 'never accepted'
        with self.assertRaises(RuntimeError):
            snapshot.validate_snapshot(data, 1234)

    def test_compare_requires_same_lifetime(self):
        proof = dict(schema='stepx_native_gui_display_snapshot_v1', process=dict(pid=1234, start_ticks='50', boot_id='a'),
                     image_hashes=snapshot.IMAGE_HASHES, snapshot=sample())
        self.assertTrue(all(snapshot.compare(proof, copy.deepcopy(proof)).values()))
        changed = copy.deepcopy(proof); changed['snapshot']['redirect_ref_count'] = 1
        self.assertFalse(snapshot.compare(proof, changed)['redirect_ref_count'])
        changed['process']['start_ticks'] = '51'
        with self.assertRaises(RuntimeError):
            snapshot.compare(proof, changed)

    def test_only_fixed_readback_payload_is_injected(self):
        if not snapshot.DEX.is_file():
            self.skipTest('optional local display snapshot research DEX not built')
        code = snapshot.build_worker(snapshot.DEX.read_bytes(), dict(pid=1234, start_ticks='50', boot_id='a' * 36))
        self.assertIn("cstr('stepx.NativeGuiDisplaySnapshot')", code)
        self.assertNotIn('GmsProbe', code)
        self.assertNotIn('ZGV4C', code.split('\n', 1)[1])
        self.assertIn("split()[19]==SNAPSHOT_START", code)
        self.assertIn("strip()==SNAPSHOT_BOOT", code)
        self.assertEqual(code.count("run=method(clazz,'run','()Ljava/lang/String;',True)"), 1)
        with self.assertRaises(RuntimeError):
            snapshot.build_worker(b'dex\nnot-reviewed', dict(pid=1234, start_ticks='50', boot_id='a' * 36))

    def test_java_has_no_control_entry(self):
        source = (ROOT / 'desktop/gui-native/NativeGuiDisplaySnapshot.java').read_text()
        for token in ('getMethod("omniCall"', '.set(null,', '.set(manager,', 'incrementAndGet(', 'decrementAndGet(',
                      'new Thread(', 'LocalServerSocket', 'getConstructor('):
            self.assertNotIn(token, source)
        self.assertIn('mRedirectRefCount', source)
        self.assertIn('mRedirectActivityComponent', source)


if __name__ == '__main__':
    unittest.main()
