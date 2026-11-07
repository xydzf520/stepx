#!/usr/bin/env python3
"""Check the current version's complete, bounded, offline release story."""
import datetime
import json
from pathlib import Path
import re
import struct
import sys
import tomllib


def check(repo):
    version = tomllib.loads((repo / "desktop/Cargo.toml").read_text())["package"]["version"]
    root = repo / "desktop/assets/release-notes"
    notes = json.loads((root / f"{version}.json").read_text())
    media = root / version
    sources = json.loads((media / "sources.json").read_text())
    assert notes["version"] == version
    datetime.date.fromisoformat(notes["date"])
    assert notes["captured_from"] == sources["capture_version"]
    assert datetime.date.fromisoformat(sources["captured_on"]) <= datetime.date.fromisoformat(notes["date"])
    assert notes["baseline_url"] == f'https://github.com/xydzf520/stepx/releases/tag/v{notes["baseline"]}'
    assert 1 <= len(notes["highlights"]) <= 8
    ids = set()
    destinations = {"home", "search", "messages", "files", "agent", "phone", "about", "history", "amoo", "studio", "calendar", "alarms", "gallery", "back_screen", "tools", "connect", "account", "settings", "notifications", "storage", "app_store"}
    total = 0
    for feature in notes["highlights"]:
        assert re.fullmatch(r"[a-z][a-z0-9-]*", feature["id"])
        assert feature["id"] not in ids
        ids.add(feature["id"])
        for key in ("title", "summary", "image_alt", "caption", "action_label"):
            assert feature[key].strip(), f'{feature["id"]}: missing {key}'
        assert feature["image"].startswith("release-notes/")
        name = feature["image"].removeprefix("release-notes/")
        assert Path(name).name == name and name.endswith(".png")
        assert name in sources["images"]
        path = media / name
        assert not path.is_symlink()
        data = path.read_bytes()
        assert data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR"
        width, height = struct.unpack(">II", data[16:24])
        assert 600 <= width <= 1600 and 240 <= height <= 1000, (name, width, height)
        assert len(data) <= 512 * 1024, (name, len(data))
        total += len(data)
    assert [c["id"] for c in notes["categories"][:4]] == ["alerts", "capsule", "workbench", "files"]
    assert len(notes["categories"]) == len({c["id"] for c in notes["categories"]})
    assert any(c["id"] == "performance" for c in notes["categories"])
    history = (repo / "CHANGELOG.md").read_text()
    features = [f for c in notes["categories"] for f in c["features"]]
    for feature in features:
        assert re.fullmatch(r"[a-z][a-z0-9-]*", feature["id"])
        assert feature["id"] not in ids
        ids.add(feature["id"])
        assert feature["title"].strip() and feature["benefit"].strip()
        assert feature["destination"] in destinations
        assert feature["kind"] in ("新增", "改进")
        assert 1 <= len(feature["details"]) <= 3 and all(feature["details"])
        assert feature["versions"]
        assert all(re.search(rf"^## {re.escape(v)}(?:\s|$)", history, re.M) for v in feature["versions"])
    # Check all category media, including focused captures outside the hero cards.
    for name in {c["image"].removeprefix("release-notes/") for c in notes["categories"]}:
        assert Path(name).name == name and name in sources["images"]
        data = (media / name).read_bytes()
        assert data[:8] == b"\x89PNG\r\n\x1a\n" and len(data) <= 512 * 1024
        w, h = struct.unpack(">II", data[16:24])
        assert 600 <= w <= 1600 and 240 <= h <= 1000
    total = sum((media / name).stat().st_size for name in sources["images"])
    assert total <= 3 * 1024 * 1024
    if notes.get("video"):
        video = notes["video"]
        assert video["file"] == "release-notes/tour.mp4"
        assert 5 <= video["duration_seconds"] <= 90
        assert video["captured_from"] == sources["video"]["captured_from"]
        assert video["poster"] in {item["image"] for item in notes["highlights"]}
        assert video["title"].strip() and video["caption"].strip()
        clip = media / "tour.mp4"
        assert not clip.is_symlink()
        data = clip.read_bytes()
        assert data[4:8] == b"ftyp" and len(data) <= 16 * 1024 * 1024
        assert b"moov" in data and b"mdat" in data
        assert video["native_file"] == "release-notes/tour.h264" and video["fps"] == 12
        stream = (media / "tour.h264").read_bytes()
        assert stream[:4] == b"\x00\x00\x00\x01" and len(stream) < 2 * 1024 * 1024
        assert 5 * video["fps"] <= video["frame_count"] <= 90 * video["fps"]
    guide = repo / "desktop/assets/connection-guide"
    attribution = json.loads((guide / "sources.json").read_text())
    assert attribution["license"] == "CC BY 4.0"
    for name, item in attribution["images"].items():
        assert Path(name).name == name and item["url"].startswith("https://developer.android.com/")
        path = guide / name
        assert not path.is_symlink()
        data = path.read_bytes()
        assert data[:8] == b"\x89PNG\r\n\x1a\n" and len(data) < 512 * 1024
    assert notes["boundaries"]
    assert re.search(rf"^## {re.escape(version)}(?:\s|$)", (repo / "CHANGELOG.md").read_text(), re.M)
    serialized = json.dumps(notes, ensure_ascii=False)
    assert not any(private in serialized for private in ("/home/", "/tmp/", "Bearer ", "api_key"))
    return version, len(features), total


if __name__ == "__main__":
    try:
        version, count, size = check(Path(__file__).resolve().parents[1])
        print(f"Release {version}: {count} evidenced user scenarios, {size} bytes of bundled media")
    except (AssertionError, OSError, ValueError, KeyError) as error:
        print(f"Release notes validation failed: {error}", file=sys.stderr)
        sys.exit(1)
