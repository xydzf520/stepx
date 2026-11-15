#!/usr/bin/env python3
"""Rebuild the Java payload locally; --write explicitly updates the bundled injector."""
import argparse
import ast
import base64
import hashlib
import os
from pathlib import Path
import re
import shutil
from common import ROOT, run, sdk_tool


def compile_java(source, output):
    sdk = os.environ.get('ANDROID_SDK_ROOT') or os.environ.get('ANDROID_HOME')
    if not sdk:
        raise RuntimeError('Set ANDROID_SDK_ROOT to the installed Android SDK')
    jar = Path(sdk) / 'platforms/android-36/android.jar'
    assert jar.is_file(), 'Install Android platform 36'
    javac = shutil.which('javac')
    assert javac, 'Put JDK 17 javac/java on PATH'
    classes, dex = output / 'classes', output / 'dex'
    classes.mkdir(parents=True, exist_ok=True)
    dex.mkdir(parents=True, exist_ok=True)
    run([javac, '-source', '8', '-target', '8', '-cp', str(jar), '-d', str(classes), str(source)])
    run([sdk_tool('d8'), '--min-api', '36', '--output', str(dex), str(classes / 'stepx/GmsProbe.class')])
    return (dex / 'classes.dex').read_bytes()


def replace_dex(source, dex):
    updated, count = re.subn(r'dex=base64\.b64decode\([^\n]+?\);dexp=',
                            'dex=base64.b64decode(' + repr(base64.b64encode(dex).decode()) + ');dexp=', source)
    assert count == 1, 'Expected exactly one embedded DEX'
    compile(updated, 'inject.py', 'exec')
    return updated


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    dex = compile_java(ROOT / 'java/GmsProbe.java', ROOT / 'build/grant')
    injector = ROOT / 'module/inject.py'
    source = injector.read_text(encoding='utf-8')
    old = base64.b64decode(ast.literal_eval(re.search(r'dex=base64\.b64decode\(([^\n]+?)\);dexp=', source).group(1)))
    print('Compiled DEX SHA-256:', hashlib.sha256(dex).hexdigest())
    print('Embedded DEX matches compilation:', dex == old)
    if args.write:
        injector.write_text(replace_dex(source, dex), encoding='utf-8')
        print('Bundle injector updated. Installed device module was not changed.')


if __name__ == '__main__':
    main()
