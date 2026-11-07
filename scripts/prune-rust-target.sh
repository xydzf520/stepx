#!/usr/bin/env bash
# Safely prune Cargo build artifacts in desktop/target to prevent unbounded disk bloat.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="${STEPX_CARGO_TARGET_DIR:-$ROOT/desktop/target}"
DEBUG_DIR="$TARGET_DIR/debug"
MAX_DEBUG_GB="${STEPX_MAX_DEBUG_TARGET_GB:-15}"

if [ ! -d "$DEBUG_DIR" ]; then
    exit 0
fi

# 1. Always remove incremental compilation cache (incremental = false in Cargo.toml,
#    and old incremental caches can easily grow past 100+ GB).
if [ -d "$DEBUG_DIR/incremental" ]; then
    rm -rf "$DEBUG_DIR/incremental"
fi

# 2. Prune compiled example binaries/artifacts older than 1 day.
if [ -d "$DEBUG_DIR/examples" ]; then
    find "$DEBUG_DIR/examples" -mindepth 1 -mtime +1 -delete 2>/dev/null || true
fi

# 3. Remove unpacked split-debuginfo (.dwo / .dSYM) or stale artifacts older than 7 days.
if [ -d "$DEBUG_DIR/deps" ]; then
    find "$DEBUG_DIR/deps" -mindepth 1 \( -name "*.dwo" -o -name "*.dwp" \) -mtime +3 -delete 2>/dev/null || true
fi

# 4. If desktop/target/debug still exceeds MAX_DEBUG_GB (default 15 GB),
#    prune deps and examples older than 1 day (or all deps/examples if still over limit).
debug_kb="$(du -sk "$DEBUG_DIR" 2>/dev/null | awk '{print $1}' || echo 0)"
max_kb=$((MAX_DEBUG_GB * 1024 * 1024))

if [ "${debug_kb:-0}" -gt "$max_kb" ]; then
    echo "[prune-rust-target] $DEBUG_DIR is $((debug_kb / 1024 / 1024)) GB (> ${MAX_DEBUG_GB} GB limit); pruning stale debug deps & examples..."
    if [ -d "$DEBUG_DIR/examples" ]; then
        rm -rf "$DEBUG_DIR/examples"
    fi
    if [ -d "$DEBUG_DIR/deps" ]; then
        find "$DEBUG_DIR/deps" -mindepth 1 -mtime +1 -delete 2>/dev/null || true
    fi
    debug_kb="$(du -sk "$DEBUG_DIR" 2>/dev/null | awk '{print $1}' || echo 0)"
    if [ "${debug_kb:-0}" -gt "$max_kb" ] && [ -d "$DEBUG_DIR/deps" ]; then
        rm -rf "$DEBUG_DIR/deps"
    fi
fi
