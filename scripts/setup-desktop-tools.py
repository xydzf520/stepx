#!/usr/bin/env python3
"""Resolve missing Ubuntu/Debian dependencies and extract into .tools, without sudo."""
import pathlib
import re
import shlex
import subprocess
import hashlib

root = pathlib.Path(__file__).resolve().parent.parent / ".tools"
packages = root / "packages"
packages.mkdir(parents=True, exist_ok=True)
plan = subprocess.check_output([
    "apt-get", "-o", "Debug::NoLocking=1", "--print-uris", "--download-only", "--yes", "install", "adb", "scrcpy"
], text=True)
names = {"adb", "scrcpy"}
for line in plan.splitlines():
    if line.startswith("'"):
        filename = shlex.split(line)[1]
        name = filename.split("_", 1)[0]
        # udev rules need a system installation; private extraction cannot activate them.
        if name not in {"android-udev-rules", "android-sdk-platform-tools-common"}:
            names.add(name)
# apt verifies downloads against its signed repository metadata.
subprocess.run(["apt", "download", *sorted(names)], cwd=packages, check=True)
for package in packages.glob("*.deb"):
    subprocess.run(["dpkg-deb", "-x", str(package), str(root / "root")], check=True)
server = root / "root/usr/share/scrcpy/scrcpy-server"
updater = root / "root/usr/libexec/scrcpy-update-server"
if updater.exists():
    text = updater.read_text()
    version = re.search(r"^VERSION=([\d.]+)$", text, re.M).group(1)
    checksum = re.search(r"^SHA256=([a-f0-9]{64})$", text, re.M).group(1)
    if not server.exists() or hashlib.sha256(server.read_bytes()).hexdigest() != checksum:
        server.parent.mkdir(parents=True, exist_ok=True)
        pending = server.with_suffix(".download")
        upstream_url = f"https://github.com/Genymobile/scrcpy/releases/download/v{version}/scrcpy-server-v{version}"
        candidates = [
            upstream_url,
            f"https://ghfast.top/{upstream_url}",
            f"https://gh-proxy.com/{upstream_url}",
        ]
        verified = False
        for candidate in candidates:
            pending.unlink(missing_ok=True)
            res = subprocess.run(
                ["curl", "-fL", "--connect-timeout", "10", "--max-time", "120", candidate, "-o", str(pending)],
                check=False,
            )
            if res.returncode == 0 and pending.is_file() and hashlib.sha256(pending.read_bytes()).hexdigest() == checksum:
                pending.replace(server)
                verified = True
                break
        if not verified:
            pending.unlink(missing_ok=True)
            raise SystemExit("scrcpy server download or checksum verification failed")
if not server.is_file():
    raise SystemExit("The distribution did not provide a scrcpy server. Install the matching official server before mirroring.")
