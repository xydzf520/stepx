"""Download selection must preserve the app and source the user chose."""
import os
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

    def test_huawei_page_resolution_stays_with_huawei_and_passes_page_url(self):
        seen_calls = []
        def fake_resolve_huawei(pkg, page_url=""):
            seen_calls.append((pkg, page_url))
            return ['https://appdl-drcn.dbankcdn.com/dl/appdl/application/apk/87/87521cf0b7f2403e885734de2b331cab/app.huawei.auto.2609021718.apk']
        self.ns['resolve_huawei'] = fake_resolve_huawei
        self.ns['resolve_tencent_detail'] = lambda _: self.fail('Unexpected Tencent fallback')
        cands = list(self.ns['iter_candidates']('https://appgallery.huawei.com/app/C104871165', 'app.huawei.auto'))
        self.assertEqual(
            cands,
            ['https://appdl-drcn.dbankcdn.com/dl/appdl/application/apk/87/87521cf0b7f2403e885734de2b331cab/app.huawei.auto.2609021718.apk'],
        )
        self.assertEqual(seen_calls, [('app.huawei.auto', 'https://appgallery.huawei.com/app/C104871165')])

    def test_resolve_huawei_normalizes_hispace_url_and_keeps_signed_fallback(self):
        import io
        import json
        import urllib.request

        class FakeResp(io.BytesIO):
            def __enter__(self):
                return self
            def __exit__(self, *args):
                pass

        # 1. Direct package|app.huawei.auto resolution + normalization
        def fake_urlopen_pkg(req, timeout=None):
            u = req.full_url
            if 'getInterfaceCode' in u:
                return FakeResp(json.dumps('FAKE_ICODE').encode('utf-8'))
            if 'package%7Capp.huawei.auto' in u:
                return FakeResp(json.dumps({
                    'layoutData': [{
                        'dataList': [{
                            'package': 'app.huawei.auto',
                            'downurl': 'https://store-drcn.hispace.dbankcloud.com/dl/appdl/application/apk/87/87521cf0b7f2403e885734de2b331cab/app.huawei.auto.2609021718.apk?sign=abc',
                        }]
                    }]
                }).encode('utf-8'))
            return FakeResp(json.dumps({'layoutData': []}).encode('utf-8'))

        with patch.object(urllib.request, 'urlopen', side_effect=fake_urlopen_pkg):
            cands = self.ns['resolve_huawei']('app.huawei.auto', 'https://appgallery.huawei.com/app/C104871165')
        self.assertEqual(
            cands,
            [
                'https://appdl-drcn.dbankcdn.com/dl/appdl/application/apk/87/87521cf0b7f2403e885734de2b331cab/app.huawei.auto.2609021718.apk',
                'https://store-drcn.hispace.dbankcloud.com/dl/appdl/application/apk/87/87521cf0b7f2403e885734de2b331cab/app.huawei.auto.2609021718.apk?sign=abc',
            ],
        )

        # 2. Fallback to app|C108819761 when package| returns empty
        def fake_urlopen_appid_fallback(req, timeout=None):
            u = req.full_url
            if 'getInterfaceCode' in u:
                return FakeResp(json.dumps('FAKE_ICODE').encode('utf-8'))
            if 'app%7CC108819761' in u:
                return FakeResp(json.dumps({
                    'layoutData': [{
                        'dataList': [{
                            'package': 'app.huawei.motor',
                            'downurl': 'https://store-drcn.hispace.dbankcloud.com/dl/appdl/application/apk/1c/1c71baebf09e49ce80b74fb13dd8ab5a/app.huawei.motor.2610191940.apk?sign=xyz',
                        }]
                    }]
                }).encode('utf-8'))
            return FakeResp(json.dumps({'layoutData': []}).encode('utf-8'))

        with patch.object(urllib.request, 'urlopen', side_effect=fake_urlopen_appid_fallback):
            cands_motor = self.ns['resolve_huawei']('app.huawei.motor', 'https://appgallery.huawei.com/app/C108819761')
        self.assertEqual(
            cands_motor,
            [
                'https://appdl-drcn.dbankcdn.com/dl/appdl/application/apk/1c/1c71baebf09e49ce80b74fb13dd8ab5a/app.huawei.motor.2610191940.apk',
                'https://store-drcn.hispace.dbankcloud.com/dl/appdl/application/apk/1c/1c71baebf09e49ce80b74fb13dd8ab5a/app.huawei.motor.2610191940.apk?sign=xyz',
            ],
        )

    @unittest.skipUnless(os.environ.get('STEPX_LIVE_STORE_TESTS') == '1',
                         'Live Huawei probe: explicitly set STEPX_LIVE_STORE_TESTS=1')
    def test_live_aito_and_hima_huawei_resolution_and_manifest_package(self):
        import struct
        import urllib.request
        import zlib

        def parse_axml_manifest_attrs(data):
            self.assertGreaterEqual(len(data), 8, 'AXML data too short')
            xml_type, xml_header_size, _xml_size = struct.unpack_from('<HHI', data, 0)
            self.assertEqual(xml_type, 0x0003, f'Expected RES_XML_TYPE (0x0003), got 0x{xml_type:04x}')
            pos = xml_header_size
            strings = []
            while pos + 8 <= len(data):
                chunk_type, header_size, chunk_size = struct.unpack_from('<HHI', data, pos)
                if chunk_size < 8 or pos + chunk_size > len(data):
                    break
                if chunk_type == 0x0001:  # RES_STRING_POOL_TYPE
                    scount, _style_count, flags, strings_start, _styles_start = struct.unpack_from('<IIIII', data, pos + 8)
                    is_utf8 = bool(flags & (1 << 8))
                    offsets_base = pos + header_size
                    pool_base = pos + strings_start
                    for i in range(scount):
                        str_off = struct.unpack_from('<I', data, offsets_base + i * 4)[0]
                        p = pool_base + str_off
                        if is_utf8:
                            u16_len = data[p]
                            p += 2 if (u16_len & 0x80) else 1
                            b_len = data[p]
                            p += 1
                            if b_len & 0x80:
                                b_len = ((b_len & 0x7F) << 8) | data[p]
                                p += 1
                            strings.append(data[p:p + b_len].decode('utf-8', errors='replace'))
                        else:
                            u16_len = struct.unpack_from('<H', data, p)[0]
                            p += 2
                            if u16_len & 0x8000:
                                u16_len = ((u16_len & 0x7FFF) << 16) | struct.unpack_from('<H', data, p)[0]
                                p += 2
                            strings.append(data[p:p + u16_len * 2].decode('utf-16le', errors='replace'))
                elif chunk_type == 0x0102:  # RES_XML_START_ELEMENT_TYPE
                    _ns_idx, name_idx = struct.unpack_from('<ii', data, pos + 16)
                    attr_start, attr_size, attr_count = struct.unpack_from('<HHH', data, pos + 24)
                    elem_name = strings[name_idx] if 0 <= name_idx < len(strings) else ''
                    if elem_name == 'manifest':
                        attrs = {}
                        attr_base = pos + 16 + attr_start
                        for i in range(attr_count):
                            a_off = attr_base + i * attr_size
                            _a_ns, a_name, a_raw_val, _a_val_size, _res0, a_val_type, a_val_data = struct.unpack_from('<iiiHBBI', data, a_off)
                            attr_name = strings[a_name] if 0 <= a_name < len(strings) else ''
                            if a_val_type == 0x03 and 0 <= a_val_data < len(strings):
                                attrs[attr_name] = strings[a_val_data]
                            elif 0 <= a_raw_val < len(strings):
                                attrs[attr_name] = strings[a_raw_val]
                            else:
                                attrs[attr_name] = a_val_data
                        return attrs
                pos += chunk_size
            self.fail('Root <manifest> element not found in binary AXML')

        def extract_remote_apk_manifest(apk_url):
            ua = self.ns['UA_MOBILE']
            req_head = urllib.request.Request(apk_url, headers={'User-Agent': ua, 'Range': 'bytes=0-3'})
            with urllib.request.urlopen(req_head, timeout=10) as r:
                magic = r.read(4)
                content_range = r.headers.get('Content-Range') or ''
                total = int(content_range.split('/')[-1]) if '/' in content_range else int(r.headers.get('Content-Length') or 0)
            self.assertEqual(magic, b'PK\x03\x04', f'Expected ZIP magic header from {apk_url}')
            self.assertGreater(total, 100_000_000, f'Expected full car APK size >100MB from {apk_url}')

            tail_len = min(65536, total)
            req_tail = urllib.request.Request(
                apk_url,
                headers={'User-Agent': ua, 'Range': f'bytes={total - tail_len}-{total - 1}'},
            )
            with urllib.request.urlopen(req_tail, timeout=10) as r:
                tail = r.read()
            eocd = tail.rfind(b'PK\x05\x06')
            self.assertGreaterEqual(eocd, 0, 'ZIP End-of-Central-Directory must be present')
            cd_size, cd_offset = struct.unpack_from('<II', tail, eocd + 12)
            req_cd = urllib.request.Request(
                apk_url,
                headers={'User-Agent': ua, 'Range': f'bytes={cd_offset}-{cd_offset + cd_size - 1}'},
            )
            with urllib.request.urlopen(req_cd, timeout=10) as r:
                cd = r.read()
            pos = 0
            while pos + 46 <= len(cd) and cd[pos:pos + 4] == b'PK\x01\x02':
                comp_method = struct.unpack_from('<H', cd, pos + 10)[0]
                comp_size, _uncomp_size, fname_len, extra_len, comment_len = struct.unpack_from('<IIHHH', cd, pos + 20)
                lfh_offset = struct.unpack_from('<I', cd, pos + 42)[0]
                fname = cd[pos + 46:pos + 46 + fname_len].decode('utf-8', errors='ignore')
                if fname == 'AndroidManifest.xml':
                    req_lfh = urllib.request.Request(
                        apk_url,
                        headers={'User-Agent': ua, 'Range': f'bytes={lfh_offset}-{lfh_offset + 30 + fname_len + 256 + comp_size}'},
                    )
                    with urllib.request.urlopen(req_lfh, timeout=10) as r:
                        lfh = r.read()
                    lfh_fname_len, lfh_extra_len = struct.unpack_from('<HH', lfh, 26)
                    start = 30 + lfh_fname_len + lfh_extra_len
                    raw = lfh[start:start + comp_size]
                    return zlib.decompress(raw, -15) if comp_method == 8 else raw
                pos += 46 + fname_len + extra_len + comment_len
            self.fail(f'AndroidManifest.xml not found in central directory of {apk_url}')

        for pkg, page_url in (
            ('app.huawei.auto', 'https://appgallery.huawei.com/app/C104871165'),
            ('app.huawei.motor', 'https://appgallery.huawei.com/app/C108819761'),
            ('app.huawei.auto', 'https://appgallery.huawei.com/app/C00000000'),
        ):
            cands = list(self.ns['iter_candidates'](page_url, pkg))
            self.assertTrue(cands, f'Huawei AppGallery must resolve at least one APK URL for {pkg} ({page_url})')
            self.assertTrue(
                cands[0].startswith('https://appdl-drcn.dbankcdn.com/dl/appdl/application/apk/'),
                f'First candidate should be normalized dbankcdn APK URL, got: {cands[0]}',
            )
            manifest = extract_remote_apk_manifest(cands[0])
            attrs = parse_axml_manifest_attrs(manifest)
            self.assertEqual(
                attrs.get('package'),
                pkg,
                f'Parsed <manifest package="..."> of {cands[0]} must equal {pkg}, got {attrs}',
            )
            self.assertGreater(int(attrs.get('versionCode') or 0), 0)
            self.assertTrue(bool(attrs.get('versionName')))

    def test_end_to_end_download_stream_resume_truncation_and_huawei_fallback(self):
        import http.server
        import socketserver
        import subprocess
        import tempfile
        import threading

        payload = b'PK\x03\x04' + (b'AITO_STREAM_TEST_' * 16384)
        total_len = len(payload)
        half_len = total_len // 2
        state = {'range_requests': 0, 'mode': 'resume'}

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, fmt, *args):
                pass
            def do_GET(self):
                if state['mode'] == 'truncate':
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/vnd.android.package-archive')
                    self.send_header('Content-Length', str(total_len))
                    self.end_headers()
                    self.wfile.write(payload[:half_len])
                    self.wfile.flush()
                    return
                rng = self.headers.get('Range') or ''
                if rng.startswith('bytes='):
                    state['range_requests'] += 1
                    start = int(rng.split('=')[1].split('-')[0])
                    rem = payload[start:]
                    self.send_response(206)
                    self.send_header('Content-Type', 'application/vnd.android.package-archive')
                    self.send_header('Content-Length', str(len(rem)))
                    self.end_headers()
                    self.wfile.write(rem)
                    self.wfile.flush()
                else:
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/vnd.android.package-archive')
                    self.send_header('Content-Length', str(total_len))
                    self.end_headers()
                    self.wfile.write(payload[:half_len])
                    self.wfile.flush()

        with socketserver.TCPServer(('127.0.0.1', 0), Handler) as httpd:
            port = httpd.server_address[1]
            t = threading.Thread(target=httpd.serve_forever, daemon=True)
            t.start()
            with tempfile.TemporaryDirectory() as tmpdir:
                dest_ok = Path(tmpdir) / 'aito_resumed.apk'
                r_ok = subprocess.run(
                    [sys.executable, '-c', SCRIPT, f'http://127.0.0.1:{port}/aito.apk', str(dest_ok), 'app.huawei.auto'],
                    capture_output=True,
                    text=True,
                    timeout=15,
                )
                self.assertEqual(r_ok.returncode, 0, f'Range resume download must succeed: {r_ok.stderr}')
                self.assertGreaterEqual(state['range_requests'], 1, 'Expected HTTP Range request after partial stream')
                self.assertEqual(dest_ok.read_bytes(), payload)
                self.assertIn('DONE', r_ok.stdout)

                state['mode'] = 'truncate'
                dest_trunc = Path(tmpdir) / 'aito_truncated.apk'
                r_trunc = subprocess.run(
                    [sys.executable, '-c', SCRIPT, f'http://127.0.0.1:{port}/trunc.apk', str(dest_trunc), 'app.huawei.auto'],
                    capture_output=True,
                    text=True,
                    timeout=15,
                )
                self.assertNotEqual(r_trunc.returncode, 0, 'Truncated APK stream without Range support must fail')
                self.assertFalse(dest_trunc.exists(), 'Truncated APK must not be left at destination path')
            httpd.shutdown()

    def test_full_download_script_compiles(self):
        compile(SCRIPT, 'store-download', 'exec')

if __name__ == '__main__':
    unittest.main()
