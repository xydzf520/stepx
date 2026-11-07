#!/usr/bin/env python3
"""Compile the shipped Metal fade mapping without unrelated UI test assets.

This checks the production Rust helper and current core GlyphFade variants, not
a copied implementation. Native macOS UI/build steps still compile AppKit,
Metal shaders, bindgen layouts and the complete renderer separately.
"""

from pathlib import Path
import json
import os
import re
import subprocess
import tempfile
import tomllib


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    manifest = root / "desktop" / "Cargo.toml"
    cargo = tomllib.loads(manifest.read_text(encoding="utf-8"))
    ui = (manifest.parent / cargo["dependencies"]["facloud_ui"]["path"]).resolve()
    scene = ui.parent / "facloud_ui_core" / "src" / "scene.rs"
    helper = ui / "src/platform/mac/rendering/metal/glyph_fade.rs"
    renderer = helper.with_name("renderer.rs").read_text(encoding="utf-8")
    if 'mod glyph_fade;' not in renderer or 'glyph_fade_uniforms(glyph.fade, scale_factor)' not in renderer:
        raise RuntimeError("The Metal renderer must use the helper being tested")
    enum_match = re.search(
        r"(#\[derive\([^\n]+\)\]\s*pub enum GlyphFade\s*\{.*?^\})",
        scene.read_text(encoding="utf-8"),
        re.MULTILINE | re.DOTALL,
    )
    if not enum_match:
        raise RuntimeError("Cannot locate the current core GlyphFade definition")
    # Keep the production import intact, resolving it to the exact core enum.
    harness = (
        "extern crate self as facloud_ui_core;\n"
        "pub mod scene {\n" + enum_match.group(1) + "\n}\n"
        "#[path = " + json.dumps(str(helper), ensure_ascii=False) + "]\n"
        "mod glyph_fade;\n"
    )
    with tempfile.TemporaryDirectory(prefix="stepx-metal-fade-") as tmp:
        source = Path(tmp) / "metal_fade_tests.rs"
        binary = Path(tmp) / ("metal_fade_tests.exe" if os.name == "nt" else "metal_fade_tests")
        source.write_text(harness, encoding="utf-8")
        subprocess.run(
            ["rustc", "--edition=2021", "--test", str(source), "-o", str(binary)],
            check=True,
        )
        subprocess.run([str(binary)], check=True)


if __name__ == "__main__":
    main()
