/* MediaServo client C++ header-only binding (舱端/消费侧) — 登录 + 视频消费 + 控制 DC。
 *
 * 薄包装 over bindings/c/mediaservo-client-c/include/mediaservo/client.h (D247)。
 * 生命周期契约（自 C ABI 头翻译成 RAII 保证，link.hpp 同形纪律）：
 *   - Session/Control 均 move-only；析构自动 close（幂等）。
 *   - 默认构造 = 已关闭（null handle）；对已关闭对象调用 API 返回
 *     Error{INVALID_ARG, "closed"}，不触碰 C ABI。
 *   - 帧/ack 回调（trampoline）在各自内部泵线程触发；std::function 堆对象在
 *     close（join 泵线程）后统一释放，防 use-after-free。回调内禁止调用
 *     close/其他方法（C 契约）；重复注册不释放旧回调对象（泵线程可能正在
 *     执行它），随 close 一起释放 —— 注册次数通常为 1，上界有界。
 *   - 错误通道为 Result（非异常）；误用 value()/error() 抛 tl::bad_expected_access<Error>。
 */
#ifndef MEDIASERVO_CLIENT_HPP
#define MEDIASERVO_CLIENT_HPP

#pragma once

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

namespace detail {

/// 从 C ABI last_error 构造错误（C 调用返回 <0 后调用）。
inline Error make_error(int code) {
    char buf[512];
    ms_client_last_error(buf, sizeof(buf));
    return Error{code, std::string(buf)};
}

} // namespace detail

/// SDK 版本 (MAJOR.MINOR.PATCH)。
inline Result<std::string> version() {
    char buf[64];
    int rc = ms_client_version(buf, sizeof(buf));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(buf));
}

/// 账号登录换 JWT（阻塞）。token 长度上限 4KiB（JWT 常规 <2KiB，超限报 INVALID_ARG）。
inline Result<std::string> login(const std::string& http_base_url,
                                 const std::string& username,
                                 const std::string& password) {
    ms_client_login_config_t c = MS_CLIENT_LOGIN_CONFIG_DEFAULT;
    c.http_base_url = http_base_url.empty() ? nullptr : http_base_url.c_str();
    c.username = username.empty() ? nullptr : username.c_str();
    c.password = password.empty() ? nullptr : password.c_str();
    char token[4096];
    int rc = ms_client_login(&c, token, sizeof(token));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(token));
}

/// 会话配置（对应 ms_client_config_t）。jwt/psk 恰一非空（connect 校验）。
struct Config {
    std::string signaling_url; // "ws://host:9800/ws"
    std::string room;          // 房间 ID
    std::string jwt;           // login() 输出（与 psk 二选一）
    std::string psk;           // PSK 直传（与 jwt 二选一）
    /// "Client"(默认)/"Viewer"/"Remote"；空串 = "Client"。
    std::string role;
};

/// 房间会话（move-only RAII；析构自动 close；默认构造 = 已关闭）。
class Session {
public:
    /// 信令连接 + 入房（阻塞；失败返回错误，不抛异常）。
    static Result<Session> connect(const Config& cfg) {
        ms_client_config_t c = MEDIASERVO_CLIENT_CONFIG_DEFAULT;
        c.signaling_url = cfg.signaling_url.empty() ? nullptr : cfg.signaling_url.c_str();
        c.room = cfg.room.empty() ? nullptr : cfg.room.c_str();
        c.jwt = cfg.jwt.empty() ? nullptr : cfg.jwt.c_str();
        c.psk = cfg.psk.empty() ? nullptr : cfg.psk.c_str();
        c.role = cfg.role.empty() ? nullptr : cfg.role.c_str();

        ms_client_session_t* h = nullptr;
        int rc = ms_client_session_create(&c, &h);
        if (rc != MEDIASERVO_OK) {
            return Result<Session>(tl::unexpect, detail::make_error(rc));
        }
        return Result<Session>(Session(h));
    }

    /// 默认构造 = 已关闭会话。
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

    /// 是否持有有效会话。
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// 谈成的方言版本（S0；旧 server = 1）。
    Result<uint32_t> negotiated() const {
        if (!h_) return Result<uint32_t>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed"});
        uint32_t p = 0;
        int rc = ms_client_session_negotiated(h_, &p);
        if (rc != MEDIASERVO_OK) {
            return Result<uint32_t>(tl::unexpect, detail::make_error(rc));
        }
        return Result<uint32_t>(p);
    }

    /// 消费房间第一路视频（阻塞至建立；帧回调在内部泵线程触发，frame 仅回调内
    /// 有效，需保留请拷贝；重复调用 = ERR_STATE）。
    Result<void> consume_video(std::function<void(const mediaservo_client_frame_t&)> cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed"});
        auto* f = new std::function<void(const mediaservo_client_frame_t&)>(std::move(cb));
        cbs_.push_back(f); // ponytail: 旧回调对象留到 close 释放（泵线程可能正在执行，防 UAF）
        int rc = ms_client_session_consume_video(h_, &Session::frame_trampoline, f);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 开出程控制通道集（每会话一次性；labels 如 {"chassis"}）。
    Result<class Control> open_control(const std::vector<std::string>& labels);

    /// 关闭会话并释放 handle（幂等；join 视频泵后才释放回调对象）。
    Result<void> close() noexcept {
        if (!h_ && cbs_.empty()) return Result<void>();
        int rc = MEDIASERVO_OK;
        if (h_) {
            rc = ms_client_session_close(h_);
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
    explicit Session(ms_client_session_t* h) : h_(h) {}
    ms_client_session_t* release_() noexcept {
        ms_client_session_t* p = h_;
        h_ = nullptr;
        return p;
    }
    static void frame_trampoline(ms_client_session_t*, const mediaservo_client_frame_t* frame, void* user) {
        if (frame) {
            (*static_cast<std::function<void(const mediaservo_client_frame_t&)>*>(user))(*frame);
        }
    }
    ms_client_session_t* h_ = nullptr;
    std::vector<std::function<void(const mediaservo_client_frame_t&)>*> cbs_;
};

/// 出程控制通道集（move-only RAII；析构自动 close）。
class Control {
public:
    /// 默认构造 = 已关闭通道。
    Control() noexcept = default;
    ~Control() { (void)close(); }

    Control(const Control&) = delete;
    Control& operator=(const Control&) = delete;
    Control(Control&& other) noexcept : h_(other.release_()), cbs_(std::move(other.cbs_)) {}
    Control& operator=(Control&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
            cbs_ = std::move(other.cbs_);
        }
        return *this;
    }

    /// 是否持有有效通道。
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// 注册 ack 回调（首次注册启动 ack 泵；重复注册替换；ack_json 仅回调内有效）。
    Result<void> on_ack(std::function<void(const std::string&)> cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed"});
        auto* f = new std::function<void(const std::string&)>(std::move(cb));
        cbs_.push_back(f); // ponytail: 同上，close 统一释放
        int rc = ms_client_control_on_ack(h_, &Control::ack_trampoline, f);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 发送命令信封（payload_json "" = null payload；seq 调用方自增维护）。
    Result<void> send(const std::string& label, uint64_t seq, const std::string& cmd,
                      const std::string& payload_json) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed"});
        const char* payload = payload_json.empty() ? nullptr : payload_json.c_str();
        int rc = ms_client_control_send(h_, label.c_str(), seq, cmd.c_str(), payload);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// server 分配的 data producer id 的 JSON 数组（观测面）。
    Result<std::string> producer_ids() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed"});
        char buf[4096];
        int rc = ms_client_control_producer_ids(h_, buf, sizeof(buf));
        if (rc != MEDIASERVO_OK) {
            return Result<std::string>(tl::unexpect, detail::make_error(rc));
        }
        return Result<std::string>(std::string(buf));
    }

    /// 关闭控制通道并释放 handle（幂等；join ack 泵后才释放回调对象）。
    Result<void> close() noexcept {
        if (!h_ && cbs_.empty()) return Result<void>();
        int rc = MEDIASERVO_OK;
        if (h_) {
            rc = ms_client_control_close(h_);
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
    friend class Session;
    explicit Control(ms_client_control_t* h) : h_(h) {}
    ms_client_control_t* release_() noexcept {
        ms_client_control_t* p = h_;
        h_ = nullptr;
        return p;
    }
    static void ack_trampoline(ms_client_control_t*, const char* ack_json, void* user) {
        if (ack_json) {
            (*static_cast<std::function<void(const std::string&)>*>(user))(std::string(ack_json));
        }
    }
    ms_client_control_t* h_ = nullptr;
    std::vector<std::function<void(const std::string&)>*> cbs_;
};

inline Result<class Control> Session::open_control(const std::vector<std::string>& labels) {
    if (!h_) return Result<Control>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed"});
    std::vector<const char*> ptrs;
    ptrs.reserve(labels.size());
    for (const auto& l : labels) ptrs.push_back(l.c_str());
    ms_client_control_t* c = nullptr;
    int rc = ms_client_open_control(h_, ptrs.empty() ? nullptr : ptrs.data(), ptrs.size(), &c);
    if (rc != MEDIASERVO_OK) {
        return Result<Control>(tl::unexpect, detail::make_error(rc));
    }
    return Result<Control>(Control(c));
}

} // namespace client
} // namespace mediaservo

#endif /* MEDIASERVO_CLIENT_HPP */
