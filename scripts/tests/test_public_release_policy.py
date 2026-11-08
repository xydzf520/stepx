#!/usr/bin/env python3
"""Privacy boundaries: fail before publishing, preserve developer/user data."""
import hashlib
import importlib.util
import io
import json
import os
import subprocess
from pathlib import Path
import sys
import tempfile
import unittest
import urllib.error
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import public_release_policy as policy
import release_download as downloads


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


stage = load('release_stage', 'stage-release-resources.py').stage
export = load('release_export', 'sync-public-repo.py')
package = load('release_package', 'package-release.py')


class ToolDownloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.target = Path(self.temp.name) / 'tool.tar.gz'
        self.url = 'https://github.com/example/tool/releases/download/v1/tool.tar.gz'
        self.payload = b'verified-upstream-tool'
        self.digest = hashlib.sha256(self.payload).hexdigest()
        self.sleep = mock.patch.object(downloads.time, 'sleep').start()
        self.addCleanup(mock.patch.stopall)

    def test_transient_failure_retries_fresh_request_and_keeps_digest_verification(self):
        failure = urllib.error.HTTPError(self.url, 504, 'Gateway timeout', {}, None)
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=[failure, io.BytesIO(self.payload)]) as opener:
            package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(self.target.read_bytes(), self.payload)
        self.assertEqual(opener.call_args_list[0].args[0].full_url, self.url)
        self.assertTrue(opener.call_args_list[1].args[0].full_url.startswith(self.url + '?stepx_retry='))
        self.assertEqual(opener.call_args_list[1].kwargs['timeout'], 30)
        self.assertEqual(list(self.target.parent.glob('*.part')), [])

    def test_wrong_hash_never_replaces_existing_archive_or_retries(self):
        self.target.write_bytes(b'previous-verified-archive')
        with mock.patch.object(downloads.urllib.request, 'urlopen', return_value=io.BytesIO(b'wrong-file')) as opener:
            with self.assertRaisesRegex(RuntimeError, 'SHA256 mismatch'):
                package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(opener.call_count, 1)
        self.assertEqual(self.target.read_bytes(), b'previous-verified-archive')
        self.assertEqual(list(self.target.parent.glob('*.part')), [])

    def test_upstream_gateway_failure_uses_same_official_release_asset(self):
        api = 'https://api.github.com/repos/example/tool/releases/assets/123'
        failure = urllib.error.HTTPError(self.url, 504, 'Gateway timeout', {}, None)
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=[failure, io.BytesIO(self.payload)]) as opener:
            package.download_verified_tool(self.url, self.target, self.digest, asset_api_url=api)
        request = opener.call_args_list[1].args[0]
        self.assertTrue(request.full_url.startswith(api + '?stepx_retry='))
        self.assertEqual(request.get_header('Accept'), 'application/octet-stream')
        self.assertEqual(self.target.read_bytes(), self.payload)

    def test_network_failure_has_bounded_retries_and_leaves_no_archive(self):
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=urllib.error.URLError('offline')) as opener:
            with self.assertRaises(urllib.error.URLError):
                package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(opener.call_count, 3)
        self.assertFalse(self.target.exists())
        self.assertEqual(list(self.target.parent.glob('*.part')), [])

    def test_permanent_http_failure_is_not_retried(self):
        failure = urllib.error.HTTPError(self.url, 404, 'Not found', {}, None)
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=failure) as opener:
            with self.assertRaises(urllib.error.HTTPError):
                package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(opener.call_count, 1)
        self.assertFalse(self.target.exists())


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

    def test_crlf_checkout_stages_identical_python_bytes_and_hashes_without_rewriting_other_inputs(self):
        source = '# coding: utf-8\nprint("你好")\n'.encode('utf-8')
        windows_source = source.replace(b'\n', b'\r\n')
        # Exercise the actual staging entry point, not only the normalizer.
        # Even an explicitly allowlisted vendor .py must remain byte-for-byte.
        untouched = {
            'desktop/assets/runtime.bin': b'\x00\xffbinary\r\n\rpayload',
            'runtime/python/Lib/vendor.py': b'# vendor\r\nVALUE = 42\r\n',
            'tools/vendor.py': b'# external tool\r\nVALUE = 7\r\n',
        }
        self.policy['runtime_files'] += [policy.MANIFEST, *untouched]
        self.save_policy()
        self.write('scripts/main.py', windows_source)
        for rel, data in untouched.items():
            self.write(rel, data)
        destination = Path(self.temp.name) / 'windows-stage'
        stage(self.root, destination)
        self.assertEqual(source, (destination / 'scripts/main.py').read_bytes())
        self.assertEqual(windows_source, (self.root / 'scripts/main.py').read_bytes(),
                         'packaging must not mutate the developer checkout')
        digests = json.loads((destination / 'release-resources.sha256.json').read_bytes())
        self.assertEqual(hashlib.sha256(source).hexdigest(), digests['scripts/main.py'])
        for rel, data in untouched.items():
            self.assertEqual(data, (destination / rel).read_bytes())
            self.assertEqual(hashlib.sha256(data).hexdigest(), digests[rel])
        for rel, digest in digests.items():
            self.assertEqual(hashlib.sha256((destination / rel).read_bytes()).hexdigest(), digest)
        self.assertEqual([], policy.audit_bundle(destination))

        self.write('scripts/main.py', source)
        reference = Path(self.temp.name) / 'lf-stage'
        stage(self.root, reference)
        self.assertEqual(digests, json.loads((reference / 'release-resources.sha256.json').read_bytes()),
                         'the same source checkout must produce the same resource digests on every OS')

    def test_noncanonical_packaged_python_is_rejected_even_with_a_matching_resource_digest(self):
        self.policy['runtime_files'].append(policy.MANIFEST)
        self.save_policy()
        destination = Path(self.temp.name) / 'output'
        stage(self.root, destination)
        for broken in [b'print("safe")\r\n', b'print("safe")\r', b'# invalid UTF-8 \xff\n']:
            with self.subTest(data=broken):
                (destination / 'scripts/main.py').write_bytes(broken)
                hashes = json.loads((destination / 'release-resources.sha256.json').read_bytes())
                hashes['scripts/main.py'] = hashlib.sha256(broken).hexdigest()
                (destination / 'release-resources.sha256.json').write_text(json.dumps(hashes))
                issues = policy.audit_bundle(destination)
                self.assertTrue(any('scripts/main.py: first-party Python source' in issue for issue in issues))

    def test_invalid_first_party_python_fails_before_staging_any_files(self):
        for index, broken in enumerate([b'print("safe")\r', b'# invalid UTF-8 \xff\n']):
            with self.subTest(data=broken):
                self.write('scripts/main.py', broken)
                destination = Path(self.temp.name) / f'invalid-stage-{index}'
                with self.assertRaises(ValueError):
                    stage(self.root, destination)
                self.assertFalse(destination.exists())

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

    def test_public_phone_fixture_preserves_normalization_and_is_idempotent(self):
        original = '133' + '2298' + '8887'
        formats = [original, '+86' + original, '0086' + original,
                   '86' + original, '+86 ' + original[:3] + '-' + original[3:7] + '-' + original[7:]]
        clean = export.fixture_placeholders('desktop/src/normalization.rs', ' '.join('"' + v + '"' for v in formats))
        self.assertNotIn(original, clean)
        local = clean.split('"')[1]
        self.assertTrue(local.startswith('1380000'))
        self.assertIn('+86' + local, clean)
        self.assertIn('0086' + local, clean)
        self.assertIn('+86 ' + local[:3] + '-' + local[3:7] + '-' + local[7:], clean)
        self.assertEqual(clean, export.fixture_placeholders('desktop/src/normalization.rs', clean))

    def test_public_recipient_fixtures_stay_distinct(self):
        originals = ['139' + '1024' + '6688', '186' + '2100' + '5520']
        clean = [export.fixture_placeholders('desktop/src/test.rs', v) for v in originals]
        self.assertEqual(len(set(clean)), len(originals))
        self.assertFalse(set(clean) & set(originals))

    def test_existing_public_inputs_receive_full_policy_before_writes(self):
        public = Path(self.temp.name) / 'public'
        public.mkdir()
        subprocess.run(['git', 'init', '-q', str(public)], check=True)
        (public / 'old.db').write_bytes(b'private runtime database')
        (public / 'old.png').write_bytes(b'unreviewed screenshot')
        with mock.patch.object(export, 'ROOT', self.root), mock.patch.object(export, 'PUBLIC_ROOT', public):
            findings = export.scan_public_repo({})
        self.assertTrue(any('old.db' in f for f in findings))
        self.assertTrue(any('old.png' in f for f in findings))

    def test_tracked_ignored_private_files_are_not_silently_skipped(self):
        public = Path(self.temp.name) / 'public'
        public.mkdir()
        subprocess.run(['git', 'init', '-q', str(public)], check=True)
        (public / 'state').mkdir()
        (public / 'state/device.json').write_text('{}')
        subprocess.run(['git', '-C', str(public), 'add', 'state/device.json'], check=True)
        (public / '.gitignore').write_text('state/\n')
        with mock.patch.object(export, 'ROOT', self.root), mock.patch.object(export, 'PUBLIC_ROOT', public):
            findings = export.scan_public_repo({})
        self.assertTrue(any('state/device.json' in f for f in findings))

    def test_retired_media_removed_only_from_public_copy_after_preflight(self):
        rel = 'docs/videos/obsolete.mp4'
        self.policy['public_removed_files'] = [rel]
        self.save_policy()
        self.write(rel, b'local recording retained')
        public = Path(self.temp.name) / 'public'
        (public / rel).parent.mkdir(parents=True)
        (public / rel).write_bytes(b'old public output')
        with mock.patch.object(export, 'ROOT', self.root), mock.patch.object(export, 'PUBLIC_ROOT', public):
            export.execute_sync({}, check_only=True)
            self.assertTrue((public / rel).exists())
            export.execute_sync({}, check_only=False)
        self.assertFalse((public / rel).exists())
        self.assertEqual(b'local recording retained', (self.root / rel).read_bytes())

    def test_retired_media_cannot_escape_public_videos(self):
        for rel in ['../private', 'state/user.db', 'desktop/src/main.rs']:
            self.policy['public_removed_files'] = [rel]
            self.save_policy()
            with mock.patch.object(export, 'ROOT', self.root), self.assertRaises(ValueError):
                export.publication_removals({})

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

    def test_packaged_documents_keep_local_files_and_pin_omitted_resources(self):
        source = ('[安装](GETTING_STARTED.md#usb)\n'
                  '[视频](videos/产品介绍.mp4 "演示")\n'
                  '![截图](images/home.png)\n'
                  '[指南](<../desktop/neo-agent-adapter/README.md#使用>)\n'
                  '[索引][guide]\n[guide]: STORAGE_EXPANSION.md#内容\n'
                  '[远程](https://example.com/docs) [本页](#目的)\n'
                  '```md\n[示例](examples/local.md)\n```\n')
        result = package.package_document('docs/RELEASE_NOTES.md', source.encode(), '1.2.3',
                                          {'docs/GETTING_STARTED.md'}).decode()
        self.assertIn('[安装](GETTING_STARTED.md#usb)', result)
        self.assertIn('releases/download/v1.2.3/%E4%BA%A7%E5%93%81%E4%BB%8B%E7%BB%8D.mp4 "演示"', result)
        self.assertIn('![截图](https://raw.githubusercontent.com/xydzf520/stepx/v1.2.3/docs/images/home.png)', result)
        self.assertIn('<https://github.com/xydzf520/stepx/blob/v1.2.3/desktop/neo-agent-adapter/README.md#使用>', result)
        self.assertIn('[guide]: https://github.com/xydzf520/stepx/blob/v1.2.3/docs/STORAGE_EXPANSION.md#内容', result)
        self.assertIn('[远程](https://example.com/docs) [本页](#目的)', result)
        self.assertIn('```md\n[示例](examples/local.md)\n```', result)

    def test_packaged_document_cannot_link_outside_repository(self):
        with self.assertRaisesRegex(ValueError, 'leaves the source tree'):
            package.package_document('README.md', b'[private](../private.txt)', '1.2.3', set())

    def test_maintenance_package_keeps_video_links_on_the_recorded_release(self):
        source = ('[产品](docs/videos/StepX-product-0.2.180.mp4)\n'
                  '[更新](docs/videos/StepX-0.2.60-to-0.2.180.mp4)\n')
        result = package.package_document('README.md', source.encode(), '0.2.181', set()).decode()
        self.assertIn('releases/download/v0.2.180/StepX-product-0.2.180.mp4', result)
        self.assertIn('releases/download/v0.2.180/StepX-0.2.60-to-0.2.180.mp4', result)
        self.assertNotIn('v0.2.181', result)

    def test_maintenance_release_stages_apk_without_fabricating_new_videos(self):
        self.write('desktop/assets/stepx-companion.apk', b'signed-apk-fixture')
        dist = Path(self.temp.name)/'dist'
        names = package.stage_release_media(self.root, dist, '0.2.181')
        self.assertEqual(names, ['StepX-companion.apk'])
        self.assertEqual((dist/'StepX-companion.apk').read_bytes(), b'signed-apk-fixture')
        self.assertEqual((dist/'media-sha256.txt').read_text().splitlines(),
                         [hashlib.sha256(b'signed-apk-fixture').hexdigest()+'  StepX-companion.apk'])

    def test_release_video_pair_is_atomic_and_each_file_must_be_nonempty(self):
        self.write('desktop/assets/stepx-companion.apk', b'signed-apk-fixture')
        product = 'docs/videos/StepX-product-0.2.181.mp4'
        update = 'docs/videos/StepX-0.2.60-to-0.2.181.mp4'
        self.write(product, b'product-video-fixture')
        dist = Path(self.temp.name)/'dist'
        with self.assertRaisesRegex(ValueError, 'both product and update'):
            package.stage_release_media(self.root, dist, '0.2.181')
        self.assertFalse(dist.exists())
        self.write(update, b'')
        with self.assertRaisesRegex(ValueError, 'empty'):
            package.stage_release_media(self.root, dist, '0.2.181')
        self.assertFalse(dist.exists())
        self.write(update, b'update-video-fixture')
        names = package.stage_release_media(self.root, dist, '0.2.181')
        self.assertEqual(len(names), 3)
        self.assertEqual(len((dist/'media-sha256.txt').read_text().splitlines()), 3)


if __name__ == '__main__':
    unittest.main()
