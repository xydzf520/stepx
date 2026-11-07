#!/usr/bin/env python3
"""Export focused, aspect-preserving UI captures for the bundled release story."""
import argparse
import json
import tomllib
from pathlib import Path


def main():
    from PIL import Image
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture_dir", type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    version = tomllib.loads((repo / "desktop/Cargo.toml").read_text())["package"]["version"]
    root = repo / "desktop/assets/release-notes" / version
    sources = json.loads((root / "sources.json").read_text())
    for name, source in sources["images"].items():
        with Image.open(args.capture_dir / source["capture"]) as im:
            x1, y1, x2, y2 = source["crop"]
            im = im.crop((round(x1 * im.width), round(y1 * im.height), round(x2 * im.width), round(y2 * im.height)))
            im.thumbnail((1400, 1000), Image.Resampling.LANCZOS)
            im.save(root / name, optimize=True)
            print(f"{name}: {im.width}×{im.height}, {(root / name).stat().st_size} bytes")


if __name__ == "__main__":
    main()
