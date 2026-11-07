"""Regression coverage for app-local MSVC deployment; no Windows DLL downloads."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import windows_crt as crt
import public_release_policy as policy


def pe_image(imports=(), delay=(), *, machine=0x8664):
    """Minimal PE32+ with real RVA-based normal/delay import tables."""
    data = bytearray(4096)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 0x3c, 0x80)
    data[0x80:0x84] = b'PE\0\0'
    struct.pack_into('<HH', data, 0x84, machine, 1)
    struct.pack_into('<H', data, 0x94, 240)
    opt = 0x98
    struct.pack_into('<H', data, opt, 0x20b)
    struct.pack_into('<Q', data, opt + 24, 0x140000000)
    struct.pack_into('<I', data, opt + 60, 0x200)
    struct.pack_into('<I', data, opt + 108, 16)
    struct.pack_into('<IIII', data, opt + 240 + 8, 0xe00, 0x1000, 0xe00, 0x200)
    string_at = 0xa00
    for names, directory, at, stride in [(imports, 1, 0x200, 20), (delay, 13, 0x600, 32)]:
        if not names:
            continue
        struct.pack_into('<II', data, opt + 112 + directory * 8,
                         at + 0xe00, (len(names) + 1) * stride)
        for index, name in enumerate(names):
            name_at = string_at + 0xe00
            values = (0, 0, 0, name_at, 0) if directory == 1 else (1, name_at, 0, 0, 0, 0, 0, 0)
            struct.pack_into('<' + 'I' * len(values), data, at + index * stride, *values)
            encoded = name.encode('ascii') + b'\0'
            data[string_at:string_at + len(encoded)] = encoded
            string_at += len(encoded)
    return bytes(data)


class WindowsCrtTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.app = self.root / 'app'
        self.app.mkdir()
        self.exe = self.app / 'stepx-companion.exe'
        self.redist = self.root / 'VS/VC/Redist/MSVC/14.44.35211/x64/Microsoft.VC143.CRT'
        self.redist.mkdir(parents=True)
        self.source = 'VC/Redist/MSVC/14.44.35211/x64/Microsoft.VC143.CRT'
        self.exe.write_bytes(pe_image(['KERNEL32.dll', 'VCRUNTIME140.dll']))
        (self.redist / 'VCRUNTIME140.dll').write_bytes(pe_image(['api-ms-win-crt-runtime-l1-1-0.dll']))

    def stage(self, **kwargs):
        return crt.stage_crt(self.exe, self.redist, '14.44.35207', self.source,
                             version_reader=lambda p: (14, 44, 35211, 0), **kwargs)

    def test_normal_and_delay_imports_are_both_checked(self):
        self.exe.write_bytes(pe_image(['KERNEL32.dll', 'VCRUNTIME140.dll'], ['MSVCP140.dll']))
        self.assertEqual(['kernel32.dll', 'msvcp140.dll', 'vcruntime140.dll'], crt.pe_imports(self.exe))
        with self.assertRaisesRegex(RuntimeError, 'missing msvcp140.dll'):
            self.stage()

    def test_crt_dlls_are_not_treated_as_system_libraries(self):
        for name in crt.CRT_DLLS:
            self.assertFalse(crt.is_system_dll(name))
        self.assertTrue(crt.is_system_dll('api-ms-win-crt-runtime-l1-1-0.dll'))
        self.assertTrue(crt.is_system_dll('KERNEL32.dll'))
        self.assertFalse(crt.is_system_dll('unknown.dll'))

    def test_transitive_dependency_and_case_insensitive_resolution(self):
        (self.redist / 'VCRUNTIME140.dll').write_bytes(pe_image(['VCRUNTIME140_1.dll']))
        (self.redist / 'vcruntime140_1.dll').write_bytes(pe_image(['kernel32.dll']))
        (self.redist / 'msvcp140.dll').write_bytes(pe_image())
        report = self.stage()
        self.assertEqual(['vcruntime140.dll', 'vcruntime140_1.dll'], report['dependencies'])
        self.assertFalse((self.app / 'msvcp140.dll').exists(), 'unused CRT must not be copied')
        self.assertEqual(report, crt.verify_manifest(self.exe))
        self.assertEqual(self.source, report['source'])
        self.assertNotIn(str(self.root), json.dumps(report))
        for name, info in report['files'].items():
            self.assertEqual(hashlib.sha256((self.redist / ('VCRUNTIME140.dll' if name == 'vcruntime140.dll' else name)).read_bytes()).hexdigest(), info['sha256'])

    def test_unknown_dependency_never_resolves_from_path(self):
        self.exe.write_bytes(pe_image(['unreviewed.dll']))
        (self.redist / 'unreviewed.dll').write_bytes(pe_image())
        with mock.patch.dict(os.environ, {'PATH': str(self.redist)}):
            with self.assertRaisesRegex(RuntimeError, 'unknown non-system dependency'):
                self.stage()

    def test_nested_python_crt_does_not_satisfy_main_executable(self):
        nested = self.app / 'runtime/python'
        nested.mkdir(parents=True)
        shutil.copy2(self.redist / 'VCRUNTIME140.dll', nested / 'vcruntime140.dll')
        with self.assertRaisesRegex(ValueError, 'missing app-local dependency'):
            crt.verify_closure(self.exe)

    def test_missing_transitive_dependency_fails_even_if_on_path(self):
        (self.redist / 'VCRUNTIME140.dll').write_bytes(pe_image(['vcruntime140_1.dll']))
        with mock.patch.dict(os.environ, {'PATH': str(self.redist)}):
            with self.assertRaisesRegex(RuntimeError, 'missing vcruntime140_1.dll'):
                self.stage()

    def test_old_crt_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, 'older than active MSVC'):
            crt.stage_crt(self.exe, self.redist, '14.44.35207', self.source,
                          version_reader=lambda p: (14, 43, 99999, 0))

    def test_only_matching_installed_release_toolset_is_selected(self):
        installation = self.root / 'VS'
        (installation / 'VC/Redist/MSVC/14.44.35220/x64/Microsoft.VC143.CRT').mkdir(parents=True)
        (installation / 'VC/Redist/MSVC/14.45.99999/x64/Microsoft.VC143.CRT').mkdir(parents=True)
        (installation / 'VC/Redist/MSVC/14.44.99999/debug_nonredist/x64/Microsoft.VC143.DebugCRT').mkdir(parents=True)
        selected = crt.select_redist(installation, '14.44.35207')
        self.assertIn('14.44.35220', selected.parts)
        with self.assertRaisesRegex(RuntimeError, 'Matching x64'):
            crt.select_redist(installation, '14.42.11111')

    def test_x86_binary_is_rejected(self):
        (self.redist / 'VCRUNTIME140.dll').write_bytes(pe_image(machine=0x14c))
        with self.assertRaisesRegex(ValueError, 'expected x64'):
            self.stage()

    def test_malformed_pe_fails_closed(self):
        for data in [b'', b'MZ' + b'\0' * 62, pe_image()[:300]]:
            self.exe.write_bytes(data)
            with self.assertRaises(ValueError):
                crt.pe_imports(self.exe)

    def test_invalid_rva_fails_closed(self):
        data = bytearray(pe_image(['kernel32.dll']))
        struct.pack_into('<I', data, 0x200 + 12, 0x7fffffff)
        self.exe.write_bytes(data)
        with self.assertRaisesRegex(ValueError, 'RVA outside'):
            crt.pe_imports(self.exe)

    def test_unsafe_dependency_name_is_rejected(self):
        self.exe.write_bytes(pe_image(['../outside.dll']))
        with self.assertRaisesRegex(ValueError, 'unsafe DLL name'):
            crt.pe_imports(self.exe)

    def test_provenance_does_not_accept_machine_paths(self):
        for source in ['/VS/VC/Redist/MSVC', 'VC/Redist/MSVC/../private', 'C:\\VS\\CRT']:
            with self.assertRaisesRegex(ValueError, 'provenance'):
                crt.stage_crt(self.exe, self.redist, '14.44.35207', source)

    def test_relocated_package_preserves_dlls_and_checksums(self):
        self.stage()
        destination = self.root / '中文 安装路径'
        destination.mkdir()
        shutil.copy2(self.exe, destination / self.exe.name)
        crt.relocate_crt(self.exe, destination)
        self.assertEqual(['vcruntime140.dll'], crt.verify_closure(destination / self.exe.name))
        crt.verify_manifest(destination / self.exe.name)
        (destination / 'vcruntime140.dll').unlink()
        with self.assertRaisesRegex(ValueError, 'do not match manifest'):
            crt.verify_manifest(destination / self.exe.name)

    def test_tampered_crt_rejected(self):
        self.stage()
        path = self.app / 'vcruntime140.dll'
        path.write_bytes(path.read_bytes() + b'changed')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            crt.verify_manifest(self.exe)

    def test_extra_or_unlisted_crt_rejected(self):
        self.stage()
        (self.app / 'msvcp140.dll').write_bytes(pe_image())
        with self.assertRaisesRegex(ValueError, 'do not match manifest'):
            crt.verify_manifest(self.exe)

    def test_policy_only_allows_verified_declared_runtime(self):
        self.stage()
        (self.app / 'scripts').mkdir()
        (self.app / policy.MANIFEST).write_text(json.dumps({'schema': 1,
            'runtime_files': [policy.MANIFEST], 'reviewed_media_sha256': {}}))
        self.assertEqual([], policy.audit_bundle(self.app))
        (self.app / 'rogue.dll').write_bytes(pe_image())
        self.assertTrue(any('rogue.dll' in item for item in policy.audit_bundle(self.app)))
        (self.app / 'vcruntime140.dll').unlink()
        self.assertTrue(any('Windows app-local CRT verification failed' in item for item in policy.audit_bundle(self.app)))

    @unittest.skipUnless(os.name == 'nt', 'requires native Visual Studio installation')
    def test_native_visual_studio_redist_version_and_closure(self):
        redist, toolset, source = crt.discover_redist()
        crt.stage_crt(self.exe, redist, toolset, source)
        crt.verify_manifest(self.exe)


if __name__ == '__main__':
    unittest.main()
