"""mediaservo — unified MediaServo build CLI (vapkg-style single entry point).

The thin shell (mediaservo.sh/.bat) guarantees this script runs after the pixi environment is activated:
PATH/LIBCLANG_PATH are already injected in the environment; subprocess calls cargo/docker directly.
Platform differences exist in only two places: e2e (bash scripts) and clean (delete commands).

Usage: mediaservo [-h] {build,run example,test example,package} ... — the ops verbs
(up down logs status clean config deploy e2e test-doctor-list …) live in
mediaservo.sh (bash-owned, SR6); the -h epilog lists the split.
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


def _cmd_build_web() -> None:
    """build web — frontend-only fast lane: incremental pnpm by mtime → out/server/web
    (does not touch cargo). SR6 batch-5: the placement rides the app-tree cmake face
    (staging=web) — the retired _stage_web_to_out's rmtree+copytree is its REMOVE-leg +
    install(DIRECTORY) pair; the vite INVOCATION stays here (language-native builder, F30)."""
    _ensure_admin_dist()
    _cmake_app_install(_out_root() / "server", "server", "", False, staging="web")






def _build_stage_weaknet(release: bool) -> None:
    """build server step 1 (ordering contract [bF4 bF5 BA-15]): cargo-compile weaknet into
    target/{profile}. SR6 batch-5: the branded placement into out/server/bin/{brand}-weaknet
    rides the app-tree cmake row — staging=build hunts the FRESH target binary first and an
    absent artifact is the same WARN-skip polarity this function carried (compile WARN here,
    placement WARN in the row). Profile follows --release propagation. Failure only WARNs and
    does not block the server main flow (weaknet-server-integration design §D1 [bF4 bF5] —
    running it last would take the mediasoup hung-build tree's exit guilt-by-association;
    on this machine recorded as [BA-15 tree-hang endorsed by CI])."""
    cmd = ["cargo", "build"] + (["--release"] if release else []) + ["-p", "mediaservo-weaknet"]
    try:
        code = _run(cmd)
    except OSError as e:  # cargo missing/cannot start (degraded path — the server main path has its own _check error exit)
        print(f"WARN: weaknet staging skipped (cargo unavailable: {e}) — not blocking the server build", file=sys.stderr)
        return
    if code != 0:
        print(f"WARN: weaknet compile failed (rc={code}) — not blocking the server build; "
              "the app-tree weaknet row WARNs if nothing stageable is found (re-run build server after troubleshooting)", file=sys.stderr)


def _cmd_build_server(image: str | None = None, native: bool = False, release: bool = False) -> None:
    """build server: default native (user ruling B — no mode written = native) | --image runtime|dev = Docker image (--native kept as compat alias)."""
    # weaknet compile comes first (design §D1 [bF4 bF5 BA-15]): it must precede the server cargo build — mediasoup
    # hangs in that posture don't guilt-by-association it; and its own failure only WARNs, never blocking server (separate try).
    _build_stage_weaknet(release)
    if native or image is None:   # default native; Docker only when --image is explicit
        _ensure_admin_dist()      # incremental frontend build by mtime (the Docker path handles it in the Dockerfile)
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
        # SR6 batch-5: the assembly (bin + weaknet + web tree + brand reclaim) rides the
        # SR7 app-tree face EXACTLY like deploy does — _cmake_app_install with staging=build:
        # swap-install rows for the FRESH target/{profile} binaries (the atomic .new/
        # os.replace semantics of the retired copy loop — build must never truncate a live
        # cluster's running bin, deploy may only because it stops first), the whitelist
        # reclaim leg replaces the legacy-name unlinks, and the web move rides the same
        # pass (retired _stage_web_to_out). No root entry shortcuts: a build-staged
        # delivery tree stays bin/etc/web (the deploy landing adds the links).
        # PIT-191 single-point branding stays CLI-resolved (env → in-tree derivation →
        # upstream) and rides -DMEDIASERVO_BRAND (resolution order shared with deploy).
        bin_dir = _out_root() / "server" / "bin"
        bin_dir.mkdir(parents=True, exist_ok=True)
        brand = os.environ.get("MEDIASERVO_BRAND", "").strip() or _derive_brand_server(bin_dir)
        _cmake_app_install(_out_root() / "server", "server", brand, release, staging="build")
        print(f"server assembly staged: out/server/ (bin/etc/web — app-tree rules, brand {brand or 'mediaservo'})")
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
    # node bindings (napi-rs .node; FFmpeg dynamic-lib linking completed via build.rs).
    # SR6 batch-5: the staging .node copy died with the python placement loop — the
    # node/mediaservo rows read target/{profile} directly (cmake/MediaServoBindings.cmake).
    _run_or_exit(["cargo", "build"] + (["--release"] if release else []) + ["-p", "mediaservo-node"])
    # stage the delivery layout out/bindings (full SDK package mirror — completed per Momus HIGH3/Task 2.5):
    # D241 version triple + N7 strip = cmake install rules (SR4 L1); D248 headers (C/cxx)
    # = cmake install rules (SR4 L2); pkgconfig .pc + cmake config transport moves at
    # SR4 L3; python WHEEL CONTENTS stay pip-side (ruled-away territory), while the
    # node package + docs volumes ride the same cmake rows (SR6 batch-5).
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
    # SRC reset is EXPLICIT: deploy may have left MEDIASERVO_BINDINGS_SRC in the cache;
    # the build assembly must configure the ROWS mode, never inherit the mirror mode.
    _run_or_exit(["cmake", "--preset", "bindings", f"-DCMAKE_BUILD_TYPE={_btype}",
                  "-DMEDIASERVO_BINDINGS_SRC="], cwd=str(ROOT))
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
        wheel_dir = bind_dst / "wheel"  # pip -w creates the dir (batch-5: the stale-whl sweep
        # — the 09-17 unordered-glob ENOENT guard — rode to the cmake bindings-face legs).
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
        # pip --prefix does not delete old data_files (V1b client.so corpse + 10-22
        # dist-info corpse catches): the pre-install sweep of the site-packages
        # mediaservo/ + mediaservo-*.dist-info trees now runs as a cmake bindings-face
        # leg in the --install pass that precedes this invocation (SR6 batch-5).
        r2 = subprocess.run([sys.executable, "-m", "pip", "install", "--prefix",
                             str(bind_dst), "--no-deps", str(fixed)],
                            capture_output=True, text=True)
        if r2.returncode != 0:
            print(f"Error: pip install failed — {r2.stderr[-500:]}", file=sys.stderr)
            sys.exit(1)
    finally:
        shutil.rmtree(libs_src, ignore_errors=True)  # clean up temp _libs (gitignore as backstop)

    # SR6 batch-5: the node package (D248 layout) and the docs/sdk-cxx volumes (the
    # remove-then-copy residue discipline) are cmake install rows on the bindings face
    # — the python placement loops die here (one leg, one owner). The summary count
    # below reads the LANDED tree. batch-5 ruled-away (remaining python): the py_src
    # staging mirror + _libs feeds + tag rewrite above = wheel CONTENTS, pip territory
    # (SR4 §1.3 verdict stands; only their stale DELIVERY sweeps moved).
    n_docs = len(list((bind_dst / "docs" / "sdk-cxx").glob("*.md")))
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


def _strip_so(path: Path) -> None:
    """strip --strip-unneeded on delivery-tree .so real files (N7 09-18 measurement: the debug cdylib was the entire
    cause of the fat wheel's 528MB — not FFmpeg — uncompressed 27M→~1M scale). Dynamic symbols (.dynsym / exported ABI) untouched,
    check-abi-drift and consumer ldd are unaware; only debug sections are removed. The target/ dev tree is untouched (gdb needs stacks)."""
    if not shutil.which("strip"):
        return
    if subprocess.run(["strip", "--strip-unneeded", str(path)],
                      capture_output=True, check=False).returncode != 0:
        print(f"  warning: strip {path.name} failed (skipped — delivery size not optimized)")


def _derive_brand_server(bin_dir: Path) -> str:
    """server tree brand derivation (branding-completion BLOCKER-2): glob bin/*-server excluding the upstream name.
    Do not reuse the retired _derive_brand (it only recognized *-agent; the server tree would always return ""). Multiple brands coexisting: take the first (same known limit as the host arm in mediaservo.sh)."""
    for f in sorted(bin_dir.glob("*-server")):
        if f.name == _exe_name("mediaservo-server"):
            continue
        return f.name[: -len("-server")]
    return ""

# SR7 T4: the Python-side stale-oxfile/Caddyfile migration guard (_drop_stale_server_oxfile)
# + its env re-absorption twin (_reapply_carried_env) are retired -- the bF3 port (5255a13)
# runs migrate_stale_files as init step 0 (BEFORE both exists-skips) with the same
# predicates, .bak rotation and merge_manual_env re-absorption (baked values win, [lck-F4]
# half-migration guard preserved by construction: one rotation, one merge, inside init).
# WEAKNET_OXFILE_APP's duty lives on as the Rust constant templates::WEAKNET_APP [mn10].


def _cmake_app_install(prefix_p: Path, tree: str, brand: str, release: bool,
                       staging: str = "") -> None:
    """SR7: run the app delivery-layout install rules (cmake/MediaServoAppTree.cmake)
    into prefix_p. Configure is RE-RUN every invocation — the source-existence hunts
    (server/weaknet dual source, host per-name skip-missing) are configure-time truth
    and must never be frozen by a stale cache (PIT-189 family: never trust yesterday's
    tree). Brand and the delivery root ride cache variables (§1 rule 3: the CLI keeps
    invoke+brand, cmake describes layout).
    staging (SR6 batch-5, server tree): '' = deploy landing (delivery-tree-first
    PROGRAMS rows + root entry shortcuts); build = build-server assembly (fresh
    target/{profile} swap rows + web move, no entry links); web = build-web fast
    lane placement only. Passed explicitly EVERY invocation so a stale cache value
    can never flip a verb's mode."""
    # Dedicated binaryDir (preset default's recipe, -B override): the app cache
    # vars must NEVER land in build/presets/default — a later plain IDE/gates
    # configure of that tree would inherit a stale MEDIASERVO_APP_TREE from the
    # cache and FATAL on a missing delivery tree (fresh-clone IDE regression).
    app_tree = ROOT / "build" / "app-tree"
    cfg = ["cmake", "--preset", "default", "-B", str(app_tree),
           f"-DMEDIASERVO_APP_TREE={tree}",
           f"-DMEDIASERVO_BRAND={brand}",
           f"-DMEDIASERVO_APP_STAGING={staging}",
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


def _platform_tag() -> str:
    """wheel platform tag (Linux x86_64 → linux_x86_64; other platforms best-effort)."""
    import platform
    mach = platform.machine().lower().replace("-", "_")
    return f"{platform.system().lower()}_{mach}"


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
    """run example <name> [passthrough…] — C++ examples runner (server/host guidance arms are shell-owned since SR6 P2)."""
    _cmd_run_example(args.rest)


def _out_root() -> Path:
    """Release root (MSRTC_OUT_ROOT env or the submodule's out/) — shared by the build delivery layout and the server runtime."""
    env_out = os.environ.get("MSRTC_OUT_ROOT", "")
    return Path(env_out) if env_out else ROOT / "out"


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
  bash-owned (SR6 batch-1/alpha/beta-1, flags unchanged, entry: mediaservo.sh): up down exec ps data dev version doctor list start stop restart run(server|host guidance) logs status clean config e2e test(bare) deploy(bindings|host|server)
  guidance arms exit 2 (C39 — instance-directory commands; the container face up/down is unchanged)
  C++ example surface (p3): list example → build example [names…] → run example <name> / test example
             (foreground dev tool; artifacts in target/examples/bin/ — never in out/; the build-c prelude is auto-added)
  exit codes (shell-owned status/logs): 0 = success, 1 = not running/target missing, 2 = bad args / probe undecidable
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



    run_p = sub.add_parser("run", help="run example <name> [args…] — C++ examples runner (server|host guidance is shell-owned)")
    run_p.add_argument("target", choices=["example"])
    run_p.add_argument("rest", nargs=argparse.REMAINDER,
                       help="run example only: <name> [app passthrough args…]")
    run_p.set_defaults(func=_cmd_run)


    test_p = sub.add_parser("test", help="example=<name> ctest surface (bare workspace tests moved to mediaservo.sh test — SR6 batch-beta-1)")
    test_p.add_argument("what", choices=["example"], help="example = GUI/SDK example ctest surface (bare test = unchanged workspace semantics, F-A-9(1))")
    test_p.add_argument("name", nargs="?", help="example: filter by example name (ctest -R)")

    package_p = sub.add_parser("package", help="package <target>: host|server|sdk-field (device half; the old bindings name stays as a one-cycle alias)|sdk-client (cockpit half) → <dist>/<brand>-<target>-<ver>.tar.gz (D-H13/F12, includes the version contract and a versioned top-level dir)")
    package_p.add_argument("target", choices=["host", "server", "bindings", "sdk-field", "sdk-client"])
    package_p.add_argument("--dist", default="", help="output dir for the package tar and staging (default dist; the MSRTC release shell defaults to out/packages)")
    package_p.add_argument("--brand", default="", help="brand package name (<dist>/<brand>-<target>-<ver>.tar.gz; default mediaservo)")
    package_p.add_argument("--release", action="store_true", help="package release artifacts (target/release, pairs with build --release)")
    package_p.set_defaults(func=_cmd_package)
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

    elif args.command == "test":
        _cmd_test_example(args.name)
    elif hasattr(args, "func"):
        args.func(args)


if __name__ == "__main__":
    main()
