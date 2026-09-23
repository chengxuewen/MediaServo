/* MediaServo deck C++ header-only binding (采集/录制/回放面) — 本地监控/NVR C++ 消费。
 *
 * 薄包装 over bindings/c/mediaservo-deck-c/include/mediaservo/deck.h (D247)。
 * 生命周期契约（自 C ABI 头翻译成 RAII 保证）：
 *   - CameraSource/Recorder/Player 均 move-only；析构自动 close（幂等）。
 *   - 默认构造 = 已关闭（null handle）；对已关闭对象调用 API 返回
 *     Error{INVALID_ARG, "closed"}，不触碰 C ABI。
 *   - 帧回调（trampoline）在内部泵线程触发；mediaservo_frame_t 的 data_*
 *     指针仅在回调内有效（需保留请拷贝）；回调内禁止调用任何 deck API。
 *     std::function 堆对象在 close（join 泵线程）后统一释放，防 use-after-free。
 *   - Recorder::record 要求 camera 已 start 且活到录制结束；关闭顺序必须
 *     recorder stop/close 先于 camera stop/close（C 契约）。
 *   - 错误通道为 Result（非异常）；误用 value()/error() 抛 tl::bad_expected_access<Error>。
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

/// 从 C ABI last_error 构造错误（C 调用返回 <0 后调用）。
inline Error make_error(int code) {
    char buf[512];
    mediaservo_deck_last_error(buf, sizeof(buf));
    return Error{code, std::string(buf)};
}

/// 帧回调 trampoline 载体（堆分配，生命周期=注册期；close join 泵后释放）。
using FrameCb = std::function<void(const mediaservo_frame_t&)>;
inline void frame_trampoline(const mediaservo_frame_t* frame, void* user) {
    (*static_cast<FrameCb*>(user))(*frame);
}

} // namespace detail





/// SDK 版本 (MAJOR.MINOR.PATCH)。
inline Result<std::string> version() {
    char buf[64];
    int rc = mediaservo_deck_version(buf, sizeof(buf));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(buf));
}

/// 设备种类（对应 mediaservo_deck_devices_enumerate 的 kind 参数）。
enum class DeviceKind {
    Camera = 0,
    Audio = 1,
    Screen = 2,
};

/// 采集选项（对应 mediaservo_deck_capture_options_t；全 0 字段 = C 默认）。
struct CaptureOptions {
    uint32_t width = 1280;
    uint32_t height = 720;
    uint32_t framerate = 30;
};

/// 枚举设备 id（双调用封装；多设备 '\n' 分隔）。失败返回空列表。
inline std::vector<std::string> enumerate_devices(DeviceKind kind) {
    size_t len = 0;
    int rc = mediaservo_deck_devices_enumerate(static_cast<int>(kind), nullptr, 0, &len);
    if (rc < MEDIASERVO_OK || len == 0) {
        return {}; // ponytail: 无 Result 通道（spec 固定签名）；失败=空列表
    }
    std::string buf(len, '\0');
    rc = mediaservo_deck_devices_enumerate(static_cast<int>(kind), &buf[0], buf.size(), &len);
    // C++11: string::data() 为 const，&buf[0] 惯用法
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

/// 相机采集（move-only RAII；析构自动 close；默认构造 = 已关闭）。
class CameraSource {
public:
    /// 打开相机（仅本地初始化；dev_id 必须存在于枚举结果）。
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

    /// 默认构造 = 已关闭相机。
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

    /// 是否持有有效相机。
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// 开始产帧（用 open 时的 opts；只允许一次）。
    Result<void> start() {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "closed"});
        int rc = mediaservo_deck_camera_start(h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 注册帧回调（泵线程逐帧触发；重复注册替换；frame 指针仅回调内有效；
    /// 回调内禁止调用任何 deck API）。
    Result<void> on_frame(detail::FrameCb cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "closed"});
        auto* f = new detail::FrameCb(std::move(cb));
        cbs_.push_back(f); // ponytail: 旧回调留到 close 释放（泵线程可能正在执行，防 UAF）；注册次数通常 1
        int rc = mediaservo_deck_camera_frames_cb(h_, &detail::frame_trampoline, f);
        if (rc != MEDIASERVO_OK) {
            cbs_.pop_back();
            delete f;
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 停止产帧（幂等）。
    Result<void> stop() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_deck_camera_stop(h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 关闭相机并释放 handle（幂等；join 帧泵后才释放回调对象）。
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

/// 录制器（move-only RAII；析构自动 close；默认构造 = 已关闭）。
class Recorder {
public:
    /// 创建录制器（默认 h264/mp4；父目录必须已存在）。
    static Result<Recorder> open(const std::string& path) {
        mediaservo_deck_recorder_t* h = nullptr;
        int rc = mediaservo_deck_recorder_new(path.empty() ? nullptr : path.c_str(), &h);
        if (rc != MEDIASERVO_OK) {
            return Result<Recorder>(tl::unexpect, detail::make_error(rc));
        }
        return Result<Recorder>(Recorder(h));
    }

    /// 默认构造 = 已关闭录制器。
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

    /// 是否持有有效录制器。
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// 桥接录制: camera 帧泵 → recorder。camera 必须已 start 且活到录制结束
    /// （关闭顺序: recorder stop/close 先于 camera stop/close，C 契约）。
    Result<void> record(CameraSource& camera) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "closed"});
        if (!camera.h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_DECK_ERR_INVALID_ARG, "camera closed"});
        int rc = mediaservo_deck_recorder_record(h_, camera.h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 请求停止录制（幂等；flush + trailer 收尾在 close 时完成）。
    Result<void> stop() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_deck_recorder_stop(h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 关闭录制器并释放 handle（幂等；join 录制任务完成 flush）。
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

/// 回放器（move-only RAII；析构自动 close；默认构造 = 已关闭）。
class Player {
public:
    /// 打开媒体文件（demux + 解码器就绪）。
    static Result<Player> open(const std::string& path) {
        mediaservo_deck_player_t* h = nullptr;
        int rc = mediaservo_deck_player_open(path.empty() ? nullptr : path.c_str(), &h);
        if (rc != MEDIASERVO_OK) {
            return Result<Player>(tl::unexpect, detail::make_error(rc));
        }
        return Result<Player>(Player(h));
    }

    /// 默认构造 = 已关闭回放器。
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

    /// 是否持有有效回放器。
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// 逐帧解码回调泵（运行至 EOF 自然结束；只允许一次；close 为阻塞 join）。
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

    /// 关闭回放器并释放 handle（幂等；join 解码泵至完成后释放回调对象）。
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
