// imgui_viewer — 舱端验收例子（p3-gui-viewer 本批唯一 GUI 例）。
//
// W3b+W4 合并态：登录 → list_rooms 勾选多路（G11 每房一 Session）→ tile 网格
// （等比 fit / 两档列数）→ mini-stats（video_stats + RateEstimator）→ 控制室
// （steer + ack RTT + 急停按钮·plain Cmd 形，签名刀 W4b 另补）。
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
#include <mutex>
#include <string>
#include <vector>

#include <imgui.h>
#include <imgui_shell/core.hpp>
#include <imgui_shell/shell.hpp>
#include <imgui_shell/texture.hpp>
#include <mediaservo/client.hpp>

namespace ms = mediaservo::client;
namespace sh = imgui_shell;
namespace core = ::viewer_core;


namespace {

const char* env_or(const char* k, const char* dflt) {
    const char* v = std::getenv(k);
    return (v && *v) ? v : dflt;
}

/// list_rooms JSON 数组的逐项提取（键均字符串形，手工扫描零依赖）。
std::vector<std::pair<std::string, std::string>> parse_rooms(const std::string& json) {
    std::vector<std::pair<std::string, std::string>> out;
    std::string::size_type pos = 0;
    while ((pos = json.find("\"room_id\"", pos)) != std::string::npos) {
        auto q1 = json.find('"', pos + 8);
        auto q2 = json.find('"', q1 + 1);
        if (q1 == std::string::npos || q2 == std::string::npos) break;
        std::string room = json.substr(q1 + 1, q2 - q1 - 1);
        std::string kind;
        auto kp = json.find("\"kind\"", q2);
        auto next_room = json.find("\"room_id\"", q2);
        if (kp != std::string::npos && (next_room == std::string::npos || kp < next_room)) {
            auto k1 = json.find('"', kp + 6);
            auto k2 = json.find('"', k1 + 1);
            if (k1 != std::string::npos && k2 != std::string::npos) kind = json.substr(k1 + 1, k2 - k1 - 1);
        }
        out.emplace_back(std::move(room), std::move(kind));
        pos = q2;
    }
    return out;
}

struct RoomRow {
    std::string room_id, kind;
    bool video = false; // kind 提示（vehicle-* 含视频；audio-* 仅音频）
    bool checked = false; // 直存 bool（vector<bool> 代理引用不可 &）
};

/// 一路已勾选房间 = 一个会话（G11：房间=流，退订=close 会话）。
struct Tile {
    explicit Tile(std::string room_) : room(std::move(room_)) {}

    std::string room;
    ms::Session sess;
    sh::FrameStaging staging;
    sh::VideoTexture tex;
    std::atomic<uint64_t> cb{0};

    // mini-stats（W3a 资产消费端）
    core::RateEstimator bytes_rate;
    uint64_t bytes_prev = 0, frames_prev = 0;
    double fps = 0.0;
    uint32_t w = 0, h = 0;
    std::chrono::steady_clock::time_point stats_t{};

    // 控制室（仅视频房开 chassis）
    std::unique_ptr<ms::Control> ctl;
    std::mutex ack_mu;
    std::string ack_last;      // 最近 ack json（面板展示）
    uint64_t want_seq = 0;     // 最近一发 seq（RTT 对齐用）
    std::atomic<int64_t> sent_us{0};
    double rtt_ms = -1.0;
    std::string err;          // 建立期失败原因（面板红行）

    void poll_stats() {
        auto js = sess.video_stats();
        if (!js) return;
        uint64_t bytes = 0, frames = 0;
        double f = 0.0;
        core::json_u64(*js, "bytes_received", &bytes);
        core::json_u64(*js, "frames_decoded", &frames);
        core::json_f64(*js, "frames_per_second", &f);
        uint64_t uw = 0, uh = 0;
        core::json_u64(*js, "frame_width", &uw);
        core::json_u64(*js, "frame_height", &uh);
        w = static_cast<uint32_t>(uw);
        h = static_cast<uint32_t>(uh);
        const auto now = std::chrono::steady_clock::now();
        const auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(now - stats_t).count();
        if (ms > 0) {
            bytes_rate.update(ms, bytes); // RateEstimator 单位=累计值/毫秒差
            fps = f;
        }
        stats_t = now;
        bytes_prev = bytes;
        frames_prev = frames;
    }

    void send_cmd(const char* label, uint64_t seq, const char* cmd, const char* payload_json) {
        if (!ctl) return;
        sent_us.store(std::chrono::duration_cast<std::chrono::microseconds>(
                          std::chrono::steady_clock::now().time_since_epoch())
                          .count());
        want_seq = seq;
        auto r = ctl->send(label, seq, cmd, payload_json);
        if (!r) {
            std::lock_guard<std::mutex> lk(ack_mu);
            ack_last = std::string("send error: ") + r.error().message;
        }
    }
};

struct Ui {
    char user[64] = "admin";
    char pass[256] = "";
    bool pass_shown = false;
    std::string status;               // 登录/列举错误行
    std::vector<RoomRow> rooms;
    bool logged_in = false;
    std::string jwt;
    std::vector<std::unique_ptr<Tile>> tiles;
    int sel_control = -1;             // 控制室选中的 tile 下标
    float steer_deg = 0.0f;
    uint64_t seq = 1;                 // 舱端自增（D-H3 会话内单调）
    bool estop_sent = false;
    int cols = 2;
};

std::string try_login(const std::string& http_base, const char* user, const char* pass) {
    auto t = ms::login(http_base, user, pass);
    if (!t) return "login: " + t.error().message;
    return *t; // 非空 = jwt；错误路径上面已带前缀区分（错误必含 "login: "）
}

} // namespace

int main() {
    const std::string ws = env_or("MSRTC_WS_URL", "ws://127.0.0.1:9800/ws");
    const std::string http_base = env_or("MSRTC_HTTP_BASE", "http://127.0.0.1:9800");
    const int run_secs = std::atoi(env_or("MSRTC_RUN_SECS", "0"));
    const char* pass_env = std::getenv("MSRTC_PASS"); // 仅无头 CI 通道（G13：GUI 面走输入框）
    const std::string auto_room = env_or("MSRTC_ROOM", "");

    imgui_shell::WindowSpec spec;
    spec.title = "MSRTC Viewer";
    spec.width = 1440;
    spec.height = 900;
    imgui_shell::App app(spec);

    Ui ui;
    bool auto_join = false;
    if (pass_env && *pass_env) {
        std::strncpy(ui.user, "admin", sizeof(ui.user) - 1);
        std::strncpy(ui.pass, pass_env, sizeof(ui.pass) - 1);
        auto_join = true;
    }

    auto join_room = [&](const std::string& room_id, bool video) {
        ms::Config cfg;
        cfg.signaling_url = ws;
        cfg.room = room_id;
        cfg.jwt = ui.jwt;
        cfg.role = "Client";
        (void)video;
        auto tile = std::make_unique<Tile>(room_id);
        Tile* tp = tile.get();
        auto sj = ms::Session::connect(cfg);
        if (!sj) {
            tile->err = "join: " + sj.error().message;
        } else {
            tile->sess = std::move(*sj);
            auto cv = tp->sess.consume_video([tp](const mediaservo_client_frame_t& f) {
                tp->staging.push(f.data, f.len, f.width, f.height);
                tp->cb.fetch_add(1, std::memory_order_relaxed);
            });
            if (!cv) tile->err = "consume: " + cv.error().message;
        }
        if (!tile->err.empty()) std::printf("[tile] %s: %s\n", room_id.c_str(), tile->err.c_str());
        ui.tiles.push_back(std::move(tile));
    };

    const auto t0 = std::chrono::steady_clock::now();
    unsigned long long last_report = 0;
    int tick = 0;
    std::vector<uint8_t> buf; // tile 泵消费复用缓冲（主线程专属）
    while (app.running()) {
        if (!app.pump_events()) break;

        // 泵线程帧 → 主线程纹理（每 tile 排空 staging，latest-only 天然限帧）
        for (auto& t : ui.tiles) {
            uint32_t w = 0, h = 0;
            while (t->staging.pop(buf, &w, &h)) {
                t->tex.update(app.sdl_renderer(), buf.data(), buf.size(), w, h);
            }
        }

        app.begin_frame();
        ++tick;

        ImGui::SetNextWindowPos(ImVec2(8, 40), ImGuiCond_FirstUseEver);
        ImGui::Begin("MSRTC Viewer");

        if (!ui.logged_in) {
            // ── 登录面 ──
            ImGui::TextDisabled("ws=%s", ws.c_str());
            ImGui::TextDisabled("http=%s", http_base.c_str());
            ImGui::InputText("user", ui.user, sizeof(ui.user));
            ImGui::InputText("password", ui.pass, sizeof(ui.pass),
                             ui.pass_shown ? ImGuiInputTextFlags_None
                                           : ImGuiInputTextFlags_Password);
            ImGui::SameLine();
            ImGui::Checkbox("show", &ui.pass_shown);
            if (!ui.status.empty()) ImGui::TextColored({1, .4f, .4f, 1}, "%s", ui.status.c_str());
            bool do_login = ImGui::Button("Login") && ui.pass[0] != '\0';
            if (auto_join) {
                do_login = true;
            }
            if (do_login) {
                auto token = ms::login(http_base, ui.user, ui.pass);
                if (!token) {
                    ui.status = "login: " + token.error().message;
                    auto_join = false;
                } else {
                    ui.jwt = *token;
                    auto lr = ms::list_rooms(http_base, ui.jwt);
                    if (!lr) {
                        ui.status = "list_rooms: " + lr.error().message;
                        auto_join = false;
                    } else {
                        for (auto& [rid, kind] : parse_rooms(*lr)) {
                            RoomRow r;
                            r.room_id = rid;
                            r.kind = kind;
                            r.video = kind == "video" || rid.rfind("vehicle", 0) == 0;
                            ui.rooms.push_back(std::move(r));
                        }
                        ui.logged_in = true;
                        ui.status.clear();
                        if (auto_join) {
                            std::string target = auto_room;
                            if (target.empty())
                                for (auto& r : ui.rooms)
                                    if (r.video) { target = r.room_id; break; }
                            if (!target.empty()) join_room(target, true);
                            else ui.status = "auto-join: no video room";
                            auto_join = false;
                        }
                    }
                }
            }
        } else {
            // ── 左栏房间 + 主区 tile 网格 + 控制室 ──
            if (ImGui::BeginTabBar("root")) {
                if (ImGui::BeginTabItem("Rooms")) {
                    for (size_t i = 0; i < ui.rooms.size(); ++i) {
                        bool already = false;
                        for (auto& t : ui.tiles)
                            if (t->room == ui.rooms[i].room_id) already = true;
                        ImGui::PushID((int)i);
                        ImGui::BeginDisabled(already);
                        ImGui::Checkbox("##ck", &ui.rooms[i].checked);
                        ImGui::EndDisabled();
                        ImGui::SameLine();
                        ImGui::Text("%s  [%s]", ui.rooms[i].room_id.c_str(), ui.rooms[i].kind.c_str());
                        if (ui.rooms[i].checked && !already) {
                            join_room(ui.rooms[i].room_id, ui.rooms[i].video);
                            ui.rooms[i].checked = false;
                        }
                        ImGui::PopID();
                    }
                    if (ImGui::Button("Reconnect")) { // 换 server/重登录逃生舱
                        ui.logged_in = false;
                        ui.tiles.clear(); // close 会话（Session dtor）
                    }
                    ImGui::EndTabItem();
                }
                if (ImGui::BeginTabItem("Tiles")) {
                    ImGui::SameLine(ImGui::GetWindowWidth() - 220);
                    int old_cols = ui.cols;
                    ImGui::SetNextItemWidth(100);
                    ImGui::Combo("cols", &ui.cols, "1\0"
                            "2\0"
                            "3\0");
                    ui.cols = std::max(1, std::min(3, ui.cols));
                    (void)old_cols;
                    const int ncol = ui.cols;
                    size_t i = 0;
                    for (int row = 0; row * ncol < ui.tiles.size(); ++row) {
                        ImGui::Columns(ncol, nullptr, false);
                        for (int c = 0; c < ncol && i < ui.tiles.size(); ++c, ++i) {
                            Tile* t = ui.tiles[i].get();
                            ImGui::BeginGroup();
                            ImGui::Text("%s", t->room.c_str());
                            if (t->tex.texture_id()) {
                                const ImVec2 avail = ImGui::GetContentRegionAvail();
                                float dw = avail.x;
                                const float ar = static_cast<float>(t->tex.width()) / static_cast<float>(t->tex.height());
                                float dh = dw / ar;
                                ImGui::Image(static_cast<ImTextureID>(reinterpret_cast<uintptr_t>(t->tex.texture_id())), ImVec2(dw, dh));
                            } else {
                                ImGui::TextDisabled("%s", t->err.empty() ? "waiting for video..." : t->err.c_str());
                            }
                            // mini-stats 一行（W3a 资产）
                            ImGui::Text("%.0fk fps=%.0f %ux%u cb=%llu", t->bytes_rate.kbps(), t->fps,
                                        t->w, t->h, static_cast<unsigned long long>(t->cb.load()));
                            ImGui::EndGroup();
                        }
                        ImGui::NextColumn();
                        ImGui::Columns(1);
                    }
                    ImGui::EndTabItem();
                }
                if (ImGui::BeginTabItem("Control")) {
                    if (ui.tiles.empty()) {
                        ImGui::TextDisabled("join a room first");
                    } else {
                        std::vector<const char*> names;
                        for (auto& t : ui.tiles) names.push_back(t->room.c_str());
                        if (ui.sel_control >= (int)names.size()) ui.sel_control = -1;
                        ImGui::Combo("tile", &ui.sel_control, names.data(), (int)names.size());
                        if (ui.sel_control >= 0) {
                            Tile* t = ui.tiles[ui.sel_control].get();
                            if (!t->ctl && t->err.empty()) {
                                // 控制通道建立（一次性；失败原因进红行，重连=Rooms 页 Reconnect）
                                auto oc = t->sess.open_control({"chassis"});
                                if (oc) {
                                    t->ctl = std::make_unique<ms::Control>(std::move(*oc));
                                    Tile* tp = t;
                                    t->ctl->on_ack([tp](const std::string& ack) {
                                        // ack 信封 `{"ack":<seq>,...}`；seq 匹配最近一发才计 RTT
                                        // （重发幂等窗内旧 ack 到达 = 只刷文本不刷 RTT）。
                                        uint64_t aseq = 0;
                                        core::json_u64(ack, "ack", &aseq);
                                        const int64_t now_us = std::chrono::duration_cast<std::chrono::microseconds>(
                                                                   std::chrono::steady_clock::now().time_since_epoch())
                                                                   .count();
                                        std::lock_guard<std::mutex> lk(tp->ack_mu);
                                        tp->ack_last = ack;
                                        if (aseq != 0 && aseq == tp->want_seq) {
                                            tp->rtt_ms = (now_us - tp->sent_us.load()) / 1000.0;
                                        }
                                    });
                                } else {
                                    t->err = "control: " + oc.error().message;
                                }
                            }
                            ImGui::SeparatorText(t->room.c_str());
                            if (!t->err.empty()) ImGui::TextColored({1, .4f, .4f, 1}, "%s", t->err.c_str());
                            ImGui::SliderFloat("steer deg", &ui.steer_deg, -90.0f, 90.0f);
                            if (ImGui::Button("send steer")) {
                                t->send_cmd("chassis", ++ui.seq, "steer",
                                            (std::string("{\"deg\":") + std::to_string(ui.steer_deg) + "}").c_str());
                            }
                            ImGui::SameLine();
                            // W4b 待补：签名急停（HMAC sig + WS 审计副本）——C 面暴露刀另立。
                            if (ImGui::Button("ESTOP (unsigned)")) {
                                t->send_cmd("chassis", 900, "estop", "{\"reason\":\"viewer\"}");
                                ui.estop_sent = true;
                            }
                            std::string acks;
                            {
                                std::lock_guard<std::mutex> lk(t->ack_mu);
                                acks = t->ack_last;
                            }
                            ImGui::Text("ack: %s", acks.empty() ? "-" : acks.c_str());
                            if (t->rtt_ms >= 0.0) ImGui::SameLine(), ImGui::Text("(rtt=%.0fms)", t->rtt_ms);
                            ImGui::Text("estop sent=%s", ui.estop_sent ? "yes" : "no");
                        }
                    }
                    ImGui::EndTabItem();
                }
                ImGui::EndTabBar();
            }
        }
        ImGui::End();
        app.end_frame();

        // stats 轮询 + 无头双计数报告（每 ~1s 一次 tick 节拍）
        if (tick % 60 == 0) {
            for (auto& t : ui.tiles) t->poll_stats();
        }
        const unsigned long long secs =
            std::chrono::duration_cast<std::chrono::seconds>(std::chrono::steady_clock::now() - t0).count();
        if (secs >= last_report + 5) {
            last_report = secs;
            uint64_t cb = 0, tex = 0;
            uint32_t w = 0, h = 0;
            for (auto& t : ui.tiles) {
                cb += t->cb.load();
                tex += t->tex.frames();
                w = t->w;
                h = t->h;
            }
            std::printf("[frame] t=%llus cb=%llu tex=%llu %ux%u\n", secs,
                        static_cast<unsigned long long>(cb), static_cast<unsigned long long>(tex), w, h);
            std::fflush(stdout);
        }
        if (run_secs > 0 && secs >= static_cast<unsigned long long>(run_secs)) break;
    }
    uint64_t cb = 0, tex = 0;
    for (auto& t : ui.tiles) {
        cb += t->cb.load();
        tex += t->tex.frames();
    }
    std::printf("viewer exit: cb=%llu tex=%llu\n",
                static_cast<unsigned long long>(cb), static_cast<unsigned long long>(tex));
    return 0;
}
