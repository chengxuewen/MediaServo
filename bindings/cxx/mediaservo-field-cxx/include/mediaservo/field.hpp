/* MediaServo field C++ header-only binding (推流面) — 车端 C++ 消费。
 *
 * 薄包装 over bindings/c/mediaservo-field-c/include/mediaservo/field.h (D247)。
 * 生命周期契约（自 C ABI 头翻译成 RAII 保证）：
 *   - PushSession move-only；析构自动 close（幂等）。
 *   - 默认构造 = 已关闭（null handle）；对已关闭会话调用 API 返回
 *     Error{INVALID_ARG, "closed"}，不触碰 C ABI。
 *   - 错误通道为 Result（非异常）；误用 value()/error() 抛 tl::bad_expected_access<Error>
 *     （livekit Result 模式一致）。
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

/// 从 C ABI last_error 构造错误（C 调用返回 <0 后调用）。
inline Error make_error(int code) {
    char buf[512];
    mediaservo_field_last_error(buf, sizeof(buf));
    return Error{code, std::string(buf)};
}

/// 空字符串 → nullptr（C ABI 对必填字段拒绝 NULL，空串等效缺省）。
inline const char* c_str_or_null(const std::string& s) {
    return s.empty() ? nullptr : s.c_str();
}

} // namespace detail





/// SDK 版本 (MAJOR.MINOR.PATCH)。
inline Result<std::string> version() {
    char buf[64];
    int rc = mediaservo_field_version(buf, sizeof(buf));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(buf));
}

/// 推流配置（对应 mediaservo_push_config_t；字段默认值与 C DEFAULT 一致）。
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

/// 推流会话（move-only RAII；析构自动 close；默认构造 = 已关闭）。
class PushSession {
public:
    /// 连接信令 + 创建会话（阻塞；失败返回错误，不抛异常）。
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

    /// 默认构造 = 已关闭会话（所有调用返回 INVALID_ARG/"closed"）。
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

    /// 是否持有有效会话。
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// 发布视频轨（阻塞协商）。返回 track id。
    Result<std::string> publish_video() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_FIELD_ERR_INVALID_ARG, "closed"});
        char track[64];
        int rc = mediaservo_field_push_publish_video(h_, track, sizeof(track));
        if (rc != MEDIASERVO_OK) {
            return Result<std::string>(tl::unexpect, detail::make_error(rc));
        }
        return Result<std::string>(std::string(track));
    }

    /// 启动视频帧生成（Squares + 时间戳水印）。
    Result<void> start_video_frames() {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_FIELD_ERR_INVALID_ARG, "closed"});
        int rc = mediaservo_field_push_start_video_frames(h_);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 停止视频帧生成（幂等；无错误通道——C ABI 为 void）。
    void stop_video_frames() noexcept {
        if (h_) mediaservo_field_push_stop_video_frames(h_);
    }

    /// 关闭会话并释放 handle（幂等；重复 close 与已关闭会话均返回 OK）。
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
