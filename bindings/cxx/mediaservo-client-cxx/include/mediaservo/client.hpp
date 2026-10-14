/* MediaServo client C++ header-only binding (cockpit/consumer side) — login + video consumption + control DC.
 *
 * Thin wrapper over bindings/c/mediaservo-client-c/include/mediaservo/client.h (D247).
 * S6 batch 1b mirror: Session.state()/on_state(), wait_video(), consume()→Consumer (multi-consumer);
 * Control.on_ack returns a token + off_ack()/ready_state()/buffered_amount(); Error carries
 * wire_code/retryable (read back via the session handle's error_t). The old consume_video/video_stats bridges are kept
 * (internally the C ⊘ forms, behavior unchanged for one period).
 *
 * Lifecycle contract (translated from the C ABI header into RAII guarantees, the same discipline as link.hpp):
 *   - Session/Control/Consumer are all move-only; the destructor closes automatically (idempotent).
 *   - Default construction = closed (null handle); calling an API on a closed object returns
 *     Error{INVALID_ARG, "closed"} without touching the C ABI.
 *   - Frame/ack/state callbacks (trampolines) fire on their own internal pump threads. The registration heap objects are freed through
 *     the C-side user_free contract (on_state→session_close; consume→consumer_close;
 *     on_ack→the off_ack pump reclamation round or control_close) — zero raw deletes leak on the C++ side;
 *     when registration fails (the C side never took over), this header's delete is the backstop. Only consume_video (⊘ the C form has
 *     no user_free) keeps its callback heap object in cbs_ and frees it at close (the pump has been joined, no UAF).
 *   - Calling close/other methods from within a callback is forbidden (the C contract).
 *   - The error channel is Result (not exceptions); misusing value()/error() throws tl::bad_expected_access<Error>.
 */
#ifndef MEDIASERVO_CLIENT_HPP
#define MEDIASERVO_CLIENT_HPP

#pragma once

#include <chrono>
#include <cstdint>
#include <functional>
#include <string>
#include <utility>
#include <vector>

#include <mediaservo/common.h>
#include <mediaservo/client.h>
#include <mediaservo/detail/result.hpp>

namespace mediaservo {
namespace client {
using mediaservo::Error;
using mediaservo::Result;

/// Session connection state (wire values 0..3 = MEDIASERVO_CLIENT_STATE_*).
enum class State : uint8_t {
    Disconnected = 0,
    Connected = 1,
    Reconnecting = 2,
    Failed = 3,
};

/// DC ready state (wire values 0..3 = MEDIASERVO_CLIENT_DC_*).
enum class DcState : uint8_t {
    Connecting = 0,
    Open = 1,
    Closing = 2,
    Closed = 3,
};

namespace detail {

/// Build an error from the C ABI global last_error (the handle-less form: free functions / guard paths).
inline Error make_error(int code) {
    char buf[512];
    mediaservo_client_last_error(buf, sizeof(buf));
    return Error{code, std::string(buf), 0, false};
}

/// The session-handle form: machine-readable bits from session_error, text from session_last_error (K3-precise attribution).
inline Error make_error(int code, mediaservo_client_session_t* h) {
    char buf[512];
    uint16_t wire = 0;
    bool retry = false;
    if (h) {
        mediaservo_client_error_t e;
        e.struct_size = sizeof(e);
        e.code = 0;
        e.wire_code = 0;
        e.retryable = 0;
        if (mediaservo_client_session_error(h, &e) == MEDIASERVO_OK) {
            wire = static_cast<uint16_t>(e.wire_code);
            retry = e.retryable != 0;
        }
        mediaservo_client_session_last_error(h, buf, sizeof(buf));
    } else {
        mediaservo_client_last_error(buf, sizeof(buf));
    }
    return Error{code, std::string(buf), wire, retry};
}

/// needed overflow contract + one automatic grow-retry (shared core by list_rooms/video_stats/producer_ids/wait_video/
/// consumer id+stats; >64KiB is rejected — over-limit reports INVALID_ARG with the required-length semantics).
/// Non-empty h = the session error channel (precise wire/retryable); empty = the global last_error.
inline Result<std::string> needed_read(
    mediaservo_client_session_t* h,
    const std::function<int(char*, size_t, size_t*)>& call) {
    size_t cap = 4096;
    for (int attempt = 0; attempt < 2; ++attempt) {
        std::vector<char> buf(cap, 0);
        size_t need = 0;
        int rc = call(buf.data(), cap, &need);
        if (rc == MEDIASERVO_OK) {
            return Result<std::string>(std::string(buf.data()));
        }
        if (need > cap && attempt == 0 && need <= 65536) {
            cap = need;
            continue;
        }
        return Result<std::string>(tl::unexpect, make_error(rc, h));
    }
    return Result<std::string>(tl::unexpect, make_error(MEDIASERVO_CLIENT_ERR_INVALID_ARG, h));
}

} // namespace detail

/// SDK version (MAJOR.MINOR.PATCH).
inline Result<std::string> version() {
    char buf[64];
    int rc = mediaservo_client_version(buf, sizeof(buf));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(buf));
}

/// Account login for a JWT (blocking; mirrors the batch-1b flat C form). Overflow auto-grows once via the needed feedback.
inline Result<std::string> login(const std::string& http_base_url,
                                 const std::string& username,
                                 const std::string& password) {
    return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
        return mediaservo_client_login(http_base_url.c_str(), username.c_str(),
                                       password.c_str(), b, c, n);
    });
}

/// Exchange an API key for a short JWT (blocking; mirrors the accountless-client-auth T2 C surface). Fill the output verbatim into
/// Config::jwt to use the existing /ws gate. 401 (unknown key/wrong secret, enumeration-safe) → error.code=UNAUTHORIZED.
inline Result<std::string> exchange(const std::string& http_base_url,
                                    const std::string& key_id,
                                    const std::string& secret) {
    return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
        return mediaservo_client_exchange(http_base_url.c_str(), key_id.c_str(),
                                          secret.c_str(), b, c, n);
    });
}

/// Room discovery (blocking, a pre-session free function). Returns the JSON array
/// `[{"room_id":..,"kind":..}]` (same shape as producer_ids: header-only with no JSON dependency,
/// the caller parses it). Overflow auto-grows once via the needed feedback.
inline Result<std::string> list_rooms(const std::string& http_base, const std::string& jwt) {
    return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
        return mediaservo_client_list_rooms(http_base.c_str(), jwt.c_str(), b, c, n);
    });
}

/// Session configuration (mirrors mediaservo_client_config_t). Exactly one of jwt/psk non-empty (validated at connect).
struct Config {
    std::string signaling_url; // "ws://host:9800/ws"
    std::string room;          // Room ID
    std::string jwt;           // the login() output (pick one of jwt/psk)
    std::string hmac_key_file; // emergency-stop key file (0600; empty = do not sign; semantics in client.h)
    std::string psk;           // PSK passed directly (pick one of jwt/psk)
    /// Device identity instance directory (viewer-auth-matrix T1; identity.json+signing.pem in the
    /// same layout as msrtc-host init). Empty = disabled; composes with jwt/psk (server device auth takes priority).
    std::string identity_dir;
    /// "Client" (default)/"Viewer"/"Remote"; empty string = "Client".
    std::string role;
};

class Consumer;

/// Room session (move-only RAII; the destructor closes automatically; default construction = closed).
class Session {
public:
    /// Connect signaling and join the room (blocking; returns an error on failure, throws nothing).
    /// The T4/F6 free function identity_label needs to read the private C handle.
    friend Result<std::string> identity_label(const Session& s);

    static Result<Session> connect(const Config& cfg) {
        mediaservo_client_config_t c = MEDIASERVO_CLIENT_CONFIG_DEFAULT;
        c.signaling_url = cfg.signaling_url.empty() ? nullptr : cfg.signaling_url.c_str();
        c.room = cfg.room.empty() ? nullptr : cfg.room.c_str();
        c.jwt = cfg.jwt.empty() ? nullptr : cfg.jwt.c_str();
        c.psk = cfg.psk.empty() ? nullptr : cfg.psk.c_str();
        c.role = cfg.role.empty() ? nullptr : cfg.role.c_str();
        c.hmac_key_file = cfg.hmac_key_file.empty() ? nullptr : cfg.hmac_key_file.c_str();
        c.identity_dir = cfg.identity_dir.empty() ? nullptr : cfg.identity_dir.c_str();

        mediaservo_client_session_t* h = nullptr;
        int rc = mediaservo_client_session_create(&c, &h);
        if (rc != MEDIASERVO_OK) {
            return Result<Session>(tl::unexpect, detail::make_error(rc));
        }
        return Result<Session>(Session(h));
    }

    /// Default construction = closed session.
    Session() noexcept = default;
    ~Session() { (void)close(); }

    Session(const Session&) = delete;
    Session& operator=(const Session&) = delete;
    Session(Session&& other) noexcept : h_(other.release_()), cbs_(std::move(other.cbs_)) {}
    Session& operator=(Session&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
            cbs_ = std::move(other.cbs_);
        }
        return *this;
    }

    /// Whether a valid session is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// The negotiated dialect version (S0; an old server = 1).
    Result<uint32_t> negotiated() const {
        if (!h_) return Result<uint32_t>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        uint32_t p = 0;
        int rc = mediaservo_client_session_negotiated(h_, &p);
        if (rc != MEDIASERVO_OK) {
            return Result<uint32_t>(tl::unexpect, detail::make_error(rc, h_));
        }
        return Result<uint32_t>(p);
    }

    /// Session connection-state snapshot (the reconnect-loop observation surface, K1).
    Result<State> state() const {
        if (!h_) return Result<State>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        uint8_t st = 0;
        int rc = mediaservo_client_session_state(h_, &st);
        if (rc != MEDIASERVO_OK) {
            return Result<State>(tl::unexpect, detail::make_error(rc, h_));
        }
        return Result<State>(static_cast<State>(st));
    }

    /// Register a connection-state change callback (cumulative form: multiple registrations = every change invokes each entry; no unregister,
    /// released together at close by the C side. Reports only changes — fetch the initial state with state()).
    Result<void> on_state(std::function<void(State)> cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        auto* f = new std::function<void(State)>(std::move(cb));
        int rc = mediaservo_client_session_on_state(h_, &Session::state_trampoline, f,
                                                    &Session::state_user_free);
        if (rc != MEDIASERVO_OK) {
            delete f; // the C side never took over; this-side backstop (no leaks)
            return Result<void>(tl::unexpect, detail::make_error(rc, h_));
        }
        return Result<void>();
    }

    /// Purely wait for a video producer in the room (no side effects; the directed entry point for multi-consumer orchestration). Timeout/disconnect = error.
    Result<std::string> wait_video(uint64_t timeout_ms) {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(h_, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_session_wait_video(h_, timeout_ms, b, c, n);
        });
    }

    /// producer-label T2.2: wait directed by label (adjudication §12-1's device-room+label form).
    /// Empty label = no filter (falls back to the first consumer); all other contracts match wait_video.
    Result<std::string> wait_video_labeled(const std::string& label, uint64_t timeout_ms) {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(h_, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_session_wait_video_labeled(
                h_, label.empty() ? nullptr : label.c_str(), timeout_ms, b, c, n);
        });
    }

    /// Subscribe to one video producer (the new multi-consumer form, K4: one independent pump per producer, no cross coupling). The returned
    /// Consumer is move-only RAII (the destructor closes automatically; Session.close does not implicitly close it).
    Result<Consumer> consume(const std::string& producer_id,
                             std::function<void(const mediaservo_client_frame_t&)> cb);

    /// ⊘ Consume the room's first video (the first-consumer bridge, kept for one period; new code should use wait_video+consume).
    /// A repeat call = ERR_STATE. The frame-callback heap object stays in cbs_ and is freed at close (the ⊘ C form has no user_free).
    Result<void> consume_video(std::function<void(const mediaservo_client_frame_t&)> cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        auto* f = new std::function<void(const mediaservo_client_frame_t&)>(std::move(cb));
        cbs_.push_back(f); // ponytail: the ⊘ form has no C-side reclamation, so old callback objects are kept until close frees them (the pump thread may be executing them, preventing UAF)
        int rc = mediaservo_client_session_consume_video(h_, &Session::frame_trampoline, f);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc, h_));
        }
        return Result<void>();
    }

    /// ⊘ video stats aggregate JSON (session-level union, kept for one period; for precise per-consumer readings use
    /// Consumer::stats(); keys in client.h; imgui_shell's core::RateEstimator is the reference consumer).
    Result<std::string> video_stats() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(h_, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_session_video_stats(h_, b, c, n);
        });
    }

    /// Open the outbound control channel set (once per session; labels like {"chassis"}).
    Result<class Control> open_control(const std::vector<std::string>& labels);

    /// Emergency-stop dual-path delivery (W4b; the signing state = Config.hmac_key_file). OK = delivery succeeded;
    /// the vehicle's execution adjudication is ctl's seq receipt ack. payload_json "" = null.
    Result<void> emergency_stop(const class Control& ctl, const std::string& label,
                                uint64_t seq, const std::string& payload_json);

    /// Close the session and release the handle (idempotent; after joining the pumps + the C side's per-entry user_free of state callbacks,
    /// this side then frees the ⊘ consume_video cbs_ heap objects).
    Result<void> close() noexcept {
        if (!h_ && cbs_.empty()) return Result<void>();
        int rc = MEDIASERVO_OK;
        if (h_) {
            rc = mediaservo_client_session_close(h_);
            h_ = nullptr;
        }
        for (auto* cb : cbs_) delete cb;
        cbs_.clear();
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

private:
    friend class Consumer;
    explicit Session(mediaservo_client_session_t* h) : h_(h) {}
    mediaservo_client_session_t* release_() noexcept {
        mediaservo_client_session_t* p = h_;
        h_ = nullptr;
        return p;
    }
    static void frame_trampoline(mediaservo_client_session_t*, const mediaservo_client_frame_t* frame, void* user) {
        if (frame && user) {
            (*static_cast<std::function<void(const mediaservo_client_frame_t&)>*>(user))(*frame);
        }
    }
    static void frame_user_free(void* user) {
        delete static_cast<std::function<void(const mediaservo_client_frame_t&)>*>(user);
    }
    static void state_trampoline(mediaservo_client_session_t*, uint8_t st, void* user) {
        if (user) {
            (*static_cast<std::function<void(State)>*>(user))(static_cast<State>(st));
        }
    }
    static void state_user_free(void* user) {
        delete static_cast<std::function<void(State)>*>(user);
    }
    mediaservo_client_session_t* h_ = nullptr;
    std::vector<std::function<void(const mediaservo_client_frame_t&)>*> cbs_;
};

/// Single video consumer (move-only RAII; the destructor closes automatically; default construction = closed).
/// PSK room discovery (psk-discover: the Legacy full view, the discovery surface for cross-room pulls).
inline Result<std::string> list_rooms_psk(const std::string& http_base, const std::string& psk) {
    return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
        return mediaservo_client_list_rooms_psk(http_base.c_str(), psk.c_str(), b, c, n);
    });
}

/// The effective identity label (the T4/F6 mirror; zero network, a static value derived locally at create). Readable even on a closed session.
inline Result<std::string> identity_label(const Session& s) {
    return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
        return mediaservo_client_session_identity(s.h_, b, c, n); // the friend form: the free function reads the private handle
    });
}

class Consumer {
public:
    Consumer() noexcept = default;
    ~Consumer() { (void)close(); }

    Consumer(const Consumer&) = delete;
    Consumer& operator=(const Consumer&) = delete;
    Consumer(Consumer&& other) noexcept
        : h_(other.release_()), qoe_ctx_(other.qoe_ctx_) {
        other.qoe_ctx_ = nullptr;
    }
    Consumer& operator=(Consumer&& other) noexcept {
        if (this != &other) {
            (void)close();  // release our own qoe_ctx + the old handle
            h_ = other.release_();
            qoe_ctx_ = other.qoe_ctx_;
            other.qoe_ctx_ = nullptr;
        }
        return *this;
    }

    /// Whether a valid consumer is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// This consumer's producer id (a setup-time constant; closed/read failure = empty string).
    std::string id() const {
        if (!h_) return std::string();
        auto r = detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_consumer_id(h_, b, c, n);
        });
        return r ? r.value() : std::string();
    }

    /// This consumer's video stats JSON (per-consumer reading, K4; the key table matches Session::video_stats).
    Result<std::string> stats() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_consumer_stats(h_, b, c, n);
        });
    }

    /// QOE periodic callback (consumer-qoe-callback, 2026-10-09; aligned with TRRO OnMediaState):
    /// the SDK handles tick/diff/conversion internally — cb receives ready-converted values. Idempotent (replaces the previous); close stops it.
    /// cb fires on the SDK tick thread — **blocking inside the callback is forbidden**.
    struct QoeSnapshot {
        uint32_t fps;
        uint32_t kbps;
        float lost_percent;
        uint64_t decoded_total;
        uint32_t avg_decode_ms;  // average decode time ms/frame
        uint32_t jitter_ms;      // jitter in ms
        uint32_t rtt_ms;         // round-trip time ms (0 when the backend has no source)
        uint32_t width;
        uint32_t height;
        uint32_t epoch;  // renegotiation-breakpoint generation (for identification only)
    };

    Result<void> on_qoe(std::chrono::milliseconds period,
                        std::function<void(const QoeSnapshot&)> cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        // Heap-dangling protection: the cb lifetime = the C-side registration period. The C layer's PIT-221 join discipline:
        // consumer_close/re-registration has already abort+join'ed the old task before returning — the delete qoe_ctx_
        // ordering is hard-guaranteed by the C layer (freed only after callbacks physically stop firing).
        auto* ctx = new std::function<void(const QoeSnapshot&)>(std::move(cb));
        static_assert(sizeof(QoeSnapshot) >= sizeof(mediaservo_client_qoe_snapshot_t) - sizeof(uint32_t),
                      "QoeSnapshot field order drift — a C ABI compatibility break");
        int rc = mediaservo_client_consumer_on_qoe(
            h_, static_cast<uint32_t>(period.count()),
            [](const mediaservo_client_qoe_snapshot_t* snap, void* user) {
                auto* fn = static_cast<std::function<void(const QoeSnapshot&)>*>(user);
                QoeSnapshot q{snap->fps, snap->kbps, snap->lost_percent,
                              snap->decoded_total, snap->avg_decode_ms, snap->jitter_ms,
                              snap->rtt_ms, snap->width, snap->height, snap->epoch};
                (*fn)(q);
            },
            ctx);
        if (rc != MEDIASERVO_OK) {
            delete ctx;
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        // Idempotent replacement: the old ctx is freed (the new registration already had the C layer abort the old task — the old ctx is never triggered again)
        delete static_cast<std::function<void(const QoeSnapshot&)>*>(qoe_ctx_);
        qoe_ctx_ = ctx;
        return Result<void>();
    }

    /// Close this consumer and release the handle (idempotent; the C side joins the pump + aborts the qoe task + user_free's the frame-callback heap objects).
    Result<void> close() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_client_consumer_close(h_);
        h_ = nullptr;
        // qoe callback heap-context release (lsan.1096280 proved a 64B leak per registration):
        // the C close has already aborted the qoe task — callbacks no longer fire, so delete is safe here.
        delete static_cast<std::function<void(const QoeSnapshot&)>*>(qoe_ctx_);
        qoe_ctx_ = nullptr;
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

private:
    friend class Session;
    explicit Consumer(mediaservo_client_consumer_t* h) : h_(h) {}
    mediaservo_client_consumer_t* release_() noexcept {
        mediaservo_client_consumer_t* p = h_;
        h_ = nullptr;
        return p;
    }
    mediaservo_client_consumer_t* h_ = nullptr;
    /// qoe callback heap context (stored by on_qoe; freed at close — moved in the move ops below)
    void* qoe_ctx_ = nullptr;
};

/// Outbound control channel set (move-only RAII; the destructor closes automatically).
class Control {
public:
    /// Default construction = closed channel.
    Control() noexcept = default;
    ~Control() { (void)close(); }

    Control(const Control&) = delete;
    Control& operator=(const Control&) = delete;
    Control(Control&& other) noexcept : h_(other.release_()) {}
    Control& operator=(Control&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
        }
        return *this;
    }

    /// Whether a valid channel is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// Register an ack callback (cumulative form, batch 1b: multiple registrations = each ack invokes them one by one; the first starts the pump).
    /// Returns the unregistration credential token (Control::off_ack); the callback heap objects are freed by the C side at the off_ack pump
    /// reclamation round or at close (this-side delete is the backstop when registration fails).
    Result<uint64_t> on_ack(std::function<void(const std::string&)> cb) {
        if (!h_) return Result<uint64_t>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        auto* f = new std::function<void(const std::string&)>(std::move(cb));
        uint64_t token = 0;
        int rc = mediaservo_client_control_on_ack(h_, &Control::ack_trampoline, f,
                                                  &Control::ack_user_free, &token);
        if (rc != MEDIASERVO_OK) {
            delete f; // the C side never took over; this-side backstop (no leaks)
            return Result<uint64_t>(tl::unexpect, detail::make_error(rc));
        }
        return Result<uint64_t>(token);
    }

    /// Unregister an ack callback by token (the pump reclaims and frees the heap object next round; an in-flight round may still fire once).
    Result<void> off_ack(uint64_t token) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        int rc = mediaservo_client_control_off_ack(h_, token);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Send a command envelope (payload_json "" = a null payload; the caller maintains seq as a monotonic counter).
    Result<void> send(const std::string& label, uint64_t seq, const std::string& cmd,
                      const std::string& payload_json) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        const char* payload = payload_json.empty() ? nullptr : payload_json.c_str();
        int rc = mediaservo_client_control_send(h_, label.c_str(), seq, cmd.c_str(), payload);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// DC ready state (the gate before sending/estop, K5).
    Result<DcState> ready_state() const {
        if (!h_) return Result<DcState>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        uint8_t st = 0;
        int rc = mediaservo_client_control_ready_state(h_, &st);
        if (rc != MEDIASERVO_OK) {
            return Result<DcState>(tl::unexpect, detail::make_error(rc));
        }
        return Result<DcState>(static_cast<DcState>(st));
    }

    /// SCTP backpressure water level (bytes; 0 = safe to append, K5).
    Result<uint64_t> buffered_amount() {
        if (!h_) return Result<uint64_t>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        uint64_t bytes = 0;
        int rc = mediaservo_client_control_buffered_amount(h_, &bytes);
        if (rc != MEDIASERVO_OK) {
            return Result<uint64_t>(tl::unexpect, detail::make_error(rc));
        }
        return Result<uint64_t>(bytes);
    }

    /// JSON array of the server-assigned data producer ids (observation surface). Overflow auto-grows once via the needed
    /// feedback (the batch-1b upgrade).
    Result<std::string> producer_ids() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_control_producer_ids(h_, b, c, n);
        });
    }

    /// Close the control channel and release the handle (idempotent; after joining the ack pump, the C side reclaims the whole registry —
    /// heap std::function objects are freed via user_free; nothing lingers on the C++ side).
    Result<void> close() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_client_control_close(h_);
        h_ = nullptr;
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

private:
    friend class Session;
    explicit Control(mediaservo_client_control_t* h) : h_(h) {}
    mediaservo_client_control_t* release_() noexcept {
        mediaservo_client_control_t* p = h_;
        h_ = nullptr;
        return p;
    }
    static void ack_trampoline(mediaservo_client_control_t*, const char* ack_json, void* user) {
        if (ack_json && user) {
            (*static_cast<std::function<void(const std::string&)>*>(user))(std::string(ack_json));
        }
    }
    static void ack_user_free(void* user) {
        delete static_cast<std::function<void(const std::string&)>*>(user);
    }
    mediaservo_client_control_t* h_ = nullptr;
};

inline Result<Consumer> Session::consume(const std::string& producer_id,
                                         std::function<void(const mediaservo_client_frame_t&)> cb) {
    if (!h_) return Result<Consumer>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
    auto* f = new std::function<void(const mediaservo_client_frame_t&)>(std::move(cb));
    mediaservo_client_consumer_t* ch = nullptr;
    int rc = mediaservo_client_session_consume(h_, producer_id.c_str(), &Session::frame_trampoline,
                                               f, &Session::frame_user_free, &ch);
    if (rc != MEDIASERVO_OK) {
        delete f; // the C side never took over; this-side backstop (no leaks)
        return Result<Consumer>(tl::unexpect, detail::make_error(rc, h_));
    }
    return Result<Consumer>(Consumer(ch));
}

inline Result<void> Session::emergency_stop(const class Control& ctl, const std::string& label,
                                            uint64_t seq, const std::string& payload_json) {
    if (!h_ || !ctl.h_) {
        return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
    }
    const int rc = mediaservo_client_session_emergency_stop(
        h_, ctl.h_, label.c_str(), seq, payload_json.empty() ? nullptr : payload_json.c_str());
    if (rc != MEDIASERVO_OK) {
        return Result<void>(tl::unexpect, detail::make_error(rc, h_));
    }
    return Result<void>();
}

inline Result<class Control> Session::open_control(const std::vector<std::string>& labels) {
    if (!h_) return Result<Control>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
    std::vector<const char*> ptrs;
    ptrs.reserve(labels.size());
    for (const auto& l : labels) ptrs.push_back(l.c_str());
    mediaservo_client_control_t* c = nullptr;
    int rc = mediaservo_client_open_control(h_, ptrs.empty() ? nullptr : ptrs.data(), ptrs.size(), &c);
    if (rc != MEDIASERVO_OK) {
        return Result<Control>(tl::unexpect, detail::make_error(rc, h_));
    }
    return Result<Control>(Control(c));
}

} // namespace client
} // namespace mediaservo

#endif /* MEDIASERVO_CLIENT_HPP */
