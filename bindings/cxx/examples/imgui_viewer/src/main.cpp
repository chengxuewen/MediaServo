// imgui_viewer — 舱端验收例子（装配层）。
//
// T3 拆分态：本文件只留「env 解析 → 建 App → 帧循环节拍」三件事（≤130 行一眼看完）。
// 状态在 app_model，SDK 动作在 sessions，UI 在 panels/*——分层铁律见 app_model.hpp 头注。
// （T4 起本文件追加 dock_layout 装配；布局细节不在这里。）
//
// 无头判据保留：MSRTC_PASS 提供口令时自动登录+勾选首个视频房；
// SDL_VIDEODRIVER=dummy + MSRTC_RUN_SECS=N 到时自退并打 `[frame] cb=/tex=`。
// 凭据纪律（G13）：口令默认 UI/stdin，永不 argv。
//
// env：MSRTC_WS_URL MSRTC_HTTP_BASE MSRTC_RUN_SECS(0=常开) MSRTC_PASS(仅无头 CI)

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

#include "app_model.hpp"
#include "panels/control.hpp"
#include "panels/login.hpp"
#include "panels/rooms.hpp"
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
    imgui_shell::App app(spec); // T3 保持默认 ShellOptions（docking 上线在 T4）

    AppModel m;
    if (const char* pass_env = std::getenv("MSRTC_PASS")) { // 仅无头 CI 通道（G13：GUI 面走输入框）
        if (*pass_env) {
            std::strncpy(m.user, "admin", sizeof(m.user) - 1);
            std::strncpy(m.pass, pass_env, sizeof(m.pass) - 1);
            m.auto_join = true;
        }
    }

    const auto t0 = std::chrono::steady_clock::now();
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

        ImGui::SetNextWindowPos(ImVec2(8, 40), ImGuiCond_FirstUseEver);
        ImGui::Begin("MSRTC Viewer");
        if (!m.logged_in) {
            render_login(m, env);
        } else {
            // T3 搬迁态维持三页签骨架；T4 换 docking 四区后此段拆散进各 dock 窗
            if (ImGui::BeginTabBar("root")) {
                if (ImGui::BeginTabItem("Rooms")) {
                    render_rooms_body(m, env);
                    ImGui::EndTabItem();
                }
                if (ImGui::BeginTabItem("Tiles")) {
                    render_grid_body(m);
                    ImGui::EndTabItem();
                }
                if (ImGui::BeginTabItem("Control")) {
                    render_control_body(m);
                    ImGui::EndTabItem();
                }
                ImGui::EndTabBar();
            }
        }
        ImGui::End();
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
