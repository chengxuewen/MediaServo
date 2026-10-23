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
    build_p.add_argument("target", nargs="?", choices=["bindings"], default="all")
    build_p.add_argument("--release", action="store_true", help="release build (bindings: target/release, strip+LTO)")
    grp = build_p.add_mutually_exclusive_group()
    grp.add_argument("--image", choices=["runtime", "dev"], default=None,
                     help="build server only: Docker image target (runtime = production slim delivery image; dev = toolchain image)")
    grp.add_argument("--native", action="store_true",
                     help="build server only: native compile (pixi toolchain — first run needs network for meson wraps; multi-IP announcement takes effect at the run stage)")
    build_p.add_argument("names", nargs="*", metavar="NAME",
                        help="build example only: example names (omit = all; list example shows the roster)")
    # no func= auto-dispatch (SR6 batch-beta-2: build handled by the explicit
    # `if args.command == "build"` arm below; _cmd_build was retired to bash).



    package_p = sub.add_parser("package", help="package <target>: host|server|sdk-field (device half; the old bindings name stays as a one-cycle alias)|sdk-client (cockpit half) → <dist>/<brand>-<target>-<ver>.tar.gz (D-H13/F12, includes the version contract and a versioned top-level dir)")
    package_p.add_argument("target", choices=["host", "server", "bindings", "sdk-field", "sdk-client"])
    package_p.add_argument("--dist", default="", help="output dir for the package tar and staging (default dist; the MSRTC release shell defaults to out/packages)")
    package_p.add_argument("--brand", default="", help="brand package name (<dist>/<brand>-<target>-<ver>.tar.gz; default mediaservo)")
    package_p.add_argument("--release", action="store_true", help="package release artifacts (target/release, pairs with build --release)")
    package_p.set_defaults(func=_cmd_package)
    args = parser.parse_args(sys.argv[1:])
    if args.command == "build":   # SR6 batch-beta-2: other targets are bash-owned
        if args.target == "bindings":
            _cmd_build_bindings(args.release)
    elif hasattr(args, "func"):
        args.func(args)


if __name__ == "__main__":
    main()
