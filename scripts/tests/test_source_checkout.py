"""Verify Windows-style Git settings preserve audited Python source bytes."""
import hashlib
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class SourceCheckoutTests(unittest.TestCase):
    def test_autocrlf_checkout_preserves_phone_source_hashes(self):
        paths = [
            'handoff/google-play-stepx/module/launcher.py',
            'handoff/google-play-stepx/module/boot.py',
            'handoff/google-play-stepx/module/inject.py',
            'handoff/google-play-stepx/tools/root_worker.py',
            'scripts/companion_boot/chain.py',
            'scripts/companion_boot/boot.py',
            'scripts/companion_boot/launcher.py',
        ]
        with tempfile.TemporaryDirectory(prefix='stepx-source-checkout-') as folder:
            repo = pathlib.Path(folder)
            def git(*args):
                return subprocess.run(['git', '-C', str(repo), *args], check=True,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            git('init', '-q')
            git('config', 'core.autocrlf', 'true')
            git('config', 'core.eol', 'crlf')
            (repo / '.gitattributes').write_bytes((ROOT / '.gitattributes').read_bytes())
            expected = {}
            for name in paths:
                data = (ROOT / name).read_bytes()
                self.assertNotIn(b'\r', data, name + ': source checkout must use LF')
                expected[name] = hashlib.sha256(data).hexdigest()
                target = repo / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            git('add', '--', '.gitattributes', *paths)
            for name in paths:
                (repo / name).unlink()
            # checkout-index exercises Git's actual smudge/eol conversion with
            # Windows settings, without requiring a commit or account identity.
            git('checkout-index', '--all', '--force')
            for name in paths:
                data = (repo / name).read_bytes()
                self.assertNotIn(b'\r', data, name)
                self.assertEqual(expected[name], hashlib.sha256(data).hexdigest(), name)


if __name__ == '__main__':
    unittest.main()
