// imgui_viewer — 舱端验收例子（p3-gui-viewer 本批唯一 GUI 例）。
//
// W3b 第一刀：登录 → 入房 → consume_video 泵 → I420 纹理 → 单 tile 出画。
// W4 长出：list_rooms 勾选多路 · 遥控面板 · 急停态机 · 完整布局。
//
// 无头判据（本机无 X）：SDL_VIDEODRIVER=dummy + MSRTC_RUN_SECS=N 到时自退，
// 期间纹理照常更新——每 5s 打 `[frame] cb=.. tex=..` 双计数（PLAN W3 判据链）。
// 凭据纪律（G13）：口令 stdin 交互读，永不 argv/env 明文。
//
// env：MSRTC_WS_URL(必需) MSRTC_HTTP_BASE(必需) MSRTC_ROOM(必需) MSRTC_USER(必需)
//      MSRTC_RUN_SECS(可选，0=常开)

#include <atomic>
#include <chrono>
#include <cstdio>
#include <iostream>
#include <cstdlib>
#include <string>
#include <vector>

#include <imgui.h>
#include <imgui_shell/shell.hpp>
#include <imgui_shell/texture.hpp>
#include <mediaservo/client.hpp>

namespace ms = mediaservo;

namespace {

const char* required_env(const char* key) {
    const char* v = std::getenv(key);
    return (v && *v) ? v : nullptr;
}

} // namespace

int main() {
    const char* ws = required_env("MSRTC_WS_URL");
    const char* http_base = required_env("MSRTC_HTTP_BASE");
    const char* room = required_env("MSRTC_ROOM");
    const char* user = required_env("MSRTC_USER");
    const char* run_env = required_env("MSRTC_RUN_SECS");
    const int run_secs = run_env ? std::atoi(run_env) : 0;
    if (!ws || !http_base || !room || !user) {
        std::fprintf(stderr,
                     "usage: env MSRTC_WS_URL=ws://host:9800/ws MSRTC_HTTP_BASE=http://host:9800 "
                     "MSRTC_ROOM=.. MSRTC_USER=.. [MSRTC_RUN_SECS=30]  (口令走 stdin)\n");
        return 2;
    }
    std::printf("password> ");
    std::fflush(stdout);
    std::string pass;
    if (!std::getline(std::cin, pass) || pass.empty()) {
        std::fprintf(stderr, "empty password\n");
        return 2;
    }

    auto token = ms::client::login(http_base, user, pass);
    if (!token) {
        std::fprintf(stderr, "login failed: [%d] %s\n", token.error().code, token.error().message.c_str());
        return 1;
    }
    std::printf("login ok\n");

    ms::client::Config cfg;
    cfg.signaling_url = ws;
    cfg.room = room;
    cfg.jwt = *token;
    cfg.role = "Client"; // C 面角色枚举（与 control_demo 同词；消费语义）
    auto sess = ms::client::Session::connect(cfg);
    if (!sess) {
        std::fprintf(stderr, "join failed: [%d] %s\n", sess.error().code, sess.error().message.c_str());
        return 1;
    }
    std::printf("joined room=%s negotiated=%u\n", room,
                static_cast<unsigned>(sess->negotiated().value_or(0)));

    imgui_shell::FrameStaging staging;
    std::atomic<uint64_t> cb_frames{0};
    auto cv = sess->consume_video([&](const mediaservo_client_frame_t& f) {
        staging.push(f.data, f.len, f.width, f.height);
        cb_frames.fetch_add(1, std::memory_order_relaxed);
    });
    if (!cv) {
        std::fprintf(stderr, "consume_video failed: [%d] %s\n", cv.error().code, cv.error().message.c_str());
        return 1;
    }
    std::printf("consume_video ok\n");

    imgui_shell::WindowSpec spec;
    spec.title = std::string("MSRTC Viewer - ") + room;
    imgui_shell::App app(spec);
    if (!app.running()) {
        // SDK 段（登录/入房/消费注册）已全通；仅 GUI 圈需要显示环境。
        // dummy 驱动可出"窗口"= 无头判据走 SDL_VIDEODRIVER=dummy。
        std::fprintf(stderr, "no window — run with SDL_VIDEODRIVER=dummy headless or a real X\n");
        return 3;
    }

    imgui_shell::VideoTexture tex;
    const auto t0 = std::chrono::steady_clock::now();
    unsigned long long last_report = 0;
    uint32_t w = 0, h = 0;
    std::vector<uint8_t> buf;
    while (app.running()) {
        if (!app.pump_events()) break;

        while (staging.pop(buf, &w, &h)) {
            tex.update(app.sdl_renderer(), buf.data(), buf.size(), w, h);
        }

        app.begin_frame();
        ImGui::Begin("MSRTC Viewer");
        if (tex.texture_id()) {
            const ImVec2 avail = ImGui::GetContentRegionAvail();
            // 等比 fit（W3b 单 tile；两档响应式/W4 网格再长）
            float dw = avail.x;
            float dh = dw * (static_cast<float>(tex.height()) / static_cast<float>(tex.width()));
            if (dh > avail.y) {
                dh = avail.y;
                dw = dh * (static_cast<float>(tex.width()) / static_cast<float>(tex.height()));
            }
            const ImTextureID tid = static_cast<ImTextureID>(reinterpret_cast<uintptr_t>(tex.texture_id()));
            ImGui::Image(tid, ImVec2(dw, dh));
        } else {
            ImGui::TextDisabled("waiting for video...");
        }
        ImGui::End();
        app.end_frame();

        const auto now = std::chrono::steady_clock::now();
        const unsigned long long secs =
            std::chrono::duration_cast<std::chrono::seconds>(now - t0).count();
        if (secs >= last_report + 5) {
            last_report = secs;
            std::printf("[frame] t=%llus cb=%llu tex=%llu %ux%u\n", secs,
                        static_cast<unsigned long long>(cb_frames.load()),
                        static_cast<unsigned long long>(tex.frames()), w, h);
            std::fflush(stdout);
        }
        if (run_secs > 0 && secs >= static_cast<unsigned long long>(run_secs)) break;
    }
    std::printf("viewer exit: cb=%llu tex=%llu\n",
                static_cast<unsigned long long>(cb_frames.load()),
                static_cast<unsigned long long>(tex.frames()));
    return 0;
}
