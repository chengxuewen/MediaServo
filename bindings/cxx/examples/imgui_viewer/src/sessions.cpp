// sessions — 实现（T3 自 main.cpp join_room lambda 与登录分支迁居；语义逐字保持）。
#include "sessions.hpp"

#include <algorithm>
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
    cfg.signaling_url = m.url_ws;
    cfg.room = room_id;
    // T4：凭证按模式带（每会话都带——server 侧每次 join 独立鉴权）。
    if (m.auth == AuthMode::Psk) cfg.psk = m.psk;
    if (m.auth == AuthMode::Device) cfg.identity_dir = m.identity_dir;
    if (!m.jwt.empty()) cfg.jwt = m.jwt; // M1/M2/M5 换发/直贴产物
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

void close_room(AppModel& m, const std::string& room) {
    const auto it = std::find_if(m.tiles.begin(), m.tiles.end(),
                                 [&room](const std::unique_ptr<Tile>& t) { return t->room == room; });
    if (it == m.tiles.end()) return;
    log().add("info", "close " + room);
    m.tiles.erase(it);
}

void perform_login(AppModel& m, const Env& env) {
    // T4：五模式两段化——acquire（M1/M2 换发；M3/M4 无）→ discover（M3 跳；M5 失败容忍）。
    switch (m.auth) {
    case AuthMode::Account: {
        auto token = ms::login(m.url_http, m.user, m.pass);
        if (!token) {
            log().add_fmt("warn", "login failed: %s", token.error().message.c_str());
            m.status = "login: " + token.error().message;
            m.auto_join = false;
            return;
        }
        m.jwt = *token;
        break;
    }
    case AuthMode::ApiKey: {
        auto token = ms::exchange(m.url_http, m.key_id, m.key_secret);
        if (!token) {
            log().add_fmt("warn", "exchange failed: %s", token.error().message.c_str());
            m.status = "exchange: " + token.error().message;
            m.auto_join = false;
            return;
        }
        m.jwt = *token;
        break;
    }
    case AuthMode::Psk:
        m.jwt.clear(); // M3：无 JWT；join 带 psk（上）
        break;
    case AuthMode::Device:
        // M4：Device 身份无 REST 发证（D283 wire 只在 WS join 面）——发现走不了，
        // 与 M3 同形直 join（房名 direct_room/MSRTC_ROOM）。
        m.jwt.clear();
        break;
    case AuthMode::JwtPaste:
        m.jwt = m.jwt_paste; // M5：直贴（坏 token 由 join 期 4013 红牌裁决）
        break;
    }
    // 发现段（psk-discover 后）：M3 有 PSK 即走发现（Legacy 全量视角——跨房间拉流主路）；
    // 填了 direct_room 则直连（可选覆盖）；M4 Device 仍无 REST 发证面 → 直 join；
    // M5 失败容忍（坏 token 测试点在 join 期红牌，非发现）。
    if (m.auth == AuthMode::Psk && m.direct_room[0]) {
        m.rooms.clear();
        if (!m.direct_room[0]) {
            m.status = "psk: room required (direct join)";
            m.auto_join = false;
            return;
        }
        RoomRow r;
        r.room_id = m.direct_room;
        // F2 kind 猜测：`_` 分隔名按流房（video）处理，否则整车房（control——不占格）。
        r.kind = r.room_id.find('_') != std::string::npos ? "video" : "control";
        r.video = r.kind == "video";
        m.rooms.push_back(std::move(r));
        m.logged_in = true;
        m.status.clear();
        log().add_fmt("info", "psk direct-join target: %s", m.rooms[0].room_id.c_str());
    } else if (m.auth == AuthMode::Psk) {
        // psk-discover：Legacy 全量列表——与账号模式同形（树勾选多房=跨房间拉流）。
        auto lr = ms::list_rooms_psk(m.url_http, m.psk);
        if (!lr) {
            m.status = "psk discover: " + lr.error().message;
            m.auto_join = false;
            return;
        }
        m.rooms.clear();
        for (auto& [rid, kind] : parse_rooms(*lr)) {
            RoomRow r;
            r.room_id = rid;
            r.kind = kind;
            r.video = kind == "video";
            m.rooms.push_back(std::move(r));
        }
        m.logged_in = true;
        m.status.clear();
        std::printf("[discover] psk rooms=%zu\n", m.rooms.size()); // 无头判据行
        log().add_fmt("info", "psk discover ok: %zu rooms", m.rooms.size());
    } else if (m.auth == AuthMode::Device) {
        m.rooms.clear();
        if (!m.direct_room[0] && m.identity_dir[0]) {
            // Device 无 REST 发证面（D283 挑战只在 WS）→ 直 join 语义；房名从
            // identity 目录旁的约定取不可能——由 MSRTC_ROOM/UI 提供。
        }
        if (!m.direct_room[0]) {
            m.status = "device: room required (no REST discovery)";
            m.auto_join = false;
            return;
        }
        RoomRow r;
        r.room_id = m.direct_room;
        r.kind = r.room_id.find('_') != std::string::npos ? "video" : "control";
        r.video = r.kind == "video";
        m.rooms.push_back(std::move(r));
        m.logged_in = true;
        m.status.clear();
        log().add_fmt("info", "device direct-join target: %s", m.rooms[0].room_id.c_str());
    } else {
        auto lr = ms::list_rooms(m.url_http, m.jwt);
        if (!lr) {
            if (m.auth == AuthMode::JwtPaste) {
                log().add_fmt("warn", "list_rooms failed (jwt paste): %s — join will judge", lr.error().message.c_str());
                m.rooms.clear();
                m.logged_in = true; // 放行到 join 期 4013 红牌（M5b 判据）
                m.status.clear();
                goto auto_join; // 不能 return——4013 判决点在下发 join
            }
            m.status = "list_rooms: " + lr.error().message;
            m.auto_join = false;
            return;
        }
        m.rooms.clear();
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
    }
auto_join:
    if (m.auto_join) {
        std::string target = env.room_hint;
        if (target.empty())
            for (auto& r : m.rooms)
                if (r.video) { target = r.room_id; break; }
        // M5b：发现被 401 打掉=rooms 空——仍按 hint 直 join，4013 判决点在 join 期。
        if (target.empty() && m.auth == AuthMode::JwtPaste) target = "vehicle_probe";
        if (!target.empty()) join_room(m, env, target, true);
        else m.status = "auto-join: no video room";
        m.auto_join = false;
    }
}

void refresh_identity_label(AppModel& m) {
    // T4/F6：任一活动会话取身份标签（静态快照，同值；无会话=空显示）。
    for (auto& t : m.tiles)
        if (t->sess) {
            auto lb = ms::identity_label(t->sess);
            m.identity_label = lb ? *lb : "";
            return;
        }
    m.identity_label.clear();
}

} // namespace viewer
