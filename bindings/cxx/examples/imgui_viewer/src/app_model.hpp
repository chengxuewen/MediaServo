// app_model — imgui_viewer 状态模型（T3 自 main.cpp 原样迁居，零行为变化）。
//
// 本例分层铁律（把这份代码当模板抄时的第一页，全项目同规）：
//   main.cpp     帧循环/线程属主/节拍（不写面板细节）
//   app_model.*  纯状态与数据工具（不碰 ImGui、不碰 SDL）
//   sessions.*   SDK 动作面（login/join/consume/control——**永不 include imgui.h**）
//   panels/*     无状态渲染函数（不建会话、不碰 SDL——要触发动作就调 sessions）
// 依赖方向单向：panels → sessions → app_model；main 装配全部。
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

/// 启动环境（main 一次性解析，此后只读——凭据走 env/stdin，G13 永不 argv）。
struct Env {
    std::string ws;             // MSRTC_WS_URL
    std::string http_base;      // MSRTC_HTTP_BASE
    std::string room_hint;      // MSRTC_ROOM（无头 auto-join 目标，空=首个视频房）
    std::string key_file;       // MSRTC_ESTOP_KEY_FILE（空=不签名形）
    int run_secs = 0;           // MSRTC_RUN_SECS（0=常开）
};

const char* env_or(const char* k, const char* dflt);

struct RoomRow {
    std::string room_id, kind;
    bool video = false; // kind 提示（vehicle-* 含视频；audio-* 仅音频）
    bool checked = false; // 直存 bool（vector<bool> 代理引用不可 &）
};

/// 一路已勾选房间 = 一个会话（G11：房间=流，退订=close 会话）。
struct Tile {
    explicit Tile(std::string room_) : room(std::move(room_)) {}

    std::string room;
    bool video = false;
    mediaservo::client::Session sess;
    imgui_shell::FrameStaging staging;
    imgui_shell::VideoTexture tex;
    std::atomic<uint64_t> cb{0};

    // mini-stats（W3a 资产消费端）
    ::viewer_core::RateEstimator bytes_rate;
    uint64_t bytes_prev = 0, frames_prev = 0;
    double fps = 0.0;
    uint32_t w = 0, h = 0;
    // stats 扩面（09-24 对表 web play；键=SDK 会话级 union JSON additive 字段）
    double st_jitter = 0.0;
    uint64_t st_packets = 0, packets_lost = 0, st_dropped = 0, st_nack = 0, st_pli = 0, st_fir = 0;
    std::chrono::steady_clock::time_point stats_t{};

    // 控制室（仅视频房开 chassis）
    std::unique_ptr<mediaservo::client::Control> ctl;
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

/// 全应用状态（T3 前的旧名 Ui；字段逐字保留=搬迁可 diff 审）。
struct AppModel {
    bool key_signed = false; // estop 签名态（面板措辞；W4b）
    char user[64] = "admin";
    char pass[256] = "";
    bool pass_shown = false;
    std::string status;               // 登录/列举错误行
    std::vector<RoomRow> rooms;
    bool logged_in = false;
    std::string jwt;
    bool auto_join = false;           // 无头 CI 通道标志（原 main 局部，入模后 panels 无状态化）
    std::vector<std::unique_ptr<Tile>> tiles;
    int sel_control = -1;             // 控制室选中的 tile 下标
    int sel_info = 0;                 // 右区详情聚焦的 tile 下标
    float steer_deg = 0.0f;
    uint64_t seq = 1;                 // 舱端自增（D-H3 会话内单调）
    bool estop_sent = false;
    int cols = 2;
};

/// list_rooms JSON 数组的逐项提取（键均字符串形，手工扫描零依赖）。
std::vector<std::pair<std::string, std::string>> parse_rooms(const std::string& json);

} // namespace viewer

#endif // MSRTC_VIEWER_APP_MODEL_HPP
