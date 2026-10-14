// app_model — the imgui_viewer state model (relocated verbatim from main.cpp in T3, zero behavior change).
//
// This example's layering iron rules (page one for anyone copying this code as a template; the same rules project-wide):
//   main.cpp     the frame loop / thread ownership / beats (no panel details)
//   app_model.*  pure state and data utilities (no ImGui, no SDL)
//   sessions.*   the SDK action surface (login/join/consume/control — **never includes imgui.h**)
//   panels/*     stateless render functions (no session creation, no SDL — call sessions to trigger actions)
// The dependency direction is one-way: panels → sessions → app_model; main assembles everything.
#ifndef MSRTC_VIEWER_APP_MODEL_HPP
#define MSRTC_VIEWER_APP_MODEL_HPP

#include <atomic>
#include <chrono>
#include <cstdint>
#include <memory>
#include <mutex>
#include <string>
#include <utility>
#include <vector>

#include <imgui_shell/core.hpp>
#include <imgui_shell/texture.hpp>
#include <mediaservo/client.hpp>

namespace viewer {

/// Startup environment (parsed once by main, read-only afterwards — credentials go via env/stdin; G13: never argv).
struct Env {
    std::string ws;             // MSRTC_WS_URL
    std::string http_base;      // MSRTC_HTTP_BASE
    std::string room_hint;      // MSRTC_ROOM (the headless auto-join target; empty = the first video room)
    std::string key_file;       // MSRTC_ESTOP_KEY_FILE (empty = the unsigned form)
    int run_secs = 0;           // MSRTC_RUN_SECS (0 = run forever)
};

const char* env_or(const char* k, const char* dflt);

struct RoomRow {
    std::string room_id, kind;
    std::string parent; // T3: the owning whole-vehicle room (empty = a flat unit / old wire)
    bool video = false; // a kind hint (vehicle-* has video; audio-* is audio-only)
    bool checked = false; // a plain bool (a vector<bool> proxy reference cannot be taken by address)
};

/// One checked room = one session (G11: room = stream; unsubscribing = closing the session).
struct Tile {
    explicit Tile(std::string room_) : room(std::move(room_)) {}

    std::string room;
    bool video = false;
    mediaservo::client::Session sess;
    imgui_shell::FrameStaging staging;
    imgui_shell::VideoTexture tex;
    std::atomic<uint64_t> cb{0};

    // mini-stats (a W3a asset consumer)
    ::viewer_core::RateEstimator bytes_rate;
    uint64_t bytes_prev = 0, frames_prev = 0;
    double fps = 0.0;
    uint32_t w = 0, h = 0;
    // stats extensions (reconciled with web play on 09-24; keys = additive fields of the SDK session-level union JSON)
    double st_jitter = 0.0;
    uint64_t st_packets = 0, packets_lost = 0, st_dropped = 0, st_nack = 0, st_pli = 0, st_fir = 0;

    // the control room (chassis opened only for video rooms)
    std::unique_ptr<mediaservo::client::Control> ctl;
    std::mutex ack_mu;
    std::string ack_last;      // the most recent ack json (shown in the panel)
    uint64_t want_seq = 0;     // the most recently sent seq (for RTT alignment)
    std::atomic<int64_t> sent_us{0};
    double rtt_ms = -1.0;
    std::string err;          // the setup-failure reason (a red line in the panel)

    void poll_stats() {
        auto js = sess.video_stats();
        if (!js) return;
        uint64_t bytes = 0, frames = 0;
        double f = 0.0;
        ::viewer_core::json_u64(*js, "bytes_received", &bytes);
        ::viewer_core::json_u64(*js, "frames_decoded", &frames);
        ::viewer_core::json_f64(*js, "frames_per_second", &f);
        uint64_t uw = 0, uh = 0;
        ::viewer_core::json_u64(*js, "frame_width", &uw);
        ::viewer_core::json_u64(*js, "frame_height", &uh);
        ::viewer_core::json_u64(*js, "packets_received", &st_packets);
        ::viewer_core::json_u64(*js, "packets_lost", &packets_lost);
        ::viewer_core::json_u64(*js, "frame_dropped", &st_dropped);
        ::viewer_core::json_u64(*js, "nack_count", &st_nack);
        ::viewer_core::json_u64(*js, "pli_count", &st_pli);
        ::viewer_core::json_u64(*js, "fir_count", &st_fir);
        ::viewer_core::json_f64(*js, "jitter", &st_jitter);
        w = static_cast<uint32_t>(uw);
        h = static_cast<uint32_t>(uh);
        // RateEstimator wants **absolute timestamps + cumulative values**; the slope/re-anchor/regression handling is internal —
        // feeding "the ms delta since last time" once made dt permanently negative and the bitrate forever 0 (proven by a user on 09-24; the same family as 09-01 on web).
        const auto now = std::chrono::steady_clock::now();
        bytes_rate.update(
            std::chrono::duration_cast<std::chrono::milliseconds>(now.time_since_epoch()).count(),
            bytes);
        fps = f;
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

/// Auth modes (viewer-auth-matrix T2; the F1 five-way Radio). For the semantics table see PLAN §2.1:
/// Account/ApiKey = the exchange forms (login/exchange → discovery → join); Psk = skip discovery, join directly (the Legacy
/// discovery 401s); Device = the identity_dir public-key chain; JwtPaste = paste a token directly (a bad token → the 4013 test point).
enum class AuthMode { Account, ApiKey, Psk, Device, JwtPaste };

/// The application-wide state (the old pre-T3 name Ui; fields kept verbatim = the relocation is diff-auditable).
struct AppModel {
    bool key_signed = false; // the estop signing state (panel wording; W4b)
    char user[64] = "admin";
    char pass[256] = "";
    bool pass_shown = false;
    // The server address (preset by main from env; editable in the login panel — for env-less defaults / demo machines)
    char url_ws[256] = "";
    char url_http[256] = "";
    // The five auth modes (T2; the G13 discipline: credentials go via input fields or env, never argv).
    AuthMode auth = AuthMode::Account;
    char key_id[64] = "";           // M2
    char key_secret[256] = "";      // M2
    char psk[256] = "";             // M3
    char direct_room[128] = "";     // M3 only: the target room for joining directly, skipping discovery
    char identity_dir[512] = "";    // M4: the device identity instance directory
    char jwt_paste[2048] = "";      // M5: paste a JWT directly (the bad-token test point)
    std::string identity_label;     // T4/F6: echoed after connecting ("Device(ms-x)"/"legacy-psk"/...)
    std::string status;               // the login/listing error line
    std::vector<RoomRow> rooms;
    bool logged_in = false;
    std::string jwt;
    bool auto_join = false;           // the headless CI channel flag (formerly a main local; stateless panels after moving into the model)
    std::vector<std::unique_ptr<Tile>> tiles;
    int sel_control = -1;             // the index of the selected control-room tile
    int sel_info = 0;                 // the index of the tile focused in the right details area
    float steer_deg = 0.0f;
    uint64_t seq = 1;                 // the cockpit-side counter (D-H3: monotonic within a session)
    bool estop_sent = false;
    int cols = 2;
};

/// Item-by-item extraction from a list_rooms JSON array (all keys are strings; a hand-rolled zero-dependency scan).
std::vector<std::pair<std::string, std::string>> parse_rooms(const std::string& json);
/// room-grouping-semantics T3: the triple form (room_id, kind, parent) — the tree groups by parent.
std::vector<std::tuple<std::string, std::string, std::string>> parse_rooms_full(
    const std::string& json);

} // namespace viewer

#endif // MSRTC_VIEWER_APP_MODEL_HPP
