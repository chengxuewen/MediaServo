/* MediaServo deck C++ header-only binding (capture/record/playback plane) — C++ consumption for local monitoring/NVR.
 *
 * Thin wrapper over bindings/c/mediaservo-deck-c/include/mediaservo/deck.h (D247).
 * Lifecycle contract (translated from the C ABI header into RAII guarantees):
 *   - CameraSource/Recorder/Player are all move-only; the destructor closes automatically (idempotent).
 *   - Default construction = closed (null handle); calling an API on a closed object returns
 *     Error{INVALID_ARG, "closed"} without touching the C ABI.
 *   - Frame callbacks (trampolines) fire on the internal pump thread; the mediaservo_frame_t data_*
 *     pointers are valid only inside the callback (copy to keep them); calling any deck API from within a callback is forbidden.
 *     The heap std::function objects are released together after close (which joins the pump thread), preventing use-after-free.
 *   - Recorder::record requires the camera to be started and to outlive the recording; the close order must be
 *     recorder stop/close before camera stop/close (the C contract).
 *   - The error channel is Result (not exceptions); misusing value()/error() throws tl::bad_expected_access<Error>.
 */
#ifndef MEDIASERVO_DECK_HPP
#define MEDIASERVO_DECK_HPP

#pragma once

#include <cstdint>
#include <functional>
#include <stdexcept>
#include <string>
#include <utility>
#include <variant>
#include <vector>

#include <mediaservo/common.h>
#include <mediaservo/deck.h>
#include <mediaservo/detail/result.hpp>

namespace mediaservo {
namespace deck {
using mediaservo::Error;
using mediaservo::Result;


namespace detail {

/// Build an error from the C ABI last_error (call after a C API returns <0).
inline Error make_error(int code) {
    char buf[512];
    mediaservo_deck_last_error(buf, sizeof(buf));
    return Error{code, std::string(buf)};
}

/// Frame-callback trampoline carrier (heap-allocated, lifetime = the registration period; freed after close joins the pump).
using FrameCb = std::function<void(const mediaservo_frame_t&)>;
inline void frame_trampoline(const mediaservo_frame_t* frame, void* user) {
    (*static_cast<FrameCb*>(user))(*frame);
}

} // namespace detail





/// SDK version (MAJOR.MINOR.PATCH).
inline Result<std::string> version() {
    char buf[64];
    int rc = mediaservo_deck_version(buf, sizeof(buf));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(buf));
}

/// Device kind (matches the kind parameter of mediaservo_deck_devices_enumerate).
enum class DeviceKind {
    Camera = 0,
    Audio = 1,
    Screen = 2,
};

/// Capture options (mirrors mediaservo_deck_capture_options_t; all-zero fields = the C default).
struct CaptureOptions {
    uint32_t width = 1280;
    uint32_t height = 720;
    uint32_t framerate = 30;
};

/// Enumerate device ids (two-call wrapper; multiple devices separated by '\n'). Failure returns an empty list.
inline std::vector<std::string> enumerate_devices(DeviceKind kind) {
    size_t len = 0;
    int rc = mediaservo_deck_devices_enumerate(static_cast<int>(kind), nullptr, 0, &len);
    if (rc < MEDIASERVO_OK || len == 0) {
        return {}; // ponytail: no Result channel (the spec fixes the signature); failure = empty list
    }
    std::string buf(len, '\0');
    rc = mediaservo_deck_devices_enumerate(static_cast<int>(kind), &buf[0], buf.size(), &len);
    // C++11: string::data() is const, hence the &buf[0] idiom
    if (rc < MEDIASERVO_OK) {
        return {};
    }
    std::vector<std::string> out;
    size_t start = 0;
    for (size_t i = 0; i <= buf.size(); ++i) {
        if (i == buf.size() || buf[i] == '\n') {
            if (i > start) out.emplace_back(buf, start, i - start);
            start = i + 1;
        }
    }
    return out;
}

class Recorder;

/// Camera capture (move-only RAII; the destructor closes automatically; default construction = closed).
class CameraSource {
public:
    /// Open the camera (local initialization only; dev_id must exist in the enumeration result).
    static Result<CameraSource> open(const std::string& dev_id, const CaptureOptions& opts = CaptureOptions{}) {
        mediaservo_deck_capture_options_t c = MEDIASERVO_DECK_CAPTURE_OPTIONS_DEFAULT;
        c.width = opts.width;
        c.height = opts.height;
        c.framerate = opts.framerate;

        mediaservo_deck_camera_t* h = nullptr;
        int rc = mediaservo_deck_camera_open(dev_id.empty() ? nullptr : dev_id.c_str(), &c, &h);
        if (rc != MEDIASERVO_OK) {
            return Result<CameraSource>(tl::unexpect, detail::make_error(rc));
        }
        return Result<CameraSource>(CameraSource(h));
    }

    /// Default construction = closed camera.
    CameraSource() noexcept = default;
    ~CameraSource() { (void)close(); }

    CameraSource(const CameraSource&) = delete;
    CameraSource& operator=(const CameraSource&) = delete;
    CameraSource(CameraSource&& other) noexcept : h_(other.release_()), cbs_(std::move(other.cbs_)) {}
    CameraSource& operator=(CameraSource&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
            cbs_ = std::move(other.cbs_);
        }
        return *this;
    }

    /// Whether a valid camera is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// Start producing frames (uses the opts from open; only allowed once).
    Result<void> start() {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "closed"});
        int rc = mediaservo_deck_camera_start(h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Register the frame callback (fires per frame on the pump thread; re-registration replaces; the frame pointer is valid only inside the callback;
    /// calling any deck API from within the callback is forbidden).
    Result<void> on_frame(detail::FrameCb cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "closed"});
        auto* f = new detail::FrameCb(std::move(cb));
        cbs_.push_back(f); // ponytail: old callbacks are kept until close frees them (the pump thread may be executing them, preventing UAF); usually 1 registration
        int rc = mediaservo_deck_camera_frames_cb(h_, &detail::frame_trampoline, f);
        if (rc != MEDIASERVO_OK) {
            cbs_.pop_back();
            delete f;
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Stop producing frames (idempotent).
    Result<void> stop() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_deck_camera_stop(h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Close the camera and release the handle (idempotent; callback objects are freed only after joining the frame pump).
    Result<void> close() noexcept {
        if (!h_ && cbs_.empty()) return Result<void>();
        int rc = MEDIASERVO_OK;
        if (h_) {
            rc = mediaservo_deck_camera_close(h_);
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
    friend class Recorder;
    explicit CameraSource(mediaservo_deck_camera_t* h) : h_(h) {}
    mediaservo_deck_camera_t* release_() noexcept {
        mediaservo_deck_camera_t* p = h_;
        h_ = nullptr;
        return p;
    }
    mediaservo_deck_camera_t* h_ = nullptr;
    std::vector<detail::FrameCb*> cbs_;
};

/// Recorder (move-only RAII; the destructor closes automatically; default construction = closed).
class Recorder {
public:
    /// Create the recorder (default h264/mp4; the parent directory must already exist).
    static Result<Recorder> open(const std::string& path) {
        mediaservo_deck_recorder_t* h = nullptr;
        int rc = mediaservo_deck_recorder_new(path.empty() ? nullptr : path.c_str(), &h);
        if (rc != MEDIASERVO_OK) {
            return Result<Recorder>(tl::unexpect, detail::make_error(rc));
        }
        return Result<Recorder>(Recorder(h));
    }

    /// Default construction = closed recorder.
    Recorder() noexcept = default;
    ~Recorder() { (void)close(); }

    Recorder(const Recorder&) = delete;
    Recorder& operator=(const Recorder&) = delete;
    Recorder(Recorder&& other) noexcept : h_(other.release_()) {}
    Recorder& operator=(Recorder&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
        }
        return *this;
    }

    /// Whether a valid recorder is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// Bridge recording: camera frame pump → recorder. The camera must be started and outlive the recording
    /// (close order: recorder stop/close before camera stop/close, the C contract).
    Result<void> record(CameraSource& camera) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "closed"});
        if (!camera.h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "camera closed"});
        int rc = mediaservo_deck_recorder_record(h_, camera.h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Request stop recording (idempotent; flush + trailer finalize on close).
    Result<void> stop() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_deck_recorder_stop(h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Close the recorder and release the handle (idempotent; joins the recording task to complete the flush).
    Result<void> close() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_deck_recorder_close(h_);
        h_ = nullptr;
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

private:
    explicit Recorder(mediaservo_deck_recorder_t* h) : h_(h) {}
    mediaservo_deck_recorder_t* release_() noexcept {
        mediaservo_deck_recorder_t* p = h_;
        h_ = nullptr;
        return p;
    }
    mediaservo_deck_recorder_t* h_ = nullptr;
};

/// Player (move-only RAII; the destructor closes automatically; default construction = closed).
class Player {
public:
    /// Open a media file (demux + decoder ready).
    static Result<Player> open(const std::string& path) {
        mediaservo_deck_player_t* h = nullptr;
        int rc = mediaservo_deck_player_open(path.empty() ? nullptr : path.c_str(), &h);
        if (rc != MEDIASERVO_OK) {
            return Result<Player>(tl::unexpect, detail::make_error(rc));
        }
        return Result<Player>(Player(h));
    }

    /// Default construction = closed player.
    Player() noexcept = default;
    ~Player() { (void)close(); }

    Player(const Player&) = delete;
    Player& operator=(const Player&) = delete;
    Player(Player&& other) noexcept : h_(other.release_()), cbs_(std::move(other.cbs_)) {}
    Player& operator=(Player&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
            cbs_ = std::move(other.cbs_);
        }
        return *this;
    }

    /// Whether a valid player is held.
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// Per-frame decode callback pump (runs until EOF ends naturally; only allowed once; close is a blocking join).
    Result<void> on_frame(detail::FrameCb cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "closed"});
        auto* f = new detail::FrameCb(std::move(cb));
        cbs_.push_back(f);
        int rc = mediaservo_deck_player_frames_cb(h_, &detail::frame_trampoline, f);
        if (rc != MEDIASERVO_OK) {
            cbs_.pop_back();
            delete f;
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// Close the player and release the handle (idempotent; callback objects are freed after joining the decode pump until done).
    Result<void> close() noexcept {
        if (!h_ && cbs_.empty()) return Result<void>();
        int rc = MEDIASERVO_OK;
        if (h_) {
            rc = mediaservo_deck_player_close(h_);
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
    explicit Player(mediaservo_deck_player_t* h) : h_(h) {}
    mediaservo_deck_player_t* release_() noexcept {
        mediaservo_deck_player_t* p = h_;
        h_ = nullptr;
        return p;
    }
    mediaservo_deck_player_t* h_ = nullptr;
    std::vector<detail::FrameCb*> cbs_;
};

} // namespace deck
} // namespace mediaservo

#endif /* MEDIASERVO_DECK_HPP */
