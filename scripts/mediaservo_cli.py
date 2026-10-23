"""mediaservo — unified MediaServo build CLI (vapkg-style single entry point).

The thin shell (mediaservo.sh/.bat) guarantees this script runs after the pixi environment is activated:
PATH/LIBCLANG_PATH are already injected in the environment; subprocess calls cargo/docker directly.
Platform differences exist in only two places: e2e (bash scripts) and clean (delete commands).

Usage: mediaservo [-h] {build,build-host,build-server,up,down,logs,e2e,test,ci,deploy,install,package,clean,config,status,version} ...
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import time
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COMPOSE_BASE = ["docker", "compose", "-f", "docker-compose.dev.yml"]
HOST_CONF = ROOT / "crates/mediaservo-host/config/host.conf"
SERVER_YAML = ROOT / "config/server.docker.yaml"


# Delivery target -> version-source crate (F12: four independent version sources, packaging-side wiring; V1b four-package naming reuses this table).
_TARGET_CRATE = {"host": "mediaservo-host", "server": "mediaservo-server",
                 "bindings": "mediaservo-field", "sdk-field": "mediaservo-field",
                 "sdk-client": "mediaservo-client"}


def _crate_version(crate: str) -> str:
    """Crate-local version; falls back to root [workspace.package] when version.workspace = true."""
    text = (ROOT / "crates" / crate / "Cargo.toml").read_text()
    seg = text.split("[package]", 1)[1].split("[", 1)[0] if "[package]" in text else ""
    for line in seg.splitlines():
        line = line.strip()
        if line.startswith("version"):
            if "workspace" in line:
                return _workspace_version()
            return line.split("=", 1)[1].strip().strip('"').split("  #")[0]
    return _workspace_version()


def _frame_meta_wire_version() -> int:
    """FrameMeta wire version derived from the Rust source (D243/N4 — ends version.txt floating numbers)."""
    text = (ROOT / "crates" / "mediaservo-link" / "src" / "frame.rs").read_text()
    m = re.search(r"pub const WIRE_VERSION: u8 = (\d+)", text)
    if not m:
        raise SystemExit("Error: WIRE_VERSION constant not found in frame.rs (shape drift)")
    return int(m.group(1))


def _token_file_version() -> int:
    """MSTK token file format version derived from the Rust source (TokenFile::VERSION, D-H10)."""
    text = (ROOT / "crates" / "mediaservo-link" / "src" / "token.rs").read_text()
    m = re.search(r"pub const VERSION: u8 = (\d+)", text)
    if not m:
        raise SystemExit("Error: TokenFile::VERSION not found in token.rs (shape drift)")
    return int(m.group(1))


def _signaling_protocol_versions() -> tuple:
    """(max, min) signaling dialect constants derived from the Rust source (C50 rule 5 / G1,
    D-H14 closure — ends the version.txt signaling gap; same anti-drift shape as WIRE_VERSION)."""
    text = (ROOT / "crates" / "mediaservo-common" / "src" / "protocol.rs").read_text()
    m = re.search(r"pub const SIGNALING_PROTOCOL_VERSION: u32 = (\d+)", text)
    n = re.search(r"pub const SIGNALING_PROTOCOL_MIN_SUPPORTED: u32 = (\d+)", text)
    if not m or not n:
        raise SystemExit("Error: SIGNALING_PROTOCOL_VERSION/MIN_SUPPORTED not found in protocol.rs (shape drift)")
    return int(m.group(1)), int(n.group(1))


def _check(tool: str, hint: str) -> None:
    """Dependency check — exits with a clear error when missing (never silent)."""
    if shutil.which(tool) is None:
        print(f"Error: missing dependency '{tool}' — {hint}", file=sys.stderr)
        sys.exit(1)


def _run(cmd: list[str], env: dict[str, str] | None = None, cwd: str | None = None) -> int:
    """Run a command (inherits env by default); propagate the failure exit code."""
    print(f"$ {' '.join(cmd)}{'  (cwd: ' + cwd + ')' if cwd else ''}")
    return subprocess.run(cmd, env=env, cwd=cwd).returncode


def _run_or_exit(cmd: list[str], env: dict[str, str] | None = None, cwd: str | None = None) -> None:
    code = _run(cmd, env=env, cwd=cwd)
    if code != 0:
        sys.exit(code)


def _cmd_build_host() -> None:
    """build host — pure compile into target/ (assembly/branding moved to deploy — D266)."""
    _check("cargo", "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat")
    _run_or_exit(["cargo", "build", "-p", "mediaservo-host", "-p", "mediaservo-client"])
    print("✓ host build complete (pure compile to target/ — assembly and branding happen at deploy: mediaservo deploy host --prefix <X>)")

def _ensure_admin_dist() -> None:
    """Incremental admin frontend build (C13 dual-track parity — the Docker path builds inside the Dockerfile; the native path aligns with it).
    Decision: dist missing, or any src file newer than dist/index.html → pnpm build:admin (turbo filter).
    Zero extra cost when there are no frontend changes (mtime skip). Fixes the C24 incident (frontend edits not taking effect after rebuild)."""
    dist_index = ROOT / "www" / "apps" / "admin" / "dist" / "index.html"
    src_dir = ROOT / "www" / "apps" / "admin" / "src"
    need = True
    if dist_index.exists() and src_dir.exists():
        dist_mtime = dist_index.stat().st_mtime
        try:
            newers = [p for p in src_dir.rglob("*") if p.is_file() and p.stat().st_mtime > dist_mtime]
        except OSError:
            newers = []
        need = bool(newers)
    if not need:
        print(f"admin dist up to date ({dist_index.parent.relative_to(ROOT)} — no src changes) — skipping frontend build, reusing existing dist")
        return
    _check("pnpm", "pnpm not installed — required for the frontend build (or run manually: cd www && pnpm build:admin)")
    print("admin dist stale/missing — building frontend (tsc -b && vite build)...")
    _run_or_exit(["pnpm", "build:admin"], cwd=str(ROOT / "www"))


def _stage_web_to_out() -> None:
    """Merge dist into the out/server/web tree (frontend-process-split T2 — the only web consumer is the server deployment unit,
    single tree = single tar = single deploy; the Caddy static root points here)."""
    src = ROOT / "www" / "apps" / "admin" / "dist"
    dst = _out_root() / "server" / "web"
    if not (src / "index.html").exists():
        print(f"Error: no artifacts in {src} — pnpm build:admin not run or failed", file=sys.stderr)
        sys.exit(2)
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst)
    n = sum(1 for x in dst.rglob("*") if x.is_file())
    print(f"web deliverables staged: out/server/web/ ({n} files — Caddy static root)")


def _cmd_build_web() -> None:
    """build web — frontend-only fast lane: incremental pnpm by mtime → out/server/web (does not touch cargo)."""
    _ensure_admin_dist()
    _stage_web_to_out()






def _build_stage_weaknet(release: bool) -> None:
    """build server assembly step: compile weaknet + stage out/server/bin/{brand}-weaknet (single-point branding —
    after the PIT-191 root fix, build writes the final name directly, making deploy's rename a no-op on equal values). Profile follows --release propagation.
    Failure only WARNs and does not block the server main flow (weaknet-server-integration design §D1 [bF4 bF5] —
    running it last would take the mediasoup hung-build tree's exit guilt-by-association; on this machine recorded as [BA-15 tree-hang endorsed by CI])."""
    cmd = ["cargo", "build"] + (["--release"] if release else []) + ["-p", "mediaservo-weaknet"]
    try:
        code = _run(cmd)
    except OSError as e:  # cargo missing/cannot start (degraded path — the server main path has its own _check error exit)
        print(f"WARN: weaknet staging skipped (cargo unavailable: {e}) — not blocking the server build", file=sys.stderr)
        return
    if code != 0:
        print(f"WARN: weaknet compile failed (rc={code}) — not blocking the server build; after troubleshooting, re-run build server to complete staging", file=sys.stderr)
        return
    src = ROOT / "target" / ("release" if release else "debug") / _exe_name("mediaservo-weaknet")
    if not src.exists():
        print(f"WARN: weaknet compiled but the artifact is missing at {src} — not staging", file=sys.stderr)
        return
    # PIT-191 root-fix isomorphic extension: out/server/bin is the live cluster directory and the unit points at the brand name — build writing the upstream name
    # = freshly compiled binaries never take effect (dual-name drift; the 09-10 weaknet single-name fix only covered the deploy side). Resolution order same as server staging.
    bin_dir = _out_root() / "server" / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    brand = os.environ.get("MEDIASERVO_BRAND", "").strip() or _derive_brand_server(bin_dir)
    wname = _weaknet_bin_name(brand)
    tmp = bin_dir / (wname + ".new")
    shutil.copy2(src, tmp)
    os.replace(tmp, bin_dir / wname)
    if brand:
        legacy_w = bin_dir / _exe_name("mediaservo-weaknet")
        if legacy_w.exists():
            legacy_w.unlink()
            print("  reclaimed legacy sibling name mediaservo-weaknet (the unit only recognizes the brand name)", file=sys.stderr)
    print(f"weaknet staged: out/server/bin/{wname}")


def _cmd_build_server(image: str | None = None, native: bool = False, release: bool = False) -> None:
    """build server: default native (user ruling B — no mode written = native) | --image runtime|dev = Docker image (--native kept as compat alias)."""
    # weaknet staging comes first (design §D1 [bF4 bF5 BA-15]): it must precede the server cargo build — mediasoup
    # hangs in that posture don't guilt-by-association it; and its own failure only WARNs, never blocking server (separate try).
    _build_stage_weaknet(release)
    if native or image is None:   # default native; Docker only when --image is explicit
        _ensure_admin_dist()      # incremental frontend build by mtime (the Docker path handles it in the Dockerfile)
        _stage_web_to_out()       # after T3, default = no embedding; dist enters deliverables as a file tree (out/server/web)
        _check("cargo", "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat")
        if not os.environ.get("MESON"):
            print("Error: MESON environment variable not set — invoke via ./mediaservo.sh (source pixi-shell.sh injects the activation env)", file=sys.stderr)
            sys.exit(2)
        cmd = ["cargo", "build"] + (["--release"] if release else []) + ["-p", "mediaservo-server"]
        # tasks.py env pitfall (T1 Ruling extension, not foreseen in the brief): the MESON injected by pixi activation points to
        # a nonexistent NINJA → build.rs/tasks.py skip pip but hang when NINJA is missing; popping it matches T1 unset semantics.
        # The MESON assertion only proxies "activated via mediaservo.sh" (injected by pixi activation); after pop, cargo
        # subprocesses see no MESON/MESON_ARGS, consistent with T1 unset semantics (tasks.py falls back to official pip self-installing meson/ninja).
        env = {**os.environ}
        env.pop("MESON_ARGS", None)
        env.pop("MESON", None)
        _run_or_exit(cmd, env=env)
        # stage into out/server/bin/ (symmetric with build host/bindings — out = unified release root)
        server_bin = ROOT / "target" / ("release" if release else "debug") / "mediaservo-server"
        if server_bin.exists():
            # PIT-191 root fix: single-point brand in staging (resolution order shared with deploy: env MEDIASERVO_BRAND →
            # in-tree *-server derivation → upstream name). out/server/bin is the **live cluster directory** — build writing the legacy
            # name while the unit points at the brand name = the cluster silently runs the old binary (PIT-191 incident surface; same root as the 09-10 weaknet single-name change).
            bin_dir = _out_root() / "server" / "bin"
            bin_dir.mkdir(parents=True, exist_ok=True)
            brand = os.environ.get("MEDIASERVO_BRAND", "").strip() or _derive_brand_server(bin_dir)
            deployed, base = _server_bin_names(brand)
            tmp = bin_dir / (deployed + ".new")
            shutil.copy2(server_bin, tmp)
            os.replace(tmp, bin_dir / deployed)
            legacy = bin_dir / _exe_name("mediaservo-server")
            if brand and legacy.exists():
                legacy.unlink()
                print("  reclaimed legacy sibling name mediaservo-server (single-name bin — the unit only recognizes the brand name)", file=sys.stderr)
            print(f"server delivery layout staged: out/server/bin/{base} ({server_bin.stat().st_size // 1024} KB)")
        # ship caddy along (2026-09-29: detection only honored PATH → root fix for bare-shell start silently going --no-web) —
        # if found (PATH / in-repo pixi tree), copy it into bin/ (self-contained deployment: third probe-chain entry = bin's own directory);
        # if not found, WARN without blocking (old form: provided by PATH).
        caddy_src = shutil.which("caddy") or next(
            (c for c in (ROOT / ".pixi" / "envs" / "default" / "bin" / "caddy",
                         ROOT.parent / ".pixi" / "envs" / "default" / "bin" / "caddy")
             if c.is_file()), None)
        if caddy_src:
            caddy_dst = _out_root() / "server" / "bin" / "caddy"
            if not caddy_dst.exists() or Path(caddy_src).resolve() != caddy_dst.resolve():
                shutil.copy2(caddy_src, caddy_dst)
                print(f"caddy shipped: out/server/bin/caddy (from {caddy_src})")
        else:
            print("WARN: caddy not found (PATH / in-repo pixi tree) — out/server contains no caddy; "
                  "at start it must come from PATH, otherwise web silently degrades", file=sys.stderr)
        # stage the default config (server.yaml — derived from config/server.docker.yaml, accounts/devices relative paths)
        # PIT-158/160: skip if already present (runtime registries devices/accounts may be hot-written by the admin API,
        # and server.yaml may have been changed by ops) — build must not overwrite runtime data back with the template; to get a fresh template, delete the old file first.
        etc_dir = _out_root() / "server" / "etc"
        etc_dir.mkdir(parents=True, exist_ok=True)
        src_cfg = ROOT / "config" / "server.docker.yaml"
        if src_cfg.exists() and not (etc_dir / "server.yaml").exists():
            cfg = src_cfg.read_text()
            # point accounts/devices relative to the etc dir (after packaging: bin/../etc/accounts.yaml etc.)
            cfg = cfg.replace('/opt/mediaservo/etc/accounts.yaml', 'accounts.yaml')
            cfg = cfg.replace('/opt/mediaservo/etc/devices.yaml', 'devices.yaml')
            (etc_dir / "server.yaml").write_text(cfg)
        if src_cfg.exists():
            # copy device/account files (dev templates — filled only when missing on first run)
            for f in ("accounts.yaml", "devices.yaml"):
                src = ROOT / "config" / f
                dst = etc_dir / f
                if src.exists() and not dst.exists():
                    shutil.copy2(src, dst)
            print(f"server default config staged: {etc_dir}/server.yaml")
        return
    _check("docker", "install docker and start the daemon")
    if image:
        if image == "runtime":
            # production delivery image (--target runtime — slimmed / self-bootstrapping entrypoint)
            _run_or_exit(["docker", "build", "--target", "runtime", "-t", "mediaservo-server:latest", "."])
        else:
            _run_or_exit(COMPOSE_BASE + ["build", "server"])
        return
    _run_or_exit(COMPOSE_BASE + ["build", "server"])


def _cmd_build_client() -> None:
    _check("cargo", "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat")
    _run_or_exit(["cargo", "build", "-p", "mediaservo-client"])


def _cmd_build(target: str) -> None:
    """build <target> — all|host|server|client|bindings (default all)."""
    if target in ("all", "host"):
        _cmd_build_host()
    if target in ("all", "server"):
        _cmd_build_server()
    if target in ("all", "client"):
        _cmd_build_client()
    if target == "web":   # frontend-only fast lane (not in all — build server already stages web)
        _cmd_build_web()
    if target == "bindings":
        _cmd_build_bindings()


def _workspace_version() -> str:
    """Workspace version ([workspace.package] version, e.g. 0.1.0). py3.10 has no tomllib — lightweight parsing."""
    text = (ROOT / "Cargo.toml").read_text()
    seg = text.split("[workspace.package]", 1)[1].split("[", 1)[0]
    for line in seg.splitlines():
        line = line.strip()
        if line.startswith("version") and "=" in line:
            return line.split("=", 1)[1].strip().strip('"')
    raise SystemExit("Error: workspace version not found")


def _symlink_force(target: str, link: Path) -> None:
    """Idempotent symlink (delete first if present)."""
    try:
        link.unlink()
    except FileNotFoundError:
        pass
    os.symlink(target, link)


def _cmd_build_bindings(release: bool = False) -> None:
    """Build the three SDK cdylibs + dev .so.<MAJOR> symlinks (D241: DT_NEEDED resolution.

    ruled-away (SR4 P3 audit, T0 row #2): the DEV-tree .so.<MAJOR> symlinks under
    target/ stay python -- they are not a delivery layout concern (cmake install
    rules own the DELIVERY tree only); terminal owner ruled at SR6 batch-4 kickoff
    (drop vs tools-crate) per the ORPHAN GUARD table.

    Staging discipline (2026-09-29): the **build artifacts** of bindings/node and bindings/python
    (.node/.whl intermediates/_libs) all land in target/bindings-staging/ (gitignored area);
    the source tree returns to pure source — previously artifacts were written in-place into source dirs (bad hygiene + git noise + accidental-commit risk).
    """
    _check("cargo", "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat")
    staging = ROOT / "target" / "bindings-staging"
    staging.mkdir(parents=True, exist_ok=True)
    cmd = ["cargo", "build"]
    if release:
        cmd.append("--release")
    for _sdk in ALL_SDKS:
        cmd += ["-p", f"mediaservo-{_sdk}-c"]
    _run_or_exit(cmd)
    major = _workspace_version().split(".")[0]  # shim crates are workspace-inherited (S3)
    out_dir = ROOT / ("target/release" if release else "target/debug")
    for sdk in ALL_SDKS:
        _symlink_force(
            f"libmediaservo_{sdk}.so",
            out_dir / f"libmediaservo_{sdk}.so.{major}",
        )
    # node bindings (napi-rs .node; FFmpeg dynamic-lib linking completed via build.rs)
    _run_or_exit(["cargo", "build"] + (["--release"] if release else []) + ["-p", "mediaservo-node"])
    node_so = out_dir / "libmediaservo_node.so"
    node_node = staging / "mediaservo.node"
    if node_so.exists():
        shutil.copy2(node_so, node_node)
    # stage the delivery layout out/bindings (full SDK package mirror — completed per Momus HIGH3/Task 2.5):
    # D241 version triple + N7 strip = cmake install rules (SR4 L1); D248 headers (C/cxx)
    # = cmake install rules (SR4 L2); pkgconfig .pc + cmake config transport moves at
    # SR4 L3; python fat wheel/site-packages + node package stay language-native
    ver = _workspace_version()   # S3: deliverable .so triple = the shims' own (workspace) version
    major, minor, patch = ver.split(".")
    bind_dst = _out_root() / "bindings"
    lib_dst = bind_dst / "lib"; lib_dst.mkdir(parents=True, exist_ok=True)
    # Stale full-version sweep (09-17 wheel-sweep family) — SR4 L4: the leg
    # is a STALE_GLOB row on the merged polyorch_rust_install call
    # (cmake/MediaServoBindings.cmake; keep = the twelve landed names of this
    # install, sweep runs inside the `cmake --install` below, before the copy
    # rows). Python no longer sweeps -- one leg, one owner.

    # lib triple (D241) + N7 strip — SR4 L1: the copy/rename/link/strip legs are
    # cmake install RULES now (cmake/MediaServoBindings.cmake -> polyorch_rust_install
    # SOVERSION/VERSION/STRIP, supplier D46 P1/P7). The CLI only invokes; python no
    # longer lays any of the 12 lib names (one leg, one owner). --strip is mandatory
    # here: the STRIP CODE legs gate on CMAKE_INSTALL_DO_STRIP (install(FILES) rows
    # are NOT covered by --strip itself — that is precisely why the legs exist).
    _btype = "Release" if release else "Debug"
    _bind_build = Path("build") / "presets" / "bindings"
    _run_or_exit(["cmake", "--preset", "bindings", f"-DCMAKE_BUILD_TYPE={_btype}"], cwd=str(ROOT))
    _run_or_exit(["cmake", "--install", str(_bind_build), "--prefix", str(bind_dst), "--strip"], cwd=str(ROOT))

    # include/mediaservo (D248) -- SR4 L2: header staging is cmake install
    # rules now (HEADERS_DIRECTORY per SDK call + one stock install(DIRECTORY)
    # fan-in in cmake/MediaServoBindings.cmake). The tree form also lands the
    # recursive cxx subtree (detail/, 3rdparty/ + NOTICE) that python rglob'd.

    # SR4 L3 (#9 retired): the .pc + cmake-config renderers live on the cmake
    # bindings surface (per-domain version stamps via cargo metadata, suite
    # version for Config -- S3) and their TRANSPORT into the delivery tree is
    # install(FILES) rows in cmake/MediaServoBindings.cmake, staged by the
    # cmake --install above. The hand-suite SHAPE stays the template: the
    # supplier EXPORT stub/wrapper cannot byte-replicate mediaservo::<sdk>
    # INTERFACE targets + declarative COMPONENTS + per-domain SDK_LIST (L3
    # ruling 2b). The package-half SDK_LIST re-render stays python (tar /
    # CHANGES / manifest owner, D279 / SR6).

    # python: fat wheel (.so.<major> real files inside _libs — DT_NEEDED self-resolution) + pip --prefix assembly of site-packages
    # ruled-away (SR4 P3 audit): wheel CONTENTS are language-native packaging territory (SR4 §1.3) --
    # the _libs copies/symlinks below feed the WHEEL, never the cmake-laid delivery layout.
    # python wheel build tree = staging mirror (source files copied + _libs all live in staging;
    # zero writes into the source tree bindings/python — neither the build/ cache nor _libs enter source dirs anymore)
    # source mapping: bindings/python/mediaservo/ = pip build root (same level as pyproject) —
    # the whole layer is a staging mirror (artifacts/cache stripped); py_src is the build root.
    py_src = staging / "python-pkg"
    shutil.rmtree(py_src, ignore_errors=True)
    shutil.copytree(ROOT / "bindings" / "python" / "mediaservo", py_src,
                    ignore=shutil.ignore_patterns("build", "__pycache__", "_libs",
                                                  "*.egg-info", "dist", "examples"))
    py_pkg = py_src / "mediaservo"
    libs_src = py_pkg / "_libs"
    libs_src.mkdir(exist_ok=True)
    try:
        for sdk in ("field", "link", "deck"):  # py domain = the three device-half items (client belongs to the sdk-client package; V1b prevents redundancy in the wheel)
            so_major = libs_src / f"libmediaservo_{sdk}.so.{major}"
            shutil.copy2(out_dir / f"libmediaservo_{sdk}.so", so_major)
            _strip_so(so_major)  # same family: wheel-embedded _libs are immutable once baked in, so strip must be front-loaded
            _symlink_force(f"libmediaservo_{sdk}.so.{major}", libs_src / f"libmediaservo_{sdk}.so")
        wheel_dir = bind_dst / "wheel"; wheel_dir.mkdir(exist_ok=True)
        # stale whl sweep (caught on the first 09-17 e2e-delivery run: unordered glob + cross-run residue →
        # next() may grab an old file = the rename chain eats its own source, pip ENOENT).
        for _stale in wheel_dir.glob("*.whl"):
            _stale.unlink()
        # setuptools build/ cache same-family sweep (09-17 V1b; build/ now also lives in staging — zero source-tree pollution)
        shutil.rmtree(py_src / "build", ignore_errors=True)
        r = subprocess.run([sys.executable, "-m", "pip", "wheel", "--no-deps",
                            "--no-build-isolation", "-w", str(wheel_dir), "."],
                           cwd=str(py_src), capture_output=True, text=True)
        if r.returncode != 0:
            print(f"Error: wheel build failed — {r.stderr[-500:]}", file=sys.stderr)
            sys.exit(1)
        # force the platform tag (the wheel contains Linux .so; refuse any-tag misinstall across platforms): rewrite the Tag inside WHEEL + rename.
        # keep the version consistent with pyproject (= the embedded dist-info) — do not force workspace ver (the old install's {ver} rename
        # produced a dist-info version mismatch when workspace ver ≠ pyproject 0.1.0 → pip refused install — this implementation has a smaller fix surface)
        import zipfile
        wheel = next(wheel_dir.glob("mediaservo-*-py3-none-any.whl"))  # any-source form only
        tag_new = f"py3-none-{_platform_tag()}"
        with zipfile.ZipFile(wheel, "r") as z:
            items = z.infolist()
            data = {i.filename: z.read(i) for i in items}
        fixed = wheel.with_name(wheel.name.replace("py3-none-any", tag_new))
        with zipfile.ZipFile(fixed, "w") as z:
            for i in items:
                content = data[i.filename]
                if i.filename.endswith(".dist-info/WHEEL"):
                    content = content.replace(b"Tag: py3-none-any", f"Tag: {tag_new}".encode())
                z.writestr(i, content)
        wheel.unlink()
        # pip --prefix does not delete old data_files (caught in V1b: after the domain shrink, site-packages/_libs retains
        # the previous round's client.so corpse — the new wheel is correct but old files contaminate the install tree). Remove the old package tree before installing.
        for _old_sp in bind_dst.glob("lib/python3.*/site-packages/mediaservo"):
            shutil.rmtree(_old_sp, ignore_errors=True)
        # 10-22 e2e first-run catch: dist-info siblings were never swept -- an 0.1.1-era
        # corpse survived every rebuild and rode into BOTH half-packages.
        for _old_di in bind_dst.glob("lib/python3.*/site-packages/mediaservo-*.dist-info"):
            shutil.rmtree(_old_di, ignore_errors=True)
        r2 = subprocess.run([sys.executable, "-m", "pip", "install", "--prefix",
                             str(bind_dst), "--no-deps", str(fixed)],
                            capture_output=True, text=True)
        if r2.returncode != 0:
            print(f"Error: pip install failed — {r2.stderr[-500:]}", file=sys.stderr)
            sys.exit(1)
    finally:
        shutil.rmtree(libs_src, ignore_errors=True)  # clean up temp _libs (gitignore as backstop)

    # node package: node/mediaservo/ (package.json + .node + lib/index.mjs) — D248 layout
    node_src = ROOT / "bindings" / "node"
    if node_node.exists():
        node_dst = bind_dst / "node" / "mediaservo"
        node_dst.mkdir(parents=True, exist_ok=True)
        shutil.copy2(node_node, node_dst / "mediaservo.node")           # artifact = staging
        shutil.copy2(node_src / "package.json", node_dst)               # source = source tree
        (node_dst / "lib").mkdir(parents=True, exist_ok=True)
        shutil.copy2(node_src / "lib" / "index.mjs", node_dst / "lib")
    # docs: SDK consumer-side help (sdk-cxx manuals — directly readable from the out/bindings tree / release packages,
    # inter-links within the same dir work; remove-then-copy each round = guards against deleted-volume residue, same discipline family as the 09-17 wheel sweep)
    docs_dst = bind_dst / "docs" / "sdk-cxx"
    shutil.rmtree(docs_dst, ignore_errors=True)
    docs_dst.mkdir(parents=True)
    for m in (ROOT / "docs" / "reference" / "sdk-cxx").glob("*.md"):
        shutil.copy2(m, docs_dst)
    n_docs = len(list(docs_dst.glob("*.md")))
    print("bindings build complete: libmediaservo_{%s}.so version-full triple + node + python + .pc + cmake (%s)"
          % (",".join(ALL_SDKS), "release" if release else "debug"))
    n_lib = len([p for p in lib_dst.glob("libmediaservo_*") if p.is_file() or p.is_symlink()])
    print(f"bindings delivery layout staged: out/bindings/ (lib {n_lib} files + include/mediaservo + pkgconfig + cmake + python + wheel + node + docs/sdk-cxx {n_docs} volumes)")


# client added to the roster (V2 completion knife 09-17): C surface 15 symbols + cxx RAII headers in repo, pc.in in place — previously ALL_SDKS
# lacking client made the delivery surface = zero (find_package/pkg-config/include all absent). All downstream steps are driven by this constant + template generalization.
ALL_SDKS = ("field", "link", "deck", "client")


# D-H13: host package = 8 process binaries (host CLI + 7 daemons; the host-legacy old single process is not packaged)
HOST_BINS = (
    "mediaservo-host", "host-agent", "host-capturer", "host-streamer",
    "host-recorder", "host-controller", "host-emergency", "host-audio",
)


def _resolve_oxmgr_or_die(role: str) -> Path:
    """oxmgr hard-dependency source hunt (config-unification pre-fix, D-H13 reinforcement): PATH → ~/.cache/oxmgr
    cache reclaim → deploy fails when both are missing. The old "non-fatal hint" semantics was a deployment trap — with oxmgr missing from bin/,
    start/stop are all dead ('No such file or directory'), one deploy cycle away from the root cause."""
    found = shutil.which("oxmgr")
    if found:
        return Path(found)
    # reclaim the local cache (same version preferred; OxMgr official downloader layout oxmgr-v<ver>-<target>/oxmgr)
    cache = sorted(Path.home().glob(".cache/oxmgr/oxmgr-v*/oxmgr"), reverse=True)
    if cache:
        print(f"  oxmgr: not found in PATH, reclaiming local cache {cache[0]}")
        return cache[0]
    print(f"Error: oxmgr found in neither PATH nor ~/.cache/oxmgr — deploy {role} must package the process manager"
          " (a hard runtime dependency of start/stop). Install: download the prebuilt Rust binary from GitHub Releases"
          " (with sha256/asc verification, https://github.com/Vladimir-Urik/OxMgr/releases),"
          "or build oxmgr-src and place it in ~/.local/bin, then re-run deploy", file=sys.stderr)
    sys.exit(2)


def _exe_name(name: str) -> str:
    """Windows best-effort: binary names get .exe (other platforms keep the bare name)."""
    return name + (".exe" if sys.platform == "win32" else "")



def _pids_using(path: Path) -> list[int]:
    """Pids of processes occupying path: (1) fd open, (2) /proc/pid/exe==realpath (exec memory-mapped occupancy).
    Exact match — no basename/cmdline fuzzy matching (same-signature sibling processes get guilt-by-associated; on multi-instance machines that is a production incident
    — PIT-166; the 2026-08-31 deploy drill proved the old basename matching could hit other instances' daemons)."""
    target = os.path.realpath(path)
    pids: list[int] = []
    try:
        for p in os.listdir("/proc"):
            if not p.isdigit():
                continue
            pid = int(p)
            try:
                # (1) fd scan (open file descriptors)
                for fd in os.listdir(f"/proc/{p}/fd"):
                    try:
                        if os.path.realpath(f"/proc/{p}/fd/{fd}") == target:
                            pids.append(pid)
                            break
                    except OSError:
                        continue
                if pid in pids:
                    continue
                # (2) exe symlink (exec occupancy — the kernel records exactly this inode, no false positives)
                if os.path.realpath(f"/proc/{p}/exe") == target:
                    pids.append(pid)
            except OSError:
                continue
    except FileNotFoundError:
        pass
    return pids

def _kill_using(path: Path) -> None:
    """Kill processes occupying path (_pids_using exact predicate — kill by pid only, graceful SIGTERM exit)."""
    for pid in _pids_using(path):
        try:
            os.kill(pid, 15)  # SIGTERM (graceful daemon exit)
            print(f"  killed occupying process {pid}")
        except OSError:
            pass
    time.sleep(1)


def _copy_with_kill(src, dst: Path) -> None:
    """Copy with busy retry: Text file busy (occupied by a running process) → kill the occupier → retry (max 3 attempts)."""
    for attempt in range(3):
        try:
            shutil.copy2(src, dst)
            return
        except OSError as e:
            if e.errno != 26 or attempt == 2:  # 26 = Text file busy
                raise
            print(f"  {dst.name} occupied (Text file busy) — killing the occupier then retrying ({attempt + 1}/3)")
            _kill_using(dst)
    raise OSError(f"failed to copy {dst} (still busy after multiple retries)")

def _strip_so(path: Path) -> None:
    """strip --strip-unneeded on delivery-tree .so real files (N7 09-18 measurement: the debug cdylib was the entire
    cause of the fat wheel's 528MB — not FFmpeg — uncompressed 27M→~1M scale). Dynamic symbols (.dynsym / exported ABI) untouched,
    check-abi-drift and consumer ldd are unaware; only debug sections are removed. The target/ dev tree is untouched (gdb needs stacks)."""
    if not shutil.which("strip"):
        return
    if subprocess.run(["strip", "--strip-unneeded", str(path)],
                      capture_output=True, check=False).returncode != 0:
        print(f"  warning: strip {path.name} failed (skipped — delivery size not optimized)")


def _derive_brand(bin_dir: Path) -> str:
    """Derive the brand from an installed/staged bin directory (used by triple-stop/init name/shortcut names — no parameters, no hardcoding).
    Rule: if a <brand>-agent (any branded role) exists → that prefix; otherwise empty (official name host-*).
    Known limit: with multiple brands coexisting, glob order is filesystem-dependent (first match) — current deployments are single-brand; multi-brand needs a sorted/longest-prefix strategy."""
    for p in bin_dir.glob("*-agent"):
        name = p.name
        if name.startswith("host-"):
            continue
        return name[: -len("-agent")]
    return ""

def _derive_brand_server(bin_dir: Path) -> str:
    """server tree brand derivation (branding-completion BLOCKER-2): glob bin/*-server excluding the upstream name.
    Do not reuse _derive_brand (it only recognizes *-agent; the server tree would always return ""). Multiple brands coexisting: take the first (same known limit as host)."""
    for f in sorted(bin_dir.glob("*-server")):
        if f.name == _exe_name("mediaservo-server"):
            continue
        return f.name[: -len("-server")]
    return ""

def _server_bin_names(brand: str) -> tuple[str, str]:
    """(deployed bin name, root shortcut name) — empty brand → upstream name (never render a "-server" leftover name)."""
    base = f"{brand}-server" if brand else "mediaservo-server"
    return _exe_name(base), base

def _weaknet_bin_name(brand: str) -> str:
    """weaknet physical bin name — empty brand → upstream mediaservo-weaknet; never emit a "-weaknet" leftover name
    (weaknet-server-integration design §D1 [BA-2(i)], isomorphic contract with _server_bin_names).
    build only stages the upstream name; branding is single-pointed at deploy (D266/D269 isomorphic)."""
    return _exe_name(f"{brand}-weaknet" if brand else "mediaservo-weaknet")

# SR7 T4: the Python-side stale-oxfile/Caddyfile migration guard (_drop_stale_server_oxfile)
# + its env re-absorption twin (_reapply_carried_env) are retired -- the bF3 port (5255a13)
# runs migrate_stale_files as init step 0 (BEFORE both exists-skips) with the same
# predicates, .bak rotation and merge_manual_env re-absorption (baked values win, [lck-F4]
# half-migration guard preserved by construction: one rotation, one merge, inside init).
# WEAKNET_OXFILE_APP's duty lives on as the Rust constant templates::WEAKNET_APP [mn10].

def _cmd_deploy_host(prefix: str, release: bool = False, allow_stop: bool = True) -> None:
    """deploy host (stateful deployment — assemble + land, D266): identity idempotence/oxmgr/systemd/env.sh.
    Reused by package host (Momus b — Task 2.5): run this function inside the staging dir to assemble identity/etc/env.sh;
    the triple-stop is not triggered (staging has no run/oxfile.toml — cli F5 read-only safety).

    D4: --prefix mandatory (no default — protects out/ stateless delivery; vehicles use /opt/mediaservo-host).
    Brand: env MEDIASERVO_BRAND first (injected by msrtc.sh), falling back to _derive_brand of the installed instance (deploy-ops hazard 2)."""
    if not prefix:
        print("Error: deploy requires --prefix (no default — prevents polluting out/ stateless delivery; vehicles use /opt/mediaservo-host)", file=sys.stderr)
        sys.exit(2)
    prefix_p = Path(prefix)
    bin_dir = prefix_p / "bin"
    # Brand: env (msrtc.sh injects MEDIASERVO_BRAND) first, falling back to derivation from the existing instance (zero parameters on re-deploy)
    # Python layout aligned with Rust media_brand: "mediaservo" is the explicit default, mapping to legacy host-* strings.
    brand_input = os.environ.get("MEDIASERVO_BRAND", "") or _derive_brand(bin_dir)
    brand = "" if brand_input == "mediaservo" else brand_input
    rust_brand = brand or "mediaservo"
    try:
        bin_dir.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        print(f"Error: cannot create {bin_dir}: {e} — for deployment use sudo msrtc.sh deploy host --prefix /opt/mediaservo-host", file=sys.stderr)
        sys.exit(1)
    if str(prefix_p).startswith("/opt") and hasattr(os, "geteuid") and os.geteuid() != 0:
        print("Error: /opt deployment needs root (sudo msrtc.sh deploy host --prefix /opt/mediaservo-host)", file=sys.stderr)
        sys.exit(1)
    host_cli = _exe_name(f"{brand}-host") if brand else _exe_name("mediaservo-host")

    # Running instance binaries are occupied (Text file busy) → stop the process family automatically before copying.
    # Note: do not call the oxmgr CLI (its IPC commands auto-spawn the daemon when it isn't running — creating busy instead);
    # kill the host process family + the daemon occupying bin/oxmgr directly.
    oxfile = prefix_p / "run" / "oxfile.toml"
    if oxfile.exists() and (bin_dir / host_cli).exists() and not allow_stop:
        print("packaging staging: skipping process-family stop (a read-only package run never touches running instances)")
    elif oxfile.exists() and (bin_dir / host_cli).exists():
        print("Running host instance detected — stopping the process family first (reinstall does not overwrite etc/ credentials)")
        # Triple stop of revival sources: (1) systemd auto-start unit (Restart=always respawns right after the daemon is killed)
        # (2) the daemon itself (3) the host process family (restart_policy=always restarted by the daemon — the daemon must die first)
        # systemctl does not accept globs — enumerate only oxmgr-{brand}-*.service by the derived brand (deploy-ops hazard 2)
        units_dir = Path.home() / ".config" / "systemd" / "user"
        if shutil.which("systemctl") and units_dir.is_dir():
            want_home = f"Environment=OXMGR_HOME={(prefix_p / 'run' / 'oxmgr').resolve()}"
            for u in sorted(units_dir.glob(f"oxmgr-{brand}-*.service")):
                try:
                    # C50 prefix-scope: only units whose OXMGR_HOME is THIS instance's
                    # daemon dir. (2026-10-19 incident: a brand-glob stop from a package
                    # staging run tore down the LIVE vehicle unit — never again.)
                    txt = u.read_text(encoding="utf-8", errors="replace")
                except OSError:
                    continue
                if want_home not in txt:
                    print(f"  note: leaving unit {u.name} untouched (belongs to another instance home)")
                    continue
                subprocess.run(["systemctl", "--user", "stop", u.name], check=False)
                subprocess.run(["systemctl", "--user", "reset-failed", u.name], check=False)
        _kill_using(bin_dir / _exe_name("oxmgr"))
        for base in ("agent", "streamer", "capturer", "recorder",
                     "controller", "emergency", "audio"):
            subprocess.run(["pkill", "-x", _exe_name(f"{brand}-{base}" if brand else f"host-{base}")], check=False)
        time.sleep(1)

    # SR7 T3: branded bin assembly (D266 source = target/{debug,release}), the
    # bin whitelist residue reclaim and the root entry shortcut are cmake
    # install rules now (cmake/MediaServoAppTree.cmake — one leg, one owner).
    # The per-name N3 census: all 8 names are cmake-visible target-dir
    # artifacts -> zero ruled-away bins. ruled-away (vendor layer, plan §5):
    # the oxmgr copy below stays python — the archive unpack/cache is not a
    # delivery layout concern.
    _cmake_app_install(prefix_p, "host", brand, release)

    # lock the oxmgr version with the deployment (D-H13): PATH → cache reclaim → fail on missing hard dependency
    oxmgr_src = _resolve_oxmgr_or_die("host")
    _copy_with_kill(oxmgr_src, bin_dir / _exe_name("oxmgr"))
    ver = subprocess.run([str(oxmgr_src), "--version"], capture_output=True, text=True, check=False)
    print(f"  oxmgr packaged: {ver.stdout.strip() or ver.stderr.strip() or '?'}")

    # host init: generate etc/ + identity.json + tokens — idempotent write-only, never delete (skip if present; reinstall keeps credentials)
    # brand: env injection (init is a separate process — the device prefix must match the layout brand)
    identity_file = prefix_p / "identity.json"
    if identity_file.exists():
        print(f"  identity.json exists — write-only, never delete (device identity preserved): {identity_file}")
    init_env = dict(os.environ)
    init_env["MEDIASERVO_BRAND"] = rust_brand
    _run_or_exit([str(bin_dir / host_cli), "init", str(prefix_p)], env=init_env)

    # SR7 T3: the root entry shortcut (MED4 shape, relocatable relative link +
    # copy fallback) is a cmake link leg of the --install above.
    shortcut_names = (f"{brand}-host",) if brand else ("host", "mediaservo-host")

    # env.sh one-shot activation (carrying arch MED5 — generated at deploy; relocatable: derived relative to itself, not hardcoded)
    env_host = f"{brand}-host" if brand else "mediaservo-host"
    env_sh = prefix_p / "env.sh"
    env_lines = [
        "#!/usr/bin/env bash",
        f"# {env_host} environment activation (generated at deploy) — all commands available after sourcing",
        "# Usage: source env.sh (still valid under any prefix / after relocation, derived relative to this file)",
        '__MSRTC_ENV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"',
        'export PATH="${__MSRTC_ENV_DIR}/bin:$PATH"',
    ]
    env_lines.append(f'export MEDIASERVO_BRAND="{rust_brand}"   # brand env (D252); the deployment package pins the layout explicitly'),
    env_lines += [
        'BIN_DIR_MSRTC="${__MSRTC_ENV_DIR}/bin"   # for scripts/aliases to reference',
        "unset __MSRTC_ENV_DIR",
        f'echo "[{brand or "mediaservo"}] {env_host} environment activated — bin/: $BIN_DIR_MSRTC"',
    ]
    env_sh.write_text("\n".join(env_lines) + "\n")
    env_sh.chmod(0o755)
    print(f"  env.sh generated: {env_sh} (after sourcing, PATH=bin/ makes all commands available)")

    for d in ("run/logs", "recordings"):
        (prefix_p / d).mkdir(parents=True, exist_ok=True)

    print(f"host deployed to {prefix} ({'release' if release else 'debug'})")
    print(f"  bin/    {', '.join(sorted(x.name for x in bin_dir.iterdir()))}")
    print("  etc/    host.yaml + link/{signing.pem,*.token} (generated by host init, kept on reinstall)")
    print("  run/    logs/")
    print("  recordings/")
    print("  identity.json (0600, device identity — write-only, do not delete)")
    print(f"Usage: export PATH={bin_dir}:$PATH && {host_cli} doctor {prefix} && {host_cli} start {prefix}")
    if sys.platform == "win32":
        print("Note: Windows best-effort (not fully validated) — production deployment recommends Linux vehicles", file=sys.stderr)

def _cmake_app_install(prefix_p: Path, tree: str, brand: str, release: bool) -> None:
    """SR7: run the app delivery-layout install rules (cmake/MediaServoAppTree.cmake)
    into prefix_p. Configure is RE-RUN every invocation — the source-existence hunts
    (server/weaknet dual source, host per-name skip-missing) are configure-time truth
    and must never be frozen by a stale cache (PIT-189 family: never trust yesterday's
    tree). Brand and the delivery root ride cache variables (§1 rule 3: the CLI keeps
    invoke+brand, cmake describes layout)."""
    # Dedicated binaryDir (preset default's recipe, -B override): the app cache
    # vars must NEVER land in build/presets/default — a later plain IDE/gates
    # configure of that tree would inherit a stale MEDIASERVO_APP_TREE from the
    # cache and FATAL on a missing delivery tree (fresh-clone IDE regression).
    app_tree = ROOT / "build" / "app-tree"
    cfg = ["cmake", "--preset", "default", "-B", str(app_tree),
           f"-DMEDIASERVO_APP_TREE={tree}",
           f"-DMEDIASERVO_BRAND={brand}",
           f"-DCMAKE_BUILD_TYPE={'Release' if release else 'Debug'}"]
    if tree == "server":
        cfg.append(f"-DMEDIASERVO_SERVER_OUT_DIR={_out_root() / 'server'}")
    _run_or_exit(cfg, cwd=str(ROOT))
    inst = ["cmake", "--install", str(app_tree), "--prefix", str(prefix_p)]
    # ETXTBSY policy stays invocation-side (SR4 L5 precedent): graceful stop ran
    # above; a survivor (daemon respawn race) kills the occupier BY EXACT PATH and
    # re-runs the idempotent install (rerun-safe by cmake semantics).
    for attempt in range(3):
        r = subprocess.run(inst, cwd=str(ROOT), capture_output=True, text=True)
        if r.returncode == 0:
            return
        blob = (r.stdout or "") + (r.stderr or "")
        if ("busy" not in blob.lower() and "Text file" not in blob) or attempt == 2:
            print(f"Error: cmake --install {tree} app tree into {prefix_p} failed (rc={r.returncode}):\n{blob[-600:]}", file=sys.stderr)
            sys.exit(1)
        print("  install hit busy artifacts — killing occupiers, retrying "
              f"({attempt + 1}/3)", file=sys.stderr)
        b = prefix_p / "bin"
        for f in sorted(b.glob("*")) if b.is_dir() else []:
            if f.is_file():
                _kill_using(f)


def _cmd_deploy_server(prefix: str) -> None:
    """deploy server (stateful landing — T20, D266 isomorphic with host; deploy does not trigger builds — source = the single out/server delivery tree).
    Brand: env MEDIASERVO_BRAND first, falling back to target-tree bin/*-server derivation (_derive_brand_server) —
    the physical binary name = {brand}-server (D269 brand = physical name, isomorphic with host; cargo/bin unit names unchanged).
    BLOCKER-1 dual source hunt (cmake rows, SR7 T1): upstream name (fresh build) → {brand}-server (already migrated, so a second deploy needs no rebuild) — python repeats the hunt only to pick the pre-install stop target.
    BLOCKER-2 empty-brand guard: brand resolves empty while the target tree already has branded bin → reject (empty brand never deletes/renames branded files).
    BLOCKER-3 oxfile/Caddyfile refresh: now owned by init alone (bF3 port — migrate_stale_files rotates the stale files to .bak BEFORE its exists-skips and re-absorbs [apps.env] via merge_manual_env; SR7 T4 retired the python guard twin).
    bin/etc placement = cmake install rules (MediaServoAppTree.cmake via _cmake_app_install);
    etc landing is first-writer-wins (PIT-160 — ops changes/registry data survive redeploys); the web tree still moves wholesale here (rmtree+copytree — pnpm product, rule 6);
    init renders idempotently (run/oxfile.toml + Caddyfile + PSK/JWT secret bootstrap — lifecycle modules are built into the binary).
    D4: --prefix mandatory (no default); /opt needs root (same rule as host)."""
    if not prefix:
        print("Error: deploy requires --prefix (no default — prevents polluting out/ stateless delivery; convention: /opt/mediaservo-server)", file=sys.stderr)
        sys.exit(2)
    src_root = _out_root() / "server"
    src_bin_dir = src_root / "bin"
    prefix_p = Path(prefix)
    bin_dir = prefix_p / "bin"
    # Brand: env first → target-tree derivation → source-tree derivation (same as host L554: zero-parameter idempotent re-deploy;
    # the source-tree fallback covers the "out/server already branded in place → deploy /opt without env" scenario).
    brand = (os.environ.get("MEDIASERVO_BRAND", "")
             or _derive_brand_server(bin_dir)
             or _derive_brand_server(src_bin_dir))
    if not brand:
        unexpected = sorted({f.name for d in (bin_dir, src_bin_dir) for f in d.glob("*-server")
                             if f.name != _exe_name("mediaservo-server")})
        if unexpected:
            # derivation always hits branded files, theoretically unreachable — defensive assertion: empty brand never reaches delete/rename
            print(f"Error: {', '.join(unexpected)} present but brand resolved empty — export MEDIASERVO_BRAND and retry", file=sys.stderr)
            sys.exit(2)
    bin_name, shortcut = _server_bin_names(brand)
    server_bin_name = bin_name
    # BLOCKER-1 dual source hunt (ordered): upstream name (fresh build output) → {brand}-server (migrated tree, no rebuild needed)
    candidates = [src_bin_dir / _exe_name("mediaservo-server")]
    if brand:
        candidates.append(src_bin_dir / _exe_name(f"{brand}-server"))
    src_bin = next((cand for cand in candidates if cand.exists()), None)
    if src_bin is None:
        print(f"Error: none of {' / '.join(str(c) for c in candidates)} exists — run build server first (deploy does not trigger builds — D266)", file=sys.stderr)
        sys.exit(2)
    src_web = src_root / "web"
    if not (src_web / "index.html").exists():
        print(f"Error: no frontend artifacts in {src_web} — run build web first (deploy does not trigger builds — D266)", file=sys.stderr)
        sys.exit(2)
    try:
        bin_dir.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        print(f"Error: cannot create {bin_dir}: {e} — for deployment use sudo mediaservo deploy server --prefix /opt/mediaservo-server", file=sys.stderr)
        sys.exit(1)
    if str(prefix_p).startswith("/opt") and hasattr(os, "geteuid") and os.geteuid() != 0:
        print("Error: /opt deployment needs root (sudo mediaservo deploy server --prefix /opt/mediaservo-server)", file=sys.stderr)
        sys.exit(1)

    # build:deploy sugar injects --prefix out/<target> (host convention) — for server the source is the target: in-place assembly mode,
    # cross-tree copies and the web rmtree must be skipped (SameFileError guard + delete-source self-destruction guard); init rendering proceeds;
    # in-place rename (upstream→brand) is a same-directory os.rename, allowed (branding-completion T1.3).
    inplace = prefix_p.resolve() == src_root.resolve()
    dst_bin = bin_dir / bin_name
    server_cli = dst_bin
    # stop before move (PIT-170 isomorphic: graceful stop first; never force through a running binary via _copy/replace — otherwise
    # _copy_with_kill's inode kill would bypass the cluster's graceful stop, and inplace rename opens a window where the daemon relaunches dead paths from a stale oxfile).
    # check both old and new paths (before rename = old name running; last deploy = new name running); stop uses the live lifecycle binary.
    stop_bin = dst_bin if dst_bin.exists() else src_bin
    running = _pids_using(dst_bin)
    for cand in candidates:
        if cand.name != dst_bin.name:
            running = running or _pids_using(bin_dir / cand.name)
    if running or _pids_using(bin_dir / _exe_name("oxmgr")):
        print("Running server instance detected — stopping this cluster first (reinstall does not overwrite etc/ credentials)")
        try:
            subprocess.run([str(stop_bin), "stop", str(prefix_p)], capture_output=True, timeout=30, check=False)
        except subprocess.TimeoutExpired:
            print("  ⚠ old instance stop timed out (possibly a pre-lifecycle old binary) — continuing reinstall", file=sys.stderr)
    # SR7 T1: bin placement = cmake install rules (cmake/MediaServoAppTree.cmake) —
    # the rows land {brand}-server, reclaim the whitelist and re-lay the entry links
    # in one --install pass (the weaknet row joins below). In-place assembly (prefix =
    # out/server) same-name rows are file(INSTALL) no-ops; the upstream→brand
    # migration lands as copy + sweep reclaim (final bytes identical). The renamed/
    # oxfile-refresh trigger moved to the authoritative side: Rust init's
    # migrate_stale_files (bF3 port) detects the drift itself.
    _cmake_app_install(prefix_p, "server", brand, False)
    server_cli = dst_bin
    # lifecycle smoke test: out/ may be an old build without init/start subcommands (old binaries treat unknown args as daemon args and boot straight through)
    ver_line: str | None = None
    try:
        probe = subprocess.run([str(server_cli), "version"], capture_output=True, text=True, timeout=10, check=False)
        if probe.returncode == 0:
            out_lines = (probe.stdout or "").strip().splitlines()
            ver_line = out_lines[0] if out_lines else "version ok"
    except subprocess.TimeoutExpired:
        pass
    if ver_line is None:
        print(f"Error: {server_cli} lifecycle smoke failed (version nonzero/timeout — out/ is likely a pre-lifecycle old build) — rebuild server then re-run deploy", file=sys.stderr)
        sys.exit(2)
    print(f"  binary ready: {ver_line}")

    # lock the oxmgr version with the deployment (D-H13, same treatment as host): PATH → cache reclaim → fail on missing hard dependency
    oxmgr_src = _resolve_oxmgr_or_die("server")
    _copy_with_kill(oxmgr_src, bin_dir / _exe_name("oxmgr"))
    os.chmod(bin_dir / _exe_name("oxmgr"), 0o755)
    ov = subprocess.run([str(oxmgr_src), "--version"], capture_output=True, text=True, check=False)
    print(f"  oxmgr packaged: {ov.stdout.strip() or ov.stderr.strip() or '?'}")

    # SR7 T1: weaknet staging = cmake row (same --install above — dual source,
    # target/{release,debug} fallback and the absent WARN skip live in
    # MediaServoAppTree.cmake [bF1 bF4 bF5]).
    # SR7 T4: the BLOCKER-3 stale-oxfile/Caddyfile rotation decision belongs to
    # the managed binary alone — init's migrate_stale_files (bF3) runs BEFORE
    # its exists-skips, rotates on drift, re-renders and re-absorbs [apps.env]
    # via merge_manual_env. The Python guard and its reapply twin are gone.

    # SR7 T2: the etc template mirror (first-writer-wins, PIT-160) is a cmake
    # install(CODE) leg now (MediaServoAppTree.cmake) — run by the --install
    # above; the per-file "exists — kept" quietness moved into the leg.

    # the web tree moves wholesale (caddy static root — absolute paths written into oxfile/Caddyfile by init)
    dst_web = prefix_p / "web"
    if inplace:
        print("  web/ in place (source = target) — skipping move (rmtree guard)")
    else:
        if dst_web.exists():
            shutil.rmtree(dst_web)
        shutil.copytree(src_web, dst_web)
    (prefix_p / "run").mkdir(parents=True, exist_ok=True)

    # init renders idempotently: etc/Caddyfile + run/oxfile.toml (server+caddy always + weaknet-serve conditionally
    # rendered [M-1] — written only when bin is present; the check lives in Rust render_oxfile) + secret bootstrap (0600)
    # SFU port/announce isolation: export MEDIASERVO_SFU_PORT / MEDIASERVO_SFU_ANNOUNCED_IP before deploy; init bakes them into the oxfile env
    _run_or_exit([str(server_cli), "init", str(prefix_p)])

    # SR7 T1: the bin whitelist reclaim and the root entry shortcuts (incl. the
    # old-brand link cleanup) are cmake install legs now (MediaServoAppTree.cmake,
    # run by the --install above). Names below are kept print-only for the summary.
    shortcut_names = (shortcut,) if brand else ("server", "mediaservo-server")

    print(f"server deployed to {prefix}" + (" (assembled in place — rendering complete, start directly)" if inplace else "")
          + (f" (brand {brand})" if brand else ""))
    print(f"  bin/    {server_bin_name} (physical name branded — D269) + oxmgr (pinned by D-H13)")
    print("  etc/    server/devices/accounts.yaml + Caddyfile + secret (redeploys keep existing — PIT-160)")
    print("  web/    frontend deliverables moved as a whole tree (caddy static root)")
    print("  run/    oxfile.toml + oxmgr/ (OXMGR_DATA_DIR — C32 isolation) + logs/")
    print("  entrypoints: " + ", ".join(str(prefix_p / s) for s in shortcut_names) + f" → bin/{bin_name}")
    print("⚠ accounts.yaml is a dev placeholder template (admin123 etc.) — bare-metal startup will fail-fast (C35 guard):")
    print('    production: export MEDIASERVO_ADMIN_PASSWORD=*** then delete etc/accounts.yaml and re-run this deploy')
    print('    integration: add MEDIASERVO_ALLOW_DEV_CREDENTIALS = "1" under the server entry [apps.env] in run/oxfile.toml')
    print("Next steps (instance-directory commands; this CLI's run branch is retired — C39):")
    print(f"  start:   {prefix_p / shortcut} start {prefix} (backend only: add --no-web; frontend transition: mediaservo run web)")
    print(f"  probe:   {prefix_p / shortcut} status {prefix} (exit codes 0/1/2) | doctor {prefix}")
    print(f"  boot anchor (operator step, one-time): {prefix_p / shortcut} startup on {prefix}")


def _platform_tag() -> str:
    """wheel platform tag (Linux x86_64 → linux_x86_64; other platforms best-effort)."""
    import platform
    mach = platform.machine().lower().replace("-", "_")
    return f"{platform.system().lower()}_{mach}"


def _cmd_deploy_bindings(prefix: str, release: bool = False) -> None:
    """Deploy the full SDK layout from out/bindings (D241 version-full triple + include C/cxx + .pc + cmake
    + python/wheel + node — build already assembled it; deploy is a pure copy). SDK is brand-independent (libmediaservo_* kept verbatim, D3).
    prefix default = `<out>/bindings` (same root as room-grouping: the out root resolves per calling shell — msrtc.sh injects
    MSRTC_OUT_ROOT = the main repo's out; mediaservo.sh / bare CLI fall back to the submodule's out. Same-tree deploy = idempotent guard).
    D4 narrowed (2026-09-29): bindings are pure-static/stateless → defaultable; host/server still mandatory (protects runtime data from mis-writes)."""
    if not prefix:
        prefix = str(_out_root() / "bindings")
        print(f"[deploy bindings] prefix default → {prefix} (out root follows the calling shell; --prefix overrides)")
    src = _out_root() / "bindings"
    if not (src / "lib").is_dir():
        print(f"Error: no assembled artifacts in {src} — run build bindings first", file=sys.stderr)
        sys.exit(1)
    prefix_p = Path(prefix)
    try:
        prefix_p.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        print(f"Error: cannot create {prefix_p}: {e} — for deployment use sudo msrtc.sh deploy bindings --prefix /opt/mediaservo-sdk", file=sys.stderr)
        sys.exit(1)
    lib_dir = prefix_p / "lib"
    lib_dir.mkdir(parents=True, exist_ok=True)
    # same-tree idempotent guard (build:deploy bindings default prefix = out/bindings = the assembly source itself):
    # the build phase already ran the install rules into it; a second full pass is pure churn (09-21 SameFileError lesson).
    if prefix_p.resolve() == src.resolve():
        sos = sorted(x.name for x in (src / "lib").glob("libmediaservo_*.so*"))
    else:
        # SR4 L5 (T0 #17): the cmake-owned half (lib triple + symlinks + strip,
        # include tree, pkgconfig + cmake suite) reaches ANY prefix through the SAME
        # install rules the build used -- one route, one owner, zero hand-laid copies.
        # ETXTBSY policy stays invocation-side: retry the install after killing the
        # occupiers (install is idempotent -- rerun-safe by cmake semantics).
        _bind_build = ROOT / "build" / "presets" / "bindings"
        if not (_bind_build / "CMakeCache.txt").exists():
            _run_or_exit(["cmake", "--preset", "bindings",
                          f"-DCMAKE_BUILD_TYPE={'Release' if release else 'Debug'}"], cwd=str(ROOT))
        ctypes = ["cmake", "--install", str(_bind_build), "--prefix", str(prefix_p), "--strip"]
        for attempt in range(3):
            r = subprocess.run(ctypes, cwd=str(ROOT), capture_output=True, text=True)
            if r.returncode == 0:
                break
            blob = (r.stdout or "") + (r.stderr or "")
            if "busy" not in blob.lower() and "Text file" not in blob or attempt == 2:
                print(f"Error: cmake --install into {prefix_p} failed (rc={r.returncode}):\n{blob[-600:]}", file=sys.stderr)
                sys.exit(1)
            print(f"  install hit busy artifacts — killing occupiers, retrying ({attempt + 1}/3)")
            for so in sorted(lib_dir.glob("libmediaservo_*.so*")):
                _kill_using(so)
        sos = sorted(x.name for x in lib_dir.glob("libmediaservo_*.so*"))
        # language-native half (wheel/, node/, docs/, pip site-packages under lib/)
        # stays python-side per SR4 §1.3 -- the cmake rules never touch them.
        # pkgconfig/ + cmake/ under lib/ are RULE-OWNED (L3) -- re-copying them from
        # the out tree would resurrect the two-writer disease; only python3.* moves.
        for sub in sorted((src / "lib").glob("python3.*/")):
            shutil.copytree(sub, lib_dir / sub.name, dirs_exist_ok=True)
        for e in sorted(src.iterdir()):
            if e.name in ("lib", "include"):
                continue  # cmake-owned (L1-L3 rules) -- do not double-lay
            dst = prefix_p / e.name
            if e.is_dir():
                shutil.copytree(e, dst, dirs_exist_ok=True)
            elif e.is_file() or e.is_symlink():
                shutil.copy2(e, dst, follow_symlinks=False)
    print(f"bindings deployed to {prefix} (lib/ {', '.join(sos)} + include/mediaservo + pkgconfig + cmake + python + wheel + node + docs/sdk-cxx)")
    print("  usage: export LD_LIBRARY_PATH=<prefix>/lib; link per the bindings headers; python: pip install <prefix>/wheel/*.whl")


# ── package: CPack release tars (D-H13 / SR6 P1) ─────────────────────────
def _git_out(args: list[str], timeout: int = 10) -> str | None:
    """Read-only git query (captures stdout, cwd = repo root); any failure (no git / shallow clone / timeout) → None, never raises
    — the degraded surface of the release-integrity guard (package-changes design §boundaries)."""
    try:
        r = subprocess.run(["git", *args], cwd=str(ROOT), capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout if r.returncode == 0 else None


def _cpack_stage_sdk(staging: Path, args: argparse.Namespace) -> str:
    """L1: stage the sdk halves via install rules (cmake/MediaServoPack.cmake gates
    on -DMEDIASERVO_PKG). Returns the package-root name the tree computed — naming
    owner = cmake (C42 + 10-19 platform-suffix formula ported; package-root.txt is
    its readback channel, so python never re-derives the tar name)."""
    src = _out_root() / "bindings"
    if not src.exists():
        print("Error: out/bindings does not exist — run build bindings first", file=sys.stderr)
        sys.exit(1)
    pkg = "sdk-field" if args.target in ("bindings", "sdk-field") else args.target
    bdir = ROOT / "build" / "packages" / args.target   # alias keeps its own cache dir
    cfg = ["cmake", "--preset", "default", "-B", str(bdir),
           f"-DMEDIASERVO_PKG={pkg}",
           f"-DMEDIASERVO_PKG_BRAND={args.brand}",
           f"-DMEDIASERVO_BIND_OUT={src}"]
    if args.target == "bindings":
        cfg.append("-DMEDIASERVO_PKG_ALIAS=sdk")   # one-cycle naming slot (F12)
    _run_or_exit(cfg, cwd=str(ROOT))
    # reconfigure-always at the call site (SR7 discipline): the source-existence
    # globs are configure-time truth (PIT-189 family)
    _run_or_exit(["cmake", "--install", str(bdir), "--prefix", str(staging)], cwd=str(ROOT))
    return (bdir / "package-root.txt").read_text().strip()


def _cpack_stage_app(staging: Path, args: argparse.Namespace) -> str:
    """L3/L4: stage the host/server release tree via the SR7 placement rows plus the
    stateful install legs (design (e)): vendor copy row, web move, lifecycle probe and
    init run against the STAGED root inside cmake --install — same managed binaries as
    the deploy verb, zero second implementation. Returns the cmake-computed root name."""
    target = args.target
    oxmgr = str(_resolve_oxmgr_or_die(target))
    bdir = ROOT / "build" / "packages" / target
    cfg = ["cmake", "--preset", "default", "-B", str(bdir),
           f"-DMEDIASERVO_PKG={target}", f"-DMEDIASERVO_APP_TREE={target}",
           f"-DMEDIASERVO_OXMGR_BIN={oxmgr}",
           f"-DMEDIASERVO_PKG_BRAND={args.brand}"]
    if target == "server":
        src_root = _out_root() / "server"
        if not (src_root / "web" / "index.html").exists():
            print(f"Error: no frontend artifacts in {src_root / 'web'} — run build web first (deploy does not trigger builds — D266)", file=sys.stderr)
            sys.exit(2)
        # deploy server's brand chain with the always-empty staging arm dropped
        # (a fresh staging dir derives nothing): env -> source-tree hunt
        brand = (os.environ.get("MEDIASERVO_BRAND", "")
                 or _derive_brand_server(src_root / "bin"))
        cfg += [f"-DMEDIASERVO_BRAND={brand}",
                f"-DMEDIASERVO_SERVER_OUT_DIR={src_root}",
                "-DCMAKE_BUILD_TYPE=Debug"]   # deploy server assembly is Debug-pinned
    else:
        brand_input = os.environ.get("MEDIASERVO_BRAND", "").strip()
        brand = "" if brand_input == "mediaservo" else brand_input
        cfg += [f"-DMEDIASERVO_BRAND={brand}",
                f"-DCMAKE_BUILD_TYPE={'Release' if args.release else 'Debug'}"]
    _run_or_exit(cfg, cwd=str(ROOT))
    _run_or_exit(["cmake", "--install", str(bdir), "--prefix", str(staging)], cwd=str(ROOT))
    return (bdir / "package-root.txt").read_text().strip()


def _cpack_pack(staging: Path, package_root: str, ver: str, dist: Path) -> Path:
    """CPack TGZ over the staged tree (design (a)/(c)): the wrapper dir and the tar
    name both come from CPACK_PACKAGE_FILE_NAME; symlinks and empty dirs survive
    (measured on this cpack). Scratch goes to a dedicated temp work dir INSIDE dist
    (same filesystem for the move; the work dir self-cleans — dist keeps only the
    tar, matching the python path's footprint)."""
    _check("cpack", "cmake's packaging tool missing — install cmake (>= 3.25)")
    work = Path(tempfile.mkdtemp(prefix=".cpack-work-", dir=str(dist)))
    try:
        cfgp = work / "cpack-config.cmake"
        cfgp.write_text(
            'set(CPACK_GENERATOR "TGZ")\n'
            'set(CPACK_PACKAGE_NAME "mediaservo")\n'
            f'set(CPACK_PACKAGE_VERSION "{ver}")\n'
            'set(CPACK_PACKAGE_DESCRIPTION "MediaServo release package")\n'
            f'set(CPACK_PACKAGE_FILE_NAME "{package_root}")\n'
            f'set(CPACK_INSTALLED_DIRECTORIES "{staging};/")\n')
        _run_or_exit(["cpack", "--config", str(cfgp), "-B", str(work)], cwd=str(ROOT))
        out = dist / f"{package_root}.tar.gz"
        shutil.move(str(work / f"{package_root}.tar.gz"), str(out))
        return out
    finally:
        shutil.rmtree(work, ignore_errors=True)


def _cmd_package(args: argparse.Namespace) -> None:
    """package <target> — <dist>/<brand>-<target>-<ver>-<platform>-<arch>.tar.gz (D-H13/F12).
    SR6 P1 end state: the release tree is described by cmake install rules
    (cmake/MediaServoPack.cmake; host/server reuse the SR7 app-tree rows and run their
    stateful steps — init/oxmgr/env.sh/strip — as install-time legs against the staging
    root via the SAME managed binaries, design (e)) and packed by native CPack TGZ.
    Python owns invoke + brand + lifecycle only (F29/F30 retreating-audit green).
    dist: --dist selects the output dir; defaults to the submodule dist/ when unspecified
    (the MSRTC release shell injects out/packages/). The tar's top level is the versioned
    dir, so a bare unpack never scatters credentials/binaries; to land directly into a
    prefix dir use `tar xzf package.tar.gz -C <prefix> --strip-components=1`."""
    if sys.platform == "win32":
        print("package: Windows best-effort — validation checklist in scripts/e2e-win-validate.ps1", file=sys.stderr)
    if args.target == "bindings":
        print("WARN: target `bindings` renamed to `sdk-field` (F12 dual-half split; this alias stays for one version cycle)",
              file=sys.stderr)
    ver = _crate_version(_TARGET_CRATE[args.target])
    # C43(8) release integrity guard: the workspace Cargo.toml differs from HEAD and git is available → the packaged version has no
    # git anchor (no tag to attach, same family as the v0.1.8.2 floating-number incident) — WARN without blocking (debug packages may be dirty).
    if _git_out(["log", "-1", "--format=%s"]) is not None and \
            _git_out(["diff", "--quiet", "HEAD", "--", "Cargo.toml",
                                  "crates/mediaservo-host/Cargo.toml",
                                  "crates/mediaservo-server/Cargo.toml",
                                  "crates/mediaservo-field/Cargo.toml",
                                  "crates/mediaservo-client/Cargo.toml"]) is None:
        print(f"WARN: Cargo.toml has uncommitted changes carrying version {ver} — before release: commit the bump + tag v{ver}"
              " (this package's CHANGES header version has no git anchor)", file=sys.stderr)
    dist = Path(args.dist) if getattr(args, "dist", "") else ROOT / "dist"
    dist.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f"ms-{args.target}-pkg-", dir=str(dist)))
    try:
        # naming + layout owner = cmake (package-root.txt readback; the platform/arch
        # formula of the 2026-10-19 ruling lives in MediaServoPack.cmake)
        if args.target in ("host", "server"):
            package_root = _cpack_stage_app(staging, args)   # design (e) stateful legs
        else:  # bindings (one-cycle alias, WARN above) / sdk-field / sdk-client
            package_root = _cpack_stage_sdk(staging, args)
        out = _cpack_pack(staging, package_root, ver, dist)
        print(f"✓ packaged: {out} ({out.stat().st_size // 1024} KiB)")
        print(f"  top-level dir: {package_root}/")
        print(f"  contents: {', '.join(sorted(e.name for e in staging.iterdir()))}")
    finally:
        # staging hygiene: a run against a fresh tree must never leave a daemon whose
        # OXMGR_HOME points into dist/ (PIT-224 family — auto-spawn during staging init).
        for q in Path("/proc").iterdir():
            if not q.name.isdigit():
                continue
            try:
                env = (q / "environ").read_bytes().split(b"\0")
                home = next((e.split(b"=", 1)[1].decode(errors="replace") for e in env
                             if e.startswith(b"OXMGR_HOME=") and e.split(b"=", 1)[1].decode(errors="replace").startswith(str(dist))), None)
                comm = (q / "comm").read_text().strip()
                if home and comm == "oxmgr":
                    print(f"  note: reaping staging-scoped daemon pid={q.name} home={home}")
                    os.kill(int(q.name), 15)
            except OSError:
                continue
        shutil.rmtree(staging, ignore_errors=True)

# Excluded interface types/names: docker bridges, virtual interfaces (these IPs are unreachable by clients).
# Note: tun/vpn are **not excluded** — in VPN-mesh scenarios the tunnel IP is exactly the client-reachable path (proven at 10.144.0.3);
# extra announced candidates are harmless (ICE multi-candidate; peers skip unreachable ones automatically).
_ANNOUNCED_IP_BLOCKED_IFACE = ("lo", "docker", "br-", "veth", "virbr")


def _detect_announced_ips() -> list[str]:
    """Detect all real NIC IPs of the host machine (for mediasoup announced_address inside containers).

    Host IPs change and may be multiple (multi-NIC/DHCP) — returns all real IPs (comma-separated),
    and the server side creates a ListenInfo per IP (WebRtcServer multi-announced).
    Filters docker bridges (br-*) / virtual interfaces (veth/virbr) by name; tun/vpn are **kept**
    (VPN-mesh tunnel IPs are exactly the client-reachable path — PIT-143 multi-IP announcement semantics, not filter targets)."""
    ips: list[str] = []
    try:
        out = subprocess.run(
            ["ip", "-o", "-4", "addr", "show"], capture_output=True, text=True, timeout=5, check=False
        )
        for line in out.stdout.splitlines():
            # format: "2: ens32    inet 192.168.2.127/24 brd ... scope global ..."
            parts = line.split()
            if len(parts) < 4 or parts[2] != "inet":
                continue
            iface = parts[1].rstrip(":")
            if any(iface.startswith(p) for p in _ANNOUNCED_IP_BLOCKED_IFACE):
                continue
            ip = parts[3].split("/")[0]
            if ip.startswith("127."):
                continue
            if ip not in ips:
                ips.append(ip)
    except OSError:
        # ip command unavailable → fall back to hostname -I (coarse filter)
        try:
            out = subprocess.run(
                ["hostname", "-I"], capture_output=True, text=True, timeout=5, check=False
            )
            for ip in out.stdout.split():
                ip = ip.strip()
                if ip and not ip.startswith("127.") and ip not in ips:
                    ips.append(ip)
        except OSError:
            pass
    return ips


def _compose_env(announced_ip: str | None = None) -> dict[str, str]:
    """docker compose invocation env — ensures MEDIASERVO_SFU_ANNOUNCED_IP has a value.
    PIT-79: if the CLI starts the server without injection, mediasoup announces 0.0.0.0 → browser pull fails.
    Explicit --announced-ip (arg/env) wins; otherwise auto-detect all real host IPs (comma-separated, multi-NIC support).
    When given explicitly, skip the "auto-detect" print (T3 minor)."""
    env = {**os.environ}
    if announced_ip:
        env["MEDIASERVO_SFU_ANNOUNCED_IP"] = announced_ip
    elif not env.get("MEDIASERVO_SFU_ANNOUNCED_IP"):
        ips = _detect_announced_ips()
        if ips:
            env["MEDIASERVO_SFU_ANNOUNCED_IP"] = ",".join(ips)
            print(f"MEDIASERVO_SFU_ANNOUNCED_IP auto-detected: {env['MEDIASERVO_SFU_ANNOUNCED_IP']}")
    return env


def _host_runtime_hint(action: str) -> None:
    """host runtime: the CLI wrapper was removed (D266) — build = compile / deploy = install / run = manual."""
    print(f"{action} host: CLI wrapper removed — run manually:", file=sys.stderr)
    print("  deployed instance: systemctl --user start|stop oxmgr-<brand>-host.service", file=sys.stderr)
    print("            or <prefix>/bin/msrtc-host start|stop <prefix>", file=sys.stderr)
    print("  development: target/debug/mediaservo-host init . && token issue --all . && start|stop .", file=sys.stderr)
    print("            clear iceoryx2 residue before running: rm -rf /tmp/iceoryx2 /dev/shm/iox2_* (C25)", file=sys.stderr)

def _server_runtime_hint(action: str) -> None:
    """server runtime: the CLI wrapper was removed (T21 — C39, same treatment as host): build = compile / deploy = install / run = instance-directory commands.
    The bin name is brand-aware (D269 — env MEDIASERVO_BRAND; defaults to the upstream name)."""
    sb = f"{os.environ.get('MEDIASERVO_BRAND', '')}-server" if os.environ.get("MEDIASERVO_BRAND", "") else "mediaservo-server"
    print(f"{action} server: CLI wrapper removed — run via the instance directory:", file=sys.stderr)
    print(f"  deployed instance: <prefix>/bin/{sb} start|stop|restart|status <prefix> (not deployed yet: mediaservo deploy server --prefix <X>; bare-metal entry <prefix>/{sb})", file=sys.stderr)
    print("  development: target/debug/mediaservo-server init . && start . --no-web (frontend dev: dev web = vite:5173 pairs with --no-web; the browser entry itself is cluster-managed)", file=sys.stderr)
    print("  containers (modes 2/3): mediaservo up / down --env dev|prod unchanged", file=sys.stderr)
    print("  read-only probes kept: mediaservo status server | logs server", file=sys.stderr)


def _cmd_start(args: argparse.Namespace) -> None:
    """start <target> — server/host retired → guidance + exit 2 (T21/C39: instance-directory commands; containers use up)."""
    target = args.target
    if target == "server":
        _server_runtime_hint("start")
        sys.exit(2)
    elif target == "host":
        _host_runtime_hint("start")
        sys.exit(2)


def _cmd_restart(args: argparse.Namespace) -> None:
    target = args.target
    """restart <target> — server/host retired → guidance + exit 2 (instance-directory commands; containers up/down)."""
    if target == "server":
        _server_runtime_hint("restart")
        sys.exit(2)
    elif target == "host":
        _host_runtime_hint("restart")
        sys.exit(2)


# ───────── example command surface (p3-gui-viewer W0, G7) ─────────
#
# Directory contract = bindings/cxx/examples/<name>/CMakeLists.txt (dir name == target == artifact name, G4).
# the CLI and the aggregate-root CMake glob by the same literal rule (F-A-6). Artifacts in
# target/examples/bin/ (island-pinned), never in out/, never packaged (example productization gate, PLAN §6).
# F25: examples lost its preset — the island SELF-CHECKS the shell (activated conda/pixi =>
# warn + force-off). The CLI is the sanitized bridge: it hands its cmake children a clean
# system environment (the one legit strip site), so `build example` keeps working from an
# activated shell while manual runs need a plain shell + -DMEDIASERVO_BUILD_EXAMPLES=ON.

_EXAMPLES_DIR = ROOT / "bindings" / "cxx" / "examples"
_EXAMPLES_BUILD = ROOT / "target" / "examples" / "build"
_EXAMPLES_BIN = ROOT / "target" / "examples" / "bin"

_SYSTEM_STRIP = frozenset({
    "CC", "CXX",
    "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "CPATH",
    "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "CONDA_PREFIX",
    "CMAKE_PREFIX_PATH", "PKG_CONFIG_PATH", "PKG_CONFIG_SYSROOT_DIR",
    "COMPILER_PATH", "LIBRARY_PATH",
})


def _system_shell_env() -> dict[str, str]:
    """F25 bridge: env for the examples cmake children = system constitution.
    Drops conda build vars outright and scrubs .pixi/envs bins from PATH (the island's
    own F25 probe reads the same signals — keep both lists aligned if either grows)."""
    env = {k: v for k, v in os.environ.items() if k not in _SYSTEM_STRIP}
    parts = [p for p in env.get("PATH", "").split(os.pathsep) if "/.pixi/envs/" not in p]
    env["PATH"] = os.pathsep.join(["/usr/bin", *parts])
    return env

# compile-related env injected by pixi/conda activation — stripped from configure subprocesses (see the _cmd_build_example note)




def _example_names() -> list[tuple[str, bool]]:
    """[(dir name, is-pure-library)] — same literal rule as the aggregate root's file(GLOB */CMakeLists.txt)."""
    out = []
    for cml in sorted(_EXAMPLES_DIR.glob("*/CMakeLists.txt")):
        name = cml.parent.name
        is_lib = "add_library(" in cml.read_text(encoding="utf-8")
        out.append((name, is_lib))
    return out


def _cmd_list_example() -> None:
    names = _example_names()
    if not names:
        print(f"(no examples in {_EXAMPLES_DIR.relative_to(ROOT)} yet)", file=sys.stderr)
        return
    print("Available examples (dir = target = artifact name; [lib] = non-executable):")
    for name, is_lib in names:
        art = _EXAMPLES_BIN / name
        if is_lib:
            tag = "[lib]"
        else:
            tag = "[built]" if art.exists() else "[not built]"
        print(f"  {name:24s} {tag}")


def _cmd_build_example(names: list[str], release: bool = False) -> None:
    # build-c prelude (PLAN G7; auto-added when missing — freshness is cargo's incremental job).
    profile = "release" if release else "debug"
    if not (ROOT / "target" / profile / "libmediaservo_client.so").exists():
        print("[example] libmediaservo_client.so missing → running build-c first", file=sys.stderr)
        _run_or_exit([os.environ.get("PIXI_BIN", "pixi"), "run", "build-c"])
    # F25 bridge route (see header): direct island configure with sanitized env; SDK guard +
    # env probe live in the island CMakeLists. Release flips via cache, not a preset.
    if names:
        known = {n for n, _ in _example_names()}
        for n in names:
            if n not in known:
                print(f"[example] unknown example '{n}' (see ./mediaservo.sh list example)",
                      file=sys.stderr)
                sys.exit(2)
    gen = "Release" if release else "Debug"
    # no -G: bridge-scrubbed PATH may lack ninja (pixi-only bin); cmake picks its
    # platform default (Makefiles on bare Linux) — same auto-adapt e2e-delivery uses.
    cfg = ["cmake", "-S", str(_EXAMPLES_DIR), "-B", str(_EXAMPLES_BUILD),
           "-DCMAKE_BUILD_TYPE=" + gen,
           "-DMEDIASERVO_BUILD_EXAMPLES=ON",
           "-DMEDIASERVO_SDK_DIR=" + str(ROOT / "target" / profile)]
    if not (_EXAMPLES_BUILD / "CMakeCache.txt").exists() or profile != "debug":
        _run_or_exit(cfg, env=_system_shell_env(), cwd=str(ROOT))
    cmd = ["cmake", "--build", str(_EXAMPLES_BUILD)]
    if names:
        cmd += ["--target"] + list(names)
    else:
        cmd += ["--parallel"]
    _run_or_exit(cmd, env=_system_shell_env(), cwd=str(ROOT))
    print(f"[example] artifacts → {_EXAMPLES_BIN.relative_to(ROOT)}/")


def _cmd_run_example(rest: list[str]) -> None:
    """run example <name> [passthrough…] — build first if needed, then exec in foreground (dev-tool surface,
    distinct from the retired host-run semantics (C39 = deployed instances))."""
    if not rest:
        print("Usage: run example <name> [args…] (./mediaservo.sh list example)",
              file=sys.stderr)
        sys.exit(2)
    name, extra = rest[0], rest[1:]
    known = dict(_example_names())
    if name not in known:
        print(f"[example] unknown example '{name}'", file=sys.stderr)
        sys.exit(2)
    if known[name]:  # pure library, nothing to run
        _cmd_build_example([name])
        print(f"[example] '{name}' is a library (no runnable artifact)", file=sys.stderr)
        sys.exit(1)
    _cmd_build_example([name])
    exe = _EXAMPLES_BIN / name
    if not exe.exists():
        print(f"[example] still no artifact after build: {exe}", file=sys.stderr)
        sys.exit(1)
    # Windowing hint (probe + state, never set for the user — display is session semantics; a script guessing from outside the session = a wrong guess is harder to diagnose than fail-soft;
    # the correct path = ssh -X / a desktop session's own DISPLAY. This block only states what is missing and the exits.)
    if not os.environ.get("DISPLAY") and not os.environ.get("SDL_VIDEODRIVER"):
        print("[example] no display env vars (neither DISPLAY nor SDL_VIDEODRIVER set):", file=sys.stderr)
        xsock = "/tmp/.X11-unix"
        xs = sorted(f":{f[1:]}" for f in os.listdir(xsock) if f.startswith("X")) if os.path.isdir(xsock) else []
        if xs:
            print(f"  • windowed: local displays available: {' '.join(xs)} → export DISPLAY=:N and re-run"
                  " (if it reports x11 not available = this build has SDL_X11=OFF; reconfigure with cmake -DSDL_X11=ON)", file=sys.stderr)
        print("  • headless acceptance: SDL_VIDEODRIVER=dummy MSRTC_RUN_SECS=20 re-run (cb/tex dual-count predicate)", file=sys.stderr)
    env = dict(os.environ, LD_LIBRARY_PATH=str(ROOT / "target" / "debug") +
               (":" + os.environ["LD_LIBRARY_PATH"] if os.environ.get("LD_LIBRARY_PATH") else ""))
    sys.exit(subprocess.run([str(exe), *extra], env=env).returncode)


def _cmd_test_example(name: str | None) -> None:
    """test example [<name>] — ctest (-R filter); a missing build dir = build first."""
    if not (_EXAMPLES_BUILD / "CTestTestfile.cmake").exists():
        _cmd_build_example([])
    cmd = ["ctest", "--test-dir", str(_EXAMPLES_BUILD), "--output-on-failure"]
    if name:
        cmd += ["-R", name]
    _run_or_exit(cmd)


def _cmd_run(args: argparse.Namespace) -> None:
    """run <target> — example = C++ examples runner; server/host retired → guidance + exit 2 (T21/C39)."""
    if args.target == "example":
        _cmd_run_example(args.rest)
    if args.target == "server":
        _server_runtime_hint("run")
        sys.exit(2)
    elif args.target == "host":
        _host_runtime_hint("run")
        sys.exit(2)


def _out_root() -> Path:
    """Release root (MSRTC_OUT_ROOT env or the submodule's out/) — shared by the build delivery layout and the server runtime."""
    env_out = os.environ.get("MSRTC_OUT_ROOT", "")
    return Path(env_out) if env_out else ROOT / "out"


def _native_runtime_dirs() -> tuple[Path, Path, Path]:
    """Native server runtime dirs (the MSRTC release shell injects MSRTC_OUT_ROOT → ${out}/server; bare CLI falls back to the submodule data/).
    Returns (etc_dir, logs_dir, data_dir) — config/pid/log all converge under the release root out/ (main-repo scenario) or data/ (standalone use)."""
    env_out = os.environ.get("MSRTC_OUT_ROOT", "")
    if env_out:
        base = Path(env_out) / "server"
        base.mkdir(parents=True, exist_ok=True)
        (base / "etc").mkdir(parents=True, exist_ok=True)
        (base / "logs").mkdir(parents=True, exist_ok=True)
        return base / "etc", base / "logs", base
    base = ROOT / "data"
    (base / "etc").mkdir(parents=True, exist_ok=True)
    (base / "logs").mkdir(parents=True, exist_ok=True)
    return base / "etc", base / "logs", base



def _port_owner(port: int) -> str:
    """Parse the occupier (name,pid) of the :port line from ss -tulnp. Under non-root, container ports have no users section — reported honestly (review ops#2)."""
    try:
        out = subprocess.run(["ss", "-tulnp"], capture_output=True, text=True, timeout=5, check=False).stdout
        for line in out.splitlines():
            if f":{port} " not in line:
                continue
            m = re.search(r'users:\s*\(\("([^"]+)",pid=(\d+)', line)
            if m:
                name, pid = m.group(1), m.group(2)
                cmd = Path(f"/proc/{pid}/cmdline").read_text().replace("\0", " ").split()[:1]
                return f"{name} (pid {pid})" + (f" — {Path(cmd[0]).name}" if cmd else "")
            return "occupier invisible (host root / container port — check with sudo ss)"
    except OSError:
        pass
    try:  # darwin: lsof fallback
        out = subprocess.run(["lsof", "-i", f":{port}"], capture_output=True, text=True, timeout=5, check=False).stdout
        for line in out.splitlines()[1:]:
            parts = line.split()
            if len(parts) >= 2:
                return f"{parts[0]} (pid {parts[1]})"
    except OSError:
        pass
    return "unknown (neither ss nor lsof available)"


def _check_port_free(port: int, name: str) -> None:
    """Port occupancy check (TCP/UDP; if occupied, advise stopping the conflicting party). The platform guard and the probe command share the same platform branching (Momus MEDIUM: the original picked lsof for the guard but still ss for probing — darwin traceback)."""
    if sys.platform == "darwin":
        _check("lsof", "lsof unavailable")
        out = subprocess.run(["lsof", "-i", f":{port}"], capture_output=True, text=True, timeout=5, check=False).stdout
        busy = f":{port}" in out
    else:
        _check("ss", "ss unavailable — install iproute2")
        out = subprocess.run(["ss", "-tulnp"], capture_output=True, text=True, timeout=5, check=False).stdout
        busy = f":{port} " in out
    if busy:
        print(f"Error: port {name} occupied: {_port_owner(port)} — run first: mediaservo stop server (works for both bare metal and containers)", file=sys.stderr)
        sys.exit(1)


def _check_port_range_free(start: int, end: int, name: str) -> None:
    """RTP range scan (mediasoup binds lazily — health 200 but transports fail intermittently; an explicit check is mandatory). Same platform branching — darwin uses the lsof fallback."""
    if sys.platform == "darwin":
        _check("lsof", "lsof unavailable")
        out = subprocess.run(["lsof", "-i", f":{start}-{end}"], capture_output=True, text=True, timeout=5, check=False).stdout
        busy = [p for p in range(start, end + 1) if f":{p}" in out]
    else:
        _check("ss", "ss unavailable — install iproute2")
        out = subprocess.run(["ss", "-tulnp"], capture_output=True, text=True, timeout=5, check=False).stdout
        busy = [p for p in range(start, end + 1) if f':{p} ' in out]
    if busy:
        print(f'Error: ports {name} occupied at {busy[:5]}... first occupier: {_port_owner(busy[0])} — stop the occupying process first', file=sys.stderr)
        sys.exit(1)


def _cmd_stop(args: argparse.Namespace) -> None:
    """stop <target> — server/host retired → guidance + exit 2 (containers: mediaservo down)."""
    target = args.target
    if target == "server":
        _server_runtime_hint("stop")
        sys.exit(2)
    elif target == "host":
        _host_runtime_hint("stop")
        sys.exit(2)


def _cmd_logs(args: argparse.Namespace) -> None:
    """logs [<target>] [--follow] [--mode native|compose] — server: bare-metal log (default — user ruling B) | compose container log; host: log directory."""
    target = args.target
    if target == "server":
        mode = _resolve_mode(args, default="native")
        if mode == "native":
            # batch-2 rewiring: the fossil server-native.log has no writer since the
            # T21 run-server retirement; the truth lives in the instance's oxmgr run
            # tree (cluster layout: <dir>/run/logs/<app>.{out,err}.log).
            inst = _out_root() / "server"
            logs = sorted((inst / "run" / "logs").glob("*.log"))
            if args.follow:
                if not logs:
                    print(f"logs: no cluster logs under {inst / 'run' / 'logs'} — start the instance first", file=sys.stderr)
                    sys.exit(1)
                _run_or_exit(["tail", "-f"] + [str(f) for f in logs])
                return
            exe = _server_instance_exe()
            if exe is None:
                print("logs: no deployed server instance under <out>/server — deploy first, or --mode compose", file=sys.stderr)
                sys.exit(1)
            _run_or_exit([str(exe), "logs"])
            return
        _check("docker", "install docker and start the daemon")
        _run_or_exit(COMPOSE_BASE + ["logs"] + (["-f"] if args.follow else []) + ["server"])
    elif target == "host":
        log_path = Path("/tmp/mediaservo-host.log")
        if log_path.exists():
            _run_or_exit(["tail", "-n", "100"] + (["-f"] if args.follow else []) + [str(log_path)])
        ox_logs = Path.home() / ".local/share/oxmgr/logs"
        print(f"host log directory: {ox_logs} (instance logs: OXMGR_DATA_DIR/run/logs/msrtc-streamer-*.err.log)", file=sys.stderr)
    # (client has no CLI-side log target: choices are server|host)


def _cmd_e2e(args: argparse.Namespace) -> None:
    if sys.platform == "win32" and args.suite not in ("host", "package", "brand", "bindings", "smoke"):
        print("e2e: Windows supports only bash-script suites", file=sys.stderr)
        sys.exit(1)
    for tool, hint in (
        ("cargo", "pixi environment not activated?"),
        ("docker", "the server container requires docker"),
        ("bash", "e2e scripts require bash"),
    ):
        _check(tool, hint)
    cmd = _E2E_SUITES[args.suite]
    print(f"e2e {args.suite}: {' '.join(cmd)}")
    _run_or_exit(cmd)


def _cmd_test() -> None:
    _check("cargo", "pixi environment not activated?")
    _run_or_exit(["cargo", "test", "--workspace", "--exclude", "mediaservo-server"])



def _rm_path(path: Path) -> None:
    """Delete a file or directory (general clean helper — binary files via unlink, directories via rmtree)."""
    try:
        if path.is_dir():
            shutil.rmtree(path)
        else:
            path.unlink(missing_ok=True)
    except FileNotFoundError:
        pass


def _rm_tree(path: Path) -> None:
    """Cross-platform directory deletion (Windows uses rmdir /s /q semantics, Unix uses rmtree).
    Root files created by containers trigger PermissionError — caught with a manual-delete hint; later cleanup continues."""
    if not path.exists():
        return
    try:
        if sys.platform == "win32":
            _run_or_exit(["rmdir", "/s", "/q", str(path)])
        else:
            shutil.rmtree(path)
        print(f"deleted: {path}")
    except PermissionError:
        print(
            f"Warning: cannot delete {path} (contains container-created root files) — run manually: sudo rm -rf {path}",
            file=sys.stderr,
        )


def _cmd_deploy(args: argparse.Namespace) -> None:
    """deploy <target> — bindings | host | server (source = the out/ delivery tree; --prefix mandatory per D4; deploy does not trigger builds)."""
    if args.target == "bindings":
        _cmd_deploy_bindings(args.prefix, args.release)
    elif args.target == "host":
        _cmd_deploy_host(args.prefix, args.release)
    elif args.target == "server":
        _cmd_deploy_server(args.prefix)



def _cmd_clean(args: argparse.Namespace) -> None:
    """clean <target> — all|server|host|client (default all).
    server: default native (ruling B — clears bare-metal artifacts + stops processes first); --mode both clears both tracks / compose clears containers only; host/client: clears the host cargo target."""
    target = args.target
    if target in ("all", "server"):
        mode = _resolve_mode(args, default="native")
        # native: stop bare-metal processes first (read pid files), then delete artifacts (prevents orphans — clean once left a process running with its pid already deleted)
        if mode in ("both", "native"):
            pid_file = _native_runtime_dirs()[1] / "server-native.pid"
            if pid_file.exists():
                try:
                    pid = int(pid_file.read_text().strip())
                except ValueError:
                    pid = -1
                if pid > 0 and Path(f"/proc/{pid}").exists():
                    print(f"clean server: stopping bare-metal process pid={pid}")
                    subprocess.run(["kill", str(pid)], check=False)
                    for _ in range(4):
                        if not Path(f"/proc/{pid}").exists():
                            break
                        time.sleep(0.5)
                    if Path(f"/proc/{pid}").exists():
                        subprocess.run(["kill", "-9", str(pid)], check=False)
            etc_d, logs_d, _ = _native_runtime_dirs()
            out_server = _out_root() / "server"
            targets = [logs_d / "server-native.pid", logs_d / "server-native.log",
                       out_server / "bin" / _exe_name("mediaservo-server"),
                       # weaknet staging reclaim (weaknet-server-integration [bF10]): out upstream name + both target profiles
                       out_server / "bin" / _exe_name("mediaservo-weaknet"),
                       ROOT / "target/debug/mediaservo-weaknet", ROOT / "target/release/mediaservo-weaknet",
                       out_server / "web",  # frontend delivery tree (T21: build web artifacts reclaimed with clean)
                       ROOT / "target/debug/mediaservo-server", ROOT / "target/release/mediaservo-server"]
            # MAJOR-C: clear both branded names + root-level shortcut links (glob collects only files/links; directories are never touched)
            targets += [f for f in sorted((out_server / "bin").glob("*-server")) if f.is_file()]
            targets += [f for f in sorted((out_server / "bin").glob("*-weaknet")) if f.is_file()]
            targets += [f for f in sorted(out_server.glob("*-server")) if f.is_symlink()]
            _sv = out_server / "server"  # unbranded shortcut link (brand="" layout)
            if _sv.is_symlink():
                targets.append(_sv)
            for f in targets:
                _rm_path(f)
            for d in sorted((_out_root() / "server" / "run").glob("oxmgr*")):
                _rm_path(d)  # daemon data left by out-track drills (C32 — run/ under a prefix belongs to the instance lifecycle, not listed here)
        # compose containers (under both/compose)
        if mode in ("both", "compose"):
            _check("docker", "install docker and start the daemon")
            down = COMPOSE_BASE + ["down"]
            if args.all:
                down.append("-v")  # --all explicitly deletes volumes (cargo-cache) → the next build-server rebuild takes 15-30 minutes
                print("Warning: clean --all deletes the cargo-cache named volume (the next server build recompiles fully, 15-30 minutes)")
            _run_or_exit(down)
    if target in ("all", "host", "client"):
        # project-root target (workspace default, shared by host/client)
        _rm_tree(ROOT / "target")
        # CARGO_TARGET_DIR branch (review: project-root cleanup misses it when the user sets it)
        cargo_target = os.environ.get("CARGO_TARGET_DIR")
        if cargo_target:
            print(f"Note: CARGO_TARGET_DIR={cargo_target} (possibly shared by multiple projects)")
            _rm_tree(Path(cargo_target))
    # --all additionally clears the docker builder cache; never touches .pixi-cache (package cache)
    if args.all and target in ("all", "server"):
        _run_or_exit(["docker", "builder", "prune", "-f"])


def _cmd_config(args: argparse.Namespace) -> None:
    if args.config_cmd == "show":
        for path in (HOST_CONF, SERVER_YAML):
            print(f"--- {path.relative_to(ROOT)} ---")
            if path.exists():
                print(path.read_text(encoding="utf-8"))
            else:
                print(f"(missing: {path})", file=sys.stderr)
        return
    if args.config_cmd == "set":
        if not args.key or args.value is None:
            print("Usage: mediaservo config set <key> <value>", file=sys.stderr)
            sys.exit(2)
        # dev track: writes the host config/ dir (inside prod volumes use the exec path — see docs/modules/26)
        # only the signaling.psk key is supported (other keys pending evolution)
        if args.key != "signaling.psk":
            print(f"config set: currently supports only signaling.psk (got {args.key})", file=sys.stderr)
            sys.exit(2)
        p = ROOT / "config" / "host.conf"
        p.write_text(f"signaling.psk = {args.value}\n", encoding="utf-8")
        print(f"wrote {p.relative_to(ROOT)} (dev track — in prod, use exec to edit the in-volume server.yaml)")
        return
    # validate — real pyyaml parsing (review BLOCKER-2: the dependency was already added to pixi.toml)
    try:
        import yaml  # noqa: PLC0415
    except ImportError:
        print("Error: pyyaml missing — run: pixi install", file=sys.stderr)
        sys.exit(1)
    ok = True
    for path in (HOST_CONF, SERVER_YAML):
        if not path.exists():
            print(f"missing: {path.relative_to(ROOT)}", file=sys.stderr)
            ok = False
            continue
        try:
            yaml.safe_load(path.read_text(encoding="utf-8"))
            print(f"OK: {path.relative_to(ROOT)}")
        except yaml.YAMLError as e:
            print(f"YAML error: {path.relative_to(ROOT)}: {e}", file=sys.stderr)
            ok = False
    sys.exit(0 if ok else 1)


def _cmd_env_diagnose() -> None:
    """Environment diagnosis — check each tool's version, mark missing ones MISSING."""
    pixi_bin = shutil.which("pixi") or str(Path.home() / ".pixi/bin/pixi")
    tools = [
        ("pixi", [pixi_bin, "--version"]),
        ("cargo", ["cargo", "--version"]),
        ("docker", ["docker", "--version"]),
        ("node", ["node", "--version"]),
    ]
    for name, cmd in tools:
        try:
            result = subprocess.run(
                cmd, capture_output=True, text=True, timeout=10, check=False
            )
        except (OSError, subprocess.TimeoutExpired):
            print(f"{name:8s} MISSING (or timeout)")
            continue
        output = (result.stdout or result.stderr).strip().splitlines()
        print(f"{name:8s} {output[0] if output else '?'}")


def _cmd_status_runtime(args: argparse.Namespace) -> None:
    """status <target> — health probe (borrowed from AccessBase cmd_status_native, absorbed per review).
    Exit codes: 0 = running, 1 = not running, 2 = probe failed / bad arg. ps = listing vs status = health verdict (helps cross-reference)."""
    code = 2
    if args.target == "server":
        mode = _resolve_mode(args, default="native")   # default native (flipped per user ruling B)
        code = _status_server_native() if mode == "native" else _status_server_container(args.env)
    elif args.target == "host":
        code = _status_host()
    sys.exit(code)


def _curl_health_code() -> str:
    """curl -o /dev/null -w %{http_code} against 9800 (2s timeout)."""
    try:
        out = subprocess.run(
            ["curl", "--noproxy", "*", "-s", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "2", "http://127.0.0.1:9800/health"],
            capture_output=True, text=True, timeout=5, check=False,
        ).stdout
        return out.strip()
    except OSError:
        return ""


def _ss_listening(port: int) -> bool | None:
    """Whether ss -tulnp contains :port. None = undecidable (C15 — review arch F4 false-negative fix)."""
    try:
        out = subprocess.run(["ss", "-tulnp"], capture_output=True, text=True, timeout=5, check=False).stdout
        return f":{port} " in out
    except OSError:
        return None


def _server_instance_exe() -> "Path | None":
    """Deployed server instance binary under <out>/server/bin (brand-aware glob —
    D269 named the file, the instance dir stays the locator). None = not deployed."""
    inst = _out_root() / "server"
    cands = [c for c in sorted((inst / "bin").glob("*-server"))
             if c.is_file() and not c.name.endswith(".exe.bak")]
    return cands[0] if cands else None


def _status_server_native() -> int:
    """Server status = the managed binary's own registry view (SR6 batch-2).
    The old pid-file probe died with the T21 run-server retirement — nobody
    writes server-native.pid anymore, so under oxmgr management the instance
    command IS the truth (F31 doctrine: ops lives in the binary, the CLI is a
    forwarder). Exit codes 0/1/2 propagate from the binary."""
    exe = _server_instance_exe()
    inst = _out_root() / "server"
    if exe is None:
        print("server: no deployed instance under <out>/server — deploy first "
              "(mediaservo deploy server --prefix <X>), or --mode compose for containers", file=sys.stderr)
        return 1
    return subprocess.run([str(exe), "status", str(inst)]).returncode


def _compose_running(env: str) -> bool:
    """Whether env has containers running (dev/prod share container names, and compose ps aggregates by directory regardless of env —
    distinguished by the config_files label, same source as _detect_running_env)."""
    try:
        out = subprocess.run(["docker", "ps", "--format", "{{.Names}}"],
                             capture_output=True, text=True, timeout=5, check=False).stdout
        if "mediaservo-server-1" not in out:
            return False
        return _detect_running_env() == env
    except OSError:
        return False


def _status_server_container(env_arg: str | None = None) -> int:
    """Container runtime state. env_arg reuses --env (review cli F1: no help-vs-implementation drift allowed)."""
    code = _curl_health_code()
    env_now = env_arg or _detect_running_env()
    print(f"server (compose env={env_now}):", flush=True)  # flush: _run subprocess output lands before print (pipe buffering)
    _check("docker", "install docker and start the daemon")
    _run(["docker", "compose", "-f", _compose_file(env_now), "ps"])  # process/container listing (a subset of ps — review arch F1)
    print(f"  health 9800: {'200 OK' if code == '200' else code or 'no response'}")
    av = _compose_env().get("MEDIASERVO_SFU_ANNOUNCED_IP", "")
    if av:
        print(f"  announced: {av}")   # C38 layer-1 observation
    return 0 if _compose_running(env_now) else 1


def _status_host() -> int:
    """Host runtime state. Process names have dual prefixes (review ops#1 CRITICAL: branded msrtc-* vs official host-*)."""
    print("host:")
    procs = ["agent", "streamer", "capturer"]  # branded roles (dual prefixes — ops#1 CRITICAL)
    brands = ["msrtc", "host"]
    bare_procs = ["oxmgr"]  # Momus HIGH: the oxmgr daemon's process name is bare `oxmgr` (npm binary bin/oxmgr;
    # the brand prefix applies only to translate-generated <brand>-<app> — only agent/streamer/capturer carry prefixes)
    any_alive = False
    probe_failed = False
    for role in procs + bare_procs:
        pids: list[str] = []
        cands = [f"{b}-{role}" for b in brands] if role not in bare_procs else [role]
        for cname in cands:
            out = subprocess.run(["pgrep", "-x", cname], capture_output=True, text=True, check=False).stdout.strip()
            if out:
                pids = out.splitlines()
                break
        if pids:
            any_alive = True
            print(f"  {role}: running pid={' '.join(pids)}")
        else:
            print(f"  {role}: not running")
    st = _ss_listening(17980)
    if st is None:
        probe_failed = True
    print(f"  gateway 17980: {'listening' if st else ('undecidable' if st is None else 'free')}")
    # instance log path (review ops#5/docs#3: C32 instance isolation — no hardcoded placeholders)
    inst = os.environ.get("OXMGR_DATA_DIR", "")
    log_hint = (Path(inst).parent / "run" / "logs") if inst else (ROOT.parent / "out" / "host" / "run" / "logs")
    print(f"  logs: {log_hint}/msrtc-streamer-<stream>.err.log (C38 layer-2: grep subscription/acl denied/OpenH264)")
    if not any_alive:
        print("  → host not running — manual: target/debug/mediaservo-host init . && token issue --all . && start . (development) or systemctl --user start oxmgr-<brand>-host.service (deployed)", file=sys.stderr)
    return 2 if probe_failed else (0 if any_alive else 1)


def _add_mode_args(p, *, env_choices=("dev", "prod"), allow_both=False):
    """Unified mode parameters (--mode native|compose[/both] + short aliases --native/--env; mutually exclusive).
    Semantics: commands default to native (user ruling B) — compose requires an explicit --mode compose / --env;
    both is enabled only for stop/clean (stops/clears both tracks)."""
    choices = ["native", "compose"] + (["both"] if allow_both else [])
    grp = p.add_mutually_exclusive_group()
    grp.add_argument("--mode", choices=choices, default=None,
                     help="run mode: native = bare metal (default) | compose = containers (--env equivalent)"
                     + (" | both = handle both tracks" if allow_both else ""))
    grp.add_argument("--native", action="store_true", help="(short alias for --mode native — default mode, omittable)")
    grp.add_argument("--env", choices=list(env_choices), default=None,
                     help="(short alias for --mode compose = process-family container mode; --env in the compose family up/down/ps selects the environment — two distinct semantics)")
    p.set_defaults(_resolve_mode_into=None)


def _resolve_mode(args, default: str = "native") -> str:
    """Resolve args → mode uniformly (--mode > --native/--env > default)."""
    if getattr(args, "mode", None):
        return args.mode
    if getattr(args, "native", False):
        return "native"
    if getattr(args, "env", None) is not None:
        return "compose"
    return default


def _compose_file(env: str) -> str:
    """Environment → compose file mapping (shared by up/down/ps/exec)."""
    return "docker-compose.yml" if env == "prod" else "docker-compose.dev.yml"



def _detect_running_env() -> str:
    """Detect which compose project the running server container belongs to (dev/prod — same container name,
    distinguished by the compose config_files label; no container = default dev)."""
    try:
        out = subprocess.run(
            ["docker", "ps", "--format", "{{.Names}}"],
            capture_output=True, text=True, timeout=5, check=False,
        ).stdout
        if "mediaservo-server-1" in out:
            cfg = subprocess.run(
                ["docker", "inspect", "mediaservo-server-1", "--format",
                 '{{index .Config.Labels "com.docker.compose.project.config_files"}}'],
                capture_output=True, text=True, timeout=5, check=False,
            ).stdout.strip()
            if "docker-compose.yml" in cfg and "dev" not in cfg:
                return "prod"
        return "dev"
    except (OSError, subprocess.TimeoutExpired):
        return "dev"






def _cmd_doctor(_args: argparse.Namespace | None = None) -> None:
    """Environment diagnosis (toolchain versions — renamed from the original status to distinguish it from the runtime health probe status)."""
    _cmd_env_diagnose()



def main() -> None:
    parser = argparse.ArgumentParser(
        prog="mediaservo",
        description="MediaServo unified build CLI (terms: native = bare metal, compose = containers)",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""Three-mode quick reference (native = bare metal, compose = containers):
  Mode 1 bare-metal delivery: build server (assembles bin/etc/web in out/) → deploy server --prefix <X> (bin/etc layout = cmake install rules) →
             <X>/<brand>-server init/start/stop/status/restart/doctor <X> (startup on = boot anchor; bin physical name branded per D269)
    fast lanes: build web (frontend only → out/server/web) | dev web (vite:5173 hot reload, pair with start --no-web)
  Mode 2 single-container production: build server --image runtime → up --env prod → logs server --env prod
  Mode 3 compose development: up --env dev (hot reload) → logs -f → down --env dev
  bash-owned (SR6 batch-1, flags unchanged, entry: mediaservo.sh): up down exec ps data dev version
  retired → guidance + exit 2: run/start/stop/restart server|host (C39 — use instance-directory commands; the container face up/down is unchanged)
  read-only probes kept: status server|host / logs server / clean server
  C++ example surface (p3): list example → build example [names…] → run example <name> / test example
             (foreground dev tool; artifacts in target/examples/bin/ — never in out/; the build-c prelude is auto-added)
  exit codes: status/logs — 0 = success, 1 = not running/target missing, 2 = bad args / probe undecidable; run/start/stop/restart — retired arms land at 2 with the instance-command guidance
""")

    sub = parser.add_subparsers(dest="command", required=True)

    build_p = sub.add_parser("build", help="build <target> [--image runtime|dev]: all|web|host|server|client|bindings (default all; server = non-embedded variant + one-step web assembly; web = frontend-only fast lane; only --image takes the Docker path)")
    build_p.add_argument("target", nargs="?", choices=["all", "web", "host", "server", "client", "bindings", "example"], default="all")
    build_p.add_argument("--release", action="store_true", help="release build (bindings: target/release, strip+LTO)")
    grp = build_p.add_mutually_exclusive_group()
    grp.add_argument("--image", choices=["runtime", "dev"], default=None,
                     help="build server only: Docker image target (runtime = production slim delivery image; dev = toolchain image)")
    grp.add_argument("--native", action="store_true",
                     help="build server only: native compile (pixi toolchain — first run needs network for meson wraps; multi-IP announcement takes effect at the run stage)")
    build_p.add_argument("names", nargs="*", metavar="NAME",
                        help="build example only: example names (omit = all; list example shows the roster)")
    build_p.set_defaults(func=_cmd_build)



    restart_p = sub.add_parser("restart", help="restart <target>: server/host retired → guidance + exit 2 (<prefix>/bin/mediaservo-server restart <dir>; containers up/down)")
    restart_p.add_argument("target", choices=["host", "server"])
    restart_p.set_defaults(func=_cmd_restart)
    run_p = sub.add_parser("run", help="run <target>: server/host retired → guidance + exit 2 (instance commands: <prefix>/bin/mediaservo-server -h); example = C++ examples")
    run_p.add_argument("target", choices=["server", "host", "example"])
    run_p.add_argument("rest", nargs=argparse.REMAINDER,
                       help="run example only: <name> [app passthrough args…]")
    run_p.set_defaults(func=_cmd_run)

    start_p = sub.add_parser("start", help="start <target> (default server): server/host retired → guidance + exit 2 (containers use up; instance commands for the rest)")
    start_p.add_argument("target", nargs="?", choices=["host", "server"], default="server")
    start_p.set_defaults(func=_cmd_start)

    stop_p = sub.add_parser("stop", help="stop <target>: server/host retired → guidance + exit 2 (instance command stop <dir>; containers: mediaservo down)")
    stop_p.add_argument("target", choices=["host", "server"])
    stop_p.set_defaults(func=_cmd_stop)


    logs_p = sub.add_parser("logs", help="logs [<svc>] [--follow] [--mode native|compose]: server = bare-metal log (default) | compose container log | host log")
    logs_p.add_argument("target", nargs="?", choices=["server", "host"], default="server")
    _add_mode_args(logs_p)
    logs_p.add_argument("--follow", "-f", action="store_true", help="follow output")
    logs_p.set_defaults(func=_cmd_logs)



    e2e_p = sub.add_parser("e2e", help="test suites: sfu|push|ui|host|package|brand|bindings|client|smoke")
    e2e_p.add_argument("suite", choices=["sfu", "push", "ui", "host", "package", "brand", "bindings", "client", "smoke"])
    e2e_p.set_defaults(func=_cmd_e2e)

    list_p = sub.add_parser("list", help="listing <target>: example = C++ examples (directory contract = G4 three-forms-in-one)")
    list_p.add_argument("target", choices=["example"])
    list_p.set_defaults(func=lambda a: _cmd_list_example())
    test_p = sub.add_parser("test", help="workspace tests (excluding mediaservo-server) | example=<name> ctest")
    test_p.add_argument("what", nargs="?", choices=["example"], default=None,
                        help="example = GUI/SDK example ctest surface (bare test = unchanged workspace semantics, F-A-9(1))")
    test_p.add_argument("name", nargs="?", help="example: filter by example name (ctest -R)")

    deploy_p = sub.add_parser("deploy", help="deploy <target> (stateful landing — deploy does not trigger builds; app bin/etc layout rides cmake install rules (cmake/MediaServoAppTree.cmake, cmake must be on PATH), stateful steps run in the managed binary): host|server|bindings; host/server --prefix mandatory (/opt needs root); bindings defaults to <out>/bindings")
    deploy_p.add_argument("target", choices=["bindings", "host", "server"])
    deploy_p.add_argument("--prefix", default=None, help="deployment prefix (mandatory for host/server: /opt/mediaservo-{host,server}; bindings defaults to <out>/bindings, out root follows the calling shell)")
    deploy_p.add_argument("--release", action="store_true", help="deploy release-profile assembled artifacts")
    deploy_p.set_defaults(func=_cmd_deploy)
    package_p = sub.add_parser("package", help="package <target>: host|server|sdk-field (device half; the old bindings name stays as a one-cycle alias)|sdk-client (cockpit half) → <dist>/<brand>-<target>-<ver>.tar.gz (D-H13/F12, includes the version contract and a versioned top-level dir)")
    package_p.add_argument("target", choices=["host", "server", "bindings", "sdk-field", "sdk-client"])
    package_p.add_argument("--dist", default="", help="output dir for the package tar and staging (default dist; the MSRTC release shell defaults to out/packages)")
    package_p.add_argument("--brand", default="", help="brand package name (<dist>/<brand>-<target>-<ver>.tar.gz; default mediaservo)")
    package_p.add_argument("--release", action="store_true", help="package release artifacts (target/release, pairs with build --release)")
    package_p.set_defaults(func=_cmd_package)
    clean_p = sub.add_parser("clean", help="clean <target>: all|server|host|client (default all; server = native clean (default) | --mode both clears both)")
    clean_p.add_argument("target", nargs="?", choices=["all", "server", "host", "client"], default="all")
    _add_mode_args(clean_p, allow_both=True)
    clean_p.add_argument("--all", action="store_true", help="explicitly delete volumes + docker builder prune (15-30 minute rebuild cost)")
    clean_p.set_defaults(func=_cmd_clean)
    config_p = sub.add_parser("config", help="config show|validate|set <key> <value> (dev-track config/ directory)")
    config_p.add_argument("config_cmd", choices=["show", "validate", "set"])
    config_p.add_argument("key", nargs="?", help="set: config key (e.g. signaling.psk)")
    config_p.add_argument("value", nargs="?", help="set: config value")
    config_p.set_defaults(func=_cmd_config)
    doctor_p = sub.add_parser("doctor", help="environment diagnosis (pixi/cargo/docker/node — the original status)")
    doctor_p.set_defaults(func=_cmd_doctor)
    status_p = sub.add_parser("status", help="health probe <target> (exit codes 0/1/2): server = bare metal (default) | compose containers (--mode compose) | host push processes")
    status_p.add_argument("target", choices=["server", "host"])
    _add_mode_args(status_p)
    status_p.set_defaults(func=_cmd_status_runtime)


    # compat aliases (build-* kept — e2e scripts/habitual usage): run-host/up/down aliases removed (up/down are now first-level commands)
    ALIASES = {
        "build-host": ["build", "host"],
        "build-server": ["build", "server"],
    }
    argv = sys.argv[1:]
    if argv and argv[0] in ALIASES:
        argv = ALIASES[argv[0]] + argv[1:]
    args = parser.parse_args(argv)
    if args.command == "build":
        if getattr(args, "image", None) and args.target != "server":
            print(f"build --image only supports build server (current target={args.target})", file=sys.stderr)
            sys.exit(2)
        if getattr(args, "native", False) and args.target != "server":
            print(f"build --native only supports build server (current target={args.target})", file=sys.stderr)
            sys.exit(2)
        if args.target == "bindings":
            _cmd_build_bindings(args.release)
        elif args.target == "all":
            _cmd_build_host()
            _cmd_build_server()
            _cmd_build_client()
        elif args.target == "host":
            _cmd_build_host()
        elif args.target == "server":
            _cmd_build_server(args.image, args.native, args.release)
        elif args.target == "client":
            _cmd_build_client()
        elif args.target == "web":
            _cmd_build_web()
        elif args.target == "example":
            _cmd_build_example(args.names, args.release)
    elif args.command == "run":
        _cmd_run(args)
    elif args.command == "start":
        _cmd_start(args)
    elif args.command in ("stop", "restart", "logs"):
        globals()[f"_cmd_{args.command}"](args)
    elif args.command == "status":
        _cmd_status_runtime(args)

    elif args.command == "test":
        if args.what == "example":
            _cmd_test_example(args.name)
        else:
            _cmd_test()
    elif hasattr(args, "func"):
        args.func(args)


if __name__ == "__main__":
    main()
