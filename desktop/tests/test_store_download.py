"""Download selection must preserve the app and source the user chose."""
import re
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / 'src/app_store.rs'
SCRIPT = re.search(r'const PY_DOWNLOAD_APK_STREAM: &str = r#"(.*?)"#;', SOURCE.read_text(), re.S).group(1)

class StoreDownloadTests(unittest.TestCase):
    def setUp(self):
        self.ns = {}
        with patch.object(sys, 'argv', ['probe', 'https://example.test/app.apk', '/unused/app.apk', 'com.example.target']):
            exec(compile(SCRIPT.split('os.makedirs(os.path.dirname(dest)')[0], 'store-resolvers', 'exec'), self.ns)

    def test_pinned_version_never_falls_back_to_another_source(self):
        self.ns['get_text'] = lambda *_: self.fail('Pinned URL must not query other stores')
        self.assertEqual(list(self.ns['iter_candidates']('https://example.test/v1.apk', 'com.example.target')), ['https://example.test/v1.apk'])

    def test_other_search_result_is_not_the_target_apk(self):
        self.ns['get_text'] = lambda *_: '{"datalist":{"list":[{"package":"com.example.other","file":{"path":"https://example.test/other.apk"}}]}}'
        self.assertEqual(self.ns['resolve_aptoide_search']('com.example.target'), [])

    def test_page_resolution_stays_with_selected_source(self):
        self.ns['resolve_fdroid'] = lambda _: ['https://f-droid.org/repo/test.apk']
        self.ns['resolve_aptoide_meta'] = lambda _: self.fail('Unexpected alternate store')
        self.assertEqual(list(self.ns['iter_candidates']('https://f-droid.org/packages/com.example.target', 'com.example.target')), ['https://f-droid.org/repo/test.apk'])

    def test_full_download_script_compiles(self):
        compile(SCRIPT, 'store-download', 'exec')

if __name__ == '__main__':
    unittest.main()
