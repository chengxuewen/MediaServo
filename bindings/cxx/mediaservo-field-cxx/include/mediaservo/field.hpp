/* MediaServo field C++ header-only binding (push plane) — vehicle-side C++ consumption.
 *
 * Thin wrapper over bindings/c/mediaservo-field-c/include/mediaservo/field.h (D247).
 * Lifecycle contract (translated from the C ABI header into RAII guarantees):
 *   - PushSession is move-only; the destructor closes automatically (idempotent).
 *   - Default construction = closed (null handle); calling an API on a closed session returns
 *     Error{INVALID_ARG, "closed"} without touching the C ABI.
 *   - The error channel is Result (not exceptions); misusing value()/error() throws tl::bad_expected_access<Error>
 *     (the same pattern as livekit's Result).
 */
#ifndef MEDIASERVO_FIELD_HPP
#define MEDIASERVO_FIELD_HPP

#pragma once

#include <cstdint>
#include <stdexcept>
#include <string>
#include <utility>
#include <variant>

#include <mediaservo/common.h>
#include <mediaservo/field.h>
#include <mediaservo/detail/result.hpp>

namespace mediaservo {
namespace field {
using mediaservo::Error;
using mediaservo::Result;


namespace detail {

/// Build an error from the C ABI last_error (call after a C API returns <0).
inline Error make_error(int code) {
    char buf[512];
    mediaservo_field_last_error(buf, sizeof(buf));
    return Error{code, std::string(buf)};
}

/// Empty string → nullptr (the C ABI rejects NULL for required fields; an empty string is equivalent to the default).
inline const char* c_str_or_null(const std::string& s) {
    return s.empty() ? nullptr : s.c_str();
}

} // namespace detail





/// SDK version (MAJOR.MINOR.PATCH).
inline Result<std::string> version() {
    char buf[64];
    int rc = mediaservo_field_version(buf, sizeof(buf));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(buf));
}

/// Push configuration (mirrors mediaservo_push_config_t; field defaults match the C DEFAULT).
struct PushConfig {
    std::string url;
    std::string psk;
    std::string room;
    uint32_t width = 1280;
    uint32_t height = 720;
    uint32_t framerate = 30;
    uint32_t bitrate_kbps = 2000;
    uint64_t keyframe_interval = 2;
};

/// Push session (move-only RAII; the destructor closes automatically; default construction = closed).
class PushSession {
public:
    /// Connect signaling and create the session (blocking; returns an error on failure, throws nothing).
    static Result<PushSession> connect(const PushConfig& cfg) {
        mediaservo_push_config_t c = MEDIASERVO_PUSH_CONFIG_DEFAULT;
        c.url = detail::c_str_or_null(cfg.url);
        c.psk = detail::c_str_or_null(cfg.psk);
        c.room = detail::c_str_or_null(cfg.room);
        c.width = cfg.width;
        c.height = cfg.height;
        c.framerate = cfg.framerate;
        c.bitrate_kbps = cfg.bitrate_kbps;
        c.keyframe_interval = cfg.keyframe_interval;

        mediaservo_field_push_t* h = nullptr;
        int rc = mediaservo_field_push_connect(&c, &h);
        if (rc != MEDIASERVO_OK) {
            return Result<PushSession>(tl::unexpect, detail::make_error(rc));
        }
        return Result<PushSession>(PushSession(h));
    }

    /// Default construction = closed session (all calls return INVALID_ARG/"closed").
    PushSession() noexcept = default;
    ~PushSession() { (void)close(); }

    PushSession(const PushSession&) = delete;
    PushSession& operator=(const PushSession&) = delete;
    PushSession(PushSession&& other) noexcept : h_(other.release_()) {}
    PushSession& operator=(PushSession&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
        }
        return *this;
    }

    /// Whether a valid session is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// Publish the video track (blocking negotiation). Returns the track id.
    Result<std::string> publish_video() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_FIELD_ERR_INVALID_ARG, "closed"});
        char track[64];
        int rc = mediaservo_field_push_publish_video(h_, track, sizeof(track));
        if (rc != MEDIASERVO_OK) {
            return Result<std::string>(tl::unexpect, detail::make_error(rc));
        }
        return Result<std::string>(std::string(track));
    }

    /// Start video frame generation (squares + timestamp watermark).
    Result<void> start_video_frames() {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_FIELD_ERR_INVALID_ARG, "closed"});
        int rc = mediaservo_field_push_start_video_frames(h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Stop video frame generation (idempotent; no error channel — the C ABI is void).
    void stop_video_frames() noexcept {
        if (h_) mediaservo_field_push_stop_video_frames(h_);
    }

    /// Close the session and release the handle (idempotent; both a repeated close and an already-closed session return OK).
    Result<void> close() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_field_push_close(h_);
        h_ = nullptr;
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

private:
    explicit PushSession(mediaservo_field_push_t* h) : h_(h) {}
    mediaservo_field_push_t* release_() noexcept {
        mediaservo_field_push_t* p = h_;
        h_ = nullptr;
        return p;
    }
    mediaservo_field_push_t* h_ = nullptr;
};

} // namespace field
} // namespace mediaservo

#endif /* MEDIASERVO_FIELD_HPP */
