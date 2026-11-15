#!/usr/bin/env python3
"""Read-only certification diagnostics. Never changes identity, accounts or integrity verdicts."""
import json
import time
import urllib.request
from html.parser import HTMLParser
from common import STATE, guard, shell, save_json

SOURCE = 'https://storage.googleapis.com/play_public/supported_devices.html'


class DeviceTable(HTMLParser):
    def __init__(self):
        super().__init__()
        self.rows, self.row, self.cell = [], [], None

    def handle_starttag(self, tag, attrs):
        if tag == 'tr':
            self.row = []
        elif tag in ('td', 'th'):
            self.cell = []

    def handle_data(self, text):
        if self.cell is not None:
            self.cell.append(text)

    def handle_endtag(self, tag):
        if tag in ('td', 'th') and self.cell is not None:
            self.row.append(' '.join(''.join(self.cell).split()))
            self.cell = None
        elif tag == 'tr' and len(self.row) == 4:
            self.rows.append(self.row)


def listed(html, identity):
    table = DeviceTable()
    table.feed(html)
    if '</table>' not in html.lower() or len(table.rows) < 2 or [s.casefold() for s in table.rows[0]] != ['retail branding', 'marketing name', 'device', 'model']:
        raise ValueError('Google 官方列表格式变化，暂不能判断')
    brands = {identity['brand'].strip().casefold(), identity['manufacturer'].strip().casefold()} - {''}
    model = identity['model'].strip().casefold()
    if not model or not brands:
        raise ValueError('机型信息不完整')
    # A shared device codename (e.g. pandora) is not evidence of certification.
    return any(r[0].casefold() in brands and r[3].casefold() == model for r in table.rows[1:])


def catalog(identity):
    now = time.time()
    stale_cached = None
    try:
        cached = json.loads((STATE / 'certification-catalog.json').read_text(encoding='utf-8'))
        if cached.get('identity') == identity and isinstance(cached.get('listed'), bool):
            if 0 <= now - cached['checked_at'] < 86400:
                return cached
            stale_cached = cached
    except (OSError, ValueError, KeyError, TypeError):
        pass
    try:
        with urllib.request.urlopen(SOURCE, timeout=4) as response:
            content = response.read(8 * 1024 * 1024 + 1)
            if len(content) > 8 * 1024 * 1024:
                raise ValueError('Google 官方列表超过大小上限')
    except Exception:
        if stale_cached is not None:
            return stale_cached
        raise
    result = {'identity': identity, 'listed': listed(content.decode('utf-8'), identity), 'checked_at': now, 'source': SOURCE}
    save_json('certification-catalog.json', result)
    return result


def inspect():
    guard()
    identity = {k: shell('getprop', 'ro.product.' + k) for k in ('brand', 'manufacturer', 'model')}
    locked = shell('getprop', 'ro.boot.flash.locked')
    avb = shell('getprop', 'ro.boot.verifiedbootstate')
    activities = shell('dumpsys', 'activity', 'activities')
    blocked = any('topResumedActivity=' in line and 'com.android.vending/com.google.android.gms.gmscompliance.ui.UncertifiedDeviceActivity' in line for line in activities.splitlines())
    result = {'identity': identity, 'bootloader_locked': locked, 'verified_boot': avb, 'certification_block_visible': blocked, 'certification': 'rejected_on_screen' if blocked else 'not_verified'}
    try:
        result['catalog'] = catalog(identity)
        result['catalog_note'] = '官方机型列表已收录；当前设备认证仍需在 Play 中确认。' if result['catalog']['listed'] else '官方机型列表未收录此品牌和型号；安装 APK 或授予本机权限不等于获得 Google 认证。'
    except Exception:
        result['catalog_note'] = '暂时无法读取 Google 官方机型列表，请联网后重试；没有据此判断认证通过。'
    disabled = set(shell('pm', 'list', 'packages', '-d', '--user', '0').splitlines())
    result['disabled_components'] = [p for p in ('com.google.android.gsf', 'com.google.android.gms', 'com.android.vending') if 'package:' + p in disabled]
    return result


if __name__ == '__main__':
    print(json.dumps(inspect(), ensure_ascii=False))
