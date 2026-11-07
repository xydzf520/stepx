#!/usr/bin/env python3
"""Unit tests for Android APK Signature Scheme v3.1 key-rotation signer parsing."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import companion_apk as apk


class CompanionApkSignerTests(unittest.TestCase):
    def test_signer_digests_parses_standard_and_v3_1_rotation_labels(self):
        d1 = 'a' * 64
        d2 = 'b' * 64
        d3 = 'c' * 64
        d4 = 'd' * 64
        proof = '\n'.join([
            f'Signer (minSdkVersion=33, maxSdkVersion=2147483647) certificate DN: CN=Google',
            f'Signer (minSdkVersion=33, maxSdkVersion=2147483647) certificate SHA-256 digest: {d1}',
            f'Signer (minSdkVersion=24, maxSdkVersion=32) certificate SHA-256 digest: {d2}',
            f'V2 Signer #1: certificate SHA-256 digest: {d3}',
            f'V3.1 Signer (minSdkVersion=33, maxSdkVersion=2147483647): certificate SHA-256 digest: {d4}',
            f'Signer #1 certificate SHA-256 digest: {d1.upper()}',
        ])
        self.assertEqual(apk.signer_digests(proof), {d1, d2, d3, d4})

    def test_apk_identity_accepts_v3_1_multi_signer_rotation(self):
        d1 = '3d7a1223019aa39d9ea0e3436ab7c0896bfb4fb679f4de5fe7c23f326c8f994a'
        d2 = 'afb0fed5eeaebdd86f56a97742f4b6b33ef598751022055b7008469e200070ee'
        proof = (
            f'Signer (minSdkVersion=33, maxSdkVersion=2147483647) certificate SHA-256 digest: {d1}\n'
            f'Signer (minSdkVersion=24, maxSdkVersion=32) certificate SHA-256 digest: {d2}\n'
        )
        with tempfile.TemporaryDirectory() as temp:
            sample = Path(temp) / 'youtube.apk'
            sample.write_bytes(b'apk')
            with patch.object(apk, 'apk_package', return_value='com.google.android.youtube'), \
                 patch.object(apk, 'sdk_command', return_value=['apksigner']), \
                 patch.object(apk, 'run', return_value=proof):
                pkg, cert = apk.apk_identity(sample)
                self.assertEqual(pkg, 'com.google.android.youtube')
                self.assertIn(cert, {d1, d2})

    def test_apk_identity_rejects_empty_signer_digests(self):
        with tempfile.TemporaryDirectory() as temp:
            sample = Path(temp) / 'unsigned.apk'
            sample.write_bytes(b'apk')
            with patch.object(apk, 'apk_package', return_value='com.example.unsigned'), \
                 patch.object(apk, 'sdk_command', return_value=['apksigner']), \
                 patch.object(apk, 'run', return_value='WARNING: no signers'):
                with self.assertRaisesRegex(RuntimeError, 'APK 必须具有一个可验证的签名'):
                    apk.apk_identity(sample)


if __name__ == '__main__':
    unittest.main()
