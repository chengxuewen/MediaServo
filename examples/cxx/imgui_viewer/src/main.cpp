// imgui_viewer — the cockpit acceptance example (the assembly layer).
//
// Layering iron rules (page one for anyone copying this code as a template; the same rules project-wide):
//   main.cpp     the frame loop / thread ownership / beats (this file, ≤150 lines at a glance)
//   app_model.*  pure state and data utilities (no ImGui, no SDL)
//   sessions.*   the SDK action surface (login/join/consume/control — **never includes imgui.h**)
//   dock_layout  the docking canvas and the first-run four panes (the main battleground of the five docking-elements notes)
//   panels/*     one stateless render function per pane (Begins its own dock window; actions call sessions)
// The dependency direction is one-way: panels → sessions/dock_layout → app_model; main assembles everything.
//
// Layout (the docking version; dragging panes / double-click-to-maximize / dragging out floats are all native):
//   left Streams | center Video Grid | right-top Stream Info | right-bottom Control | bottom Log
//   Reset: the Log panel's "reset layout" button (or delete imgui_viewer.ini and restart).
//
// The headless judgment is kept: with MSRTC_PASS providing the password it auto-logs-in and checks the first video room;
// SDL_VIDEODRIVER=dummy + MSRTC_RUN_SECS=N exits on schedule and prints `[frame] cb=/tex=`
// (writing the ini is disabled under dummy — the headless acceptance keeps git status always clean).
// Credential discipline (G13): the password defaults to UI/stdin, never argv.
//
// env: MSRTC_WS_URL MSRTC_HTTP_BASE MSRTC_RUN_SECS(0=run forever) MSRTC_PASS(headless CI only)
//      MSRTC_ROOM(headless auto-join target) MSRTC_ESTOP_KEY_FILE

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
#include "sessions.hpp"
#include "panels/control.hpp"
#include "panels/log_view.hpp"
#include "panels/login.hpp"
#include "panels/stream_info.hpp"
#include "panels/streams_tree.hpp"
#include "panels/video_grid.hpp"

using namespace viewer;  // main must be at global scope (the link entry); all names come from this example's own modules

int main() {
    Env env;
    env.ws = env_or("MSRTC_WS_URL", "ws://127.0.0.1:9800/ws");
    env.http_base = env_or("MSRTC_HTTP_BASE", "http://127.0.0.1:9800");
    env.room_hint = env_or("MSRTC_ROOM", "");
    if (const char* k = std::getenv("MSRTC_ESTOP_KEY_FILE")) env.key_file = k; // injected via the G13 file channel
    env.run_secs = std::atoi(env_or("MSRTC_RUN_SECS", "0"));

    imgui_shell::WindowSpec spec;
    spec.title = "MSRTC Viewer"; // size uses WindowSpec's default adaptive form (80% of the primary display + centered);
                                 // assign width/height explicitly only if a fixed initial value is needed
    imgui_shell::ShellOptions sopt;
    sopt.docking = true;                 // this example is the four-pane IDE layout (contrast: control_demo keeps the default off)
    sopt.ini_path = "imgui_viewer.ini";  // layout persistence (the shell auto-disables writing under the dummy driver)
    imgui_shell::App app(spec, sopt);

    AppModel m;
    std::snprintf(m.url_ws, sizeof(m.url_ws), "%s", env.ws.c_str());
    std::snprintf(m.url_http, sizeof(m.url_http), "%s", env.http_base.c_str());
    if (const char* pass_env = std::getenv("MSRTC_PASS")) { // the headless CI channel only (G13: the GUI face uses input fields)
        if (*pass_env) {
            std::strncpy(m.user, "admin", sizeof(m.user) - 1);
            std::strncpy(m.pass, pass_env, sizeof(m.pass) - 1);
            m.auto_join = true;
        }
    }
    if (const char* k = std::getenv("MSRTC_KEY_ID"); k && *k) { // the headless CI API-key channel (the same G13 discipline)
        const char* sec = std::getenv("MSRTC_KEY_SECRET");
        if (sec && *sec) {
            std::strncpy(m.key_id, k, sizeof(m.key_id) - 1);
            std::strncpy(m.key_secret, sec, sizeof(m.key_secret) - 1);
            m.auth = AuthMode::ApiKey;
            m.auto_join = true;
        }
    }
    // T2: the five-mode headless injection channel (MSRTC_AUTH_MODE=account|apikey|psk|device|jwt).
    if (const char* mode = std::getenv("MSRTC_AUTH_MODE"); mode && *mode) {
        auto pick = [](const char* k2) -> const char* {
            const char* v = std::getenv(k2);
            return (v && *v) ? v : nullptr;
        };
        if (std::strcmp(mode, "psk") == 0) {
            if (auto* v = pick("MSRTC_PSK")) {
                std::strncpy(m.psk, v, sizeof(m.psk) - 1);
                if (auto* r = pick("MSRTC_ROOM")) std::strncpy(m.direct_room, r, sizeof(m.direct_room) - 1);
                m.auth = AuthMode::Psk;
                m.auto_join = true;
            }
        } else if (std::strcmp(mode, "device") == 0) {
            if (auto* v = pick("MSRTC_IDENTITY_DIR")) {
                std::strncpy(m.identity_dir, v, sizeof(m.identity_dir) - 1);
                m.auth = AuthMode::Device;
                m.auto_join = true;
            }
        } else if (std::strcmp(mode, "jwt") == 0) {
            if (auto* v = pick("MSRTC_JWT")) {
                std::strncpy(m.jwt_paste, v, sizeof(m.jwt_paste) - 1);
                m.auth = AuthMode::JwtPaste;
                m.auto_join = true;
            }
        }
        // account/apikey go through the existing channels above (PASS / KEY_ID+SECRET).
    }

    const auto t0 = std::chrono::steady_clock::now();
    log().set_time_origin(t0);
    log().add("info", "viewer start");
    unsigned long long last_report = 0;
    int tick = 0;
    std::vector<uint8_t> buf; // the reuse buffer consumed by the tile pumps (main-thread only)
    while (app.running()) {
        if (!app.pump_events()) break;

        // pump-thread frames → the main-thread texture (drain each tile's staging; latest-only naturally bounds the frame rate)
        for (auto& t : m.tiles) {
            uint32_t w = 0, h = 0;
            while (t->staging.pop(buf, &w, &h)) {
                t->tex.update(app.sdl_renderer(), buf.data(), buf.size(), w, h);
            }
        }

        app.begin_frame();
        ++tick;

        dock::render_root(app);  // the canvas is laid every frame; the first-run four panes split only when there is "no archived layout"
        if (!m.logged_in) {
            render_login(m, env);  // before login: a floating window (not docked yet)
        } else {
            render_streams_tree(m, env);
            render_grid(m);
            render_stream_info(m);
            render_control(m);
        }
        render_log_view();         // the log area stays open before and after login (the startup process itself must be visible)
        app.end_frame();

        // stats polling + the headless double-count report (a tick beat about once per second)
        if (tick % 60 == 0) {
            refresh_identity_label(m); // T4/F6: low-frequency back-fill of the identity line (cleared when tiles are empty)
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
