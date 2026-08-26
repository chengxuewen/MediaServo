#!/usr/bin/env python3
"""mediaservo — MediaServo 统一构建 CLI（vapkg 式单入口）。

薄壳（mediaservo.sh/.bat）保证 pixi 环境激活后调用本脚本：
环境内 PATH/LIBCLANG_PATH 已注入，subprocess 直接调 cargo/docker 等。
平台差异仅 e2e（bash 脚本）与 clean（删除命令）两处。

用法: mediaservo [-h] {build,build-host,build-server,up,down,logs,e2e,test,ci,install,package,clean,config,status,version} ...
"""

from __future__ import annotations

import argparse
import os
import shutil
import time
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

VERSION = "0.1.0"
ROOT = Path(__file__).resolve().parent.parent
COMPOSE_BASE = ["docker", "compose", "-f", "docker-compose.dev.yml"]
HOST_CONF = ROOT / "crates/mediaservo-host/config/host.conf"
SERVER_YAML = ROOT / "config/server.docker.yaml"


def _check(tool: str, hint: str) -> None:
    """依赖检查 — 缺失时明确报错退出（不静默）。"""
    if shutil.which(tool) is None:
        print(f"错误: 缺少依赖 '{tool}' — {hint}", file=sys.stderr)
        sys.exit(1)


def _run(cmd: list[str], env: dict[str, str] | None = None) -> int:
    """执行命令（默认继承环境），失败透传退出码。"""
    print(f"$ {' '.join(cmd)}")
    return subprocess.run(cmd, env=env).returncode


def _run_or_exit(cmd: list[str], env: dict[str, str] | None = None) -> None:
    code = _run(cmd, env=env)
    if code != 0:
        sys.exit(code)


def _cmd_build_host() -> None:
    _check("cargo", "pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat")
    _run_or_exit(["cargo", "build", "-p", "mediaservo-host", "-p", "mediaservo-client"])


def _cmd_build_server(image: str | None = None, native: bool = False, release: bool = False) -> None:
    """build server: --native=原生编译（pixi 工具链）| --image runtime|dev=Docker 镜像。"""
    if native:
        _check("cargo", "pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat")
        if not os.environ.get("MESON"):
            print("错误: MESON 环境变量未设置——请经 ./mediaservo.sh 调用（source pixi-shell.sh 注入 activation env）", file=sys.stderr)
            sys.exit(2)
        cmd = ["cargo", "build"] + (["--release"] if release else []) + ["-p", "mediaservo-server"]
        # tasks.py env 坑（T1 Ruling 延伸，brief 未预见）: pixi activation 注入的 MESON 指向
        # 不存在的 NINJA → build.rs/tasks.py 跳过 pip 但 NINJA 缺失会挂；pop 掉与 T1 unset 语义一致。
        env = {**os.environ}
        env.pop("MESON_ARGS", None)
        env.pop("MESON", None)
        _run_or_exit(cmd, env=env)
        return
    _check("docker", "安装 docker 并启动 daemon")
    if image:
        if image == "runtime":
            # 生产交付镜像（--target runtime——瘦身/自举 entrypoint）
            _run_or_exit(["docker", "build", "--target", "runtime", "-t", "mediaservo-server:latest", "."])
        else:
            _run_or_exit(COMPOSE_BASE + ["build", "server"])
        return
    _run_or_exit(COMPOSE_BASE + ["build", "server"])


def _cmd_build_client() -> None:
    _check("cargo", "pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat")
    _run_or_exit(["cargo", "build", "-p", "mediaservo-client"])


def _cmd_build(target: str) -> None:
    """build <target> — all|host|server|client|bindings（默认 all）。"""
    if target in ("all", "host"):
        _cmd_build_host()
    if target in ("all", "server"):
        _cmd_build_server()
    if target in ("all", "client"):
        _cmd_build_client()
    if target == "bindings":
        _cmd_build_bindings()


def _workspace_version() -> str:
    """workspace 版本（[workspace.package] version，如 0.1.0）。py3.10 无 tomllib，轻量解析。"""
    text = (ROOT / "Cargo.toml").read_text()
    seg = text.split("[workspace.package]", 1)[1].split("[", 1)[0]
    for line in seg.splitlines():
        line = line.strip()
        if line.startswith("version") and "=" in line:
            return line.split("=", 1)[1].strip().strip('"')
    raise SystemExit("错误: workspace version 未找到")


def _symlink_force(target: str, link: Path) -> None:
    """幂等符号链接（存在则先删）。"""
    try:
        link.unlink()
    except FileNotFoundError:
        pass
    os.symlink(target, link)


def _cmd_build_bindings(release: bool = False) -> None:
    """构建三 SDK cdylib + dev .so.<MAJOR> symlink（D241: DT_NEEDED 解析）。"""
    _check("cargo", "pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat")
    cmd = ["cargo", "build"]
    if release:
        cmd.append("--release")
    cmd += ["-p", "mediaservo-field-c", "-p", "mediaservo-link-c", "-p", "mediaservo-deck-c"]
    _run_or_exit(cmd)
    major = _workspace_version().split(".")[0]
    out_dir = ROOT / ("target/release" if release else "target/debug")
    for sdk in ("field", "link", "deck"):
        _symlink_force(
            f"libmediaservo_{sdk}.so",
            out_dir / f"libmediaservo_{sdk}.so.{major}",
        )
    # node 绑定（napi-rs .node；FFmpeg 动态库链接经 build.rs 补齐）
    _run_or_exit(["cargo", "build"] + (["--release"] if release else []) + ["-p", "mediaservo-node"])
    node_so = out_dir / "libmediaservo_node.so"
    if node_so.exists():
        shutil.copy2(node_so, ROOT / "bindings/node/mediaservo.node")
    print("bindings 构建完成: libmediaservo_{field,link,deck}.so + mediaservo.node (%s, symlink .so.%s)"
          % ("release" if release else "debug", major))


ALL_SDKS = ("field", "link", "deck")


# D-H13: host 包 8 进程二进制（host CLI + 7 守护进程；host-legacy 旧单进程不入包）
HOST_BINS = (
    "mediaservo-host", "host-agent", "host-capturer", "host-streamer",
    "host-recorder", "host-controller", "host-emergency", "host-audio",
)


def _exe_name(name: str) -> str:
    """Windows best-effort: 二进制名带 .exe（其余平台原名）。"""
    return name + (".exe" if sys.platform == "win32" else "")



def _kill_using(path: Path) -> None:
    """kill 占用 path 的进程（exec 占用不通过 fd——文本 busy 是内存映射，需 cmdline 匹配）。"""
    target = os.path.realpath(path)
    killed = []
    try:
        for p in os.listdir("/proc"):
            if not p.isdigit():
                continue
            pid = int(p)
            try:
                # ① fd 扫描（打开文件占用）
                for fd in os.listdir(f"/proc/{p}/fd"):
                    try:
                        if os.path.realpath(f"/proc/{p}/fd/{fd}") == target:
                            killed.append(pid)
                            break
                    except OSError:
                        continue
                if pid in killed:
                    continue
                # ② cmdline 匹配（exec 占用——daemon 进程自身持有二进制映射）
                cmd = open(f"/proc/{p}/cmdline", "rb").read().replace(b"\0", b" ").decode("utf-8", "replace")
                if target in cmd or os.path.basename(target) in cmd:
                    killed.append(pid)
            except OSError:
                continue
    except FileNotFoundError:
        pass
    for pid in killed:
        try:
            os.kill(pid, 15)  # SIGTERM（daemon 优雅退出）
            print(f"  已终止占用进程 {pid}")
        except OSError:
            pass
    time.sleep(1)


def _copy_with_kill(src, dst: Path) -> None:
    """复制带 busy 重试: Text file busy（运行中进程占用）→ 杀占用者 → 重试（最多 3 次）。"""
    for attempt in range(3):
        try:
            shutil.copy2(src, dst)
            return
        except OSError as e:
            if e.errno != 26 or attempt == 2:  # 26 = Text file busy
                raise
            print(f"  {dst.name} 被占用（Text file busy）— 终止占用进程后重试 ({attempt + 1}/3)")
            _kill_using(dst)
    raise OSError(f"复制 {dst} 失败（多次重试仍 busy）")

def strip_package_binaries(staging: Path) -> None:
    """打包前 strip 二进制符号（debug 构建 135-155MB/个——gzip 1.2GB 压缩超时根因，PIT-119）。
    保留可执行（发布物 = strip 后体积 -90%+）；strip 失败仅警告不阻断（不可缺失路径场景）。"""
    if not shutil.which("strip"):
        return
    for f in sorted((staging / "bin").iterdir()) if (staging / "bin").is_dir() else []:
        if f.is_file() and os.access(f, os.X_OK):
            code = subprocess.run(["strip", "--strip-unneeded", str(f)], capture_output=True, check=False).returncode
            if code != 0:
                print(f"  warning: strip {f.name} 失败（跳过——包体积未优化）")


def _cmd_install_host(prefix: str, release: bool = False, brand: str = "") -> None:
    """安装 host 包（D-H13 /opt/mediaservo-host 布局）: bin 8 + oxmgr 打包（版本锁定）
    + host init 生成 etc/{host.toml,link/*} + identity.json（幂等——重装保留凭据）
    + run/logs + recordings。生产部署: --prefix /opt/mediaservo-host。"""
    src_dir = ROOT / ("target/release" if release else "target/debug")
    bin_dir = Path(prefix) / "bin"
    try:
        bin_dir.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        print(f"错误: 无法创建 {bin_dir}: {e} — 生产部署用 --prefix /opt/mediaservo-host（需 root）", file=sys.stderr)
        sys.exit(1)

    # 运行中的实例二进制被占用（Text file busy）→ 复制前自动停进程族。
    # 注意: 不调 oxmgr CLI（其 IPC 命令在 daemon 未跑时自动拉起 daemon——反而制造 busy）；
    # 直接 kill host 进程族 + 占用 bin/oxmgr 的 daemon。
    oxfile = Path(prefix) / "run" / "oxfile.toml"
    installed_cli = bin_dir / _exe_name("mediaservo-host")
    if oxfile.exists() and installed_cli.exists():
        print("检测到运行中的 host 实例 — 先停进程族（重装不覆盖 etc/ 凭据）")
        # 复活源三连停: ① systemd 自启 unit（Restart=always 会在 daemon 被杀后立即拉起）
        # ② daemon 自身 ③ host 进程族（restart_policy=always 由 daemon 重启——必须先杀 daemon）
        # systemctl 不接受 glob——枚举 unit 文件逐个停（self-startup unit 是 daemon 复活源）
        units_dir = Path.home() / ".config" / "systemd" / "user"
        if shutil.which("systemctl") and units_dir.is_dir():
            # 双前缀枚举: legacy oxmgr-host-* + 品牌化 oxmgr-<brand>-*（install 传 --brand）
            patterns = ["oxmgr-host-*.service"]
            if brand:
                patterns.append(f"oxmgr-{brand}-*.service")
            seen = set()
            for pat in patterns:
                for u in sorted(units_dir.glob(pat)):
                    if u.name in seen:
                        continue
                    seen.add(u.name)
                    subprocess.run(["systemctl", "--user", "stop", u.name], check=False)
                    subprocess.run(["systemctl", "--user", "reset-failed", u.name], check=False)
        _kill_using(bin_dir / "oxmgr")
        for name in ("host-agent", "host-streamer", "host-capturer", "host-recorder",
                     "host-controller", "host-emergency", "host-audio"):
            subprocess.run(["pkill", "-x", name], check=False)
        time.sleep(1)

    missing = [str(src_dir / _exe_name(b)) for b in HOST_BINS if not (src_dir / _exe_name(b)).exists()]
    if missing:
        print(f"错误: 缺失二进制 {missing} — 先运行: mediaservo build host{' --release' if release else ''}，或 mediaservo install host --build", file=sys.stderr)
        sys.exit(1)
    for b in HOST_BINS:
        _copy_with_kill(src_dir / _exe_name(b), bin_dir / _exe_name(b))

    # oxmgr 随包锁定版本（D-H13）: PATH 找到则复制打包；缺 → 清晰指引（非致命,
    # 运行时 PATH 缺 oxmgr 由 `host doctor` 检出）
    oxmgr_src = shutil.which("oxmgr")
    if oxmgr_src is not None:
        _copy_with_kill(oxmgr_src, bin_dir / _exe_name("oxmgr"))
        ver = subprocess.run([oxmgr_src, "--version"], capture_output=True, text=True, check=False)
        print(f"  oxmgr 已打包: {ver.stdout.strip() or ver.stderr.strip() or '?'}")
    else:
        print("错误: PATH 未找到 oxmgr — 未打包（运行时需它拉起进程）。安装: npm install -g oxmgr（https://github.com/Vladimir-Urik/OxMgr#install），或构建 oxmgr-src 后放 ~/.local/bin，再重跑 install host", file=sys.stderr)

    # host init: 生成 etc/ + identity.json + 令牌（幂等——已存在跳过, 重装保留凭据）
    # brand: env 注入（init 是独立进程——device 前缀需与布局品牌一致）
    init_env = dict(os.environ)
    if brand:
        init_env["MEDIASERVO_BRAND"] = brand
    _run_or_exit([str(bin_dir / _exe_name("mediaservo-host")), "init", str(Path(prefix))], env=init_env)

    # 创建 <prefix>/{host,mediaservo-host} → bin/mediaservo-host 快捷方式（品牌化: cp + cp-host）
    shortcut_names = ("host", "mediaservo-host") if not brand else (brand, f"{brand}-host")
    for link_name in shortcut_names:
        link = Path(prefix) / link_name
        if link.exists():
            link.unlink()
        try:
            link.symlink_to("bin/mediaservo-host")  # 相对路径，前缀可搬迁
            print(f"  已创建符号链接 {link} → bin/mediaservo-host")
        except OSError:
            shutil.copy2(bin_dir / "mediaservo-host", link)
            print(f"  已复制 mediaservo-host 到 {link}（符号链接失败，回退到拷贝）")

    # 品牌 app 名符号链接: oxmgr 执行 cp-agent 等（translate 生成 <brand>-<app> 命令）
    # 默认品牌 app 名 == host-*（二进制本身），无需链接；品牌化需 bin/<brand>-<app> → bin/host-<app>
    if brand:
        app_bases = ("agent", "recorder", "controller", "emergency", "audio", "capturer", "streamer", "legacy")
        for base in app_bases:
            src = bin_dir / _exe_name(f"host-{base}")
            if not src.exists():
                continue
            dst = bin_dir / _exe_name(f"{brand}-{base}")
            if dst.exists() and dst.is_symlink():
                dst.unlink()
            try:
                dst.symlink_to(src.name)
                print(f"  bin/{brand}-{base} → host-{base}")
            except OSError:
                shutil.copy2(src, dst)
                print(f"  bin/{brand}-{base} ← 复制（符号链接失败）")

    for d in ("run/logs", "recordings"):
        (Path(prefix) / d).mkdir(parents=True, exist_ok=True)

    print(f"host 已安装到 {prefix}（{'release' if release else 'debug'}）")
    print(f"  bin/    {', '.join(_exe_name(b) for b in HOST_BINS)} + oxmgr")
    print("  etc/    host.toml + link/{signing.pem,*.token}（host init 生成, 重装保留）")
    print("  run/    logs/")
    print("  recordings/")
    print("  identity.json（0600, 设备身份 — 重装保留, 勿删）")
    print(f"使用: export PATH={bin_dir}:$PATH && mediaservo-host doctor {prefix} && mediaservo-host start {prefix}")
    if sys.platform == "win32":
        print("注意: Windows best-effort（未全面验证）— 生产部署建议 Linux 车端", file=sys.stderr)

def _platform_tag() -> str:
    """wheel 平台 tag（Linux x86_64 → linux_x86_64；其余平台 best-effort）。"""
    import platform
    mach = platform.machine().lower().replace("-", "_")
    return f"{platform.system().lower()}_{mach}"


def _cmd_install_bindings(prefix: str, components: str = "all", release: bool = False) -> None:
    """安装 bindings（按需分发）: libmediaservo_<sdk>.so.<MAJOR>.<MINOR>.<PATCH> 三件套（D241）
    + C/cxx 头文件（D248 include/mediaservo 布局）+ .pc + cmake config（组件裁剪）。"""
    ver = _workspace_version()
    src_dir = ROOT / ("target/release" if release else "target/debug")
    major, minor, patch = ver.split(".")
    if components == "all":
        sdks = list(ALL_SDKS)
    else:
        sdks = [c.strip() for c in components.split(",")]
        unknown = [c for c in sdks if c not in ALL_SDKS]
        if unknown:
            print(f"错误: 未知组件 {unknown}（可选: {ALL_SDKS} 或 all）", file=sys.stderr)
            sys.exit(1)
        sdks = [c for c in ALL_SDKS if c in sdks]  # 保持稳定顺序

    lib_dir = Path(prefix) / "lib"
    inc_dir = Path(prefix) / "include" / "mediaservo"
    lib_dir.mkdir(parents=True, exist_ok=True)
    inc_dir.mkdir(parents=True, exist_ok=True)

    for sdk in sdks:
        src = src_dir / f"libmediaservo_{sdk}.so"
        if not src.exists():
            print(f"错误: {src} 不存在 — 先运行: mediaservo build bindings{' --release' if release else ''}，或 mediaservo install bindings --build", file=sys.stderr)
            sys.exit(1)
        real = lib_dir / f"libmediaservo_{sdk}.so.{major}.{minor}.{patch}"
        shutil.copy2(src, real)
        _symlink_force(real.name, lib_dir / f"libmediaservo_{sdk}.so.{major}")
        _symlink_force(f"libmediaservo_{sdk}.so.{major}", lib_dir / f"libmediaservo_{sdk}.so")

    for h in (ROOT / "bindings/c/include/mediaservo").glob("*.h"):
        shutil.copy2(h, inc_dir)  # common.h 总是装（所有头依赖）
    # C++ 共享目录（detail/result.hpp + 3rdparty/tl/expected.hpp + NOTICE）: 总是装
    for f in (ROOT / "bindings/cxx/include/mediaservo").rglob("*"):
        if not f.is_file():
            continue
        dst = inc_dir / f.relative_to(ROOT / "bindings/cxx/include/mediaservo")
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(f, dst)
    for sdk in sdks:
        for h in (ROOT / f"bindings/c/mediaservo-{sdk}-c/include/mediaservo").glob("*.h"):
            shutil.copy2(h, inc_dir)
        for h in (ROOT / f"bindings/cxx/mediaservo-{sdk}-cxx/include/mediaservo").glob("*.hpp"):
            shutil.copy2(h, inc_dir)

    # pkg-config (.pc) + CMake package config：模板随归属（.pc 在各自 SDK 包目录，
    # cmake 聚合模板在 bindings/c/cmake/——iceoryx2 每包自带配方模式）
    pc_dir = lib_dir / "pkgconfig"
    cmake_dir = lib_dir / "cmake" / "mediaservo"
    pc_dir.mkdir(parents=True, exist_ok=True)
    cmake_dir.mkdir(parents=True, exist_ok=True)
    prefix_abs = str(lib_dir.parent)  # 规范绝对 prefix（configure 传统；模板用 ${pcfiledir} 保持可重定位）
    for sdk in sdks:  # 按需: 只装选中的 .pc
        t = ROOT / f"bindings/c/mediaservo-{sdk}-c/mediaservo-{sdk}.pc.in"
        content = t.read_text().replace("@VERSION@", ver).replace("${pcfiledir}/../..", prefix_abs)
        (pc_dir / f"mediaservo-{sdk}.pc").write_text(content)
    sdk_list = " ".join(sdks)  # cmake config 组件裁剪
    cmake_tpl = ROOT / "bindings/c/cmake"
    for name, t in (("mediaservoConfig.cmake", "mediaservoConfig.cmake.in"),
                    ("mediaservoConfigVersion.cmake", "mediaservoConfigVersion.cmake.in")):
        content = (cmake_tpl / t).read_text().replace("@VERSION@", ver).replace("@MAJOR@", major)
        content = content.replace("@SDK_LIST@", sdk_list)
        (cmake_dir / name).write_text(content)

    # Python 绑定: 构建 fat wheel（选中 SDK 的 .so 三件套进包内 _libs/，
    # CDLL 打开 .so.<MAJOR> 实体使 DT_NEEDED 自解析）→ pip install --prefix
    py_src = ROOT / "bindings/python/mediaservo"
    py_pkg = py_src / "mediaservo"
    libs_src = py_pkg / "_libs"
    libs_src.mkdir(exist_ok=True)
    for sdk in sdks:
        so = src_dir / f"libmediaservo_{sdk}.so"
        so_major = libs_src / f"libmediaservo_{sdk}.so.{major}"
        shutil.copy2(so, so_major)  # 实体（SONAME 文件名，DT_NEEDED 自解析）
        _symlink_force(f"libmediaservo_{sdk}.so.{major}", libs_src / f"libmediaservo_{sdk}.so")
    for stale in libs_src.glob("libmediaservo_*.so*"):
        if not any(sdk in stale.name for sdk in sdks):
            stale.unlink()  # 组件缩减时防残留
    try:
        wheel_dir = Path(prefix) / "wheel"
        wheel_dir.mkdir(parents=True, exist_ok=True)
        r = subprocess.run(
            ["pip", "wheel", "--no-deps", "--no-build-isolation", "-w", str(wheel_dir), str(py_src)],
            capture_output=True, text=True,
        )
        if r.returncode != 0:
            print(f"错误: wheel 构建失败 — {r.stderr[-500:]}", file=sys.stderr)
            sys.exit(1)
        # 强制平台 tag（wheel 含 Linux .so，拒绝 any-tag 误装跨平台）: 改 WHEEL 内 Tag + 重命名
        import zipfile
        wheel = next(wheel_dir.glob("mediaservo-*.whl"))
        tag_new = f"py3-none-{_platform_tag()}"
        with zipfile.ZipFile(wheel, "r") as z:
            items = z.infolist()
            data = {i.filename: z.read(i) for i in items}
        fixed = wheel.with_name(f"mediaservo-{ver}-{tag_new}.whl")
        with zipfile.ZipFile(fixed, "w") as z:
            for i in items:
                content = data[i.filename]
                if i.filename == "mediaservo-{ver}.dist-info/WHEEL":
                    content = content.replace(b"Tag: py3-none-any", f"Tag: {tag_new}".encode())
                z.writestr(i, content)
        wheel.unlink()
        r2 = subprocess.run(
            ["pip", "install", "--prefix", str(Path(prefix)), "--no-deps", str(fixed)],
            capture_output=True, text=True,
        )
        if r2.returncode != 0:
            print(f"错误: pip install 失败 — {r2.stderr[-500:]}", file=sys.stderr)
            sys.exit(1)
    finally:
        shutil.rmtree(libs_src, ignore_errors=True)  # 清理临时 _libs（gitignore 兜底）

    # Node 绑定: 包目录复制到 <prefix>/node/mediaservo/（lib + package.json + .node）
    node_src = ROOT / "bindings/node"
    node_dst = Path(prefix) / "node" / "mediaservo"
    if (node_src / "mediaservo.node").exists():
        node_dst.mkdir(parents=True, exist_ok=True)
        for f in ("package.json", "mediaservo.node"):
            shutil.copy2(node_src / f, node_dst)
        lib_dst = node_dst / "lib"
        lib_dst.mkdir(exist_ok=True)
        shutil.copy2(node_src / "lib/index.mjs", lib_dst)
        print(f"  node/    mediaservo 包（package.json + mediaservo.node + lib/）→ {node_dst}")
        print("  使用: NODE_PATH=<prefix>/node npm 或 import '<prefix>/node/mediaservo/lib/index.mjs'")

    site_packages = lib_dir / f"python{sys.version_info.major}.{sys.version_info.minor}" / "site-packages"
    print(f"bindings 已安装到 {prefix}（组件: {', '.join(sdks)}；{'release' if release else 'debug'}）")
    print(f"  lib/    {', '.join(f'libmediaservo_{s}.so.{major}.{minor}.{patch}' for s in sdks)} + .so.{major} + .so")
    print(f"  lib/pkgconfig/   {', '.join(f'mediaservo-{s}.pc' for s in sdks)}（pkg-config 消费）")
    print(f"  lib/cmake/mediaservo/  mediaservoConfig.cmake + ConfigVersion.cmake（find_package(mediaservo COMPONENTS {'|'.join(sdks)})）")
    print(f"  include/mediaservo/  common.h + {', '.join(f'{s}.h' for s in sdks)} + {', '.join(f'{s}.hpp' for s in sdks)}")
    print(f"Python: 包已装到 {site_packages}（fat 自包含, 含 _libs）")
    print(f"  使用: export PYTHONPATH={site_packages} && python3 app.py")


# ── package: dist/ 双包发布（D-H13）──────────────────────────
def _write_version_file(dst: Path, target: str) -> None:
    """版本契约文件（D-H13: 版本兼容靠协议契约显式配对, 非同包隐含）。
    host-version.txt / sdk-version.txt: workspace 版本 + FrameMeta wire 版本 + 令牌 schema 版本。
    消费方（ROS/算法）校验 sdk-version.txt 与 host 包配对; 信令 wire 契约随 workspace 演进,
    独立协议版本号 = 后续工作（D-H14 全量版本化方案）。"""
    ver = _workspace_version()
    lines = [
        f"# mediaservo-{target}-{ver} — 协议契约版本声明（D-H13/D-H14 最小版）",
        f"workspace_version: {ver}",
        "frame_meta_version: 1",     # FrameMeta 定长 LE 36B wire format（D243; link frame.rs）
        "token_schema_version: 1",   # MSTK 单文件自描述令牌字节版本 0x01（D238/D243; link token.rs）
        "# host 包部署: tar 解包到前缀目录; 多设备共用同一包时, 每台删除 identity.json 后重跑",
        "# `host init <prefix>`（幂等, 已存在凭据保留）生成独立设备身份（G4）",
    ]
    (dst / f"{target}-version.txt").write_text("\n".join(lines) + "\n")


def _cmd_package(args: argparse.Namespace) -> None:
    """package <target> — dist/mediaservo-host|sdk-<ver>.tar.gz 双包发布（D-H13）。
    host: install host 布局（bin 8 + oxmgr + etc/ + run/logs + recordings + identity.json）
    bindings: install bindings 布局（lib/include/python/node/pkgconfig/cmake + wheel）→ sdk 包
    staging 临时目录 → tar.gz; 包内含版本契约文件（host-version.txt / sdk-version.txt）。"""
    if sys.platform == "win32":
        print("package: Windows best-effort — 验证清单见 scripts/e2e-win-validate.ps1", file=sys.stderr)
    pkg_name = "sdk" if args.target == "bindings" else "host"
    ver = _workspace_version()
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f"ms-{args.target}-pkg-", dir=str(dist)))
    try:
        if args.target == "host":
            _cmd_install_host(str(staging), args.release, args.brand)
        else:
            _cmd_install_bindings(str(staging), "all", args.release)
        _write_version_file(staging, pkg_name)
        prefix_name = args.brand if args.brand else "mediaservo"
        out = dist / f"{prefix_name}-{pkg_name}-{ver}.tar.gz"
        strip_package_binaries(staging)  # PIT-119: debug 二进制未 strip（单 135-155MB）→ gzip 1.2GB 超时
        with tarfile.open(out, "w:gz", compresslevel=6) as tar:  # 默认 9 最慢; 6 工程折中
            for entry in sorted(staging.iterdir()):
                tar.add(entry, arcname=entry.name)  # 解包到前缀目录即为 D-H13 布局
        print(f"✓ 打包完成: {out}（{out.stat().st_size // 1024} KiB）")
        print(f"  内容: {', '.join(sorted(e.name for e in staging.iterdir()))}")
    finally:
        shutil.rmtree(staging, ignore_errors=True)

# 排除的接口类型/名称：docker 网桥、虚拟接口（这些 IP 客户端不可达）。
# 注意: tun/vpn **不排除**——VPN 组网场景下隧道 IP 正是客户端可达路径（10.144.0.3 实证）；
# 公告多余候选无害（ICE 多候选，对端连不通自动跳过）。
_ANNOUNCED_IP_BLOCKED_IFACE = ("lo", "docker", "br-", "veth", "virbr")


def _detect_announced_ips() -> list[str]:
    """探测宿主机全部真实网卡 IP（供容器内 mediasoup announced_address 使用）。

    宿主 IP 会变且可能有多个（多网卡/DHCP）——返回全部真实 IP（逗号分隔），
    由 server 侧为每个 IP 创建 ListenInfo（WebRtcServer 多 announced）。
    按接口名过滤 docker 网桥(br-*)/VPN(tun*)/虚拟接口，仅保留物理/真实网卡。"""
    ips: list[str] = []
    try:
        out = subprocess.run(
            ["ip", "-o", "-4", "addr", "show"], capture_output=True, text=True, timeout=5, check=False
        )
        for line in out.stdout.splitlines():
            # 格式: "2: ens32    inet 192.168.2.127/24 brd ... scope global ..."
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
        # ip 命令不可用 → 回退 hostname -I（粗过滤）
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


def _compose_env() -> dict[str, str]:
    """docker compose 调用环境 — 确保 MEDIASERVO_SFU_ANNOUNCED_IP 有值。
    PIT-79: CLI 启动 server 时若未注入，mediasoup 公告 0.0.0.0 → 浏览器拉流失败。
    显式 env 优先，否则自动探测宿主机全部真实 IP（逗号分隔，多网卡支持）。"""
    env = {**os.environ}
    if not env.get("MEDIASERVO_SFU_ANNOUNCED_IP"):
        ips = _detect_announced_ips()
        if ips:
            env["MEDIASERVO_SFU_ANNOUNCED_IP"] = ",".join(ips)
            print(f"MEDIASERVO_SFU_ANNOUNCED_IP 自动探测: {env['MEDIASERVO_SFU_ANNOUNCED_IP']}")
    return env


def _cmd_start(target: str, foreground: bool = False, legacy: bool = False) -> None:
    """start <target> [--foreground] [--legacy] — server: compose 幂等启动; host: 多进程推流。
    --foreground/-f: 阻塞前台运行，输出实时透传（开发调试）。
    --legacy: host 回退旧单进程 host-legacy（C4 前默认路径）。"""
    if target == "server":
        _check("docker", "安装 docker 并启动 daemon")
        cmd = COMPOSE_BASE + (["up"] if foreground else ["up", "-d", "server"])
        _run_or_exit(cmd, env=_compose_env())
    elif target == "host":
        if foreground:
            if legacy:
                _run_host_foreground(_find_host_binary())
            else:
                print("多进程 host 由 oxmgr 守护（无前台模式）— 去掉 --foreground 或用 --legacy", file=sys.stderr)
                sys.exit(1)
        else:
            _cmd_run_host(legacy=legacy)
    else:  # client
        print("start client: 待实现（client 骨架阶段）", file=sys.stderr)
        sys.exit(1)


def _cmd_restart(args: argparse.Namespace) -> None:
    target = args.target
    """restart <target> — 清除已运行的再启动（显式中断语义）。"""
    if target == "server":
        _check("docker", "安装 docker 并启动 daemon")
        print("重启 server: 停止旧容器...")
        subprocess.run(COMPOSE_BASE + ["down"], check=False, env=_compose_env())  # 无容器时忽略错误
        _run_or_exit(COMPOSE_BASE + ["up", "-d", "server"], env=_compose_env())
        print("✓ server 已重启")
    elif target == "host":
        _cmd_run_host()
    else:  # client
        print("restart client: 待实现（client 骨架阶段）", file=sys.stderr)
        sys.exit(1)


def _find_host_binary() -> Path:
    """找 host 二进制（优先 CARGO_TARGET_DIR，回退项目 target）— host-legacy 单进程（--legacy 路径）。"""
    cargo_target = os.environ.get("CARGO_TARGET_DIR")
    candidates = []
    if cargo_target:
        candidates.append(Path(cargo_target) / "debug/host-legacy")
    candidates += [
        ROOT / "target/debug/host-legacy",
        ROOT / "target/release/host-legacy",
    ]
    bin_path = next((p for p in candidates if p.exists()), None)
    if bin_path is None:
        print("错误: 未找到 host-legacy 二进制 — 先运行: mediaservo build host", file=sys.stderr)
        sys.exit(1)
    return bin_path


def _find_host_cli() -> Path:
    """找多进程 host CLI 二进制（host init/start/stop/token issue 入口）。"""
    cargo_target = os.environ.get("CARGO_TARGET_DIR")
    candidates = []
    if cargo_target:
        candidates.append(Path(cargo_target) / "debug/mediaservo-host")
    candidates += [
        ROOT / "target/debug/mediaservo-host",
        ROOT / "target/release/mediaservo-host",
    ]
    bin_path = next((p for p in candidates if p.exists()), None)
    if bin_path is None:
        print("错误: 未找到 mediaservo-host CLI 二进制 — 先运行: mediaservo build host", file=sys.stderr)
        sys.exit(1)
    return bin_path


def _run_host_foreground(bin_path: Path) -> None:
    """前台阻塞运行 host — 输出实时透传终端，Ctrl+C 同步退出（开发调试用）。
    host 单实例端口 9801 独占：启动前必须清旧（与后台路径一致）。"""
    subprocess.run(["pkill", "-x", "host-legacy"], check=False)
    time.sleep(1)
    env = {**os.environ, "RUST_LOG": "info"}
    proc = subprocess.Popen([str(bin_path)], cwd=ROOT, env=env)
    try:
        proc.wait()
    except KeyboardInterrupt:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        sys.exit(130)
    sys.exit(proc.returncode)


def _cmd_run_host(legacy: bool = False) -> None:
    """启动多进程 host — init（缺省）→ token issue --all → host start（oxmgr 拉起全部进程）。
    --legacy: 回退旧单进程 host-legacy（pkill + 后台 Popen，日志 /tmp/mediaservo-host.log）。"""
    if sys.platform == "win32":
        print("run-host: Windows 暂不支持", file=sys.stderr)
        sys.exit(1)
    if legacy:
        _run_host_legacy()
        return
    _check("oxmgr", "多进程 host 需 oxmgr — npm install -g oxmgr")
    host = _find_host_cli()
    # C25: 清 iceoryx2 固定 topic 残留（上次运行崩溃/被 SIGKILL 的发布端会留下 service 状态
    # → 跨 run 二次 subscribe/open 持久 SystemInFlux）。仅清 MediaServo 自有运行时目录。
    subprocess.run(["rm", "-rf", "/tmp/iceoryx2"], check=False)
    for entry in Path("/dev/shm").glob("iox2_*"):
        entry.unlink(missing_ok=True)
    # 1) init（幂等）— host.toml / signing.pem 已存在则跳过
    if not (ROOT / "etc" / "host.toml").exists():
        _run_or_exit([str(host), "init", str(ROOT)])
    # 2) token issue --all（幂等 — 已存在不覆盖，D-H10 固定令牌；host init 已签发时即 no-op）
    #    标准集从 host.toml 推导: <cam>.token/<stream>.token/recorder.token/agent.token
    _run_or_exit([str(host), "token", "issue", "--all", str(ROOT)])
    # 3) host start（oxmgr apply）
    _run_or_exit([str(host), "start", str(ROOT)])
    print("✓ host 多进程已启动（oxmgr 管理; 日志: ~/.local/share/oxmgr/logs 或 `oxmgr logs all`）")


def _run_host_legacy() -> None:
    """旧单进程 host-legacy 后台启动（--legacy 回退路径）。"""
    bin_path = _find_host_binary()
    if bin_path is None:
        print("错误: 未找到 host-legacy 二进制 — 先运行: mediaservo build-host", file=sys.stderr)
        sys.exit(1)
    # 2) 杀旧进程（pkill -x 精确进程名，避免误杀）
    subprocess.run(["pkill", "-x", "host-legacy"], check=False)
    time.sleep(1)
    # 3) 后台启动（start_new_session 脱离终端，日志 /tmp/mediaservo-host.log）
    log_path = Path("/tmp/mediaservo-host.log")
    env = {**os.environ, "RUST_LOG": "info"}
    proc = subprocess.Popen(
        [str(bin_path)],
        cwd=ROOT,
        env=env,
        stdout=open(log_path, "wb"),
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    time.sleep(3)
    if proc.poll() is None:
        print(f"✓ host 已启动 (PID {proc.pid}) — 配置: crates/mediaservo-host/config/host.conf")
        print(f"  日志: {log_path}")
    else:
        print(f"✗ host 启动失败 (exit {proc.returncode}) — 日志: {log_path}", file=sys.stderr)
        sys.exit(1)

def _cmd_stop(args: argparse.Namespace) -> None:
    target = args.target
    """stop <target> — server: compose stop（保留容器，秒级再启）; host/client: 杀进程。"""
    if target == "server":
        _check("docker", "安装 docker 并启动 daemon")
        _run_or_exit(COMPOSE_BASE + ["stop", "server"])
    elif target == "host":
        # 双路径幂等停止: legacy 单进程（pkill）+ 多进程（host stop, oxmgr）
        subprocess.run(["pkill", "-x", "host-legacy"], check=False)
        host_cli = next((p for p in (ROOT / "target/debug/mediaservo-host", ROOT / "target/release/mediaservo-host") if p.exists()), None)
        if host_cli is not None:
            _run_or_exit([str(host_cli), "stop", str(ROOT)])
        print("✓ host 已停止")
    else:  # client
        subprocess.run(["pkill", "-x", "mediaservo-client"], check=False)
        print("✓ client 已停止")


def _cmd_logs(args: argparse.Namespace) -> None:
    """logs [<target>] [--follow] — server: compose 日志; host: /tmp/mediaservo-host.log。"""
    target = args.target
    follow = ["-f"] if args.follow else []
    if target == "server":
        _check("docker", "安装 docker 并启动 daemon")
        _run_or_exit(COMPOSE_BASE + ["logs"] + follow + ["server"])
    elif target == "host":
        log_path = Path("/tmp/mediaservo-host.log")
        if log_path.exists():  # legacy 单进程日志
            _run_or_exit(["tail", "-f", str(log_path)])
        ox_logs = Path.home() / ".local/share/oxmgr/logs"
        if ox_logs.is_dir():  # 多进程日志（oxmgr 管理）
            _run_or_exit(["tail", "-f", *sorted(ox_logs.glob("host-*.out.log"))])
        print(f"错误: 无 host 日志（{log_path} 与 {ox_logs} 均不存在）— 先运行: mediaservo up host", file=sys.stderr)
        sys.exit(1)
    else:  # client
        print("logs client: 待实现（client 骨架阶段）", file=sys.stderr)
        sys.exit(1)


_E2E_SUITES = {
    "sfu": ["cargo", "test", "-p", "mediaservo-host", "--test", "e2e_sfu"],
    "push": ["cargo", "test", "-p", "mediaservo-field", "--test", "push_e2e"],
    "ui": ["bash", "-c", "cd www/apps/admin && npx playwright test e2e"],
    "host": ["bash", "scripts/e2e-install-host.sh"],
    "package": ["bash", "scripts/e2e-package.sh"],
    "brand": ["bash", "scripts/e2e-brand.sh"],
    "bindings": ["bash", "scripts/e2e-bindings.sh"],
    "client": ["bash", "scripts/e2e-test.sh"],  # macOS client 9/9（I4 环境阻塞可跳过）
    "smoke": ["bash", "scripts/e2e-prod-smoke.sh"],  # 生产冒烟（Phase 4 产物——up --env prod + admin/配发断言）
}


def _cmd_e2e(args: argparse.Namespace) -> None:
    if sys.platform == "win32" and args.suite not in ("host", "package", "brand", "bindings", "smoke"):
        print("e2e: Windows 仅支持 bash 脚本套件", file=sys.stderr)
        sys.exit(1)
    for tool, hint in (
        ("cargo", "pixi 环境未激活?"),
        ("docker", "server 容器需要 docker"),
        ("bash", "e2e 脚本需要 bash"),
    ):
        _check(tool, hint)
    cmd = _E2E_SUITES[args.suite]
    print(f"e2e {args.suite}: {' '.join(cmd)}")
    _run_or_exit(cmd)


def _cmd_test() -> None:
    _check("cargo", "pixi 环境未激活?")
    _run_or_exit(["cargo", "test", "--workspace", "--exclude", "mediaservo-server"])


def _cmd_ci() -> None:
    _check("cargo", "pixi 环境未激活?")
    _check("docker", "安装 docker 并启动 daemon")
    steps = [
        ["cargo", "fmt", "--all", "--", "--check"],
        ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
        ["cargo", "test", "--workspace", "--exclude", "mediaservo-server"],
    ]
    for step in steps:
        code = _run(step)
        if code != 0:
            sys.exit(code)
    _cmd_e2e(argparse.Namespace(suite="sfu"))


def _rm_tree(path: Path) -> None:
    """跨平台目录删除（Windows 用 rmdir /s /q 语义，Unix 用 rmtree）。
    容器生成的 root 文件会导致 PermissionError — 捕获并提示手动删除，不中断后续清理。"""
    if not path.exists():
        return
    try:
        if sys.platform == "win32":
            _run_or_exit(["rmdir", "/s", "/q", str(path)])
        else:
            shutil.rmtree(path)
        print(f"已删除: {path}")
    except PermissionError:
        print(
            f"警告: 无法删除 {path}（含容器生成的 root 文件）— 手动执行: sudo rm -rf {path}",
            file=sys.stderr,
        )


def _cmd_install(args: argparse.Namespace) -> None:
    """install <target> — bindings | host。"""
    if args.target == "bindings":
        if args.build:
            _cmd_build_bindings(args.release)  # 一体化: 先构建再安装
        _cmd_install_bindings(args.prefix or str(ROOT / "install"), args.components, args.release)
    elif args.target == "host":
        if args.build:
            _check("cargo", "pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat")
            _run_or_exit(["cargo", "build"] + (["--release"] if args.release else []) + ["-p", "mediaservo-host"])
        _cmd_install_host(args.prefix or str(ROOT / "install" / "host"), args.release, args.brand)


def _cmd_clean(args: argparse.Namespace) -> None:
    """clean <target> — all|server|host|client（默认 all）。
    server: 停容器(+--all 删卷+builder prune); host/client: 清宿主 cargo target。"""
    target = args.target
    if target in ("all", "server"):
        _check("docker", "安装 docker 并启动 daemon")
        down = COMPOSE_BASE + ["down"]
        if args.all:
            down.append("-v")  # --all 显式删卷（cargo-cache）→ 下次 build-server 15-30 分钟重建
            print("警告: clean --all 将删除 cargo-cache 命名卷（下次 server 构建全量重编 15-30 分钟）")
        _run_or_exit(down)
    if target in ("all", "host", "client"):
        # 项目根 target（workspace 默认，host/client 共享）
        _rm_tree(ROOT / "target")
        # CARGO_TARGET_DIR 分支（审核: 用户设置时项目根清理会漏）
        cargo_target = os.environ.get("CARGO_TARGET_DIR")
        if cargo_target:
            print(f"注意: CARGO_TARGET_DIR={cargo_target}（可能被多项目共享）")
            _rm_tree(Path(cargo_target))
    # --all 额外清 docker builder 缓存；不碰 .pixi-cache（包缓存）
    if args.all and target in ("all", "server"):
        _run_or_exit(["docker", "builder", "prune", "-f"])


def _cmd_config(args: argparse.Namespace) -> None:
    if args.config_cmd == "show":
        for path in (HOST_CONF, SERVER_YAML):
            print(f"--- {path.relative_to(ROOT)} ---")
            if path.exists():
                print(path.read_text(encoding="utf-8"))
            else:
                print(f"(缺失: {path})", file=sys.stderr)
        return
    if args.config_cmd == "set":
        if not args.key or args.value is None:
            print("用法: mediaservo config set <key> <value>", file=sys.stderr)
            sys.exit(2)
        # dev 轨道: 写宿主 config/ 目录（prod 卷内用 exec 路径——见 docs/modules/26）
        # 仅支持 signaling.psk 键（其他键待演进）
        if args.key != "signaling.psk":
            print(f"config set: 暂仅支持 signaling.psk（当前键 {args.key}）", file=sys.stderr)
            sys.exit(2)
        p = ROOT / "config" / "host.conf"
        p.write_text(f"signaling.psk = {args.value}\n", encoding="utf-8")
        print(f"已写入 {p.relative_to(ROOT)}（dev 轨道——prod 用 exec 编辑卷内 server.yaml）")
        return
    # validate — pyyaml 真解析（审核 BLOCKER-2: pixi.toml 已加依赖）
    try:
        import yaml  # noqa: PLC0415
    except ImportError:
        print("错误: 缺少 pyyaml — 运行: pixi install", file=sys.stderr)
        sys.exit(1)
    ok = True
    for path in (HOST_CONF, SERVER_YAML):
        if not path.exists():
            print(f"缺失: {path.relative_to(ROOT)}", file=sys.stderr)
            ok = False
            continue
        try:
            yaml.safe_load(path.read_text(encoding="utf-8"))
            print(f"OK: {path.relative_to(ROOT)}")
        except yaml.YAMLError as e:
            print(f"YAML 错误: {path.relative_to(ROOT)}: {e}", file=sys.stderr)
            ok = False
    sys.exit(0 if ok else 1)


def _cmd_status() -> None:
    """环境诊断 — 逐工具检查版本，缺失标 MISSING。"""
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
            print(f"{name:8s} MISSING（或超时）")
            continue
        output = (result.stdout or result.stderr).strip().splitlines()
        print(f"{name:8s} {output[0] if output else '?'}")


def _compose_file(env: str) -> str:
    """环境 → compose 文件映射（up/down/ps/exec 共用）。"""
    return "docker-compose.yml" if env == "prod" else "docker-compose.dev.yml"


def _cmd_up(args: argparse.Namespace) -> None:
    """部署生命周期: up [--env dev|prod] [svc]——prod=单容器+命名卷+entrypoint 自举。"""
    cf = _compose_file(args.env)
    cmd = ["docker", "compose", "-f", cf, "up", "-d", args.svc]
    if args.build:
        cmd.insert(5, "--build")
    env = _compose_env()
    if args.announced_ip:
        env["MEDIASERVO_SFU_ANNOUNCED_IP"] = args.announced_ip
        print(f"MEDIASERVO_SFU_ANNOUNCED_IP 显式指定: {args.announced_ip}")
    # PIT-79 接线: 未显式设置时自动探测宿主机全部真实 IP（多网卡/VPN 场景）注入 env
    _run_or_exit(cmd, env=env)


def _detect_running_env() -> str:
    """检测当前运行中的 server 容器属于哪个 compose 项目（dev/prod——容器名相同，
    按 compose config_files label 区分；无容器 = 默认 dev）。"""
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


def _cmd_down(args: argparse.Namespace) -> None:
    """停止部署（卷保留——prod 数据持久）。无 --env 时自动检测当前运行环境。"""
    env = args.env if args.env is not None else _detect_running_env()
    print(f"down: 检测到环境 {env}")
    cf = _compose_file(env)
    _run_or_exit(["docker", "compose", "-f", cf, "down"])


def _cmd_exec(args: argparse.Namespace) -> None:
    """容器内命令（调试）: exec <svc> -- <cmd>（argparse REMAINDER 自动剥离 --）。"""
    if not args.cmd:
        print("用法: mediaservo exec <svc> -- <cmd>", file=sys.stderr)
        sys.exit(2)
    _run_or_exit(["docker", "compose", "-f", "docker-compose.dev.yml", "exec", "-T", args.svc] + args.cmd)


def _cmd_ps(args: argparse.Namespace) -> None:
    """运行态: compose ps（server 部署）+ host ps（进程实例——cwd 推断）。"""
    cf = _compose_file(getattr(args, "env", "dev"))
    _run(["docker", "compose", "-f", cf, "ps"])


def _cmd_data(args: argparse.Namespace) -> None:
    """数据卷管理: backup|reset|inspect（mediaservo-data/recordings）。"""
    vol_data = "mediaservo_mediaservo-data"
    vol_rec = "mediaservo_mediaservo-recordings"
    if args.data_cmd == "inspect":
        _run_or_exit(["docker", "volume", "ls", "--format", "{{.Name}} {{.Mountpoint}}", "--filter", "name=mediaservo_"])
    elif args.data_cmd == "backup":
        target = Path(args.dir or "backup")
        target.mkdir(parents=True, exist_ok=True)
        for vol in (vol_data, vol_rec):
            out = target / f"{vol}.tar.gz"
            _run_or_exit([
                "docker", "run", "--rm", "-v", f"{vol}:/data:ro",
                "-v", f"{target.absolute()}:/backup",
                "alpine", "sh", "-c", f"tar czf /backup/{vol}.tar.gz -C /data .",
            ])
            print(f"已备份 {vol} → {out}")
    elif args.data_cmd == "reset":
        if not args.force:
            ans = input(f"重置 {vol_data} + {vol_rec}（删除全部数据）？[y/N] ")
            if ans.strip().lower() != "y":
                print("已取消")
                return
        # 语义: 先 down（容器停）再删卷（entrypoint 下次 up 重新生成）
        _cmd_down(argparse.Namespace(env="prod"))
        _run_or_exit(["docker", "volume", "rm", vol_data, vol_rec])
        print("卷已删除——`mediaservo up --env prod` 重新初始化（密钥重新生成）")


def _cmd_doctor(_args: argparse.Namespace | None = None) -> None:
    """环境诊断（原 status——团队审核: doctor 统一诊断入口，与 mediaservo-host doctor 区分）。"""
    _cmd_status()


def _cmd_version() -> None:
    print(VERSION)


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="mediaservo",
        description="MediaServo 统一构建 CLI（单入口: build/up/e2e/clean/config/status...）",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    build_p = sub.add_parser("build", help="构建 <target> [--image runtime|dev|--native]: all|host|server|client|bindings（默认 all）")
    build_p.add_argument("target", nargs="?", choices=["all", "host", "server", "client", "bindings"], default="all")
    build_p.add_argument("--release", action="store_true", help="release 构建（bindings: target/release，strip+LTO）")
    grp = build_p.add_mutually_exclusive_group()
    grp.add_argument("--image", choices=["runtime", "dev"], default=None,
                     help="仅 build server: Docker 镜像 target（runtime=生产交付瘦身镜像；dev=工具链镜像）")
    grp.add_argument("--native", action="store_true",
                     help="仅 build server: 原生编译（pixi 工具链——首次需联网拉 meson wrap；多 IP 公告在 run 阶段生效）")
    build_p.set_defaults(func=_cmd_build)

    up_p = sub.add_parser("up", help="启动部署 <svc> [--env dev|prod] [--announced-ip IP]: dev=热更 compose；prod=单容器+命名卷+entrypoint 自举（公告地址显式覆盖）")
    up_p.add_argument("svc", nargs="?", default="server", help="服务（默认 server）")
    up_p.add_argument("--env", choices=["dev", "prod"], default="dev", help="环境（默认 dev——可省略）")
    up_p.add_argument("--build", action="store_true", help="先构建镜像再 up")
    up_p.add_argument(
        "--announced-ip",
        metavar="IP[,IP...]",
        default=None,
        help="容器 mediasoup 公告地址（覆盖自动探测——如 10.144.0.3 或 192.168.2.127,10.144.0.3；"
        "容器仅单公告生效——首个 IP；多地址需裸机运行）",
    )
    up_p.set_defaults(func=_cmd_up)

    down_p = sub.add_parser("down", help="停止部署 [--env dev|prod]（卷保留；无 --env 自动检测当前环境）")
    down_p.add_argument("--env", choices=["dev", "prod"], default=None)
    down_p.set_defaults(func=_cmd_down)

    restart_p = sub.add_parser("restart", help="重启 <target>: host/client 进程（server 部署用 up/down）")
    restart_p.add_argument("target", choices=["host", "client"])
    restart_p.set_defaults(func=_cmd_restart)

    logs_p = sub.add_parser("logs", help="日志 [<svc>] [--follow]: server(compose) | host(/tmp/mediaservo-host.log) | <svc> 容器")
    logs_p.add_argument("target", nargs="?", choices=["server", "host", "client"], default="server")
    logs_p.add_argument("--follow", "-f", action="store_true", help="跟踪输出")
    logs_p.set_defaults(func=_cmd_logs)

    ps_p = sub.add_parser("ps", help="运行态: compose ps（server 部署——按 --env）")
    ps_p.add_argument("--env", choices=["dev", "prod"], default="dev")
    ps_p.set_defaults(func=_cmd_ps)

    exec_p = sub.add_parser("exec", help="容器内执行: exec <svc> -- <cmd>（compose exec——调试用）")
    exec_p.add_argument("svc", help="服务名（server/proxy）")
    exec_p.add_argument("cmd", nargs=argparse.REMAINDER, help="容器内命令（-- 后）")
    exec_p.set_defaults(func=_cmd_exec)

    e2e_p = sub.add_parser("e2e", help="测试套件: sfu|push|ui|host|package|brand|bindings|client|smoke（smoke=生产冒烟）")
    e2e_p.add_argument("suite", choices=["sfu", "push", "ui", "host", "package", "brand", "bindings", "client", "smoke"])
    e2e_p.set_defaults(func=_cmd_e2e)

    sub.add_parser("test", help="workspace 测试（排除 mediaservo-server）")
    sub.add_parser("ci", help="CI 全链: fmt → clippy → test → e2e sfu")

    install_p = sub.add_parser("install", help="安装 <target>: bindings（lib 三件套 D241 + include/mediaservo 头 D248）| host（D-H13 /opt/mediaservo-host 车端布局）")
    install_p.add_argument("target", choices=["bindings", "host"])
    install_p.add_argument("--prefix", default=None, help="安装前缀（默认 bindings: <项目根>/install；host: <项目根>/install/host）")
    install_p.add_argument("--brand", default="", help="品牌前缀（如 --brand cp → bin/cp-agent 符号链接 + 快捷名 cp/cp-host；缺省 = 官方 mediaservo 命名）")
    install_p.add_argument("--build", action="store_true", help="先构建再安装（等价 build && install）")
    install_p.add_argument("--components", default="all",
                           help="按需分发: field|link|deck|all|逗号组合（如 link,deck；默认 all；仅 bindings）")
    install_p.add_argument("--release", action="store_true", help="安装 release 产物（target/release，配合 build --release）")
    install_p.set_defaults(func=_cmd_install)
    package_p = sub.add_parser("package", help="打包 <target>: host（车端包）| bindings（SDK 包）→ dist/mediaservo-<target>-<ver>.tar.gz（D-H13 双包发布, 含版本契约文件）")
    package_p.add_argument("target", choices=["host", "bindings"])
    package_p.add_argument("--brand", default="", help="品牌包名（dist/<brand>-host-<ver>.tar.gz；缺省 mediaservo-host-<ver>）")
    package_p.add_argument("--release", action="store_true", help="打包 release 产物（target/release, 配合 build --release）")
    package_p.set_defaults(func=_cmd_package)
    clean_p = sub.add_parser("clean", help="清理 <target>: all|server|host|client（默认 all）")
    clean_p.add_argument("target", nargs="?", choices=["all", "server", "host", "client"], default="all")
    clean_p.add_argument("--all", action="store_true", help="显式删卷 + docker builder prune（15-30 分钟重建代价）")
    clean_p.set_defaults(func=_cmd_clean)
    config_p = sub.add_parser("config", help="配置 show|validate|set <key> <value>（dev 轨道 config/ 目录）")
    config_p.add_argument("config_cmd", choices=["show", "validate", "set"])
    config_p.add_argument("key", nargs="?", help="set: 配置键（如 signaling.psk）")
    config_p.add_argument("value", nargs="?", help="set: 配置值")
    config_p.set_defaults(func=_cmd_config)
    data_p = sub.add_parser("data", help="数据卷管理: backup [<dir>]| reset | inspect（mediaservo-data/recordings）")
    data_p.add_argument("data_cmd", choices=["backup", "reset", "inspect"])
    data_p.add_argument("dir", nargs="?", help="backup: 目标目录（默认 ./backup）")
    data_p.add_argument("--force", action="store_true", help="reset: 跳过交互确认")
    data_p.set_defaults(func=_cmd_data)
    doctor_p = sub.add_parser("doctor", help="环境诊断（pixi/cargo/docker/node——原 status）")
    doctor_p.set_defaults(func=_cmd_doctor)
    sub.add_parser("version", help="CLI 版本")

    # 兼容别名（保留 build-*——e2e 脚本/习惯用法）: run-host/up/down 别名已移除（up/down 现一级命令）
    ALIASES = {
        "build-host": ["build", "host"],
        "build-server": ["build", "server"],
    }
    argv = sys.argv[1:]
    if argv and argv[0] in ALIASES:
        argv = ALIASES[argv[0]] + argv[1:]
    args = parser.parse_args(argv)
    if args.command == "start":
        # host/client 进程语义（server 部署用 up）
        _cmd_start(args.target, args.foreground, args.legacy)
    elif args.command == "build":
        if getattr(args, "image", None) and args.target != "server":
            print(f"build --image 仅支持 build server（当前 target={args.target}）", file=sys.stderr)
            sys.exit(2)
        if getattr(args, "native", False) and args.target != "server":
            print(f"build --native 仅支持 build server（当前 target={args.target}）", file=sys.stderr)
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
    elif args.command in ("stop", "restart", "logs"):
        globals()[f"_cmd_{args.command}"](args)
    elif args.command in ("test", "ci"):
        globals()[f"_cmd_{args.command}"]()
    elif args.command == "version":
        _cmd_version()
    elif hasattr(args, "func"):
        args.func(args)


if __name__ == "__main__":
    main()
