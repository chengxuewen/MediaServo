// sessions — implementation (the join_room lambda and login branches relocated from main.cpp in T3; semantics preserved verbatim).
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
    // T4: attach credentials per mode (every session carries them — the server authenticates each join independently).
    if (m.auth == AuthMode::Psk) cfg.psk = m.psk;
    if (m.auth == AuthMode::Device) cfg.identity_dir = m.identity_dir;
    if (!m.jwt.empty()) cfg.jwt = m.jwt; // the M1/M2/M5 exchange/paste output
    cfg.role = "Client";
    if (!env.key_file.empty()) {
        cfg.hmac_key_file = env.key_file;
        m.key_signed = true;
    }
    auto tile = std::make_unique<Tile>(room_id);
    tile->video = video; // a control-room tile does not consume (the whole-vehicle room has no media = a wait-producer black hole)
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
    // The W4d dual-room convention: a stream room (`<base>_<stream>`) is auto-paired with the whole-vehicle room as a control-plane tile
    // (the G11 multi-session form; the control channel is created only in the whole-vehicle room — PIT-140 v2 + the W4c ruling).
    const auto sep = room_id.rfind('_');
    if (video && sep != std::string::npos) {
        const std::string base = room_id.substr(0, sep);
        bool has = base.empty();
        for (auto& t : m.tiles)
            if (t->room == base) has = true;
        if (!has) join_room(m, env, base, false); // the only recursion site (the old form used a self-referencing std::function; direct recursion after relocation = equivalent)
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
        // the ack envelope {"ack":<seq>,...}; RTT counts only when the seq matches the most recent send
        // (an old ack arriving within the idempotent re-send window refreshes the text but not the RTT).
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
    // T4: the five modes in two phases — acquire (M1/M2 exchange; M3/M4 have none) → discover (M3 skips; M5 tolerates failure).
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
        m.jwt.clear(); // M3: no JWT; join carries the psk (above)
        break;
    case AuthMode::Device:
        // M4: Device identity has no REST issuance (the D283 wire exists only on the WS join face) — discovery is impossible,
        // so join directly like M3 (the room name from direct_room/MSRTC_ROOM).
        m.jwt.clear();
        break;
    case AuthMode::JwtPaste:
        m.jwt = m.jwt_paste; // M5: paste directly (a bad token is adjudicated by the 4013 red card at join time)
        break;
    }
    // The discover phase (after psk-discover): M3 goes through discovery when it has a PSK (the Legacy full view — the main path for cross-room pulls);
    // with direct_room filled it connects directly (an optional override); M4's Device still has no REST issuance face → join directly;
    // M5 tolerates failure (the bad-token test point is the join-time red card, not discovery).
    if (m.auth == AuthMode::Psk && m.direct_room[0]) {
        m.rooms.clear();
        if (!m.direct_room[0]) {
            m.status = "psk: room required (direct join)";
            m.auto_join = false;
            return;
        }
        RoomRow r;
        r.room_id = m.direct_room;
        // The F2 kind guess: a `_`-separated name is treated as a stream room (video); otherwise a whole-vehicle room (control — takes no grid slot).
        r.kind = r.room_id.find('_') != std::string::npos ? "video" : "control";
        r.video = r.kind == "video";
        m.rooms.push_back(std::move(r));
        m.logged_in = true;
        m.status.clear();
        log().add_fmt("info", "psk direct-join target: %s", m.rooms[0].room_id.c_str());
    } else if (m.auth == AuthMode::Psk) {
        // psk-discover: the Legacy full list — the same form as the account modes (checking multiple rooms in the tree = cross-room pulls).
        auto lr = ms::list_rooms_psk(m.url_http, m.psk);
        if (!lr) {
            m.status = "psk discover: " + lr.error().message;
            m.auto_join = false;
            return;
        }
        m.rooms.clear();
        for (auto& [rid, kind, parent] : parse_rooms_full(*lr)) {
            RoomRow r;
            r.room_id = rid;
            r.kind = kind;
            r.parent = parent;
            r.video = kind == "video";
            m.rooms.push_back(std::move(r));
        }
        m.logged_in = true;
        m.status.clear();
        std::printf("[discover] psk rooms=%zu\n", m.rooms.size()); // the headless judgment line
        log().add_fmt("info", "psk discover ok: %zu rooms", m.rooms.size());
    } else if (m.auth == AuthMode::Device) {
        m.rooms.clear();
        if (!m.direct_room[0] && m.identity_dir[0]) {
            // Device has no REST issuance face (the D283 challenge exists only on WS) → direct-join semantics; the room name cannot
            // come from a convention beside the identity directory — it is provided by MSRTC_ROOM/the UI.
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
                m.logged_in = true; // let it through to the join-time 4013 red card (the M5b judgment)
                m.status.clear();
                goto auto_join; // cannot return — the 4013 adjudication point is in the join below
            }
            m.status = "list_rooms: " + lr.error().message;
            m.auto_join = false;
            return;
        }
        m.rooms.clear();
        for (auto& [rid, kind, parent] : parse_rooms_full(*lr)) {
            RoomRow r;
            r.room_id = rid;
            r.kind = kind;
            r.parent = parent;
            r.video = kind == "video"; // W4d: judged directly from the three-kind face; the name guessing is retired
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
        // M5b: discovery knocked out by a 401 = empty rooms — still join directly by the hint; the 4013 adjudication point is at join time.
        if (target.empty() && m.auth == AuthMode::JwtPaste) target = "vehicle_probe";
        if (!target.empty()) join_room(m, env, target, true);
        else m.status = "auto-join: no video room";
        m.auto_join = false;
    }
}

void refresh_identity_label(AppModel& m) {
    // T4/F6: take the identity label from any active session (a static snapshot, identical values; no session = empty display).
    for (auto& t : m.tiles)
        if (t->sess) {
            auto lb = ms::identity_label(t->sess);
            m.identity_label = lb ? *lb : "";
            return;
        }
    m.identity_label.clear();
}

} // namespace viewer
