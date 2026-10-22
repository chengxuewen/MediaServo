#!/usr/bin/env bash
# package e2e — dist/ dual-package release regression (D-H13): host package versioned top-level dir asserts -> unpack
# doctor/start/status/stop smoke -> SDK package versioned top-level dir key-file asserts -> version contract files -> dist/ gitignore. C22: runs natively on the host.
# Prereq: the 8 host binaries in target/debug + 3 SDK .so + node binding (built automatically if missing).
set -euo pipefail
cd "$(dirname "$0")/.."

# SR4/P-0 (full-review B-3): the host section runs doctor/start/status/stop smoke AND
# its EXIT trap wipes /tmp/iceoryx2 = the live-cluster mutual-exclusion face (PIT-192/
# PIT-235 family). SDK-layout leg judges may NOT run it whole. `--sdk-only` skips the
# host packaging/lifecycle entirely and asserts NO managed process was touched.
SDK_ONLY=0
[ "${1:-}" = "--sdk-only" ] && SDK_ONLY=1

export PATH="$HOME/.pixi/bin:$PATH"

FAIL=0
note() { echo "== $1"; }

# Workspace version (same source as mediaservo_cli.py _workspace_version: Cargo.toml [workspace.package])
# After V1a the four sources are independent: reading workspace is still correct while all four values match; when V1b
# splits the four package names this must switch to per-target domain reads (same source as the cli _TARGET_CRATE table) — host package checks the host crate, sdk package checks the field crate.
VER=$(sed -n '/\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -1)
[ -n "$VER" ] || { echo "FAIL: cannot resolve workspace version"; exit 1; }
MAJOR=${VER%%.*}

# ── 0. Prereq build (build whatever is missing, same pattern as e2e-deploy-host.sh) ──
if [ ! -x target/debug/mediaservo-host ]; then
    note "building mediaservo-host (debug)"
    pixi run cargo build -p mediaservo-host
fi
if [ ! -f target/debug/libmediaservo_field.so ] || [ ! -f target/debug/libmediaservo_link.so ] \
   || [ ! -f target/debug/libmediaservo_deck.so ] || [ ! -f bindings/node/mediaservo.node ]; then
    note "building SDK cdylibs + node binding (debug)"
    pixi run python3 scripts/mediaservo_cli.py build bindings
fi

TMP=$(mktemp -d)
if [ "$SDK_ONLY" = 1 ]; then
    trap 'rm -rf "$TMP"' EXIT   # no SHM wipes: sdk legs must not touch shared runtime (PIT-192)
else
    trap 'rm -rf "$TMP"; rm -rf /tmp/iceoryx2 /dev/shm/iox2_*' EXIT
fi
PROC_BASE=$(ps -eo comm 2>/dev/null | grep -cE '^(mediaservo-|msrtc-|oxmgr)' || true)

# Platform/arch suffix in package names (user ruling 2026-10-19, scheme A): the same
# package name ships different binaries per machine; use a glob so any arch matches.
ARCH=$(python3 -c 'import platform; print({"x86_64":"amd64","amd64":"amd64","aarch64":"arm64","arm64":"arm64"}.get(platform.machine().lower(), platform.machine().lower()))')
HOST_TGZ="dist/mediaservo-host-$VER-linux-$ARCH.tar.gz"
HOST_ROOT="mediaservo-host-$VER-linux-$ARCH"
SDK_TGZ="dist/mediaservo-sdk-$VER-linux-$ARCH.tar.gz"
SDK_ROOT="mediaservo-sdk-$VER-linux-$ARCH"

if [ "$SDK_ONLY" = 0 ]; then
# ── 1. package host ─────────────────────────────────────
note "package host (ver=$VER)"
env MEDIASERVO_BRAND=mediaservo pixi run python3 scripts/mediaservo_cli.py package host
[ -f "$HOST_TGZ" ] || { echo "FAIL: missing $HOST_TGZ"; FAIL=1; }

# ── 2. host package content asserts (versioned top dir + D-H13 layout + host-version.txt) ────
note "host tar contents"
LIST=$(tar tzf "$HOST_TGZ" || true)
[ -n "$LIST" ] || { echo "FAIL: cannot read $HOST_TGZ"; FAIL=1; }
echo "$LIST" | grep -q "^$HOST_ROOT/$" || { echo "FAIL: package lacks versioned top dir $HOST_ROOT/"; FAIL=1; }
for b in mediaservo-host host-agent host-capturer host-streamer host-recorder \
         host-controller host-emergency host-audio; do
    echo "$LIST" | grep -q "^$HOST_ROOT/bin/$b$" || { echo "FAIL: package lacks $HOST_ROOT/bin/$b"; FAIL=1; }
done
# Root shortcuts (host + mediaservo-host) — bin/ holds mediaservo-host; shortcuts hang at the versioned top-dir root
echo "$LIST" | grep -q "^$HOST_ROOT/host$" || { echo "FAIL: package lacks $HOST_ROOT/host"; FAIL=1; }
# Conditional oxmgr assert (fixed in I2 review): oxmgr on PATH at packaging time → package must contain $HOST_ROOT/bin/oxmgr;
# environment without oxmgr → not packaged (cli errors explicitly), skip and explain
if command -v oxmgr >/dev/null 2>&1; then
    echo "$LIST" | grep -q "^$HOST_ROOT/bin/oxmgr$" || { echo "FAIL: oxmgr on PATH but package lacks $HOST_ROOT/bin/oxmgr"; FAIL=1; }
else
    echo "NOTE: no oxmgr on PATH — skipping the bin/oxmgr assert (deploy host did not package it; doctor failure expected)"
fi
for f in etc/host.yaml etc/link/signing.pem etc/link/cam0.token etc/link/cam0-stream-stream.token \
         etc/link/recorder.token etc/link/agent.token etc/link/ros_bridge.yaml \
         etc/link/issuance.jsonl identity.json host-version.txt CHANGES.md; do
    echo "$LIST" | grep -q "^$HOST_ROOT/$f$" || { echo "FAIL: package lacks $HOST_ROOT/$f"; FAIL=1; }
done
echo "$LIST" | grep -q "^$HOST_ROOT/run/logs/$" || { echo "FAIL: package lacks $HOST_ROOT/run/logs/"; FAIL=1; }
echo "$LIST" | grep -q "^$HOST_ROOT/recordings/$" || { echo "FAIL: package lacks $HOST_ROOT/recordings/"; FAIL=1; }
echo "OK: host package layout complete (versioned top dir $HOST_ROOT/ + bin 8 + oxmgr + etc/link credentials + run/logs + recordings + identity.json + host-version.txt)"

# ── 3. extract to a clean dir → doctor + start/status/stop smoke ──────
note "extract + smoke"
mkdir -p "$TMP/extract"
tar xzf "$HOST_TGZ" -C "$TMP/extract"
PKG_DIR="$TMP/extract/$HOST_ROOT"
BIN="$PKG_DIR/bin"
if [ ! -d "$PKG_DIR" ]; then
    echo "FAIL: versioned top dir missing after extraction: $PKG_DIR"; FAIL=1
elif [ ! -x "$BIN/mediaservo-host" ]; then
    echo "FAIL: executable missing after extraction: $BIN/mediaservo-host"; FAIL=1
else
    echo "OK: host tar extracts to a versioned top dir $HOST_ROOT/"
fi
if command -v oxmgr >/dev/null 2>&1 && [ -x "$BIN/mediaservo-host" ]; then
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" doctor "$PKG_DIR" 2>&1 || true)
    echo "$out" | grep -q "all checks passed" || { echo "FAIL: doctor not all-passed after extraction"; echo "$out" | tail -8; FAIL=1; }
    rm -rf /tmp/iceoryx2 /dev/shm/iox2_*   # C25: clear iceoryx2 residue
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" start "$PKG_DIR" 2>&1 || true)
    if ! echo "$out" | grep -q "start"; then
        echo "FAIL: host start after extraction"; echo "$out" | tail -8; FAIL=1
    fi
    sleep 4
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" status "$PKG_DIR" 2>&1 || true)
    echo "$out" | grep -q "host-agent" || { echo "FAIL: no host-agent in status"; echo "$out" | tail -8; FAIL=1; }
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" stop "$PKG_DIR" 2>&1 || true)
    if ! echo "$out" | grep -q "stop"; then
        echo "FAIL: host stop"; echo "$out" | tail -8; FAIL=1
    fi
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" status "$PKG_DIR" 2>&1 || true)
    echo "$out" | grep -q "no managed processes" || { echo "FAIL: processes still remain after stop"; echo "$out" | tail -8; FAIL=1; }
    echo "OK: post-extraction doctor → start(host-agent) → stop smoke passed"
else
    echo "NOTE: no oxmgr on PATH — skipping the host doctor/start/status/stop runtime smoke"
fi

fi   # SDK_ONLY==0 host sections 1-3 end

# ── 4. package bindings ──────────────────────────────
note "package bindings (ver=$VER)"
env MEDIASERVO_BRAND=mediaservo pixi run python3 scripts/mediaservo_cli.py package bindings
[ -f "$SDK_TGZ" ] || { echo "FAIL: missing $SDK_TGZ"; FAIL=1; }

# ── 5. SDK package key-file asserts ────────────────────────
note "sdk tar contents"
LIST=$(tar tzf "$SDK_TGZ" || true)
[ -n "$LIST" ] || { echo "FAIL: cannot read $SDK_TGZ"; FAIL=1; }
MINOR=$(echo "$VER" | cut -d. -f2); PATCH=$(echo "$VER" | cut -d. -f3)
for f in \
    lib/libmediaservo_field.so.$MAJOR.$MINOR.$PATCH \
    lib/libmediaservo_link.so.$MAJOR.$MINOR.$PATCH \
    lib/libmediaservo_deck.so.$MAJOR.$MINOR.$PATCH \
    lib/libmediaservo_field.so lib/libmediaservo_field.so.$MAJOR \
    lib/libmediaservo_link.so lib/libmediaservo_link.so.$MAJOR \
    lib/libmediaservo_deck.so lib/libmediaservo_deck.so.$MAJOR \
    include/mediaservo/common.h include/mediaservo/field.h \
    include/mediaservo/link.h include/mediaservo/deck.h \
    lib/pkgconfig/mediaservo-field.pc lib/pkgconfig/mediaservo-link.pc lib/pkgconfig/mediaservo-deck.pc \
    lib/cmake/mediaservo/mediaservoConfig.cmake lib/cmake/mediaservo/mediaservoConfigVersion.cmake \
    node/mediaservo/package.json node/mediaservo/mediaservo.node node/mediaservo/lib/index.mjs \
    docs/sdk-cxx/field.md docs/sdk-cxx/link.md docs/sdk-cxx/deck.md \
    sdk-version.txt manifest.json CHANGES.md; do
    echo "$LIST" | grep -q "^$SDK_ROOT/$f$" || { echo "FAIL: SDK package lacks $f"; FAIL=1; }
done
echo "$LIST" | grep -qE "^$SDK_ROOT/lib/python3\.[0-9]+/site-packages/mediaservo/" || { echo "FAIL: SDK package lacks the python package"; FAIL=1; }
echo "$LIST" | grep -qE "^$SDK_ROOT/wheel/mediaservo-.*\.whl$" || { echo "FAIL: SDK package lacks the wheel"; FAIL=1; }
echo "$LIST" | grep -q "^$SDK_ROOT/docs/sdk-cxx/client.md$" && { echo "FAIL: sdk-field package polluted by cockpit-side docs"; FAIL=1; }
echo "OK: SDK package key files complete (versioned top dir $SDK_ROOT/ + lib triplet real files+symlinks + include + python + wheel + node + .pc + cmake + sdk-version.txt)"

# ── V1b cockpit-half package (sdk-client standalone form) ──
# pre-existing bug caught at first-ever execution of this section (10-22): MSRTC_CLI was
# referenced but never defined -- full runs died earlier (host smoke), nothing reached :156.
if [ -z "${MSRTC_CLI:-}" ]; then
    _cli=(pixi run python3 scripts/mediaservo_cli.py)
else
    read -r -a _cli <<< "$MSRTC_CLI"
fi
"${_cli[@]}" package sdk-client --dist "$TMP/dist" >/dev/null 2>&1 || { echo "FAIL: package sdk-client failed"; exit 1; }
CLI_TGZ=$(ls "$TMP/dist"/mediaservo-sdk-client-*.tar.gz | head -1)
[ -n "$CLI_TGZ" ] || { echo "FAIL: sdk-client tar not generated"; exit 1; }
# list to a file FIRST (two latent bugs retired at first real execution 10-22:
# `tar | head` = SIGPIPE under set -o pipefail; MSRTC_CLI was unbound)
tar tzf "$CLI_TGZ" > "$TMP/cli-list.txt"
CLI_ROOT=$(head -1 "$TMP/cli-list.txt" | cut -d/ -f1)
for f in lib/libmediaservo_client.so "lib/libmediaservo_client.so.$MAJOR" \
         include/mediaservo/client.h include/mediaservo/client.hpp \
         lib/pkgconfig/mediaservo-client.pc lib/cmake/mediaservo/mediaservoConfig.cmake \
         docs/sdk-cxx/client.md \
         sdk-client-version.txt CHANGES.md; do
    grep -q "^$CLI_ROOT/$f$" "$TMP/cli-list.txt" || { echo "FAIL: sdk-client package lacks $f"; FAIL=1; }
done
grep -qE "libmediaservo_(field|link|deck)" "$TMP/cli-list.txt" && { echo "FAIL: sdk-client package polluted by device-side libs"; FAIL=1; }
grep -q "site-packages" "$TMP/cli-list.txt" && { echo "FAIL: sdk-client package polluted by the python device side"; FAIL=1; }
grep -qE "^$CLI_ROOT/docs/sdk-cxx/(field|link|deck).md$" "$TMP/cli-list.txt" && { echo "FAIL: sdk-client package polluted by device-side docs"; FAIL=1; }
echo "OK: sdk-client half-package shaped (client-only + zero device-side contamination)"
# N2 manifest content spot-check: the client package lists exactly one cmake component and ffmpeg=false (readelf-backed)
mkdir -p "$TMP/cli"
tar xzf "$CLI_TGZ" -C "$TMP/cli" "$CLI_ROOT/manifest.json" 2>/dev/null
SPV=$(grep -oE "pub const SIGNALING_PROTOCOL_VERSION: u32 = [0-9]+" crates/mediaservo-common/src/protocol.rs | grep -oE "[0-9]+$")
SMIN=$(grep -oE "pub const SIGNALING_PROTOCOL_MIN_SUPPORTED: u32 = [0-9]+" crates/mediaservo-common/src/protocol.rs | grep -oE "[0-9]+$")
export SPV SMIN
python3 - "$TMP/cli/$CLI_ROOT/manifest.json" <<'PYV'
import json, os, sys
d = json.load(open(sys.argv[1]))
assert d["package"] == "sdk-client" and d["schema"] == 1
c = [x for x in d["components"] if "cmake_component" in x]
assert [x["cmake_component"] for x in c] == ["client"], c
assert c[0]["requires_ffmpeg"] in (False, None), c
assert d["protocol"]["frame_meta_wire_version"] == 0, "wire version must equal the generated source value"
assert d["protocol"]["signaling_protocol_version"] == int(os.environ["SPV"]), "manifest signaling_protocol_version != protocol.rs const"
assert d["protocol"]["signaling_min_supported"] == int(os.environ["SMIN"]), "manifest signaling_min_supported != protocol.rs const"
assert set(d["protocol"]) == {"frame_meta_wire_version", "token_schema_version",
                              "signaling_protocol_version", "signaling_min_supported"}, d["protocol"]
print("OK: manifest.json content reconciled (client-only / ffmpeg evidence / four-field protocol tuple)")
PYV

# ── 6. Version contract file content (D-H14 minimal: workspace + FrameMeta wire version) ──
note "version contract file"
mkdir -p "$TMP/sdk"
tar xzf "$SDK_TGZ" -C "$TMP/sdk" "$SDK_ROOT/sdk-version.txt"
grep -q "workspace_version: $VER" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: sdk-version.txt lacks workspace_version=$VER"; FAIL=1; }
FW=$(grep -oE "pub const WIRE_VERSION: u8 = [0-9]+" crates/mediaservo-link/src/frame.rs | grep -oE "[0-9]+$")
grep -q "^frame_meta_version: $FW$" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: sdk-version.txt frame_meta_version != source value $FW (floating-number pin)"; FAIL=1; }
TV=$(grep -oE "pub const VERSION: u8 = [0-9]+" crates/mediaservo-link/src/token.rs | grep -oE "[0-9]+$")
grep -q "^token_schema_version: $TV$" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: token_schema_version != source value $TV (floating-number pin)"; FAIL=1; }
grep -q "^signaling_protocol_version: $SPV$" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: sdk-version.txt signaling_protocol_version != source value $SPV (C50/G1 pin)"; FAIL=1; }
grep -q "^signaling_min_supported: $SMIN$" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: sdk-version.txt signaling_min_supported != source value $SMIN (C50/G1 pin)"; FAIL=1; }
grep -q "^token_schema_version: 1$" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: sdk-version.txt lacks token_schema_version"; FAIL=1; }
grep -q "workspace_version: $VER" <(tar xzf "$HOST_TGZ" -O "$HOST_ROOT/host-version.txt") || { echo "FAIL: host-version.txt lacks workspace_version=$VER"; FAIL=1; }
echo "OK: version contract files (host-version.txt + sdk-version.txt) content correct"

# ── 7. dist/ gitignore ──────────────────────────────
note "dist/ gitignore"
git check-ignore dist/ >/dev/null || { echo "FAIL: dist/ is not gitignored"; FAIL=1; }
echo "OK: dist/ is gitignored"

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: package e2e all green ($HOST_TGZ + $SDK_TGZ)"
else
    echo "FAIL: $FAIL checks failed"; exit 1
fi

# P-0 guard: the sdk-only slice must leave the managed-process census untouched.
if [ "$SDK_ONLY" = 1 ]; then
    PROC_NOW=$(ps -eo comm 2>/dev/null | grep -cE '^(mediaservo-|msrtc-|oxmgr)' || true)
    if [ "$PROC_NOW" != "$PROC_BASE" ]; then
        echo "FAIL: sdk-only run changed the managed-process census ($PROC_BASE -> $PROC_NOW)"
        exit 1
    fi
    echo "OK: sdk-only guard held (managed-process census $PROC_BASE unchanged, no SHM wipe)"
fi
