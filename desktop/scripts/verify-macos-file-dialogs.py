#!/usr/bin/env python3
"""Interactive real-Mac acceptance. Run python3 desktop/scripts/verify-macos-file-dialogs.py
Optional --client-pid PID checks a running StepX client remains alive.
This tests the shipped JXA script; it does NOT substitute for the final client UI steps.
No phone state is changed and no files are sent to a phone.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client-pid', type=int)
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('此脚本必须在真实 macOS 上运行；Linux 的 mock 通过不能当作 Mac 验收')
    source = Path(__file__).resolve().parents[1] / 'src/file_dialog_macos.js'
    script = source.read_text()
    report = {'platform': platform.platform(), 'script': str(source), 'cases': [], 'client_ui': 'not_verified'}
    def alive():
        if args.client_pid:
            os.kill(args.client_pid, 0)
    def choose(kind, title, location, filename='', cancel=False):
        alive()
        payload = {'kind': kind, 'title': title, 'filename': filename, 'location': str(location), 'extensions': []}
        started = time.monotonic()
        # Same command construction as Rust. No shell and no interpolated source.
        output = subprocess.run(['/usr/bin/osascript', '-l', 'JavaScript', '-e', script, '--', json.dumps(payload, ensure_ascii=False)], capture_output=True, text=True, timeout=300)
        assert output.returncode == 0, f'osascript exit={output.returncode}: {output.stderr}'
        data = json.loads(output.stdout)
        assert data['status'] == ('cancelled' if cancel else 'selected'), data
        if not cancel:
            assert data['paths'] and all(Path(p).is_absolute() for p in data['paths']), data
            if kind != 'open_files':
                assert len(data['paths']) == 1, data
        alive()
        report['cases'].append({'kind': kind, 'cancel': cancel, 'elapsed_s': round(time.monotonic()-started, 2), 'passed': True})
        return data.get('paths', [])
    with tempfile.TemporaryDirectory(prefix='StepX 文件选择验收 ') as fixture_dir:
        folder = Path(fixture_dir)
        names = ['中文 空格.txt', '换行\n双引号".txt']
        for name in names:
            (folder / name).write_text('StepX local file chooser verification\n')
        (folder / '选择此文件夹').mkdir()
        print('先验收独立文件选择器。请保留 StepX 运行；所有文件均为临时本地样例。')
        paths = choose('open_file', '验收 1/5：选择“中文 空格.txt”', folder)
        assert paths == [str(folder / names[0])], repr(paths)
        paths = choose('open_files', '验收 2/5：同时选择两个 txt 文件（Command 多选）', folder)
        assert set(paths) == {str(folder / n) for n in names}, repr(paths)
        paths = choose('open_folder', '验收 3/5：选择“选择此文件夹”', folder)
        assert paths == [str(folder / '选择此文件夹')], repr(paths)
        name = '保存 中文 "引号".txt'
        paths = choose('save_file', '验收 4/5：保留名称和目录，点击保存（不实际写入文件）', folder, name)
        assert paths == [str(folder / name)], repr(paths)
        choose('open_file', '验收 5/5：点击取消或按 Escape', folder, cancel=True)
    print('\n独立脚本通过。接下来必须在实际 StepX 客户端完成：')
    print('1. 发送文件/图片：打开选择器期间切换页面、打字，UI 应流畅；取消后草稿不变。')
    print('2. 下载到电脑、导出 APK、安装 APK：只打开窗口并取消，不进行安装或传输。')
    print('3. 设置的下载目录和 Work 目录、背屏选图、作品导入/导出二维码：逐项打开并取消。')
    print('4. 多次快速点击仅有一个选择器；完成或取消后应能再次打开。')
    print('5. 选择器打开时切换手机/会话再选文件：提示目标变化，不把附件交给新目标。')
    print('6. 等待选择时手机消息仍能同步；系统诊断报告无新增 StepX SIGABRT。')
    input('完成后按回车记录脚本结果（这不会自动宣称客户端步骤通过）...')
    alive()
    target = Path(tempfile.gettempdir()) / 'stepx-macos-file-dialog-verification.json'
    target.write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(f'脚本测试通过，记录：{target}。客户端 UI 与手机联调结果请另行记录。')

if __name__ == '__main__':
    main()
