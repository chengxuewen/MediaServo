"""mediaservo — MediaServo 统一构建 CLI（vapkg 式单入口）。

薄壳（mediaservo.sh/.bat）保证 pixi 环境激活后调用本脚本：
环境内 PATH/LIBCLANG_PATH 已注入，subprocess 直接调 cargo/docker 等。
平台差异仅 e2e（bash 脚本）与 clean（删除命令）两处。

用法: mediaservo [-h] {build,build-host,build-server,up,down,logs,e2e,test,ci,deploy,install,package,clean,config,status,version} ...
"""

from __future__ import annotations

import argparse
import os
import re
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


def _run(cmd: list[str], env: dict[str, str] | None = None, cwd: str | None = None) -> int:
    """执行命令（默认继承环境），失败透传退出码。"""
    print(f"$ {' '.join(cmd)}{'  (cwd: ' + cwd + ')' if cwd else ''}")
    return subprocess.run(cmd, env=env, cwd=cwd).returncode


def _run_or_exit(cmd: list[str], env: dict[str, str] | None = None, cwd: str | None = None) -> None:
    code = _run(cmd, env=env, cwd=cwd)
    if code != 0:
        sys.exit(code)


def _cmd_build_host() -> None:
    _check("cargo", "pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat")
    _run_or_exit(["cargo", "build", "-p", "mediaservo-host", "-p", "mediaservo-client"])
    # 组装交付布局 out/host/bin（HOST_BINS 8 件——cargo target 保留，out 为产物镜像）
    # brand 非空时 _stage_to_out 物理重命名: host-<app> → <brand>-<app>、mediaservo-host → <brand>-host
    brand = os.environ.get("MEDIASERVO_BRAND", "")
    tgt = ROOT / "target/debug"
    staged = _stage_to_out("host", [tgt / _exe_name(b) for b in HOST_BINS], brand=brand)
    staged += _stage_to_out("host", [tgt / _exe_name("mediaservo-client")], sub="bin")  # D3: client 不品牌
    print(f"host 交付布局组装: out/host/bin/ 共 {len(staged)} 件（{', '.join(p.name for p in staged[:4])}...）")

    # host-streamer 兼容链接（gateway.rs 硬编码 src="host-streamer"——品牌命名下运行时断链）
    # _stage_to_out 已把 host-streamer rename 为 <brand>-streamer；建兼容链接使 gateway 旧名可达
    bin_dir = _out_root() / "host" / "bin"
    if brand:
        link = bin_dir / _exe_name("host-streamer")
        target = _exe_name(f"{brand}-streamer")
        target_path = bin_dir / target
        if target_path.exists() and not link.exists():
            try:
                link.symlink_to(target)
                print(f"  host-streamer → {target}（品牌兼容链接）")
            except OSError:
                pass


def _ensure_admin_dist() -> None:
    """admin 前端增量构建（C13 双轨一致性——Docker 路径 Dockerfile 内现场构建，原生路径对齐）。
    判定: dist 缺失 或 src 有文件比 dist/index.html 新 → pnpm build:admin（turbo filter）。
    无前端变更时零额外成本（mtime 跳过）。修复 C24 事故（改前端 rebuild 不生效）。"""
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
        print(f"admin dist 最新（{dist_index.parent.relative_to(ROOT)}——src 无变更）— 跳过前端构建，嵌入既有 dist")
        return
    _check("pnpm", "pnpm 未安装——前端构建需要（或先手动 cd www && pnpm build:admin）")
    print("admin dist 过期/缺失 — 构建前端（tsc -b && vite build）...")
    _run_or_exit(["pnpm", "build:admin"], cwd=str(ROOT / "www"))


def _cmd_build_server(image: str | None = None, native: bool = False, release: bool = False) -> None:
    """build server: 默认 native（用户裁决 B——不写模式=原生）| --image runtime|dev=Docker 镜像（--native 兼容别名）。"""
    if native or image is None:   # 默认 native；--image 显式才走 Docker
        _ensure_admin_dist()      # 前置增量构建前端（C13 双轨对齐——Docker 路径 Dockerfile 内自理）
        _check("cargo", "pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat")
        if not os.environ.get("MESON"):
            print("错误: MESON 环境变量未设置——请经 ./mediaservo.sh 调用（source pixi-shell.sh 注入 activation env）", file=sys.stderr)
            sys.exit(2)
        cmd = ["cargo", "build"] + (["--release"] if release else []) + ["-p", "mediaservo-server"]
        # tasks.py env 坑（T1 Ruling 延伸，brief 未预见）: pixi activation 注入的 MESON 指向
        # 不存在的 NINJA → build.rs/tasks.py 跳过 pip 但 NINJA 缺失会挂；pop 掉与 T1 unset 语义一致。
        # MESON 断言仅作为「经 mediaservo.sh 激活」代理（pixi activation 注入）；pop 后 cargo
        # 子进程无 MESON/MESON_ARGS，与 T1 unset 语义一致（tasks.py 回落官方 pip 自装 meson/ninja）。
        env = {**os.environ}
        env.pop("MESON_ARGS", None)
        env.pop("MESON", None)
        _run_or_exit(cmd, env=env)
        # 组装到 out/server/bin/（与 build host/bindings 对称——out = 统一发布根）
        server_bin = ROOT / "target" / ("release" if release else "debug") / "mediaservo-server"
        if server_bin.exists():
            _stage_to_out("server", [server_bin], sub="bin")  # brand=""（server 不品牌化——D3）
            print(f"server 交付布局组装: out/server/bin/mediaservo-server（{server_bin.stat().st_size // 1024} KB）")
        # 组装默认配置（server.yaml——从 config/server.docker.yaml 派生，accounts/devices 相对路径）
        etc_dir = _out_root() / "server" / "etc"
        etc_dir.mkdir(parents=True, exist_ok=True)
        src_cfg = ROOT / "config" / "server.docker.yaml"
        if src_cfg.exists() and not (etc_dir / "server.yaml").exists():
            cfg = src_cfg.read_text()
            # accounts/devices 改为相对于 etc 目录（打包后 bin/../etc/accounts.yaml 等）
            cfg = cfg.replace('/opt/mediaservo/etc/accounts.yaml', 'accounts.yaml')
            cfg = cfg.replace('/opt/mediaservo/etc/devices.yaml', 'devices.yaml')
            (etc_dir / "server.yaml").write_text(cfg)
            # 拷贝设备/账号文件（dev 模板——运行时可覆盖）
            import shutil
            for f in ("accounts.yaml", "devices.yaml"):
                src = ROOT / "config" / f
                if src.exists():
                    shutil.copy2(src, etc_dir / f)
            print(f"server 默认配置组装: {etc_dir.relative_to(ROOT)}/server.yaml")
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
    # 组装交付布局 out/bindings（完整 SDK 包镜像——Momus HIGH3/Task 2.5 补齐）:
    # D241 三件套 version-full（.so.<M.m.p> 实体 + .so.<major> + .so 链接）+ D248 头（C/cxx）
    # + pkgconfig .pc + cmake config + python fat wheel/site-packages + node 包
    ver = _workspace_version()
    major, minor, patch = ver.split(".")
    bind_dst = _out_root() / "bindings"
    lib_dst = bind_dst / "lib"; lib_dst.mkdir(parents=True, exist_ok=True)
    pc_dst = lib_dst / "pkgconfig"; pc_dst.mkdir(parents=True, exist_ok=True)
    cmake_dst = lib_dst / "cmake" / "mediaservo"; cmake_dst.mkdir(parents=True, exist_ok=True)
    inc_dst = bind_dst / "include" / "mediaservo"; inc_dst.mkdir(parents=True, exist_ok=True)

    # lib 三件套（D241）: .so.<M.m.p> 实体（SONAME 文件名——DT_NEEDED 自解析）+ 两级符号链接
    for sdk in ALL_SDKS:
        so_src = out_dir / f"libmediaservo_{sdk}.so"
        if not so_src.exists():
            print(f"错误: {so_src} 不存在 — cargo 未产出 {sdk} SDK", file=sys.stderr)
            sys.exit(1)
        full = lib_dst / f"libmediaservo_{sdk}.so.{major}.{minor}.{patch}"
        shutil.copy2(so_src, full)
        _symlink_force(full.name, lib_dst / f"libmediaservo_{sdk}.so.{major}")
        _symlink_force(f"libmediaservo_{sdk}.so.{major}", lib_dst / f"libmediaservo_{sdk}.so")

    # include/mediaservo（D248）: C common.h + per-sdk .h；cxx per-sdk .hpp + 共享 detail/3rdparty
    for h in (ROOT / "bindings" / "c" / "include" / "mediaservo").glob("*.h"):
        shutil.copy2(h, inc_dst)
    for sdk in ALL_SDKS:
        for h in (ROOT / f"bindings/c/mediaservo-{sdk}-c/include/mediaservo").glob("*.h"):
            shutil.copy2(h, inc_dst)
        for h in (ROOT / f"bindings/cxx/mediaservo-{sdk}-cxx/include/mediaservo").glob("*.hpp"):
            shutil.copy2(h, inc_dst)
    for f in (ROOT / "bindings" / "cxx" / "include" / "mediaservo").rglob("*"):
        if not f.is_file():
            continue
        dst = inc_dst / f.relative_to(ROOT / "bindings" / "cxx" / "include" / "mediaservo")
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(f, dst)

    # pkg-config .pc（${pcfiledir} 保持可重定位——不绑定绝对 prefix）
    for sdk in ALL_SDKS:
        tpl = ROOT / f"bindings/c/mediaservo-{sdk}-c/mediaservo-{sdk}.pc.in"
        (pc_dst / f"mediaservo-{sdk}.pc").write_text(tpl.read_text().replace("@VERSION@", ver))

    # cmake package config（find_package(mediaservo COMPONENTS ...)——D248 输出纪律）
    sdk_list = " ".join(ALL_SDKS)
    for name, tpl_name in (("mediaservoConfig.cmake", "mediaservoConfig.cmake.in"),
                           ("mediaservoConfigVersion.cmake", "mediaservoConfigVersion.cmake.in")):
        content = (ROOT / "bindings" / "c" / "cmake" / tpl_name).read_text() \
            .replace("@VERSION@", ver).replace("@MAJOR@", major).replace("@SDK_LIST@", sdk_list)
        (cmake_dst / name).write_text(content)

    # python: fat wheel（_libs 内 .so.<major> 实体——DT_NEEDED 自解析）+ pip --prefix 组装 site-packages
    py_src = ROOT / "bindings" / "python" / "mediaservo"
    py_pkg = py_src / "mediaservo"
    libs_src = py_pkg / "_libs"
    libs_src.mkdir(exist_ok=True)
    try:
        for sdk in ALL_SDKS:
            so_major = libs_src / f"libmediaservo_{sdk}.so.{major}"
            shutil.copy2(out_dir / f"libmediaservo_{sdk}.so", so_major)
            _symlink_force(f"libmediaservo_{sdk}.so.{major}", libs_src / f"libmediaservo_{sdk}.so")
        wheel_dir = bind_dst / "wheel"; wheel_dir.mkdir(exist_ok=True)
        r = subprocess.run([sys.executable, "-m", "pip", "wheel", "--no-deps",
                            "--no-build-isolation", "-w", str(wheel_dir), str(py_src)],
                           capture_output=True, text=True)
        if r.returncode != 0:
            print(f"错误: wheel 构建失败 — {r.stderr[-500:]}", file=sys.stderr)
            sys.exit(1)
        # 强制平台 tag（wheel 含 Linux .so，拒绝 any-tag 误装跨平台）: 改 WHEEL 内 Tag + 重命名。
        # 版本保持与 pyproject（=内 dist-info）一致——不强制 workspace ver（原 install 的 {ver} 重命名
        # 在 workspace ver ≠ pyproject 0.1.0 时 dist-info 版本不一致 → pip 拒绝安装——本实现更小修复面）
        import zipfile
        wheel = next(wheel_dir.glob("mediaservo-*.whl"))
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
        r2 = subprocess.run([sys.executable, "-m", "pip", "install", "--prefix",
                             str(bind_dst), "--no-deps", str(fixed)],
                            capture_output=True, text=True)
        if r2.returncode != 0:
            print(f"错误: pip install 失败 — {r2.stderr[-500:]}", file=sys.stderr)
            sys.exit(1)
    finally:
        shutil.rmtree(libs_src, ignore_errors=True)  # 清理临时 _libs（gitignore 兜底）

    # node 包: node/mediaservo/（package.json + .node + lib/index.mjs）——D248 布局
    node_src = ROOT / "bindings" / "node"
    if (node_src / "mediaservo.node").exists():
        node_dst = bind_dst / "node" / "mediaservo"
        node_dst.mkdir(parents=True, exist_ok=True)
        for f in ("package.json", "mediaservo.node"):
            shutil.copy2(node_src / f, node_dst)
        (node_dst / "lib").mkdir(parents=True, exist_ok=True)
        shutil.copy2(node_src / "lib" / "index.mjs", node_dst / "lib")
    print("bindings 构建完成: libmediaservo_{field,link,deck}.so 三件套 version-full + node + python + .pc + cmake (%s)"
          % ("release" if release else "debug"))
    n_lib = len([p for p in lib_dst.glob("libmediaservo_*") if p.is_file() or p.is_symlink()])
    print(f"bindings 交付布局组装: out/bindings/（lib {n_lib} 件 + include/mediaservo + pkgconfig + cmake + python + wheel + node）")


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


def _derive_brand(bin_dir: Path) -> str:
    """从已装/组装 bin 目录推导品牌（三连停/init 名/快捷名全用它——禁止参数与硬编码）。
    规则: 存在 <brand>-agent（任意品牌化角色）→ 该前缀；否则空（官方名 host-*）。
    已知限制: 多品牌共存时 glob 顺序文件系统相关（取首个）——当前部署单品牌；若多品牌需改排序/最长前缀策略。"""
    for p in bin_dir.glob("*-agent"):
        name = p.name
        if name.startswith("host-"):
            continue
        return name[: -len("-agent")]
    return ""


def _cmd_deploy_host(prefix: str, release: bool = False) -> None:
    """deploy host（有状态部署——源 out/host 已组装）：identity 幂等/oxmgr/systemd/env.sh。
    package host 复用（Momus b——Task 2.5）: staging 目录内跑本函数组装身份/etc/env.sh；
    三连停不触发（staging 无 run/oxfile.toml——cli F5 只读安全）。

    D4: --prefix 必填（无默认——防污染 out/ 无状态交付；车端用 /opt/mediaservo-host）。
    品牌全链经 _derive_brand(src) 推导（deploy-ops 高危②/arch HIGH1——禁参数与字面 hardcode）。"""
    if not prefix:
        print("错误: deploy 必须 --prefix（无默认——防止污染 out/ 无状态交付；车端用 /opt/mediaservo-host）", file=sys.stderr)
        sys.exit(2)
    src = _out_root() / "host" / "bin"
    if not src.is_dir():
        print(f"错误: {src} 无组装产物 — 先 build host", file=sys.stderr)
        sys.exit(1)
    brand = _derive_brand(src)
    prefix_p = Path(prefix)
    bin_dir = prefix_p / "bin"
    try:
        bin_dir.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        print(f"错误: 无法创建 {bin_dir}: {e} — 部署用 sudo msrtc.sh deploy host --prefix /opt/mediaservo-host", file=sys.stderr)
        sys.exit(1)
    if str(prefix_p).startswith("/opt") and hasattr(os, "geteuid") and os.geteuid() != 0:
        print("错误: /opt 部署需 root（sudo msrtc.sh deploy host --prefix /opt/mediaservo-host）", file=sys.stderr)
        sys.exit(1)
    host_cli = _exe_name(f"{brand}-host") if brand else _exe_name("mediaservo-host")

    # 运行中的实例二进制被占用（Text file busy）→ 复制前自动停进程族。
    # 注意: 不调 oxmgr CLI（其 IPC 命令在 daemon 未跑时自动拉起 daemon——反而制造 busy）；
    # 直接 kill host 进程族 + 占用 bin/oxmgr 的 daemon。
    oxfile = prefix_p / "run" / "oxfile.toml"
    if oxfile.exists() and (bin_dir / host_cli).exists():
        print("检测到运行中的 host 实例 — 先停进程族（重装不覆盖 etc/ 凭据）")
        # 复活源三连停: ① systemd 自启 unit（Restart=always 会在 daemon 被杀后立即拉起）
        # ② daemon 自身 ③ host 进程族（restart_policy=always 由 daemon 重启——必须先杀 daemon）
        # systemctl 不接受 glob——按推导 brand 只枚举 oxmgr-{brand}-*.service（deploy-ops 高危②）
        units_dir = Path.home() / ".config" / "systemd" / "user"
        if shutil.which("systemctl") and units_dir.is_dir():
            for u in sorted(units_dir.glob(f"oxmgr-{brand}-*.service")):
                subprocess.run(["systemctl", "--user", "stop", u.name], check=False)
                subprocess.run(["systemctl", "--user", "reset-failed", u.name], check=False)
        _kill_using(bin_dir / _exe_name("oxmgr"))
        for base in ("agent", "streamer", "capturer", "recorder",
                     "controller", "emergency", "audio"):
            subprocess.run(["pkill", "-x", _exe_name(f"{brand}-{base}" if brand else f"host-{base}")], check=False)
        time.sleep(1)

    # 拷贝组装产物（out/host/bin 全件——build 已物理改名 <brand>-*；含 host-streamer 兼容链接 + client）
    src_names = set()
    for f in sorted(src.iterdir()):
        if not (f.is_file() or f.is_symlink()):
            continue
        src_names.add(f.name)
        _copy_with_kill(f, bin_dir / f.name)

    # bin 白名单（deploy-ops ④ 品牌切换残留清理）：非当前布局的 app 二进制删除
    # （build 已物理改名——残留 = 旧品牌/旧 install 遗留）
    roles = ("agent", "streamer", "capturer", "recorder", "controller", "emergency", "audio", "legacy", "host")
    for p in sorted(bin_dir.iterdir()):
        if p.is_symlink() or not p.is_file():
            continue
        if p.name in src_names or p.name == _exe_name("oxmgr"):
            continue
        if any(p.name.endswith(f"-{r}") for r in roles):
            print(f"  清理部署残留: {p.name}")
            p.unlink()

    # oxmgr 随部署锁定版本（D-H13）: PATH 找到则复制打包；缺 → 清晰指引（非致命,
    # 运行时 PATH 缺 oxmgr 由 `host doctor` 检出）
    oxmgr_src = shutil.which("oxmgr")
    if oxmgr_src is not None:
        _copy_with_kill(oxmgr_src, bin_dir / _exe_name("oxmgr"))
        ver = subprocess.run([oxmgr_src, "--version"], capture_output=True, text=True, check=False)
        print(f"  oxmgr 已打包: {ver.stdout.strip() or ver.stderr.strip() or '?'}")
    else:
        print("错误: PATH 未找到 oxmgr — 未打包（运行时需它拉起进程）。安装: npm install -g oxmgr（https://github.com/Vladimir-Urik/OxMgr#install），或构建 oxmgr-src 后放 ~/.local/bin，再重跑 deploy host", file=sys.stderr)

    # host init: 生成 etc/ + identity.json + 令牌——幂等只写不删（已存在跳过, 重装保留凭据）
    # brand: env 注入（init 是独立进程——device 前缀需与布局品牌一致）
    identity_file = prefix_p / "identity.json"
    if identity_file.exists():
        print(f"  identity.json 已存在——只写不删（设备身份保留）: {identity_file}")
    init_env = dict(os.environ)
    if brand:
        init_env["MEDIASERVO_BRAND"] = brand
    _run_or_exit([str(bin_dir / host_cli), "init", str(prefix_p)], env=init_env)

    # 快捷名（arch MED4：build 已物理改名为 <brand>-host——快捷指向品牌真实名；官方名回退 host/mediaservo-host）
    if brand:
        shortcut_names = (f"{brand}-host",)
        link_target = _exe_name(f"{brand}-host")
    else:
        shortcut_names = ("host", "mediaservo-host")
        link_target = _exe_name("mediaservo-host")
    for link_name in shortcut_names:
        link = prefix_p / link_name
        if link.exists():
            link.unlink()
        try:
            link.symlink_to(f"bin/{link_target}")  # 相对路径，前缀可搬迁
            print(f"  已创建符号链接 {link} → bin/{link_target}")
        except OSError:
            shutil.copy2(bin_dir / link_target, link)
            print(f"  已复制 {link_target} 到 {link}（符号链接失败，回退到拷贝）")

    # env.sh 一键激活（arch MED5 承接——deploy 生成；可搬迁：相对自身推导，非硬编码路径）
    env_host = f"{brand}-host" if brand else "mediaservo-host"
    env_sh = prefix_p / "env.sh"
    env_lines = [
        "#!/usr/bin/env bash",
        f"# {env_host} 环境激活（deploy 生成）——source 后全部命令可用",
        "# 用法: source env.sh（任意前缀/搬迁后依然有效，相对本文件推导）",
        '__MSRTC_ENV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"',
        'export PATH="${__MSRTC_ENV_DIR}/bin:$PATH"',
    ]
    if brand:
        env_lines.append(f'export MEDIASERVO_BRAND="{brand}"   # 品牌 env（D252），不覆盖用户预设')
    env_lines += [
        'BIN_DIR_MSRTC="${__MSRTC_ENV_DIR}/bin"   # 供脚本/别名引用',
        "unset __MSRTC_ENV_DIR",
        f'echo "[{brand or "mediaservo"}] {env_host} 环境已激活 — bin/: $BIN_DIR_MSRTC"',
    ]
    env_sh.write_text("\n".join(env_lines) + "\n")
    env_sh.chmod(0o755)
    print(f"  env.sh 已生成: {env_sh}（source 后 PATH=bin/ 全部命令可用）")

    for d in ("run/logs", "recordings"):
        (prefix_p / d).mkdir(parents=True, exist_ok=True)

    print(f"host 已部署到 {prefix}（{'release' if release else 'debug'}）")
    print(f"  bin/    {', '.join(sorted(src_names))} + oxmgr")
    print("  etc/    host.toml + link/{signing.pem,*.token}（host init 生成, 重装保留）")
    print("  run/    logs/")
    print("  recordings/")
    print("  identity.json（0600, 设备身份 — 只写不删, 勿删）")
    print(f"使用: export PATH={bin_dir}:$PATH && {host_cli} doctor {prefix} && {host_cli} start {prefix}")
    if sys.platform == "win32":
        print("注意: Windows best-effort（未全面验证）— 生产部署建议 Linux 车端", file=sys.stderr)


def _platform_tag() -> str:
    """wheel 平台 tag（Linux x86_64 → linux_x86_64；其余平台 best-effort）。"""
    import platform
    mach = platform.machine().lower().replace("-", "_")
    return f"{platform.system().lower()}_{mach}"


def _cmd_deploy_bindings(prefix: str, release: bool = False) -> None:
    """从 out/bindings 部署完整 SDK 布局（D241 三件套 version-full + include C/cxx + .pc + cmake
    + python/wheel + node——build 已组装，deploy 纯拷贝）。SDK 品牌无关（libmediaservo_* 原样，D3）。
    D4: --prefix 必填（无默认——防止污染 out/ 无状态交付；SDK 用 /opt/mediaservo-sdk）。"""
    if not prefix:
        print("错误: deploy 必须 --prefix（无默认——防止污染 out/ 无状态交付；SDK 用 /opt/mediaservo-sdk）", file=sys.stderr)
        sys.exit(2)
    src = _out_root() / "bindings"
    if not (src / "lib").is_dir():
        print(f"错误: {src} 无组装产物 — 先 build bindings", file=sys.stderr)
        sys.exit(1)
    prefix_p = Path(prefix)
    try:
        prefix_p.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        print(f"错误: 无法创建 {prefix_p}: {e} — 部署用 sudo msrtc.sh deploy bindings --prefix /opt/mediaservo-sdk", file=sys.stderr)
        sys.exit(1)
    lib_dir = prefix_p / "lib"
    lib_dir.mkdir(parents=True, exist_ok=True)
    sos = []
    for so in sorted((src / "lib").glob("libmediaservo_*.so*")):
        _copy_with_kill(so, lib_dir / so.name)  # busy（运行中加载）→ 杀占用重试
        sos.append(so.name)
    # lib/ 内子目录（pkgconfig/cmake）——.so 已单独 busy 重试，子目录直接整树复制
    for sub in sorted((src / "lib").iterdir()):
        if sub.is_dir():
            shutil.copytree(sub, lib_dir / sub.name, dirs_exist_ok=True)
    # 其余（include/.pc/cmake/python/wheel/node）整树复制——.so 已单独 busy 重试
    for e in sorted(src.iterdir()):
        if e.name == "lib":
            continue
        dst = prefix_p / e.name
        if e.is_dir():
            shutil.copytree(e, dst, dirs_exist_ok=True)
        elif e.is_file() or e.is_symlink():
            shutil.copy2(e, dst, follow_symlinks=False)
    print(f"bindings 已部署到 {prefix}（lib/ {', '.join(sos)} + include/mediaservo + pkgconfig + cmake + python + wheel + node）")
    print("  使用: export LD_LIBRARY_PATH=<prefix>/lib && 链接库见 bindings 头文件；python: pip install <prefix>/wheel/*.whl")


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
    """package <target> — dist/<brand>-host|sdk-<ver>.tar.gz 双包发布（D-H13）。
    host: staging 内跑 _cmd_deploy_host(staging)（Momus 裁决 b——identity 初始/oxmgr 锁定/
    etc 模板/env.sh 文件组装；build 仍无状态）+ tar 含 bin 8/oxmgr/etc/identity/run/logs/recordings。
    bindings: staging 拷贝 out/bindings（完整 SDK 布局——D241 三件套 version-full/.pc/cmake/
    wheel/node/cxx 头，Task 2.5 补齐）→ sdk 包。
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
            _cmd_deploy_host(str(staging), args.release)  # brand 由 out/host/bin 推导
        else:
            _cmd_deploy_bindings(str(staging), args.release)
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
    按接口名过滤 docker 网桥(br-*)/虚拟接口(veth/virbr)；tun/vpn **保留**
    （VPN 组网隧道 IP 正是客户端可达路径——PIT-143 多 IP 公告语义，非过滤对象）。"""
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


def _compose_env(announced_ip: str | None = None) -> dict[str, str]:
    """docker compose 调用环境 — 确保 MEDIASERVO_SFU_ANNOUNCED_IP 有值。
    PIT-79: CLI 启动 server 时若未注入，mediasoup 公告 0.0.0.0 → 浏览器拉流失败。
    显式 --announced-ip（参数/env）优先，否则自动探测宿主机全部真实 IP（逗号分隔，多网卡支持）。
    显式给值时跳过"自动探测"打印（T3 minor）。"""
    env = {**os.environ}
    if announced_ip:
        env["MEDIASERVO_SFU_ANNOUNCED_IP"] = announced_ip
    elif not env.get("MEDIASERVO_SFU_ANNOUNCED_IP"):
        ips = _detect_announced_ips()
        if ips:
            env["MEDIASERVO_SFU_ANNOUNCED_IP"] = ",".join(ips)
            print(f"MEDIASERVO_SFU_ANNOUNCED_IP 自动探测: {env['MEDIASERVO_SFU_ANNOUNCED_IP']}")
    return env


def _cmd_start(args: argparse.Namespace) -> None:
    """start <target> — server: 裸机 native（默认——用户裁决 B）| compose（--mode compose/--env）；host: 多进程推流。"""
    target = args.target
    if target == "server":
        mode = _resolve_mode(args, default="native")   # 默认 native（翻转：原 compose 移入 --mode compose）
        if mode == "native":
            _run_server_native(args)
        else:
            _check("docker", "安装 docker 并启动 daemon")
            cmd = COMPOSE_BASE + (["up"] if args.foreground else ["up", "-d", "server"])
            _run_or_exit(cmd, env=_compose_env())
    elif target == "host":
        if args.foreground:
            if args.legacy:
                _run_host_foreground(_find_host_binary())
            else:
                print("多进程 host 由 oxmgr 守护（无前台模式）— 去掉 --foreground 或用 --legacy", file=sys.stderr)
                sys.exit(1)
        else:
            _cmd_run_host(legacy=args.legacy)
    else:  # client
        print("start client: 待实现（client 骨架阶段）", file=sys.stderr)
        sys.exit(1)


def _cmd_restart(args: argparse.Namespace) -> None:
    target = args.target
    """restart <target> — server: 默认 native（B 裁决）；--mode compose=容器重启；host: 多进程重启。"""
    if target == "server":
        mode = _resolve_mode(args, default="native")
        if mode == "native":
            # 停裸机（stop native 语义）+ 重启
            pid_file = _native_runtime_dirs()[1] / "server-native.pid"
            if pid_file.exists():
                try:
                    pid = int(pid_file.read_text().strip())
                except ValueError:
                    pid = -1
                if pid > 0 and Path(f"/proc/{pid}").exists():
                    print(f"重启 server: 停止裸机进程 pid={pid}")
                    subprocess.run(["kill", str(pid)], check=False)
                    for _ in range(4):
                        if not Path(f"/proc/{pid}").exists():
                            break
                        time.sleep(0.5)
                    if Path(f"/proc/{pid}").exists():
                        subprocess.run(["kill", "-9", str(pid)], check=False)
                pid_file.unlink(missing_ok=True)
            _run_server_native(args)
        else:
            _check("docker", "安装 docker 并启动 daemon")
            print("重启 server: 停止旧容器...")
            subprocess.run(COMPOSE_BASE + ["down"], check=False, env=_compose_env())  # 无容器时忽略错误
            _run_or_exit(COMPOSE_BASE + ["up", "-d", "server"], env=_compose_env())
            print("✓ server 已重启（容器）")
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


def _cmd_run(args: argparse.Namespace) -> None:
    """run <target>: server=裸机（唯一模式——评审 H4 死参移除）| host=oxmgr 多进程。"""
    if args.target == "server":
        _run_server_native(args)
    elif args.target == "host":
        _cmd_run_host()
    else:
        print('run client: 待实现（client 骨架阶段）', file=sys.stderr)
        sys.exit(1)


def _out_root() -> Path:
    """发布根（MSRTC_OUT_ROOT env or 子模块 out/）——build 交付布局与 server 运行时共用。"""
    env_out = os.environ.get("MSRTC_OUT_ROOT", "")
    return Path(env_out) if env_out else ROOT / "out"


def _stage_to_out(target: str, files: list[Path], sub: str = "bin", brand: str = "") -> list[Path]:
    """拷贝 target/debug|release 产物到 out/<target>/<sub>（交付布局镜像）；brand 非空时物理重命名：host-<app> → <brand>-<app>、mediaservo-host → <brand>-host（D3：client/其他不品牌）。"""
    dst = _out_root() / target / sub
    dst.mkdir(parents=True, exist_ok=True)
    staged = []
    for src in files:
        if not src.exists():
            continue
        name = src.name
        if brand:
            if name.startswith("host-"):      # host-agent → msrtc-agent（D3：仅 host- 子进程品牌化）
                name = f"{brand}-{name[5:]}"
            elif name == "mediaservo-host":
                name = f"{brand}-host"
        shutil.copy2(src, dst / name)
        staged.append(dst / name)
    return staged


def _native_runtime_dirs() -> tuple[Path, Path, Path]:
    """native server 运行目录（MSRTC 发布壳注入 MSRTC_OUT_ROOT → ${out}/server；裸 CLI fallback 子模块 data/）。
    返回 (etc_dir, logs_dir, data_dir)——config/pid/log 全收敛于发布根 out/（主仓场景）或 data/（裸用）。"""
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


def _run_server_native(args: argparse.Namespace) -> None:
    """裸机跑 server：幂等（已在跑→提示跳过）；bin/../etc/server.yaml 默认配置 + 公告注入 + 端口守卫。"""
    # 幂等: start/restart/run 共用——pid 文件存活即已运行（防自家进程撞端口卫士）
    pid_file = _native_runtime_dirs()[1] / "server-native.pid"
    if pid_file.exists():
        try:
            alive_pid = int(pid_file.read_text().strip())
        except ValueError:
            alive_pid = -1
        if alive_pid > 0 and Path(f"/proc/{alive_pid}").exists():
            print(f"✓ server 裸机已在运行 pid={alive_pid}（server-native.pid）— 跳过启动")
            sys.exit(0)
    _check('cargo', 'pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat')
    # 优先 out/server/bin（build server 组装）→回退 target/（兼容未组装场景）
    bin_path = _out_root() / "server" / "bin" / "mediaservo-server"
    if not bin_path.exists():
        bin_path_fallback = ROOT / 'target' / ('release' if getattr(args, 'release', False) else 'debug') / 'mediaservo-server'
        if bin_path_fallback.exists():
            print("⚠ 使用 target/ 二进制（建议先: mediaservo build server — 组装到 out/server/bin）", file=sys.stderr)
            bin_path = bin_path_fallback
    if not bin_path.exists():
        print(f'错误: 未找到 {bin_path.relative_to(ROOT)} — 先运行: mediaservo build server --native', file=sys.stderr)
        sys.exit(1)
    # 端口冲突守卫: 裸机 9800/20000/40000-40100 与 dev/prod 容器并行会冲突
    _check_port_free(9800, '9800(HTTP)')
    _check_port_free(20000, '20000(SFU UDP)')
    _check_port_range_free(40000, 40100, '40000-40100(RTP)')
    # announced 注入: --announced-ip > env > CLI 探测(含 tun) > 不注入(server 侧 detect_all_ips 兜底)
    env = _compose_env(getattr(args, 'announced_ip', None))  # 复用探测（显式给值时跳过自动探测打印）
    print('⚠ 警告: 裸机跑 dev 轨道 config（psk=mediaservo-dev + 占位账号 admin123 等）——', file=sys.stderr)
    print('     仅限开发联调；生产部署用 up --env prod（entrypoint 自举随机密钥）', file=sys.stderr)
    # 裸机 config: 优先使用 bin/../etc/server.yaml（build server 组装时生成）
    # target/ 回退场景需 --config 显式指定
    bin_dir = bin_path.parent  # out/server/bin
    default_cfg = bin_dir.parent / "etc" / "server.yaml"  # out/server/etc/server.yaml
    if default_cfg.exists() and bin_dir == (_out_root() / "server" / "bin"):
        # 二进制在 out/server/bin/ → 等同 build server 组装 → 不需 --config
        cmd = [str(bin_path)]
    else:
        # target/ 回退: 生成临时 config 并 --config 传入
        etc_dir, _logs_dir, _data_dir = _native_runtime_dirs()
        native_cfg = etc_dir / 'server.native.yaml'
        native_cfg.parent.mkdir(parents=True, exist_ok=True)
        src_cfg = ROOT / 'config' / 'server.docker.yaml'
        if src_cfg.exists():
            cfg_text = src_cfg.read_text()
            cfg_text = cfg_text.replace('/opt/mediaservo/etc/accounts.yaml', str(ROOT / 'config' / 'accounts.yaml'))
            cfg_text = cfg_text.replace('/opt/mediaservo/etc/devices.yaml', str(ROOT / 'config' / 'devices.yaml'))
            native_cfg.write_text(cfg_text)
        cmd = [str(bin_path), '--config', str(native_cfg)]
    # dev 占位账号豁免（fail-fast 守卫——裸机=dev 联调，与 dev compose 的 ALLOW_DEV_CREDENTIALS=1 一致）
    env.setdefault('MEDIASERVO_ALLOW_DEV_CREDENTIALS', '1')
    log_path = _native_runtime_dirs()[1] / "server-native.log"
    # export 指引（AccessBase cmd_start_native L180 借鉴——PIT-79/138 公告闭环:
    # 后续终端操作需同一公告值，零新增探测——直接读生效 env）
    announced_val = env.get("MEDIASERVO_SFU_ANNOUNCED_IP", "")
    if announced_val:
        print(f"  export MEDIASERVO_SFU_ANNOUNCED_IP='{announced_val}'")
    if getattr(args, 'foreground', False):
        _run_or_exit(cmd, env=env)
    else:
        # I-1: 清 stale pid（上次崩溃残留）——防 stop 杀错回收 pid；写后覆盖语意保留
        pid_file = _native_runtime_dirs()[1] / "server-native.pid"
        pid_file.unlink(missing_ok=True)
        # T3 minor: 启动时 truncate（崩溃残留不污染，重启从头记）+ start_new_session（脱离终端，stop 按 pid 文件可杀）
        proc = subprocess.Popen(cmd, env=env, stdout=open(log_path, 'wb'), stderr=subprocess.STDOUT, start_new_session=True)
        pid_file.write_text(str(proc.pid))
        print(f'✓ server 裸机运行中 pid={proc.pid} — 日志: {log_path}（logs server --native）')


def _port_owner(port: int) -> str:
    """从 ss -tulnp 解析 :port 行的占用方（name,pid）。非 root 下容器端口无 users 段——如实提示（评审 ops#2）。"""
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
            return "占用方不可见（宿主 root/容器端口——sudo ss 查看）"
    except OSError:
        pass
    try:  # darwin: lsof 回退
        out = subprocess.run(["lsof", "-i", f":{port}"], capture_output=True, text=True, timeout=5, check=False).stdout
        for line in out.splitlines()[1:]:
            parts = line.split()
            if len(parts) >= 2:
                return f"{parts[0]} (pid {parts[1]})"
    except OSError:
        pass
    return "未知（ss/lsof 均不可用）"


def _check_port_free(port: int, name: str) -> None:
    """端口占用检查（TCP/UDP；占用则提示先停冲突方）。平台守卫 + 探测命令同平台分支（Momus MEDIUM：原版守卫选 lsof 但探测仍 ss——darwin traceback）。"""
    if sys.platform == "darwin":
        _check("lsof", "lsof 不可用")
        out = subprocess.run(["lsof", "-i", f":{port}"], capture_output=True, text=True, timeout=5, check=False).stdout
        busy = f":{port}" in out
    else:
        _check("ss", "ss 不可用——安装 iproute2")
        out = subprocess.run(["ss", "-tulnp"], capture_output=True, text=True, timeout=5, check=False).stdout
        busy = f":{port} " in out
    if busy:
        print(f"错误: {name} 端口被占: {_port_owner(port)} — 先运行: mediaservo stop server（裸机/容器皆可）", file=sys.stderr)
        sys.exit(1)


def _check_port_range_free(start: int, end: int, name: str) -> None:
    """RTP 范围扫描（mediasoup 惰性 bind——health 200 但传输间歇失败，必须显式查）。同平台分支——darwin lsof 回退。"""
    if sys.platform == "darwin":
        _check("lsof", "lsof 不可用")
        out = subprocess.run(["lsof", "-i", f":{start}-{end}"], capture_output=True, text=True, timeout=5, check=False).stdout
        busy = [p for p in range(start, end + 1) if f":{p}" in out]
    else:
        _check("ss", "ss 不可用——安装 iproute2")
        out = subprocess.run(["ss", "-tulnp"], capture_output=True, text=True, timeout=5, check=False).stdout
        busy = [p for p in range(start, end + 1) if f':{p} ' in out]
    if busy:
        print(f'错误: {name} 端口被占 {busy[:5]}... 首占者: {_port_owner(busy[0])} — 先停止占用进程', file=sys.stderr)
        sys.exit(1)


def _cmd_stop(args: argparse.Namespace) -> None:
    """stop <target>: server=默认双停（裸机 pid + compose stop）；--mode native/compose 限定单侧；host=优雅停止。"""
    target = args.target
    if target == "server":
        mode = "compose" if getattr(args, "env", None) is not None else _resolve_mode(args, default="both")
        # ① 裸机（pid 文件驱动幂等——both/native 时）
        if mode in ("both", "native"):
            pid_file = _native_runtime_dirs()[1] / "server-native.pid"
            if pid_file.exists():
                try:
                    pid = int(pid_file.read_text().strip())
                except ValueError:
                    pid = -1
                if pid > 0 and Path(f"/proc/{pid}").exists():
                    print(f"stop server: 停止裸机进程 pid={pid}")
                    subprocess.run(["kill", str(pid)], check=False)
                    for _ in range(4):
                        if not Path(f"/proc/{pid}").exists():
                            break
                        time.sleep(0.5)
                    if Path(f"/proc/{pid}").exists():
                        subprocess.run(["kill", "-9", str(pid)], check=False)
                pid_file.unlink(missing_ok=True)
        # ② 容器（保留 compose stop——秒级再启语义；both/compose 时）
        if mode in ("both", "compose"):
            _check("docker", "安装 docker 并启动 daemon")
            _run_or_exit(COMPOSE_BASE + ["stop", "server"])
    elif target == "host":
        subprocess.run(["pkill", "-x", "host-legacy"], check=False)
        host_cli = next((p for p in (ROOT / "target/debug/mediaservo-host", ROOT / "target/release/mediaservo-host") if p.exists()), None)
        if host_cli is not None:
            _run_or_exit([str(host_cli), "stop", str(ROOT)])
        print("✓ host 已停止")
    else:  # client
        subprocess.run(["pkill", "-x", "mediaservo-client"], check=False)
        print("✓ client 已停止")


def _cmd_logs(args: argparse.Namespace) -> None:
    """logs [<target>] [--follow] [--mode native|compose] — server: 裸机日志（默认——用户裁决 B）| compose 容器日志；host: 日志目录。"""
    target = args.target
    if target == "server":
        mode = _resolve_mode(args, default="native")
        if mode == "native":
            log_path = _native_runtime_dirs()[1] / "server-native.log"
            if not log_path.exists():
                print(f"错误: {log_path} 不存在 — 裸机 server 未运行？", file=sys.stderr)
                sys.exit(1)
            cmd = (["tail", "-f"] if args.follow else ["tail", "-n", "100"]) + [str(log_path)]
            _run_or_exit(cmd)
            return
        _check("docker", "安装 docker 并启动 daemon")
        _run_or_exit(COMPOSE_BASE + ["logs"] + (["-f"] if args.follow else []) + ["server"])
    elif target == "host":
        log_path = Path("/tmp/mediaservo-host.log")
        if log_path.exists():
            _run_or_exit(["tail", "-n", "100"] + (["-f"] if args.follow else []) + [str(log_path)])
        ox_logs = Path.home() / ".local/share/oxmgr/logs"
        print(f"host 日志目录: {ox_logs}（实例日志: OXMGR_DATA_DIR/run/logs/msrtc-streamer-*.err.log）", file=sys.stderr)
    else:  # client
        print("client 日志: 待实现（client 骨架阶段）", file=sys.stderr)
        sys.exit(1)


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


def _rm_path(path: Path) -> None:
    """删文件或目录（clean 通用——二进制文件用 unlink，目录用 rmtree）。"""
    try:
        if path.is_dir():
            shutil.rmtree(path)
        else:
            path.unlink(missing_ok=True)
    except FileNotFoundError:
        pass


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


def _cmd_deploy(args: argparse.Namespace) -> None:
    """deploy <target> — bindings | host（源=out/ 组装；--prefix 必填 D4）。"""
    if args.target == "bindings":
        _cmd_deploy_bindings(args.prefix, args.release)
    elif args.target == "host":
        _cmd_deploy_host(args.prefix, args.release)


def _cmd_install_deprecated(args: argparse.Namespace) -> None:
    """D1: install 已改名 deploy——提示迁移后退出（不 alias——语义不同: 源=out/ 交付布局 + --prefix 必填）。"""
    print(
        "install 已改名 deploy：源改为 out/ 交付布局（有状态部署——identity 幂等/oxmgr/systemd/env.sh），"
        "--prefix 必填。请用: mediaservo deploy host|bindings --prefix <前缀>（车端 host: --prefix /opt/mediaservo-host）",
        file=sys.stderr,
    )
    if getattr(args, "args", []):
        print(f"  （忽略旧参数: {' '.join(args.args)}）", file=sys.stderr)
    sys.exit(2)


def _cmd_clean(args: argparse.Namespace) -> None:
    """clean <target> — all|server|host|client（默认 all）。
    server: 默认双清（native 产物 + 容器 down）；--mode native/compose 限定；host/client: 清宿主 cargo target。"""
    target = args.target
    if target in ("all", "server"):
        mode = "compose" if getattr(args, "env", None) is not None else _resolve_mode(args, default="both")
        # native：先停裸机进程（读 pid 文件）再删产物（防孤儿——clean 曾留进程在跑 pid 已删）
        if mode in ("both", "native"):
            pid_file = _native_runtime_dirs()[1] / "server-native.pid"
            if pid_file.exists():
                try:
                    pid = int(pid_file.read_text().strip())
                except ValueError:
                    pid = -1
                if pid > 0 and Path(f"/proc/{pid}").exists():
                    print(f"clean server: 停止裸机进程 pid={pid}")
                    subprocess.run(["kill", str(pid)], check=False)
                    for _ in range(4):
                        if not Path(f"/proc/{pid}").exists():
                            break
                        time.sleep(0.5)
                    if Path(f"/proc/{pid}").exists():
                        subprocess.run(["kill", "-9", str(pid)], check=False)
            etc_d, logs_d, _ = _native_runtime_dirs()
            for f in (logs_d / "server-native.pid", logs_d / "server-native.log",
                     _out_root() / "server" / "bin" / "mediaservo-server",
                     ROOT / "target/debug/mediaservo-server", ROOT / "target/release/mediaservo-server"):
                _rm_path(f)
        # compose 容器（both/compose 时）
        if mode in ("both", "compose"):
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


def _cmd_env_diagnose() -> None:
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


def _cmd_status_runtime(args: argparse.Namespace) -> None:
    """status <target> — 健康探测（AccessBase cmd_status_native 借鉴，评审吸收）。
    退出码: 0=运行中 1=未运行 2=探测失败/参数错。ps=清单 vs status=健康结论（help 互引）。"""
    code = 2
    if args.target == "server":
        mode = _resolve_mode(args, default="native")   # 默认 native（用户裁决 B 翻转）
        code = _status_server_native() if mode == "native" else _status_server_container(args.env)
    elif args.target == "host":
        code = _status_host()
    else:
        print("status client: 待实现（client 骨架阶段）", file=sys.stderr)
    sys.exit(code)


def _curl_health_code() -> str:
    """curl -o /dev/null -w %{http_code} 9800（2s 超时）。"""
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
    """ss -tulnp 是否含 :port。None=无法判定（C15——评审 arch F4 假阴性修正）。"""
    try:
        out = subprocess.run(["ss", "-tulnp"], capture_output=True, text=True, timeout=5, check=False).stdout
        return f":{port} " in out
    except OSError:
        return None


def _status_server_native() -> int:
    """裸机运行态。返回 0/1。"""
    print("server (native):")
    pid_file = _native_runtime_dirs()[1] / "server-native.pid"
    running = False
    if pid_file.exists():
        try:
            pid = int(pid_file.read_text().strip())
        except ValueError:
            pid = 0
        alive = pid > 0 and Path(f"/proc/{pid}").exists()
        print(f"  pid 文件: {pid_file.name} {'存活 pid=' + str(pid) if alive else '(进程不在 — stale)'}")
        running = alive
    else:
        print("  pid 文件: 不存在（未运行）")
    code = _curl_health_code()  # 观察量——dev/prod 容器也占 9800，不作"裸机在跑"判据（pid 文件是 run/stop 契约）
    print(f"  health 9800: {'200 OK' if code == '200' else code or '无响应'}")
    av = _compose_env().get("MEDIASERVO_SFU_ANNOUNCED_IP", "")
    if av:
        print(f"  announced: {av}")   # C38 ①层观察量（评审 arch F3）
    probe_failed = False
    for port, name in ((20000, "SFU UDP"), (40000, "RTP 起"), (40100, "RTP 止")):
        st = _ss_listening(port)
        if st is None:
            probe_failed = True
        print(f"  {port} {name}: {'监听' if st else ('无法判定' if st is None else '空闲')}")
    # Momus LOW：探测失败契约——无法判定 → 退出码 2（优先于 0/1）
    return 2 if probe_failed else (0 if running else 1)


def _compose_running(env: str) -> bool:
    """env 是否有容器在跑（dev/prod 同容器名、compose ps 按目录聚合不分 env——
    按 config_files label 区分，_detect_running_env 同源）。"""
    try:
        out = subprocess.run(["docker", "ps", "--format", "{{.Names}}"],
                             capture_output=True, text=True, timeout=5, check=False).stdout
        if "mediaservo-server-1" not in out:
            return False
        return _detect_running_env() == env
    except OSError:
        return False


def _status_server_container(env_arg: str | None = None) -> int:
    """容器运行态。env_arg 复接 --env（评审 cli F1：不许 help-实现漂移）。"""
    code = _curl_health_code()
    env_now = env_arg or _detect_running_env()
    print(f"server (compose env={env_now}):", flush=True)  # flush: _run 子进程输出先于 print（pipe 缓冲）
    _check("docker", "安装 docker 并启动 daemon")
    _run(["docker", "compose", "-f", _compose_file(env_now), "ps"])  # 进程/容器清单（ps 子集——评审 arch F1）
    print(f"  health 9800: {'200 OK' if code == '200' else code or '无响应'}")
    av = _compose_env().get("MEDIASERVO_SFU_ANNOUNCED_IP", "")
    if av:
        print(f"  announced: {av}")   # C38 ①层观察量
    return 0 if _compose_running(env_now) else 1


def _status_host() -> int:
    """host 运行态。进程名双前缀（评审 ops#1 CRITICAL：品牌化 msrtc-* vs 官方 host-*）。"""
    print("host:")
    procs = ["agent", "streamer", "capturer"]  # 品牌化角色（双前缀——ops#1 CRITICAL）
    brands = ["msrtc", "host"]
    bare_procs = ["oxmgr"]  # Momus HIGH：oxmgr daemon 进程名是裸 `oxmgr`（npm 二进制 bin/oxmgr；
    # 品牌前缀仅作用于 translate 生成的 <brand>-<app>——agent/streamer/capturer 才带前缀）
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
            print(f"  {role}: 运行中 pid={' '.join(pids)}")
        else:
            print(f"  {role}: 未运行")
    st = _ss_listening(17980)
    if st is None:
        probe_failed = True
    print(f"  网关 17980: {'监听' if st else ('无法判定' if st is None else '空闲')}")
    # 实例日志路径（评审 ops#5/docs#3：C32 实例隔离——不硬编码占位符）
    inst = os.environ.get("OXMGR_DATA_DIR", "")
    log_hint = (Path(inst).parent / "run" / "logs") if inst else (ROOT.parent / "out" / "host" / "run" / "logs")
    print(f"  日志: {log_hint}/msrtc-streamer-<stream>.err.log（C38 ②层：grep 订阅/acl denied/OpenH264）")
    if not any_alive:
        print("  → host 未运行：./mediaservo.sh start host（或 restart host）", file=sys.stderr)
    return 2 if probe_failed else (0 if any_alive else 1)


def _add_mode_args(p, *, env_choices=("dev", "prod")):
    """统一模式参数（--mode native|compose + 短别名 --native/--env；三选一互斥）。
    语义: 命令默认 native（用户裁决 B）——compose 需显式 --mode compose / --env。"""
    grp = p.add_mutually_exclusive_group()
    grp.add_argument("--mode", choices=["native", "compose"], default=None,
                     help="运行模式: native=裸机（默认）| compose=容器（--env 同效）")
    grp.add_argument("--native", action="store_true", help="（--mode native 短别名——默认模式，可省略）")
    grp.add_argument("--env", choices=list(env_choices), default=None,
                     help="（--mode compose 短别名 = 进程族容器模式；compose 族 up/down/ps 的 --env 是环境选择——两套语义）")
    p.set_defaults(_resolve_mode_into=None)


def _resolve_mode(args, default: str = "native") -> str:
    """统一解析 args → mode（--mode > --native/--env > default）。"""
    if getattr(args, "mode", None):
        return args.mode
    if getattr(args, "native", False):
        return "native"
    if getattr(args, "env", None) is not None:
        return "compose"
    return default


def _compose_file(env: str) -> str:
    """环境 → compose 文件映射（up/down/ps/exec 共用）。"""
    return "docker-compose.yml" if env == "prod" else "docker-compose.dev.yml"


def _cmd_up(args: argparse.Namespace) -> None:
    """部署生命周期: up [--env dev|prod] [svc]——prod=单容器+命名卷+entrypoint 自举。"""
    cf = _compose_file(args.env)
    cmd = ["docker", "compose", "-f", cf, "up", "-d", args.svc]
    if args.build:
        cmd.insert(5, "--build")
    env = _compose_env(args.announced_ip)
    if args.announced_ip:
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
    """环境诊断（工具链版本——原 status 更名，与运行态探测 status 区分）。"""
    _cmd_env_diagnose()


def _cmd_version() -> None:
    print(VERSION)


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="mediaservo",
        description="MediaServo 统一构建 CLI（术语: native=裸机, compose=容器）",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""三模式速查（native=裸机 compose=容器）:
  模式① 本机原生:  build server --native → run server → status server → stop server
  模式② 单容器生产: build server --image runtime → up --env prod → logs server --env prod（--native 并行另一套）
  模式③ compose开发: up --env dev（热更）→ logs -f → down --env dev
  up        = compose 部署（模式②③）；run/start = 进程（模式①：run server=裸机；start server --mode compose=容器）
  退出码    : status/logs/stop/start —— 0=成功 1=未运行/目标缺失 2=参数错/互斥
""")

    sub = parser.add_subparsers(dest="command", required=True)

    build_p = sub.add_parser("build", help="构建 <target> [--image runtime|dev]: all|host|server|client|bindings（默认 all；server 默认 native 编译，--image 才走 Docker）")
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

    restart_p = sub.add_parser("restart", help="重启 <target>: server=裸机（默认）| compose（--mode）| host 进程")
    restart_p.add_argument("target", choices=["host", "server"])
    _add_mode_args(restart_p)
    restart_p.set_defaults(func=_cmd_restart)
    run_p = sub.add_parser("run", help="运行 <target> [--announced-ip IP[,IP...]]: server=裸机（唯一模式）| host 多进程")
    run_p.add_argument("target", choices=["server", "host"])
    run_p.add_argument("--foreground", "-f", action="store_true", help="前台阻塞运行，输出实时透传")
    run_p.add_argument("--release", action="store_true", help="用 target/release 二进制")
    run_p.add_argument("--announced-ip", metavar="IP[,IP...]", default=None,
                       help="裸机公告地址（覆盖自动探测——默认自动含 tun/vpn、ens 全部真实 IP，符合 PIT-143 多网卡语义）")
    run_p.set_defaults(func=_cmd_run)

    start_p = sub.add_parser("start", help="启动 <target>（默认 server）: server=native（默认）| compose（--mode）| host 多进程")
    start_p.add_argument("target", nargs="?", choices=["host", "server"], default="server")
    _add_mode_args(start_p)
    start_p.add_argument("--foreground", "-f", action="store_true", help="前台阻塞（host 仅 --legacy 支持）")
    start_p.add_argument("--legacy", action="store_true", help="host 回退旧单进程 host-legacy")
    start_p.set_defaults(func=_cmd_start)

    stop_p = sub.add_parser("stop", help="停止 <target>: server=双停（默认）| --mode 限定；host=优雅停止")
    stop_p.add_argument("target", choices=["host", "server"])
    _add_mode_args(stop_p)
    stop_p.set_defaults(func=_cmd_stop)

    logs_p = sub.add_parser("logs", help="日志 [<svc>] [--follow] [--mode native|compose]: server=裸机日志（默认）| compose 容器日志 | host 日志")
    logs_p.add_argument("target", nargs="?", choices=["server", "host"], default="server")
    _add_mode_args(logs_p)
    logs_p.add_argument("--follow", "-f", action="store_true", help="跟踪输出")
    logs_p.set_defaults(func=_cmd_logs)

    ps_p = sub.add_parser("ps", help="运行态: compose ps（server 部署——按 --env）")
    ps_p.add_argument("--env", choices=["dev", "prod"], default="dev")
    ps_p.set_defaults(func=_cmd_ps)

    exec_p = sub.add_parser("exec", help="容器内执行: exec <svc> -- <cmd>（compose exec——调试用）")
    exec_p.add_argument("svc", help="服务名（server/proxy）")
    exec_p.add_argument("cmd", nargs=argparse.REMAINDER, help="容器内命令（-- 后）")
    exec_p.set_defaults(func=_cmd_exec)

    e2e_p = sub.add_parser("e2e", help="测试套件: sfu|push|ui|host|package|brand|bindings|client|smoke")
    e2e_p.add_argument("suite", choices=["sfu", "push", "ui", "host", "package", "brand", "bindings", "client", "smoke"])
    e2e_p.set_defaults(func=_cmd_e2e)

    sub.add_parser("test", help="workspace 测试（排除 mediaservo-server）")
    sub.add_parser("ci", help="CI 全链: fmt → clippy → test → e2e sfu")

    deploy_p = sub.add_parser("deploy", help="部署 <target>（有状态：identity 幂等/oxmgr/systemd）：host|bindings——源=out/；--prefix 必填（/opt 需 root）")
    deploy_p.add_argument("target", choices=["bindings", "host"])
    deploy_p.add_argument("--prefix", default=None, help="部署前缀（必填——车端 /opt/mediaservo-host）")
    deploy_p.add_argument("--release", action="store_true", help="部署 release 组装产物")
    deploy_p.set_defaults(func=_cmd_deploy)
    # D1: install 隐藏 prompt（不 alias——语义不同: 源=out/ 交付布局 + --prefix 必填）
    install_p = sub.add_parser("install", help="已改名 deploy（提示迁移后退出 exit 2）")
    install_p.add_argument("args", nargs=argparse.REMAINDER, help="吞掉旧调用点透传参数（T3 迁移前 msrtc.sh 仍注入 --prefix 等）——仅提示改名")
    install_p.set_defaults(func=_cmd_install_deprecated)
    package_p = sub.add_parser("package", help="打包 <target>: host（车端包）| bindings（SDK 包）→ dist/mediaservo-<target>-<ver>.tar.gz（D-H13 双包发布, 含版本契约文件）")
    package_p.add_argument("target", choices=["host", "bindings"])
    package_p.add_argument("--brand", default="", help="品牌包名（dist/<brand>-host-<ver>.tar.gz；缺省 mediaservo-host-<ver>）")
    package_p.add_argument("--release", action="store_true", help="打包 release 产物（target/release, 配合 build --release）")
    package_p.set_defaults(func=_cmd_package)
    clean_p = sub.add_parser("clean", help="清理 <target>: all|server|host|client（默认 all；server --mode native/compose 限定）")
    clean_p.add_argument("target", nargs="?", choices=["all", "server", "host", "client"], default="all")
    _add_mode_args(clean_p)
    clean_p.add_argument("--all", action="store_true", help="显式删卷 + docker builder prune（15-30 分钟重建代价）")
    clean_p.set_defaults(func=_cmd_clean)
    config_p = sub.add_parser("config", help="配置 show|validate|set <key> <value>（dev 轨道 config/ 目录）")
    config_p.add_argument("config_cmd", choices=["show", "validate", "set"])
    config_p.add_argument("key", nargs="?", help="set: 配置键（如 signaling.psk）")
    config_p.add_argument("value", nargs="?", help="set: 配置值")
    config_p.set_defaults(func=_cmd_config)
    data_p = sub.add_parser("data", help="数据卷: backup [<dir>]| reset | inspect")
    data_p.add_argument("data_cmd", choices=["backup", "reset", "inspect"])
    data_p.add_argument("dir", nargs="?", help="backup: 目标目录（默认 ./backup）")
    data_p.add_argument("--force", action="store_true", help="reset: 跳过交互确认")
    data_p.set_defaults(func=_cmd_data)
    doctor_p = sub.add_parser("doctor", help="环境诊断（pixi/cargo/docker/node——原 status）")
    doctor_p.set_defaults(func=_cmd_doctor)
    status_p = sub.add_parser("status", help="健康探测 <target>（退出码 0/1/2）: server=裸机（默认）| compose 容器（--mode compose）| host 推流进程")
    status_p.add_argument("target", choices=["server", "host"])
    _add_mode_args(status_p)
    status_p.set_defaults(func=_cmd_status_runtime)

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
    if args.command == "build":
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
    elif args.command == "run":
        _cmd_run(args)
    elif args.command == "start":
        _cmd_start(args)
    elif args.command in ("stop", "restart", "logs"):
        globals()[f"_cmd_{args.command}"](args)
    elif args.command == "status":
        _cmd_status_runtime(args)

    elif args.command in ("test", "ci"):
        globals()[f"_cmd_{args.command}"]()
    elif args.command == "version":
        _cmd_version()
    elif hasattr(args, "func"):
        args.func(args)


if __name__ == "__main__":
    main()
