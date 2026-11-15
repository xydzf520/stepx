#!/usr/bin/env python3
"""Portable entry point for the fixed-device, reversible compatibility repair."""

import os
from pathlib import Path
import runpy
import sys


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    os.environ["STEPX_GOOGLE_HOME"] = str(root)
    for extra in (root.parents[1] / "scripts", root / "compat/scripts", root / "tools"):
        if extra.is_dir() and str(extra) not in sys.path:
            sys.path.insert(0, str(extra))
    target = root.parents[1] / "scripts/google-play-compat.py"
    if not target.is_file():
        target = root / "compat/scripts/google-play-compat.py"
    runpy.run_path(str(target), run_name="__main__")
