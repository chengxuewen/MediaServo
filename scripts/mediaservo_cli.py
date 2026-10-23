"""mediaservo — unified MediaServo build CLI (vapkg-style single entry point).

The thin shell (mediaservo.sh/.bat) guarantees this script runs after the pixi environment is activated:
PATH/LIBCLANG_PATH are already injected in the environment; subprocess calls cargo/docker directly.
Platform differences exist in only two places: e2e (bash scripts) and clean (delete commands).

Usage: mediaservo [-h] package ... — everything else is bash-owned
(mediaservo.sh, SR6 batch-beta-2); the -h epilog lists the split.
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


    package_p = sub.add_parser("package", help="package <target>: host|server|sdk-field (device half; the old bindings name stays as a one-cycle alias)|sdk-client (cockpit half) → <dist>/<brand>-<target>-<ver>.tar.gz (D-H13/F12, includes the version contract and a versioned top-level dir)")
    package_p.add_argument("target", choices=["host", "server", "bindings", "sdk-field", "sdk-client"])
    package_p.add_argument("--dist", default="", help="output dir for the package tar and staging (default dist; the MSRTC release shell defaults to out/packages)")
    package_p.add_argument("--brand", default="", help="brand package name (<dist>/<brand>-<target>-<ver>.tar.gz; default mediaservo)")
    package_p.add_argument("--release", action="store_true", help="package release artifacts (target/release, pairs with build --release)")
    package_p.set_defaults(func=_cmd_package)
    args = parser.parse_args(sys.argv[1:])
    if hasattr(args, "func"):
        args.func(args)


if __name__ == "__main__":
    main()
