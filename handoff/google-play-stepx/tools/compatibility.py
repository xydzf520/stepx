#!/usr/bin/env python3
"""Portable entry point for the fixed-device, reversible compatibility repair."""

import os
from pathlib import Path
import runpy


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    os.environ["STEPX_GOOGLE_HOME"] = str(root)
    runpy.run_path(str(root.parents[1] / "scripts/google-play-compat.py"), run_name="__main__")
