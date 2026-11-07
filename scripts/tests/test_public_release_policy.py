#!/usr/bin/env python3
"""Privacy boundaries: fail before publishing, preserve developer/user data."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import public_release_policy as policy


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


stage = load('release_stage', 'stage-release-resources.py').stage
export = load('release_export', 'sync-public-repo.py')
package = load('release_package', 'package-release.py')


class PublicationBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'source'
        self.root.mkdir()
        self.policy = {'schema': 1, 'runtime_files': ['scripts/main.py'],
                       'reviewed_media_sha256': {}, 'public_scripts': [], 'public_docs': []}
        self.write('scripts/main.py', 'print("safe")\n')
        self.write('desktop/Cargo.toml', '[package]\nversion="1.2.3"\n')
        self.write('handoff/google-play-stepx/target.example.json', '{"serial":"","sdk":36}')
        self.save_policy()

    def write(self, rel, data):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data.encode() if isinstance(data, str) else data)

    def save_policy(self):
        self.write(policy.MANIFEST, json.dumps(self.policy))

    def test_private_paths_and_traversal_rejected(self):
        for rel in ['../escape.py', '/abs.py', 'docs/review-assets/capture.png',
                    'handoff/evidence/result.json', 'scripts/model.json', 'keys/release.p12',
                    'data/app.db-wal', 'scripts/.env', 'scripts/cache/data.json']:
            with self.subTest(rel=rel):
                self.assertTrue(policy.path_problems(rel))

    def test_secrets_are_reported_without_values(self):
        secret = 'gh' + 'p_' + 'Q' * 30
        findings = policy.content_problems('scripts/main.py', secret.encode(), self.policy)
        self.assertTrue(findings)
        self.assertNotIn(secret, str(findings))

    def test_media_changes_require_new_review(self):
        sample = b'approved-media-content'
        self.policy['reviewed_media_sha256'][hashlib.sha256(sample).hexdigest()] = {}
        self.assertEqual([], policy.content_problems('desktop/assets/guide.png', sample, self.policy))
        self.assertTrue(policy.content_problems('desktop/assets/guide.png', sample + b'changed', self.policy))

    def test_generic_target_never_copies_real_target(self):
        self.write('handoff/google-play-stepx/target.json', '{"serial":"PRIVATE-DEVICE"}')
        output = policy.runtime_plan(self.root)['handoff/google-play-stepx/target.json']
        self.assertEqual('', json.loads(output)['serial'])
        self.assertIn('PRIVATE-DEVICE', (self.root / 'handoff/google-play-stepx/target.json').read_text())

    def test_validation_precedes_writes(self):
        self.write('scripts/main.py', 'sk' + '-' + 'Z' * 30)
        destination = Path(self.temp.name) / 'output'
        with self.assertRaises(ValueError):
            stage(self.root, destination)
        self.assertFalse(destination.exists())

    def test_stage_keeps_user_data_and_only_installs_current_release(self):
        self.policy['runtime_files'] += ['desktop/assets/release-notes/1.2.2.json',
                                        'desktop/assets/release-notes/1.2.3.json']
        self.write('desktop/assets/release-notes/1.2.2.json', '{}')
        self.write('desktop/assets/release-notes/1.2.3.json', '{}')
        self.save_policy()
        destination = Path(self.temp.name) / 'output'
        destination.mkdir()
        (destination / 'user.db').write_bytes(b'user-state')
        stage(self.root, destination)
        self.assertEqual(b'user-state', (destination / 'user.db').read_bytes())
        self.assertFalse((destination / 'desktop/assets/release-notes/1.2.2.json').exists())
        self.assertTrue((destination / 'desktop/assets/release-notes/1.2.3.json').exists())

    def test_source_symlink_rejected(self):
        (self.root / 'scripts/main.py').unlink()
        external = Path(self.temp.name) / 'external.py'
        external.write_text('private')
        (self.root / 'scripts/main.py').symlink_to(external)
        with self.assertRaises(ValueError):
            policy.runtime_plan(self.root)

    def test_unlisted_installed_file_is_rejected(self):
        self.policy['runtime_files'].append(policy.MANIFEST)
        self.save_policy()
        destination = Path(self.temp.name) / 'output'
        stage(self.root, destination)
        (destination / 'scripts/unexpected.py').write_text('print("not reviewed")')
        self.assertTrue(any('unexpected.py' in issue for issue in policy.audit_bundle(destination)))

    def test_fixture_replacement_preserves_identity_relationships(self):
        original = 'ZYPZ' + '261A' + '000000001'
        replaced = export.fixture_placeholders('desktop/src/test.rs', original + ' ' + original)
        first, second = replaced.split()
        self.assertEqual(first, second)
        self.assertNotIn(original, replaced)

    def test_cross_platform_home_input_and_expected_output_remain_consistent(self):
        for family in ['home', 'Users']:
            bare = '/' + family + '/private-developer'
            text = '"' + bare + '" "' + bare + '/Library/data"'
            expected = '"/' + family + '/stepx-user" "/' + family + '/stepx-user/Library/data"'
            self.assertEqual(expected, export.fixture_placeholders('test.rs', text))
        fixture = '"/Users/macuser" "/Users/macuser/Library/data"'
        self.assertEqual(fixture, export.fixture_placeholders('test.rs', fixture))

    def test_runtime_verification_cannot_write_bytecode_to_stage_or_inherited_cache(self):
        # Exercise the actual child-process launcher, not just its argv. The
        # verifier imports a new module then starts another Python process;
        # both must preserve the candidate package byte-for-byte.
        fake_root = Path(self.temp.name) / 'packaging'
        scripts = fake_root / 'scripts'
        scripts.mkdir(parents=True)
        candidate = Path(self.temp.name) / 'candidate'
        candidate.mkdir()
        (candidate / 'probe.py').write_text('VALUE = 42\n')
        (candidate / 'child.py').write_text('VALUE = 7\n')
        (scripts / 'verify-packaged-runtime.py').write_text(
            'import sys, subprocess\n'
            'sys.path.insert(0, sys.argv[1])\n'
            'import probe\n'
            'assert probe.VALUE == 42 and sys.dont_write_bytecode\n'
            'subprocess.run([sys.executable, "-c", '
            '"import child; assert child.VALUE == 7"], cwd=sys.argv[1], check=True)\n')
        before = {p.relative_to(candidate): p.read_bytes() for p in candidate.rglob('*') if p.is_file()}
        inherited = Path(self.temp.name) / 'external-cache'
        with mock.patch.object(package, 'ROOT', fake_root), mock.patch.dict(
                os.environ, {'PYTHONDONTWRITEBYTECODE': '', 'PYTHONPYCACHEPREFIX': str(inherited)}):
            package.verify_runtime(Path(sys.executable), candidate)
        after = {p.relative_to(candidate): p.read_bytes() for p in candidate.rglob('*') if p.is_file()}
        self.assertEqual(before, after)
        self.assertFalse(inherited.exists())


if __name__ == '__main__':
    unittest.main()
