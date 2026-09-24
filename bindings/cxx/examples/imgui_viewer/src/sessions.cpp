// sessions — 实现（T3 自 main.cpp join_room lambda 与登录分支迁居；语义逐字保持）。
#include "sessions.hpp"

#include <chrono>
#include <cstdio>
#include <memory>
#include <utility>

#include "app_log.hpp"

#include <mediaservo/client.hpp>

namespace viewer {

namespace ms = mediaservo::client;

void join_room(AppModel& m, const Env& env, const std::string& room_id, bool video) {
    ms::Config cfg;
    cfg.signaling_url = env.ws;
    cfg.room = room_id;
    cfg.jwt = m.jwt;
    cfg.role = "Client";
    if (!env.key_file.empty()) {
        cfg.hmac_key_file = env.key_file;
        m.key_signed = true;
    }
    auto tile = std::make_unique<Tile>(room_id);
    tile->video = video; // 控制房 tile 不发 consume（整车房无媒体=wait-producer 黑洞）
    Tile* tp = tile.get();
    auto sj = ms::Session::connect(cfg);
    if (!sj) {
        tile->err = "join: " + sj.error().message;
    } else {
        tile->sess = std::move(*sj);
        if (video) {
            auto cv = tp->sess.consume_video([tp](const mediaservo_client_frame_t& f) {
                tp->staging.push(f.data, f.len, f.width, f.height);
                tp->cb.fetch_add(1, std::memory_order_relaxed);
            });
            if (!cv) tile->err = "consume: " + cv.error().message;
        }
    }
    if (!tile->err.empty()) std::printf("[tile] %s: %s\n", room_id.c_str(), tile->err.c_str());
    log().add_fmt(tile->err.empty() ? "info" : "warn", "%s %s%s", video ? "join" : "pair-join",
                  room_id.c_str(), tile->err.empty() ? " ok" : (": " + tile->err).c_str());
    m.tiles.push_back(std::move(tile));
    // W4d 双房约定：流房（`<base>_<stream>`）自动并入整车房做控制面 tile
    // （G11 多会话形态；控制通道只在整车房建——PIT-140 v2 + W4c 定性）。
    const auto sep = room_id.rfind('_');
    if (video && sep != std::string::npos) {
        const std::string base = room_id.substr(0, sep);
        bool has = base.empty();
        for (auto& t : m.tiles)
            if (t->room == base) has = true;
        if (!has) join_room(m, env, base, false); // 递归仅此一处（旧形 std::function 自引用，搬迁改直递归=等价）
    }
}

void ensure_control(Tile& t) {
    if (t.ctl || !t.err.empty()) return;
    auto oc = t.sess.open_control({"chassis"});
    if (!oc) {
        t.err = "control: " + oc.error().message;
        log().add_fmt("warn", "control %s: %s", t.room.c_str(), oc.error().message.c_str());
        return;
    }
    log().add_fmt("info", "control up: %s (chassis)", t.room.c_str());
    t.ctl = std::make_unique<ms::Control>(std::move(*oc));
    Tile* tp = &t;
    t.ctl->on_ack([tp](const std::string& ack) {
        // ack 信封 `{"ack":<seq>,...}`；seq 匹配最近一发才计 RTT
        // （重发幂等窗内旧 ack 到达 = 只刷文本不刷 RTT）。
        uint64_t aseq = 0;
        ::viewer_core::json_u64(ack, "ack", &aseq);
        const int64_t now_us = std::chrono::duration_cast<std::chrono::microseconds>(
                                   std::chrono::steady_clock::now().time_since_epoch())
                                   .count();
        std::lock_guard<std::mutex> lk(tp->ack_mu);
        tp->ack_last = ack;
        if (aseq != 0 && aseq == tp->want_seq) {
            tp->rtt_ms = (now_us - tp->sent_us.load()) / 1000.0;
        }
    });
}

void perform_login(AppModel& m, const Env& env) {
    auto token = ms::login(env.http_base, m.user, m.pass);
    if (!token) {
        log().add_fmt("warn", "login failed: %s", token.error().message.c_str());
        m.status = "login: " + token.error().message;
        m.auto_join = false;
        return;
    }
    m.jwt = *token;
    auto lr = ms::list_rooms(env.http_base, m.jwt);
    if (!lr) {
        m.status = "list_rooms: " + lr.error().message;
        m.auto_join = false;
        return;
    }
    for (auto& [rid, kind] : parse_rooms(*lr)) {
        RoomRow r;
        r.room_id = rid;
        r.kind = kind;
        r.video = kind == "video"; // W4d：kind 三面直判，名字猜测退役
        m.rooms.push_back(std::move(r));
    }
    m.logged_in = true;
    m.status.clear();
    log().add_fmt("info", "login ok: %zu rooms listed", m.rooms.size());
    if (m.auto_join) {
        std::string target = env.room_hint;
        if (target.empty())
            for (auto& r : m.rooms)
                if (r.video) { target = r.room_id; break; }
        if (!target.empty()) join_room(m, env, target, true);
        else m.status = "auto-join: no video room";
        m.auto_join = false;
    }
}

} // namespace viewer
