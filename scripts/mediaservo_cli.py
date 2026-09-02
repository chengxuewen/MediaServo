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
    """build host — 纯编译到 target/（组装/品牌化迁 deploy——D266）。"""
    _check("cargo", "pixi 环境未激活? 先运行: source bootstrap.sh / pixi.bat")
    _run_or_exit(["cargo", "build", "-p", "mediaservo-host", "-p", "mediaservo-client"])
    print("✓ host 构建完成（纯编译 target/——组装与品牌化在 deploy: mediaservo deploy host --prefix <X>）")

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
        print(f"admin dist 最新（{dist_index.parent.relative_to(ROOT)}——src 无变更）— 跳过前端构建，复用既有 dist")
        return
    _check("pnpm", "pnpm 未安装——前端构建需要（或先手动 cd www && pnpm build:admin）")
    print("admin dist 过期/缺失 — 构建前端（tsc -b && vite build）...")
    _run_or_exit(["pnpm", "build:admin"], cwd=str(ROOT / "www"))


def _stage_web_to_out() -> None:
    """dist → out/server/web 合并树（frontend-process-split T2——web 唯一消费方=server 部署单元，
    单树=单 tar=单 deploy；Caddy 静态 root 指此）。"""
    src = ROOT / "www" / "apps" / "admin" / "dist"
    dst = _out_root() / "server" / "web"
    if not (src / "index.html").exists():
        print(f"错误: {src} 无产物——pnpm build:admin 未执行或失败", file=sys.stderr)
        sys.exit(2)
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst)
    n = sum(1 for x in dst.rglob("*") if x.is_file())
    print(f"web 交付物组装: out/server/web/（{n} 文件——Caddy 静态 root）")


def _cmd_build_web() -> None:
    """build web — 纯前端快速通道：mtime 增量 pnpm → out/server/web（不碰 cargo）。"""
    _ensure_admin_dist()
    _stage_web_to_out()


def _web_pid_file() -> Path:
    return _native_runtime_dirs()[1] / "web-native.pid"


def _run_web_native() -> None:
    """run web — 过渡形态：独立 caddy 静态+反代理（deploy/caddy/Caddyfile.native）。
    Phase 6 后 web 归 msrtc-server 的 oxmgr 进程簇统管（T17），本命令退役。"""
    pid_file = _web_pid_file()
    if pid_file.exists():
        try:
            alive = int(pid_file.read_text().strip())
        except ValueError:
            alive = -1
        if alive > 0 and Path(f"/proc/{alive}").exists():
            print(f"✓ web(caddy) 已在运行 pid={alive} — 跳过启动")
            sys.exit(0)
    if shutil.which("caddy") is None:
        print("错误: caddy 不在 PATH——安装 GitHub Releases 预编译二进制"
              "（https://github.com/caddyserver/caddy/releases，sha512 见 checksums.txt），"
              "禁止自研静态服务器", file=sys.stderr)
        sys.exit(2)
    out_server = _out_root() / "server"
    if not (out_server / "web" / "index.html").exists():
        print(f"错误: {out_server / 'web'} 无产物——先 mediaservo build web", file=sys.stderr)
        sys.exit(2)
    cfg = ROOT / "deploy" / "caddy" / "Caddyfile.native"
    log_path = _native_runtime_dirs()[1] / "web-native.log"
    with open(log_path, "ab") as lf:
        proc = subprocess.Popen(
            ["caddy", "run", "--config", str(cfg), "--adapter", "caddyfile"],
            cwd=str(out_server), stdout=lf, stderr=lf,
            stdin=subprocess.DEVNULL, start_new_session=True)
    pid_file.write_text(str(proc.pid))
    print(f"✓ web 运行中 pid={proc.pid} — http://localhost:8080（静态 root={out_server / 'web'}，"
          f"反代 127.0.0.1:9800；日志 {log_path}）")


def _stop_web_native(allow_inactive: bool = False) -> None:
    pid_file = _web_pid_file()
    pid = -1
    if pid_file.exists():
        try:
            pid = int(pid_file.read_text().strip())
        except ValueError:
            pid = -1
    if pid > 0 and Path(f"/proc/{pid}").exists():
        subprocess.run(["kill", str(pid)], check=False)
        for _ in range(4):
            if not Path(f"/proc/{pid}").exists():
                break
            time.sleep(0.5)
        if Path(f"/proc/{pid}").exists():
            subprocess.run(["kill", "-9", str(pid)], check=False)
        print(f"web(caddy) 已停止 pid={pid}")
    elif not allow_inactive:
        print("web 未在运行（pid 文件缺失或进程已亡）", file=sys.stderr)
        pid_file.unlink(missing_ok=True)
        sys.exit(1)
    pid_file.unlink(missing_ok=True)


def _status_web_native() -> int:
    """退出码 0=运行中且入口 200 / 1=未运行或不可达 / 2=探测异常。"""
    pid_file = _web_pid_file()
    if not pid_file.exists():
        print("web: 未运行（无 pid 文件）")
        return 1
    try:
        pid = int(pid_file.read_text().strip())
    except ValueError:
        print("web: pid 文件损坏")
        return 2
    if not Path(f"/proc/{pid}").exists():
        print(f"web: 未运行（pid={pid} 已亡）")
        return 1
    try:
        code = subprocess.run(
            ["curl", "-s", "--noproxy", "*", "-o", "/dev/null", "-w", "%{http_code}",
             "--max-time", "2", "http://127.0.0.1:8080/"],
            capture_output=True, text=True, timeout=5, check=False).stdout.strip()
    except Exception as e:
        print(f"web: 探测失败 {e}", file=sys.stderr)
        return 2
    print(f"web: 运行中 pid={pid} 入口 http={code or '—'}")
    return 0 if code == "200" else 1


def _cmd_build_server(image: str | None = None, native: bool = False, release: bool = False) -> None:
    """build server: 默认 native（用户裁决 B——不写模式=原生）| --image runtime|dev=Docker 镜像（--native 兼容别名）。"""
    if native or image is None:   # 默认 native；--image 显式才走 Docker
        _ensure_admin_dist()      # mtime 增量前端构建（Docker 路径 Dockerfile 内自理）
        _stage_web_to_out()       # T3 后 default=不嵌入，dist 以文件树进交付物（out/server/web）
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
            _stage_to_out("server", [server_bin], sub="bin")  # brand=""（staging 保持 cargo 名——品牌化归 deploy，D266/D269）
            print(f"server 交付布局组装: out/server/bin/mediaservo-server（{server_bin.stat().st_size // 1024} KB）")
        # 组装默认配置（server.yaml——从 config/server.docker.yaml 派生，accounts/devices 相对路径）
        # PIT-158/160: 已存在则跳过（运行时注册表 devices/accounts 可能被 admin API 热写、
        # server.yaml 可能被运维改过）——build 不得把模板覆盖回运行时数据；需要新模板先删旧文件。
        etc_dir = _out_root() / "server" / "etc"
        etc_dir.mkdir(parents=True, exist_ok=True)
        src_cfg = ROOT / "config" / "server.docker.yaml"
        if src_cfg.exists() and not (etc_dir / "server.yaml").exists():
            cfg = src_cfg.read_text()
            # accounts/devices 改为相对于 etc 目录（打包后 bin/../etc/accounts.yaml 等）
            cfg = cfg.replace('/opt/mediaservo/etc/accounts.yaml', 'accounts.yaml')
            cfg = cfg.replace('/opt/mediaservo/etc/devices.yaml', 'devices.yaml')
            (etc_dir / "server.yaml").write_text(cfg)
        if src_cfg.exists():
            # 拷贝设备/账号文件（dev 模板——仅首次缺失时补）
            import shutil
            for f in ("accounts.yaml", "devices.yaml"):
                src = ROOT / "config" / f
                dst = etc_dir / f
                if src.exists() and not dst.exists():
                    shutil.copy2(src, dst)
            print(f"server 默认配置组装: {etc_dir}/server.yaml")
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
    if target == "web":   # 纯前端快速通道（all 不含——build server 已带 web 装配）
        _cmd_build_web()
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



def _pids_using(path: Path) -> list[int]:
    """占用 path 的进程 pid：① fd 打开 ② /proc/pid/exe==realpath（exec 内存映射占用）。
    精确匹配——禁 basename/cmdline 模糊匹配（同签名兄弟进程会被连坐，多实例机器上即生产事故
    ——PIT-166；2026-08-31 deploy drill 实证旧 basename 匹配可波及他实例 daemon）。"""
    target = os.path.realpath(path)
    pids: list[int] = []
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
                            pids.append(pid)
                            break
                    except OSError:
                        continue
                if pid in pids:
                    continue
                # ② exe 符号链接（exec 占用——内核记录的就是这个 inode，无假阳性）
                if os.path.realpath(f"/proc/{p}/exe") == target:
                    pids.append(pid)
            except OSError:
                continue
    except FileNotFoundError:
        pass
    return pids

def _kill_using(path: Path) -> None:
    """kill 占用 path 的进程（_pids_using 精确判据——只按 pid 杀，SIGTERM 优雅退）。"""
    for pid in _pids_using(path):
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

def _derive_brand_server(bin_dir: Path) -> str:
    """server 树品牌派生（branding-completion BLOCKER-2）：glob bin/*-server 排除上游名。
    禁复用 _derive_brand（只认 *-agent，server 树恒返 ""）。多品牌共存取首个（同 host 已知限制）。"""
    for f in sorted(bin_dir.glob("*-server")):
        if f.name == _exe_name("mediaservo-server"):
            continue
        return f.name[: -len("-server")]
    return ""

def _server_bin_names(brand: str) -> tuple[str, str]:
    """(deployed bin name, root shortcut name)——brand 空 → 上游名（永不渲染 "-server" 残名）。"""
    base = f"{brand}-server" if brand else "mediaservo-server"
    return _exe_name(base), base

def _drop_stale_server_oxfile(prefix_p: Path, bin_name: str) -> tuple[bool, dict[str, dict[str, str]]]:
    """BLOCKER-3：run/oxfile.toml 的 server 条目 command 基名 != 当前布局 bin 名（改名/换品牌/
    半迁移死路径）→ 备份为 oxfile.toml.bak 并移除，令随后 init 以 current_exe 重渲染。
    同时捕获各 [[apps]] 的 [apps.env] 键值——迁移后由 _reapply_carried_env 回吸收
    （运维手工 env 如 ALLOW_DEV_CREDENTIALS/RUST_LOG 不再随改名丢失，PIT-171 轮教训）。
    返回 (是否迁移, {app 名: {env 键: 值}})。"""
    oxfile = prefix_p / "run" / "oxfile.toml"
    if not oxfile.exists():
        return False, {}
    stale = False
    app_envs: dict[str, dict[str, str]] = {}
    cur_app: str | None = None
    in_env = False
    for line in oxfile.read_text().splitlines():
        t = line.strip()
        if t.startswith("[[apps]]"):
            cur_app, in_env = None, False
            continue
        if t.startswith("name") and "=" in t and cur_app is None:
            cur_app = t.split("=", 1)[1].strip().strip('"')
            in_env = False
            continue
        if t.startswith("[apps.env]"):
            in_env = True
            continue
        if t.startswith("["):
            in_env = False
            continue
        if in_env and "=" in t and cur_app:
            k, _, v = t.partition("=")
            app_envs.setdefault(cur_app, {})[k.strip()] = v.strip()
            continue
        if cur_app and t.startswith("command") and "=" in t:
            cmd = t.split("=", 1)[1].strip().strip('"')
            parts = cmd.split()
            if parts:
                stem = Path(parts[0]).name.removesuffix(".exe")
                if (stem == "mediaservo-server" or stem.endswith("-server")) and stem != bin_name:
                    stale = True
    if stale:
        bak = oxfile.with_name("oxfile.toml.bak")
        oxfile.replace(bak)
    return stale, app_envs


def _reapply_carried_env(prefix_p: Path, app_envs: dict[str, dict[str, str]]) -> list[str]:
    """迁移回吸收：新 oxfile（init 重渲染产物）缺失的旧 env 键按 app 补回。
    只补缺不覆盖（init 本次 baked 的键以新值为准）；返回被补的 "app:k" 列表。"""
    if not app_envs:
        return []
    oxfile = prefix_p / "run" / "oxfile.toml"
    if not oxfile.exists():
        return []
    lines = oxfile.read_text().splitlines()
    carried: list[str] = []
    for app, kv in app_envs.items():
        if not kv:
            continue
        # 定位该 app 块
        try:
            i = next(n for n, l in enumerate(lines) if l.strip() == "[[apps]]")
        except StopIteration:
            continue
        start = None
        n = i
        while n < len(lines):
            if lines[n].strip() == "[[apps]]":
                blk_i = n
                m = n + 1
                while m < len(lines) and not lines[m].strip().startswith("[["):
                    m += 1
                blk = lines[blk_i:m]
                if any(l.strip() == f'name = "{app}"' for l in blk):
                    start = blk_i
                    end = m
                    break
                n = m
            else:
                n += 1
        if start is None:
            continue
        blk = lines[start:end]
        existing = {l.split("=", 1)[0].strip() for l in blk if "=" in l and not l.strip().startswith(("[", "#"))}
        miss = {k: v for k, v in kv.items() if k.strip().strip('"') not in existing}
        if not miss:
            continue
        # 找块内 [apps.env] 头，无则在块尾（下一 [[apps]] 前）新建
        try:
            ei = next(j for j, l in enumerate(blk) if l.strip() == "[apps.env]")
        except StopIteration:
            ei = None
        add = [f"{k} = {v}" for k, v in sorted(miss.items())]
        if ei is None:
            new_blk = blk + ["[apps.env]"] + add
        else:
            je = ei + 1
            while je < len(blk) and "=" in blk[je] and not blk[je].strip().startswith("["):
                je += 1
            new_blk = blk[:je] + add + blk[je:]
        lines[start:end] = new_blk
        carried += [f"{app}:{k}" for k in sorted(miss)]
    if carried:
        oxfile.write_text("\n".join(lines) + "\n")
    return carried


def _assemble_host_binaries(bin_dir: Path, release: bool, brand: str) -> set[str]:
    """组装品牌化二进制到 bin_dir（D266——组装从 build 迁到 deploy，源=target/{debug,release}）。
    host-<app> → <brand>-<app>、mediaservo-host → <brand>-host；client 出树、host-streamer 兼容链已退役（D269）。
    返回组装文件名集合（供部署残留白名单）。"""
    src_dir = ROOT / "target" / ("release" if release else "debug")
    if not src_dir.is_dir():
        hint = "（--release 需配合 build --release）" if release else ""
        print(f"错误: {src_dir} 无构建产物{hint} — 先 build host", file=sys.stderr)
        sys.exit(1)
    src_names: set[str] = set()
    for b in HOST_BINS:
        src = src_dir / _exe_name(b)
        if not src.exists():
            continue
        name = src.name
        if brand:
            if name.startswith("host-"):
                name = f"{brand}-{name[5:]}"
            elif name == "mediaservo-host":
                name = f"{brand}-host"
        _copy_with_kill(src, bin_dir / name)
        src_names.add(name)
    # client 出树（D269/T3）：舱端拉流工具不再随车端 host 树装配——bin 白名单同轮清存量实例
    # host-streamer 兼容链已退役（D269/T2）：spawn 路径 exe_cmd(app_name()) 本已品牌感知，
    # src 显示串无消费方（磁盘 oxfile 全指 bin/msrtc-* 实证）——白名单同轮清存量实例残链
    return src_names

def _cmd_deploy_host(prefix: str, release: bool = False) -> None:
    """deploy host（有状态部署——组装 + 落地，D266）：identity 幂等/oxmgr/systemd/env.sh。
    package host 复用（Momus b——Task 2.5）: staging 目录内跑本函数组装身份/etc/env.sh；
    三连停不触发（staging 无 run/oxfile.toml——cli F5 只读安全）。

    D4: --prefix 必填（无默认——防污染 out/ 无状态交付；车端用 /opt/mediaservo-host）。
    品牌: 环境 MEDIASERVO_BRAND 优先（msrtc.sh 注入），回退已装实例 _derive_brand（deploy-ops 高危②）。"""
    if not prefix:
        print("错误: deploy 必须 --prefix（无默认——防止污染 out/ 无状态交付；车端用 /opt/mediaservo-host）", file=sys.stderr)
        sys.exit(2)
    prefix_p = Path(prefix)
    bin_dir = prefix_p / "bin"
    # 品牌：环境（msrtc.sh 注入 MEDIASERVO_BRAND）优先，回退已有实例推导（重复部署零参数）
    # Python 布局与 Rust media_brand 对齐："mediaservo" 是显式默认，映射 legacy host-* 串。
    brand_input = os.environ.get("MEDIASERVO_BRAND", "") or _derive_brand(bin_dir)
    brand = "" if brand_input == "mediaservo" else brand_input
    rust_brand = brand or "mediaservo"
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

    # 组装品牌化二进制（D266：源=target/{debug,release}——build 已纯编译）
    src_names = _assemble_host_binaries(bin_dir, release, brand)

    # bin 白名单（deploy-ops ④ 品牌切换残留清理）：非当前布局的 app 二进制删除
    # （build 已物理改名——残留 = 旧品牌/旧 install 遗留）
    # client/streamer 收尾 = D269 出树/拔链后存量实例残留的自动回收（白名单语义）
    roles = ("agent", "streamer", "capturer", "recorder", "controller", "emergency", "audio", "legacy", "host", "client")
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
        print("错误: PATH 未找到 oxmgr — 未打包（运行时需它拉起进程）。安装: 下载 GitHub Releases 预编译 Rust 二进制（含 sha256/asc 校验，https://github.com/Vladimir-Urik/OxMgr/releases），或构建 oxmgr-src 后放 ~/.local/bin，再重跑 deploy host", file=sys.stderr)

    # host init: 生成 etc/ + identity.json + 令牌——幂等只写不删（已存在跳过, 重装保留凭据）
    # brand: env 注入（init 是独立进程——device 前缀需与布局品牌一致）
    identity_file = prefix_p / "identity.json"
    if identity_file.exists():
        print(f"  identity.json 已存在——只写不删（设备身份保留）: {identity_file}")
    init_env = dict(os.environ)
    init_env["MEDIASERVO_BRAND"] = rust_brand
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
    env_lines.append(f'export MEDIASERVO_BRAND="{rust_brand}"   # 品牌 env（D252），部署包内显式钉死布局'),
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
    print("  etc/    host.yaml + link/{signing.pem,*.token}（host init 生成, 重装保留）")
    print("  run/    logs/")
    print("  recordings/")
    print("  identity.json（0600, 设备身份 — 只写不删, 勿删）")
    print(f"使用: export PATH={bin_dir}:$PATH && {host_cli} doctor {prefix} && {host_cli} start {prefix}")
    if sys.platform == "win32":
        print("注意: Windows best-effort（未全面验证）— 生产部署建议 Linux 车端", file=sys.stderr)

def _cmd_deploy_server(prefix: str) -> None:
    """deploy server（有状态落地——T20, D266 与 host 同构；deploy 不触发构建——源=out/server 单棵交付树）。
    品牌: env MEDIASERVO_BRAND 优先，回退目标树 bin/*-server 派生（_derive_brand_server）——
    物理二进制名 = {brand}-server（D269 品牌名=物理名，host 同构；cargo/bin 单元名不变）。
    BLOCKER-1 双探源：上游名（fresh build）→ {brand}-server（已迁移，免重 build 二次 deploy）。
    BLOCKER-2 空 brand 防护：brand 解析为空且目标树已有品牌化 bin → 拒绝（空 brand 永不删/改名品牌件）。
    BLOCKER-3 oxfile 刷新：bin 改名迁移 → 删 {prefix}/run/oxfile.toml 令 init 重渲染（手工 [apps.env] 需重加）。
    etc 已存在不覆盖（PIT-160——运维改动/注册数据不被重部署冲掉）；web 整树平移（rmtree+copytree）；
    init 幂等渲染（run/oxfile.toml + Caddyfile + PSK/JWT secret 自举——生命周期模块已并入二进制）。
    D4: --prefix 必填（无默认）；/opt 需 root（同 host 规则）。"""
    if not prefix:
        print("错误: deploy 必须 --prefix（无默认——防止污染 out/ 无状态交付；惯例 /opt/mediaservo-server）", file=sys.stderr)
        sys.exit(2)
    src_root = _out_root() / "server"
    src_bin_dir = src_root / "bin"
    prefix_p = Path(prefix)
    bin_dir = prefix_p / "bin"
    # 品牌：env 优先 → 目标树派生 → 源树派生（host L554 同款零参数幂等重部署；
    # 源树回退覆盖"out/server 已原地品牌化 → deploy /opt 未带 env"场景）。
    brand = (os.environ.get("MEDIASERVO_BRAND", "")
             or _derive_brand_server(bin_dir)
             or _derive_brand_server(src_bin_dir))
    if not brand:
        unexpected = sorted({f.name for d in (bin_dir, src_bin_dir) for f in d.glob("*-server")
                             if f.name != _exe_name("mediaservo-server")})
        if unexpected:
            # 派生恒能命中品牌件，理论不可达——防御断言：空 brand 永不走到删/改名
            print(f"错误: 在场 {', '.join(unexpected)} 但品牌解析为空——export MEDIASERVO_BRAND 后重试", file=sys.stderr)
            sys.exit(2)
    bin_name, shortcut = _server_bin_names(brand)
    server_bin_name = bin_name
    # BLOCKER-1 双探源（按序）：上游名（fresh build 产物）→ {brand}-server（已迁移树，免重 build）
    candidates = [src_bin_dir / _exe_name("mediaservo-server")]
    if brand:
        candidates.append(src_bin_dir / _exe_name(f"{brand}-server"))
    src_bin = next((cand for cand in candidates if cand.exists()), None)
    if src_bin is None:
        print(f"错误: {' 与 '.join(str(c) for c in candidates)} 均不存在 — 先 build server（deploy 不触发构建——D266）", file=sys.stderr)
        sys.exit(2)
    src_web = src_root / "web"
    if not (src_web / "index.html").exists():
        print(f"错误: {src_web} 无前端产物 — 先 build web（deploy 不触发构建——D266）", file=sys.stderr)
        sys.exit(2)
    try:
        bin_dir.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        print(f"错误: 无法创建 {bin_dir}: {e} — 部署用 sudo mediaservo deploy server --prefix /opt/mediaservo-server", file=sys.stderr)
        sys.exit(1)
    if str(prefix_p).startswith("/opt") and hasattr(os, "geteuid") and os.geteuid() != 0:
        print("错误: /opt 部署需 root（sudo mediaservo deploy server --prefix /opt/mediaservo-server）", file=sys.stderr)
        sys.exit(1)

    # build:deploy 糖注入 --prefix out/<target>（host 惯例）——server 源即目标：原地装配模式，
    # 跨树拷贝与 web rmtree 必须跳过（SameFileError 防护 + 删源自毁防护），init 渲染照常；
    # 原地改名（upstream→brand）属同目录 os.rename，允许（branding-completion T1.3）。
    inplace = prefix_p.resolve() == src_root.resolve()
    dst_bin = bin_dir / bin_name
    server_cli = dst_bin
    # 先停后动（PIT-170 同构：优雅 stop 先行，绝不靠 _copy/replace 强穿在跑二进制——否则
    # _copy_with_kill 的 inode 击杀会绕过簇优雅停、inplace 改名会开 daemon 用陈旧 oxfile 重拉死路径的窗口）。
    # 老名/新名两路径都查（改名前=老名在跑、上轮部署=新名在跑）；stop 用在场生命期二进制。
    stop_bin = dst_bin if dst_bin.exists() else src_bin
    running = _pids_using(dst_bin)
    for cand in candidates:
        if cand.name != dst_bin.name:
            running = running or _pids_using(bin_dir / cand.name)
    if running or _pids_using(bin_dir / _exe_name("oxmgr")):
        print("检测到运行中的 server 实例 — 先停本簇（重装不覆盖 etc/ 凭据）")
        try:
            subprocess.run([str(stop_bin), "stop", str(prefix_p)], capture_output=True, timeout=30, check=False)
        except subprocess.TimeoutExpired:
            print("  ⚠ 旧实例 stop 超时（可能是无生命周期的旧二进制）— 继续重装", file=sys.stderr)
    # renamed 仅在发生/需要 upstream→brand 迁移时置真；同文件/同目录 no-op 不触发 oxfile 强刷
    renamed = src_bin.name != bin_name and not (dst_bin.exists() and os.path.samefile(src_bin, dst_bin))
    if inplace:
        if renamed:
            os.replace(src_bin, dst_bin)  # 同目录原子改名（src==dst 上游无 brand 场景不置 renamed，不到这里）
            print(f"  bin 原地改名: {src_bin.name} → {bin_name}")
        else:
            print(f"  bin/{bin_name} 原地（源=目标）— 跳过拷贝")
    elif renamed:
        _copy_with_kill(src_bin, dst_bin)  # 上游名 → 品牌名（换名拷贝）
    else:
        if dst_bin.exists() and os.path.samefile(src_bin, dst_bin):
            print(f"  bin/{bin_name} 已就位（源=目标同文件）— 跳过拷贝")
        else:
            _copy_with_kill(src_bin, dst_bin)
    os.chmod(dst_bin, 0o755)
    server_cli = dst_bin
    # 生命周期冒烟：out/ 可能是无 init/start 子命令的旧构建（旧二进制会把未知参数当守护参数直启）
    ver_line: str | None = None
    try:
        probe = subprocess.run([str(server_cli), "version"], capture_output=True, text=True, timeout=10, check=False)
        if probe.returncode == 0:
            out_lines = (probe.stdout or "").strip().splitlines()
            ver_line = out_lines[0] if out_lines else "version ok"
    except subprocess.TimeoutExpired:
        pass
    if ver_line is None:
        print(f"错误: {server_cli} 生命周期冒烟失败（version 非零/超时——out/ 疑为无生命周期旧构建）— 先 build server 刷新后重跑 deploy", file=sys.stderr)
        sys.exit(2)
    print(f"  二进制就绪: {ver_line}")

    # oxmgr 随部署锁定版本（D-H13，同 host 待遇）
    oxmgr_src = shutil.which("oxmgr")
    if oxmgr_src is not None:
        _copy_with_kill(oxmgr_src, bin_dir / _exe_name("oxmgr"))
        os.chmod(bin_dir / _exe_name("oxmgr"), 0o755)
        ov = subprocess.run([oxmgr_src, "--version"], capture_output=True, text=True, check=False)
        print(f"  oxmgr 已打包: {ov.stdout.strip() or ov.stderr.strip() or '?'}")
    else:
        print("错误: PATH 未找到 oxmgr — 未打包（运行时需它拉起进程簇）。安装: 下载 GitHub Releases 预编译 Rust 二进制（含 sha256/asc 校验，https://github.com/Vladimir-Urik/OxMgr/releases），或构建 oxmgr-src 后放 ~/.local/bin，再重跑 deploy server", file=sys.stderr)

    # BLOCKER-3：bin 改名/换品牌迁移后 oxfile command 成死路径（init 遇已存在 oxfile 跳过——mod.rs 实证）
    # → 删除令下方 init 以 current_exe 重渲染。判据=server 条目 command 基名 != 当前 bin 名
    # （重复部署基名已一致 → 零 churn；fresh 树无 oxfile → init 正常渲染）。
    migrated, carried_envs = _drop_stale_server_oxfile(prefix_p, bin_name)
    if migrated:
        print("  run/oxfile.toml 陈旧（bin 改名迁移）——已备份为 run/oxfile.toml.bak 并重渲染；"
              "运维手工 env 自动回吸收，init 本次烘的端口类 env 以新值为准", file=sys.stderr)

    # etc 模板——已存在不覆盖（PIT-160；init 同样幂等，双保险）
    etc_dir = prefix_p / "etc"
    etc_dir.mkdir(parents=True, exist_ok=True)
    src_etc = src_root / "etc"
    for f in sorted(src_etc.iterdir()) if src_etc.is_dir() else []:
        if not f.is_file():
            continue
        if (etc_dir / f.name).exists():
            print(f"  etc/{f.name} 已存在—保留（PIT-160）")
        else:
            shutil.copy2(f, etc_dir / f.name)
            print(f"  etc/{f.name}: 模板落地")

    # web 整树平移（caddy 静态 root——绝对路径由 init 写进 oxfile/Caddyfile）
    dst_web = prefix_p / "web"
    if inplace:
        print("  web/ 原地（源=目标）— 跳过平移（rmtree 防护）")
    else:
        if dst_web.exists():
            shutil.rmtree(dst_web)
        shutil.copytree(src_web, dst_web)
    (prefix_p / "run").mkdir(parents=True, exist_ok=True)

    # init 幂等渲染：etc/Caddyfile + run/oxfile.toml（server+caddy 两条目）+ secret 自举（0600）
    # SFU 端口/公告隔离：部署前 export MEDIASERVO_SFU_PORT / MEDIASERVO_SFU_ANNOUNCED_IP，init 烘进 oxfile env
    _run_or_exit([str(server_cli), "init", str(prefix_p)])
    if migrated:
        got = _reapply_carried_env(prefix_p, carried_envs)
        if got:
            print(f"  运维 env 已回吸收: {', '.join(got)}")

    # bin 白名单（host 同构——deploy-ops ④）：非当前布局的 server 二进制删除
    # （upstream 旧名/旧品牌残留；mediaservo-server 仅无 brand 布局保留）
    for p in sorted(bin_dir.iterdir()):
        if p.is_symlink() or not p.is_file():
            continue
        if p.name in {_exe_name("oxmgr"), bin_name}:
            continue
        if p.name.endswith("-server") or p.name == _exe_name("mediaservo-server"):
            print(f"  清理部署残留: {p.name}")
            p.unlink()

    # 根级快捷名（host 同构——brand 非空只建 {brand}-server 链；brand 空 = 上游双快捷，永不建 "-server" 残名）
    if brand:
        shortcut_names = (shortcut,)
    else:
        shortcut_names = ("server", "mediaservo-server")
    for link_name in shortcut_names:
        link = prefix_p / link_name
        if link.is_symlink() or link.exists():
            link.unlink(missing_ok=True)
        try:
            link.symlink_to(f"bin/{bin_name}")  # 相对路径，前缀可搬迁
            print(f"  已创建符号链接 {link} → bin/{bin_name}")
        except OSError:
            shutil.copy2(bin_dir / bin_name, link)
            os.chmod(link, 0o755)
            print(f"  已复制 {bin_name} 到 {link}（符号链接失败，回退到拷贝）")
    # 旧品牌根级链清理（只碰指向本布局 bin/ 的 *-server 文件链，目录永不触碰）
    for link in prefix_p.glob("*-server"):
        if not link.is_symlink() or link.name == shortcut:
            continue
        try:
            resolved = (prefix_p / link.readlink()).resolve()
        except OSError:
            continue
        if resolved.parent == bin_dir.resolve() and link.name.endswith("-server"):
            print(f"  清理旧品牌入口链: {link.name}")
            link.unlink()

    print(f"server 已部署到 {prefix}" + ("（原地装配——渲染完成，可直接 start）" if inplace else "")
          + (f"（品牌 {brand}）" if brand else ""))
    print(f"  bin/    {server_bin_name}（物理名品牌化——D269）+ oxmgr（D-H13 锁定）")
    print("  etc/    server/devices/accounts.yaml + Caddyfile + secret（重部署保留既有——PIT-160）")
    print("  web/    前端交付物整树平移（caddy 静态 root）")
    print("  run/    oxfile.toml + oxmgr/（OXMGR_DATA_DIR——C32 隔离）+ logs/")
    print("  入口:   " + ", ".join(str(prefix_p / s) for s in shortcut_names) + f" → bin/{bin_name}")
    print("⚠ accounts.yaml 为 dev 占位模板（admin123 等）——裸机启动将 fail-fast（C35 守卫）:")
    print('    生产: export MEDIASERVO_ADMIN_PASSWORD=*** 后删 etc/accounts.yaml 重跑本 deploy')
    print('    联调: run/oxfile.toml server 条目 [apps.env] 加 MEDIASERVO_ALLOW_DEV_CREDENTIALS = "1"')
    print("下一步（实例目录命令, 本 CLI 运行分支已退役——C39）:")
    print(f"  启动:   {prefix_p / shortcut} start {prefix}（仅后端: 加 --no-web；前端面过渡可用 mediaservo run web）")
    print(f"  探测:   {prefix_p / shortcut} status {prefix}（退出码 0/1/2）| doctor {prefix}")
    print(f"  开机锚点（操作方步骤, 一次性）: {prefix_p / shortcut} startup on {prefix}")


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
        "# host 包部署: 裸解包生成版本目录；落地到前缀目录用 `tar xzf <pkg>.tar.gz -C <prefix> --strip-components=1`",
        "# 多设备共用同一包时, 每台删除 identity.json 后重跑",
        "# `host init <prefix>`（幂等, 已存在凭据保留）生成独立设备身份（G4）"
    ]
    (dst / f"{target}-version.txt").write_text("\n".join(lines) + "\n")


def _cmd_package(args: argparse.Namespace) -> None:
    """package <target> — <dist>/<brand>-{host|server|sdk}-<ver>.tar.gz 多包发布（D-H13）。
    dist: --dist 指定输出目录；未指定时默认子模块 dist/（MSRTC 发布壳会注入 out/packages/）。
    server: staging 内跑 _cmd_deploy_server（bin+oxmgr/etc/web/init 幂等——与 host 同构, T20/T21）。
    host: staging 内跑 _cmd_deploy_host(staging)（Momus 裁决 b——identity 初始/oxmgr 锁定/
    etc 模板/env.sh 文件组装；build 仍无状态）+ tar 含 bin 8/oxmgr/etc/identity/run/logs/recordings。
    bindings: staging 拷贝 out/bindings（完整 SDK 布局——D241 三件套 version-full/.pc/cmake/
    wheel/node/cxx 头，Task 2.5 补齐）→ sdk 包。
    staging 临时目录 → tar.gz; tar 内顶层为版本目录 {brand}-{target}-{ver}/，裸解包不会撒出凭据/二进制；
    需要直接落地到前缀目录时用 `tar xzf package.tar.gz -C <prefix> --strip-components=1`。"""
    if sys.platform == "win32":
        print("package: Windows best-effort — 验证清单见 scripts/e2e-win-validate.ps1", file=sys.stderr)
    pkg_name = {"bindings": "sdk", "server": "server"}.get(args.target, "host")
    ver = _workspace_version()
    dist = Path(args.dist) if getattr(args, "dist", "") else ROOT / "dist"
    dist.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f"ms-{args.target}-pkg-", dir=str(dist)))
    try:
        if args.target == "host":
            _cmd_deploy_host(str(staging), args.release)  # brand 经 MEDIASERVO_BRAND 环境/已装实例推导（D266）
        elif args.target == "server":
            _cmd_deploy_server(str(staging))  # 同 deploy 语义整树入 staging（init 幂等落 staging）
        else:
            _cmd_deploy_bindings(str(staging), args.release)
        _write_version_file(staging, pkg_name)
        prefix_name = args.brand if args.brand else "mediaservo"
        package_root = f"{prefix_name}-{pkg_name}-{ver}"
        out = dist / f"{package_root}.tar.gz"
        strip_package_binaries(staging)  # PIT-119: debug 二进制未 strip（单 135-155MB）→ gzip 1.2GB 超时
        with tarfile.open(out, "w:gz", compresslevel=6) as tar:  # 默认 9 最慢; 6 工程折中
            tar.add(staging, arcname=package_root)  # 解包生成版本目录；--strip-components=1 可落前缀目录
        print(f"✓ 打包完成: {out}（{out.stat().st_size // 1024} KiB）")
        print(f"  顶层目录: {package_root}/")
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


def _host_runtime_hint(action: str) -> None:
    """host 运行已移除 CLI 封装（D266）——build=编译 / deploy=安装 / 运行=手动。"""
    print(f"{action} host 已移除 CLI 封装——请手动运行:", file=sys.stderr)
    print("  部署实例: systemctl --user start|stop oxmgr-<brand>-host.service", file=sys.stderr)
    print("            或 <prefix>/bin/msrtc-host start|stop <prefix>", file=sys.stderr)
    print("  开发:     target/debug/mediaservo-host init . && token issue --all . && start|stop .", file=sys.stderr)
    print("            跑前清 iceoryx2 残留: rm -rf /tmp/iceoryx2 /dev/shm/iox2_*（C25）", file=sys.stderr)

def _server_runtime_hint(action: str) -> None:
    """server 运行已移除 CLI 封装（T21——C39 与 host 同待遇）：build=编译 / deploy=安装 / 运行=实例目录命令。
    bin 名品牌感知（D269——env MEDIASERVO_BRAND，缺省上游名）。"""
    sb = f"{os.environ.get('MEDIASERVO_BRAND', '')}-server" if os.environ.get("MEDIASERVO_BRAND", "") else "mediaservo-server"
    print(f"{action} server 已移除 CLI 封装——请经实例目录运行:", file=sys.stderr)
    print(f"  部署实例: <prefix>/bin/{sb} start|stop|restart|status <prefix>（未部署: mediaservo deploy server --prefix <X>；裸机入口 <prefix>/{sb}）", file=sys.stderr)
    print("  开发:     target/debug/mediaservo-server init . && start . --no-web（前端面: dev web=vite:5173 / run web=过渡 caddy）", file=sys.stderr)
    print("  容器（模式②③）: mediaservo up / down --env dev|prod 不变", file=sys.stderr)
    print("  只读探测保留: mediaservo status server | logs server", file=sys.stderr)

def _cmd_dev(args: argparse.Namespace) -> None:
    """dev web — vite dev 薄透传（pnpm dev, cwd www/:5173, proxy /api /ws→9800；前台工具无 pidfile）。"""
    _check("pnpm", "pnpm 未安装——dev web 需要（www 为 pnpm workspace，或手动 cd www && pnpm dev）")
    _run_or_exit(["pnpm", "dev"] + list(args.rest), cwd=str(ROOT / "www"))

def _cmd_start(args: argparse.Namespace) -> None:
    """start <target> — server/host 退役→指引 exit 2（T21/C39: 实例目录命令；容器用 up）；web=过渡 caddy。"""
    target = args.target
    if target == "server":
        _server_runtime_hint("start")
        sys.exit(2)
    elif target == "host":
        _host_runtime_hint("start")
        sys.exit(2)
    elif target == "web":
        _run_web_native()
    else:  # client
        print("start client: 待实现（client 骨架阶段）", file=sys.stderr)
        sys.exit(1)


def _cmd_restart(args: argparse.Namespace) -> None:
    target = args.target
    """restart <target> — web=过渡 caddy；server/host 退役→指引 exit 2（实例目录命令；容器 up/down）。"""
    if target == "web":
        _stop_web_native(allow_inactive=True)
        _run_web_native()
        return
    if target == "server":
        _server_runtime_hint("restart")
        sys.exit(2)
    elif target == "host":
        _host_runtime_hint("restart")
        sys.exit(2)
    else:  # client
        print("restart client: 待实现（client 骨架阶段）", file=sys.stderr)
        sys.exit(1)


def _cmd_run(args: argparse.Namespace) -> None:
    """run <target> — web=过渡 caddy（:8080）；server/host 退役→指引 exit 2（T21/C39）。"""
    if args.target == "server":
        _server_runtime_hint("run")
        sys.exit(2)
    elif args.target == "host":
        _host_runtime_hint("run")
        sys.exit(2)
    elif args.target == "web":
        _run_web_native()
    else:
        print('run client: 待实现（client 骨架阶段）', file=sys.stderr)
        sys.exit(1)


def _out_root() -> Path:
    """发布根（MSRTC_OUT_ROOT env or 子模块 out/）——build 交付布局与 server 运行时共用。"""
    env_out = os.environ.get("MSRTC_OUT_ROOT", "")
    return Path(env_out) if env_out else ROOT / "out"


def _stage_to_out(target: str, files: list[Path], sub: str = "bin", brand: str = "") -> list[Path]:
    """拷贝 target/debug|release 产物到 out/<target>/<sub>（交付布局镜像）；brand 非空时物理重命名：host-<app> → <brand>-<app>、mediaservo-host → <brand>-host（server 物理名品牌化在 deploy 段——D269）。"""
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
        dst_path = dst / name
        try:
            shutil.copy2(src, dst_path)
        except OSError as e:
            if e.errno == 26:  # ETXTBSY — binary is running; unlink then retry
                dst_path.unlink()
                shutil.copy2(src, dst_path)
            else:
                raise
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
        rel = bin_path.relative_to(ROOT) if bin_path.is_relative_to(ROOT) else bin_path
        print(f'错误: 未找到 {rel} — 先运行: mediaservo build server --native', file=sys.stderr)
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
    """stop <target> — web=过渡 caddy；server/host 退役→指引 exit 2（容器: mediaservo down）。"""
    target = args.target
    if target == "web":
        _stop_web_native()
        return
    if target == "server":
        _server_runtime_hint("stop")
        sys.exit(2)
    elif target == "host":
        _host_runtime_hint("stop")
        sys.exit(2)
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
    """deploy <target> — bindings | host | server（源=out/ 交付树；--prefix 必填 D4；deploy 不触发构建）。"""
    if args.target == "bindings":
        _cmd_deploy_bindings(args.prefix, args.release)
    elif args.target == "host":
        _cmd_deploy_host(args.prefix, args.release)
    elif args.target == "server":
        _cmd_deploy_server(args.prefix)


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
    server: 默认 native（B 裁决——清裸机产物+先停进程）；--mode both 双清 / compose 只清容器；host/client: 清宿主 cargo target。"""
    target = args.target
    if target in ("all", "server"):
        mode = _resolve_mode(args, default="native")
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
            out_server = _out_root() / "server"
            targets = [logs_d / "server-native.pid", logs_d / "server-native.log",
                       logs_d / "web-native.pid", logs_d / "web-native.log",  # 过渡 caddy 运行态（T6 漏网补清）
                       out_server / "bin" / _exe_name("mediaservo-server"),
                       out_server / "web",  # 前端交付树（T21: build web 产物随 clean 回收）
                       ROOT / "target/debug/mediaservo-server", ROOT / "target/release/mediaservo-server"]
            # MAJOR-C 品牌态两名全清 + 根级快捷链（glob 只收文件/链，目录永不触碰）
            targets += [f for f in sorted((out_server / "bin").glob("*-server")) if f.is_file()]
            targets += [f for f in sorted(out_server.glob("*-server")) if f.is_symlink()]
            _sv = out_server / "server"  # 无品牌快捷链（brand="" 布局）
            if _sv.is_symlink():
                targets.append(_sv)
            for f in targets:
                _rm_path(f)
            for d in sorted((_out_root() / "server" / "run").glob("oxmgr*")):
                _rm_path(d)  # out 轨演练残留 daemon 数据（C32——prefix 下的 run/ 归实例生命周期, 不在此列）
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
    elif args.target == "web":
        code = _status_web_native()
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
        print("  → host 未运行——手动: target/debug/mediaservo-host init . && token issue --all . && start .（开发）或 systemctl --user start oxmgr-<brand>-host.service（部署）", file=sys.stderr)
    return 2 if probe_failed else (0 if any_alive else 1)


def _add_mode_args(p, *, env_choices=("dev", "prod"), allow_both=False):
    """统一模式参数（--mode native|compose[/both] + 短别名 --native/--env；互斥）。
    语义: 命令默认 native（用户裁决 B）——compose 需显式 --mode compose / --env；
    both 仅 stop/clean 启用（双停/双清两轨）。"""
    choices = ["native", "compose"] + (["both"] if allow_both else [])
    grp = p.add_mutually_exclusive_group()
    grp.add_argument("--mode", choices=choices, default=None,
                     help="运行模式: native=裸机（默认）| compose=容器（--env 同效）"
                     + (" | both=两轨全处理" if allow_both else ""))
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
  模式① 裸机交付: build server（bin/etc/web 装配）→ deploy server --prefix <X> →
             <X>/<brand>-server init/start/stop/status/restart/doctor <X>（startup on=开机锚点；bin 物理名品牌化 D269）
    快速通道: build web（仅前端→out/server/web）| dev web（vite:5173 热更, 配 start --no-web）
             | run web（过渡独立 caddy :8080——Phase 6 后归实例进程簇）
  模式② 单容器生产: build server --image runtime → up --env prod → logs server --env prod
  模式③ compose开发: up --env dev（热更）→ logs -f → down --env dev
  退役→指引 exit 2: run/start/stop/restart server|host（C39——用实例目录命令；容器面 up/down 不变）
  只读探测保留: status server|web / logs server / clean server
  退出码: status/logs/stop —— 0=成功 1=未运行/目标缺失 2=参数错/退役指引
""")

    sub = parser.add_subparsers(dest="command", required=True)

    build_p = sub.add_parser("build", help="构建 <target> [--image runtime|dev]: all|web|host|server|client|bindings（默认 all；server=不嵌入变体+web 装配一步出；web=纯前端快速通道；--image 才走 Docker）")
    build_p.add_argument("target", nargs="?", choices=["all", "web", "host", "server", "client", "bindings"], default="all")
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

    restart_p = sub.add_parser("restart", help="重启 <target>: web=过渡 caddy；server/host 退役→指引 exit 2（<prefix>/bin/mediaservo-server restart <dir>；容器 up/down）")
    restart_p.add_argument("target", choices=["host", "server", "web"])
    _add_mode_args(restart_p)
    restart_p.set_defaults(func=_cmd_restart)
    run_p = sub.add_parser("run", help="运行 <target>: web=过渡 caddy（:8080）；server/host 退役→指引 exit 2（实例命令见 <prefix>/bin/mediaservo-server -h）")
    run_p.add_argument("target", choices=["server", "host", "web"])
    run_p.add_argument("--foreground", "-f", action="store_true", help="（退役遗留——server 运行分支已移除, 无效果）")
    run_p.add_argument("--release", action="store_true", help="（退役遗留——无效果）")
    run_p.add_argument("--announced-ip", metavar="IP[,IP...]", default=None,
                       help="（退役遗留——实例侧公告用 MEDIASERVO_SFU_ANNOUNCED_IP env）")
    run_p.set_defaults(func=_cmd_run)

    start_p = sub.add_parser("start", help="启动 <target>（默认 server）: server 退役→指引 exit 2（容器用 up）；host 已移除 CLI 封装；web=过渡 caddy")
    start_p.add_argument("target", nargs="?", choices=["host", "server", "web"], default="server")
    _add_mode_args(start_p)
    start_p.add_argument("--foreground", "-f", action="store_true", help="（退役遗留——server 运行分支已移除, 无效果）")
    start_p.set_defaults(func=_cmd_start)

    stop_p = sub.add_parser("stop", help="停止 <target>: server/host 退役→指引 exit 2（实例命令 stop <dir>；容器 mediaservo down）；web=过渡 caddy")
    stop_p.add_argument("target", choices=["host", "server", "web"])
    _add_mode_args(stop_p, allow_both=True)
    stop_p.set_defaults(func=_cmd_stop)

    dev_p = sub.add_parser("dev", help="开发服务透传: web=vite dev（:5173, proxy /api /ws→9800——配 mediaservo-server start --no-web 组开发栈；前台工具无 pidfile）")
    dev_p.add_argument("target", choices=["web"])
    dev_p.add_argument("rest", nargs=argparse.REMAINDER, help="透传参数（-- 之后, 如 -- --port 5174）")
    dev_p.set_defaults(func=_cmd_dev)

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

    deploy_p = sub.add_parser("deploy", help="部署 <target>（有状态落地——deploy 不触发构建, 源=out/ 交付树）：host|server|bindings；--prefix 必填（/opt 需 root）")
    deploy_p.add_argument("target", choices=["bindings", "host", "server"])
    deploy_p.add_argument("--prefix", default=None, help="部署前缀（必填——host: /opt/mediaservo-host | server: /opt/mediaservo-server | bindings: /opt/mediaservo-sdk）")
    deploy_p.add_argument("--release", action="store_true", help="部署 release 组装产物")
    deploy_p.set_defaults(func=_cmd_deploy)
    # D1: install 隐藏 prompt（不 alias——语义不同: 源=out/ 交付布局 + --prefix 必填）
    install_p = sub.add_parser("install", help="已改名 deploy（提示迁移后退出 exit 2）")
    install_p.add_argument("args", nargs=argparse.REMAINDER, help="吞掉旧调用点透传参数（T3 迁移前 msrtc.sh 仍注入 --prefix 等）——仅提示改名")
    install_p.set_defaults(func=_cmd_install_deprecated)
    package_p = sub.add_parser("package", help="打包 <target>: host|server（交付包）| bindings（SDK 包）→ <dist>/<brand>-<host|server|sdk>-<ver>.tar.gz（D-H13 多包发布, 含版本契约文件和版本顶层目录）")
    package_p.add_argument("target", choices=["host", "server", "bindings"])
    package_p.add_argument("--dist", default="", help="package tar 与 staging 输出目录（默认 dist；MSRTC 发布壳默认 out/packages）")
    package_p.add_argument("--brand", default="", help="品牌包名（<dist>/<brand>-<target>-<ver>.tar.gz；缺省 mediaservo）")
    package_p.add_argument("--release", action="store_true", help="打包 release 产物（target/release, 配合 build --release）")
    package_p.set_defaults(func=_cmd_package)
    clean_p = sub.add_parser("clean", help="清理 <target>: all|server|host|client（默认 all；server=原生清（默认）| --mode both 双清）")
    clean_p.add_argument("target", nargs="?", choices=["all", "server", "host", "client"], default="all")
    _add_mode_args(clean_p, allow_both=True)
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
    status_p.add_argument("target", choices=["server", "host", "web"])
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
        elif args.target == "web":
            _cmd_build_web()
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
