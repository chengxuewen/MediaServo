// imgui_viewer — 舱端验收例子（装配层）。
//
// 分层铁律（把这份代码当模板抄时的第一页，全项目同规）：
//   main.cpp     帧循环/线程属主/节拍（本文件，≤150 行一眼看完）
//   app_model.*  纯状态与数据工具（不碰 ImGui、不碰 SDL）
//   sessions.*   SDK 动作面（login/join/consume/control——**永不 include imgui.h**）
//   dock_layout  停靠画布与首版四区（docking 五要素注释主战场）
//   panels/*     每区一个无状态渲染函数（Begin 自己的 dock 窗；动作调 sessions）
// 依赖方向单向：panels → sessions/dock_layout → app_model；main 装配全部。
//
// 布局（docking 版，拖分栏/双击标题最大化/拖出浮动全原生）：
//   左 Streams | 中 Video Grid | 右上 Stream Info | 右下 Control | 底 Log
//   复位：Log 面板 "reset layout" 钮（或删 imgui_viewer.ini 重启）。
//
// 无头判据保留：MSRTC_PASS 提供口令时自动登录+勾选首个视频房；
// SDL_VIDEODRIVER=dummy + MSRTC_RUN_SECS=N 到时自退并打 `[frame] cb=/tex=`
// （dummy 下 ini 禁写——无头验收 git status 恒干净）。
// 凭据纪律（G13）：口令默认 UI/stdin，永不 argv。
//
// env：MSRTC_WS_URL MSRTC_HTTP_BASE MSRTC_RUN_SECS(0=常开) MSRTC_PASS(仅无头 CI)
//      MSRTC_ROOM(无头 auto-join 目标) MSRTC_ESTOP_KEY_FILE

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

#include <imgui.h>
#include <imgui_shell/shell.hpp>

#include "app_log.hpp"
#include "app_model.hpp"
#include "dock_layout.hpp"
#include "panels/control.hpp"
#include "panels/log_view.hpp"
#include "panels/login.hpp"
#include "panels/stream_info.hpp"
#include "panels/streams_tree.hpp"
#include "panels/video_grid.hpp"

using namespace viewer;  // main 必须全局作用域（链接入口）；名字均来自本例自身模块

int main() {
    Env env;
    env.ws = env_or("MSRTC_WS_URL", "ws://127.0.0.1:9800/ws");
    env.http_base = env_or("MSRTC_HTTP_BASE", "http://127.0.0.1:9800");
    env.room_hint = env_or("MSRTC_ROOM", "");
    if (const char* k = std::getenv("MSRTC_ESTOP_KEY_FILE")) env.key_file = k; // G13 文件通道注入
    env.run_secs = std::atoi(env_or("MSRTC_RUN_SECS", "0"));

    imgui_shell::WindowSpec spec;
    spec.title = "MSRTC Viewer";
    spec.width = 1440;
    spec.height = 900;
    imgui_shell::ShellOptions sopt;
    sopt.docking = true;                 // 本例是四区 IDE 布局（对照：control_demo 保持默认关）
    sopt.ini_path = "imgui_viewer.ini";  // 布局持久化（dummy 驱动下 shell 自动禁写）
    imgui_shell::App app(spec, sopt);

    AppModel m;
    if (const char* pass_env = std::getenv("MSRTC_PASS")) { // 仅无头 CI 通道（G13：GUI 面走输入框）
        if (*pass_env) {
            std::strncpy(m.user, "admin", sizeof(m.user) - 1);
            std::strncpy(m.pass, pass_env, sizeof(m.pass) - 1);
            m.auto_join = true;
        }
    }

    const auto t0 = std::chrono::steady_clock::now();
    log().set_time_origin(t0);
    log().add("info", "viewer start");
    unsigned long long last_report = 0;
    int tick = 0;
    std::vector<uint8_t> buf; // tile 泵消费复用缓冲（主线程专属）
    while (app.running()) {
        if (!app.pump_events()) break;

        // 泵线程帧 → 主线程纹理（每 tile 排空 staging，latest-only 天然限帧）
        for (auto& t : m.tiles) {
            uint32_t w = 0, h = 0;
            while (t->staging.pop(buf, &w, &h)) {
                t->tex.update(app.sdl_renderer(), buf.data(), buf.size(), w, h);
            }
        }

        app.begin_frame();
        ++tick;

        dock::render_root(app);  // 画布每帧铺；首版四区只在"无存档布局"时切一次
        if (!m.logged_in) {
            render_login(m, env);  // 登录前：浮动窗（未入 dock）
        } else {
            render_streams_tree(m, env);
            render_grid(m);
            render_stream_info(m);
            render_control(m);
        }
        render_log_view();         // 日志区登录前后都开着（启动过程本身要可见）
        app.end_frame();

        // stats 轮询 + 无头双计数报告（每 ~1s 一次 tick 节拍）
        if (tick % 60 == 0) {
            for (auto& t : m.tiles) t->poll_stats();
        }
        const unsigned long long secs =
            std::chrono::duration_cast<std::chrono::seconds>(std::chrono::steady_clock::now() - t0).count();
        if (secs >= last_report + 5) {
            last_report = secs;
            uint64_t cb = 0, tex = 0;
            uint32_t w = 0, h = 0;
            for (auto& t : m.tiles) {
                cb += t->cb.load();
                tex += t->tex.frames();
                w = t->w;
                h = t->h;
            }
            std::printf("[frame] t=%llus cb=%llu tex=%llu %ux%u\n", secs,
                        static_cast<unsigned long long>(cb), static_cast<unsigned long long>(tex), w, h);
            std::fflush(stdout);
        }
        if (env.run_secs > 0 && secs >= static_cast<unsigned long long>(env.run_secs)) break;
    }
    uint64_t cb = 0, tex = 0;
    for (auto& t : m.tiles) {
        cb += t->cb.load();
        tex += t->tex.frames();
    }
    std::printf("viewer exit: cb=%llu tex=%llu\n",
                static_cast<unsigned long long>(cb), static_cast<unsigned long long>(tex));
    return 0;
}
