/* MediaServo link C++ header-only binding (signaling + frame bus) — device-side C++ consumption.
 *
 * Thin wrapper over bindings/c/mediaservo-link-c/include/mediaservo/link.h (D247).
 * Lifecycle contract (translated from the C ABI header into RAII guarantees):
 *   - SignalSession/Bus/Stream are all move-only; the destructor closes automatically (idempotent).
 *   - Default construction = closed (null handle); calling an API on a closed object returns
 *     Error{INVALID_ARG, "closed"} without touching the C ABI.
 *   - Event/frame callbacks (trampolines) fire on the internal pump thread; the heap std::function objects are
 *     released together after close (which joins the pump thread), preventing use-after-free. Calling close/on_event from within a callback is forbidden (the C contract); re-registration does not free the old callback object (the pump thread may be executing it); it is freed together with close — registrations are usually 1, with a bounded upper limit.
 *     calling close/on_event from within a callback is forbidden (the C contract); re-registration does not free the old
 *     callback object (the pump thread may be executing it); it is freed together with close — usually 1 registration, bounded upper limit.
 *   - The error channel is Result (not exceptions); misusing value()/error() throws tl::bad_expected_access<Error>.
 */
#ifndef MEDIASERVO_LINK_HPP
#define MEDIASERVO_LINK_HPP

#pragma once

#include <cstdint>
#include <functional>
#include <stdexcept>
#include <string>
#include <utility>
#include <variant>
#include <vector>

#include <mediaservo/common.h>
#include <mediaservo/link.h>
#include <mediaservo/detail/result.hpp>

namespace mediaservo {
namespace link {
using mediaservo::Error;
using mediaservo::Result;

/// Error details (code is a MEDIASERVO_LINK_ERR_* value from mediaservo/link.h; message read from last_error).
;

namespace detail {

/// Build an error from the C ABI last_error (call after a C API returns <0).
inline Error make_error(int code) {
    char buf[512];
    mediaservo_link_last_error(buf, sizeof(buf));
    return Error{code, std::string(buf)};
}

} // namespace detail





/// SDK version (MAJOR.MINOR.PATCH).
inline Result<std::string> version() {
    char buf[64];
    int rc = mediaservo_link_version(buf, sizeof(buf));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(buf));
}

/// Signaling configuration (mirrors mediaservo_link_signal_config_t).
struct SignalConfig {
    std::string url;
    std::string psk;
    std::string room;
    /// "Host"/"Pusher" → Host, "Client"/"Puller" → Remote; empty string = Host.
    std::string role;
};

/// Signaling session (move-only RAII; the destructor closes automatically; default construction = closed).
class SignalSession {
public:
    /// Connect signaling and create the session (blocking; returns an error on failure, throws nothing).
    static Result<SignalSession> connect(const SignalConfig& cfg) {
        mediaservo_link_signal_config_t c = MEDIASERVO_LINK_SIGNAL_CONFIG_DEFAULT;
        c.url = cfg.url.empty() ? nullptr : cfg.url.c_str();
        c.psk = cfg.psk.empty() ? nullptr : cfg.psk.c_str();
        c.room = cfg.room.empty() ? nullptr : cfg.room.c_str();
        c.role = cfg.role.empty() ? nullptr : cfg.role.c_str();

        mediaservo_link_signal_t* h = nullptr;
        int rc = mediaservo_link_signal_connect(&c, &h);
        if (rc != MEDIASERVO_OK) {
            return Result<SignalSession>(tl::unexpect, detail::make_error(rc));
        }
        return Result<SignalSession>(SignalSession(h));
    }

    /// Default construction = closed session.
    SignalSession() noexcept = default;
    ~SignalSession() { (void)close(); }

    SignalSession(const SignalSession&) = delete;
    SignalSession& operator=(const SignalSession&) = delete;
    SignalSession(SignalSession&& other) noexcept : h_(other.release_()), cbs_(std::move(other.cbs_)) {}
    SignalSession& operator=(SignalSession&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
            cbs_ = std::move(other.cbs_);
        }
        return *this;
    }

    /// Whether a valid session is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// Send one signaling message (JSON; the SignalingMessage type tag is snake_case).
    Result<void> send(const std::string& json) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_LINK_ERR_INVALID_ARG, "closed"});
        int rc = mediaservo_link_signal_send(h_, json.c_str(), json.size());
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Register the event callback (any time after connect; re-registration replaces; the callback fires on the internal pump thread,
    /// the event string is valid only inside the callback — copy it to keep it; calling any method on this object from within the callback is forbidden).
    void on_event(std::function<void(const std::string&)> cb) {
        if (!h_) return; // closed: no-op
        auto* f = new std::function<void(const std::string&)>(std::move(cb));
        cbs_.push_back(f); // ponytail: old callback objects are kept until close frees them (the pump thread may be executing them, preventing UAF);
                           // registrations are usually 1; the upper bound = the registration count, bounded.
        mediaservo_link_signal_on_event(h_, &SignalSession::trampoline, f);
    }

    /// Close the session and release the handle (idempotent; callback objects are freed only after joining the event pump).
    Result<void> close() noexcept {
        if (!h_ && cbs_.empty()) return Result<void>();
        int rc = MEDIASERVO_OK;
        if (h_) {
            rc = mediaservo_link_signal_close(h_);
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
    explicit SignalSession(mediaservo_link_signal_t* h) : h_(h) {}
    mediaservo_link_signal_t* release_() noexcept {
        mediaservo_link_signal_t* p = h_;
        h_ = nullptr;
        return p;
    }
    static void trampoline(mediaservo_link_signal_t*, const char* event_json, void* user) {
        (*static_cast<std::function<void(const std::string&)>*>(user))(std::string(event_json));
    }
    mediaservo_link_signal_t* h_ = nullptr;
    std::vector<std::function<void(const std::string&)>*> cbs_;
};

/// Frame bus (move-only RAII; the destructor closes automatically).
class Bus {
public:
    /// Attach the frame bus (signature verification + ACL + iceoryx2 node, blocking). endpoint is reserved for Phase 1 (an empty string is fine).
    static Result<Bus> attach(const std::string& endpoint, const std::string& token_pem, const std::string& vk_pem) {
        mediaservo_link_bus_t* b = nullptr;
        int rc = mediaservo_link_bus_attach(endpoint.c_str(), token_pem.c_str(), vk_pem.c_str(), &b);
        if (rc != MEDIASERVO_OK) {
            return Result<Bus>(tl::unexpect, detail::make_error(rc));
        }
        return Result<Bus>(Bus(b));
    }

    /// Default construction = closed bus.
    Bus() noexcept = default;
    ~Bus() { (void)close(); }

    Bus(const Bus&) = delete;
    Bus& operator=(const Bus&) = delete;
    Bus(Bus&& other) noexcept : h_(other.release_()) {}
    Bus& operator=(Bus&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
        }
        return *this;
    }

    /// Whether a valid bus is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// Publish one frame (ACL check + SHM loan + send, blocking).
    Result<void> publish(const std::string& topic, const std::vector<uint8_t>& payload, const mediaservo_frame_meta_t& meta) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_LINK_ERR_INVALID_ARG, "closed"});
        const uint8_t* data = payload.empty() ? nullptr : payload.data(); // C contract: payload is NULL iff len==0
        int rc = mediaservo_link_bus_publish(h_, topic.c_str(), data, payload.size(), &meta);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Subscribe to a topic and create a frame stream (blocking).
    Result<class Stream> subscribe(const std::string& topic);

    /// Close the frame bus and release the handle (idempotent; shuts down all streams, stream recv returns CLOSED).
    Result<void> close() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_link_bus_close(h_);
        h_ = nullptr;
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

private:
    explicit Bus(mediaservo_link_bus_t* h) : h_(h) {}
    mediaservo_link_bus_t* release_() noexcept {
        mediaservo_link_bus_t* p = h_;
        h_ = nullptr;
        return p;
    }
    mediaservo_link_bus_t* h_ = nullptr;
};

/// One frame (metadata + payload copy).
struct Frame {
    mediaservo_frame_meta_t meta{};
    std::vector<uint8_t> data;
};

/// Frame stream (move-only RAII; the destructor closes automatically).
class Stream {
public:
    /// Default construction = closed stream.
    Stream() noexcept = default;
    ~Stream() { (void)close(); }

    Stream(const Stream&) = delete;
    Stream& operator=(const Stream&) = delete;
    Stream(Stream&& other) noexcept : h_(other.release_()) {}
    Stream& operator=(Stream&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
        }
        return *this;
    }

    /// Whether a valid stream is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// Blocking frame fetch (metadata + payload copy).
    Result<Frame> recv() {
        if (!h_) return Result<Frame>(tl::unexpect, Error{MEDIASERVO_LINK_ERR_INVALID_ARG, "closed"});
        Frame frame;
        // ponytail: a single 16MiB buffer covers 4K I420 (12.4MiB); the C ABI cannot detect truncation
        // (meta is known only after fetching the frame); larger frames need a bigger buffer — upgrade path: cache the previous
        // frame size from meta and grow adaptively.
        frame.data.resize(16 * 1024 * 1024);
        size_t len = 0;
        int rc = mediaservo_link_bus_recv(h_, &frame.meta, frame.data.data(), frame.data.size(), &len);
        if (rc != MEDIASERVO_OK) {
            return Result<Frame>(tl::unexpect, detail::make_error(rc));
        }
        frame.data.resize(len);
        return Result<Frame>(std::move(frame));
    }

    /// Close the frame stream and release the handle (idempotent; wakes a blocked recv so it returns CLOSED).
    Result<void> close() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_link_stream_close(h_);
        h_ = nullptr;
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

private:
    friend class Bus;
    explicit Stream(mediaservo_link_stream_t* h) : h_(h) {}
    mediaservo_link_stream_t* release_() noexcept {
        mediaservo_link_stream_t* p = h_;
        h_ = nullptr;
        return p;
    }
    mediaservo_link_stream_t* h_ = nullptr;
};

inline Result<class Stream> Bus::subscribe(const std::string& topic) {
    if (!h_) return Result<Stream>(tl::unexpect, Error{MEDIASERVO_LINK_ERR_INVALID_ARG, "closed"});
    mediaservo_link_stream_t* st = nullptr;
    int rc = mediaservo_link_bus_subscribe(h_, topic.c_str(), &st);
    if (rc != MEDIASERVO_OK) {
        return Result<Stream>(tl::unexpect, detail::make_error(rc));
    }
    return Result<Stream>(Stream(st));
}

} // namespace link
} // namespace mediaservo

#endif /* MEDIASERVO_LINK_HPP */
