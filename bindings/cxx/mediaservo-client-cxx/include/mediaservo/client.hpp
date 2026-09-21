/* MediaServo client C++ header-only binding (舱端/消费侧) — 登录 + 视频消费 + 控制 DC。
 *
 * 薄包装 over bindings/c/mediaservo-client-c/include/mediaservo/client.h (D247)。
 * S6 批1b 镜像：Session.state()/on_state()、wait_video()、consume()→Consumer（多路）；
 * Control.on_ack 返回 token + off_ack()/ready_state()/buffered_amount()；Error 携
 * wire_code/retryable（会话句柄 error_t 读回）。旧 consume_video/video_stats 桥保留
 * （内部 = C ⊘ 形，行为一周期不变）。
 *
 * 生命周期契约（自 C ABI 头翻译成 RAII 保证，link.hpp 同形纪律）：
 *   - Session/Control/Consumer 均 move-only；析构自动 close（幂等）。
 *   - 默认构造 = 已关闭（null handle）；对已关闭对象调用 API 返回
 *     Error{INVALID_ARG, "closed"}，不触碰 C ABI。
 *   - 帧/ack/状态回调（trampoline）在各自内部泵线程触发。注册堆对象的释放走
 *     C 侧 user_free 契约（on_state→session_close；consume→consumer_close；
 *     on_ack→off_ack 泵回收轮或 control_close）——C++ 面零裸 delete 泄漏；
 *     注册失败（C 未接管）时本头 delete 兜底。仅 consume_video（⊘ C 形无
 *     user_free）的回调堆对象留在 cbs_ 随 close 释放（泵已 join，无 UAF）。
 *   - 回调内禁止调用 close/其他方法（C 契约）。
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

/// 会话连接态（线值 0..3 = MEDIASERVO_CLIENT_STATE_*）。
enum class State : uint8_t {
    Disconnected = 0,
    Connected = 1,
    Reconnecting = 2,
    Failed = 3,
};

/// DC 就绪态（线值 0..3 = MEDIASERVO_CLIENT_DC_*）。
enum class DcState : uint8_t {
    Connecting = 0,
    Open = 1,
    Closing = 2,
    Closed = 3,
};

namespace detail {

/// 从 C ABI 全局 last_error 构造错误（无会话句柄形：自由函数/守卫路径）。
inline Error make_error(int code) {
    char buf[512];
    mediaservo_client_last_error(buf, sizeof(buf));
    return Error{code, std::string(buf), 0, false};
}

/// 会话句柄形：机读位读 session_error，文本读 session_last_error（K3 归属精确）。
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

/// needed 溢出合同 + 自动扩一次重试（list_rooms/video_stats/producer_ids/wait_video/
/// consumer id+stats 共用内核；>64KiB 拒——超限报 INVALID_ARG 带必需长度语义）。
/// h 非空 = 会话错误通道（wire/retryable 精确）；空 = 全局 last_error。
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

/// SDK 版本 (MAJOR.MINOR.PATCH)。
inline Result<std::string> version() {
    char buf[64];
    int rc = mediaservo_client_version(buf, sizeof(buf));
    if (rc != MEDIASERVO_OK) {
        return Result<std::string>(tl::unexpect, detail::make_error(rc));
    }
    return Result<std::string>(std::string(buf));
}

/// 账号登录换 JWT（阻塞；批1b 扁平 C 形镜像）。溢出经 needed 反馈自动扩一次。
inline Result<std::string> login(const std::string& http_base_url,
                                 const std::string& username,
                                 const std::string& password) {
    return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
        return mediaservo_client_login(http_base_url.c_str(), username.c_str(),
                                       password.c_str(), b, c, n);
    });
}

/// 房间发现（阻塞，会话前自由函数）。返回 JSON 数组
/// `[{"room_id":..,"kind":..}]`（与 producer_ids 同形：header-only 无 JSON 依赖，
/// 调用方解析）。溢出经 needed 反馈自动扩一次重试。
inline Result<std::string> list_rooms(const std::string& http_base, const std::string& jwt) {
    return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
        return mediaservo_client_list_rooms(http_base.c_str(), jwt.c_str(), b, c, n);
    });
}

/// 会话配置（对应 mediaservo_client_config_t）。jwt/psk 恰一非空（connect 校验）。
struct Config {
    std::string signaling_url; // "ws://host:9800/ws"
    std::string room;          // 房间 ID
    std::string jwt;           // login() 输出（与 psk 二选一）
    std::string hmac_key_file; // 急停密钥文件（0600；空 = 不签名，语义见 client.h）
    std::string psk;           // PSK 直传（与 jwt 二选一）
    /// "Client"(默认)/"Viewer"/"Remote"；空串 = "Client"。
    std::string role;
};

class Consumer;

/// 房间会话（move-only RAII；析构自动 close；默认构造 = 已关闭）。
class Session {
public:
    /// 信令连接 + 入房（阻塞；失败返回错误，不抛异常）。
    static Result<Session> connect(const Config& cfg) {
        mediaservo_client_config_t c = MEDIASERVO_CLIENT_CONFIG_DEFAULT;
        c.signaling_url = cfg.signaling_url.empty() ? nullptr : cfg.signaling_url.c_str();
        c.room = cfg.room.empty() ? nullptr : cfg.room.c_str();
        c.jwt = cfg.jwt.empty() ? nullptr : cfg.jwt.c_str();
        c.psk = cfg.psk.empty() ? nullptr : cfg.psk.c_str();
        c.role = cfg.role.empty() ? nullptr : cfg.role.c_str();
        c.hmac_key_file = cfg.hmac_key_file.empty() ? nullptr : cfg.hmac_key_file.c_str();

        mediaservo_client_session_t* h = nullptr;
        int rc = mediaservo_client_session_create(&c, &h);
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
        if (!h_) return Result<uint32_t>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        uint32_t p = 0;
        int rc = mediaservo_client_session_negotiated(h_, &p);
        if (rc != MEDIASERVO_OK) {
            return Result<uint32_t>(tl::unexpect, detail::make_error(rc, h_));
        }
        return Result<uint32_t>(p);
    }

    /// 会话连接态快照（重连环观测面，K1）。
    Result<State> state() const {
        if (!h_) return Result<State>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        uint8_t st = 0;
        int rc = mediaservo_client_session_state(h_, &st);
        if (rc != MEDIASERVO_OK) {
            return Result<State>(tl::unexpect, detail::make_error(rc, h_));
        }
        return Result<State>(static_cast<State>(st));
    }

    /// 注册连接态变化回调（累积形：多次注册 = 每次变化逐个回调；不可注销，
    /// 随 close 由 C 侧统一释放。只报变化——初始态用 state() 自取）。
    Result<void> on_state(std::function<void(State)> cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        auto* f = new std::function<void(State)>(std::move(cb));
        int rc = mediaservo_client_session_on_state(h_, &Session::state_trampoline, f,
                                                    &Session::state_user_free);
        if (rc != MEDIASERVO_OK) {
            delete f; // C 未接管，本侧兜底（禁泄漏）
            return Result<void>(tl::unexpect, detail::make_error(rc, h_));
        }
        return Result<void>();
    }

    /// 纯等待房间内视频 producer（无副作用；多路编排定向入口）。超时/断链 = 错误。
    Result<std::string> wait_video(uint64_t timeout_ms) {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(h_, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_session_wait_video(h_, timeout_ms, b, c, n);
        });
    }

    /// 订阅一路视频 producer（多路新形，K4：每路独立泵，互不连坐）。返回的
    /// Consumer 为 move-only RAII（析构自动 close；Session.close 不隐式关）。
    Result<Consumer> consume(const std::string& producer_id,
                             std::function<void(const mediaservo_client_frame_t&)> cb);

    /// ⊘ 消费房间第一路视频（首路桥，保留一周期；新代码用 wait_video+consume）。
    /// 重复调用 = ERR_STATE。帧回调堆对象留 cbs_ 随 close 释放（⊘ C 形无 user_free）。
    Result<void> consume_video(std::function<void(const mediaservo_client_frame_t&)> cb) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        auto* f = new std::function<void(const mediaservo_client_frame_t&)>(std::move(cb));
        cbs_.push_back(f); // ponytail: ⊘ 形无 C 侧回收，旧回调对象留到 close 释放（泵线程可能正在执行，防 UAF）
        int rc = mediaservo_client_session_consume_video(h_, &Session::frame_trampoline, f);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc, h_));
        }
        return Result<void>();
    }

    /// ⊘ 视频统计汇总 JSON（会话级 union，保留一周期；多路精确读数用
    /// Consumer::stats()；键见 client.h；imgui_shell core::RateEstimator 为参考消费形）。
    Result<std::string> video_stats() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(h_, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_session_video_stats(h_, b, c, n);
        });
    }

    /// 开出程控制通道集（每会话一次性；labels 如 {"chassis"}）。
    Result<class Control> open_control(const std::vector<std::string>& labels);

    /// 急停双路投递（W4b；签名态 = Config.hmac_key_file）。OK = 投递成功，
    /// 车端执行裁决看 ctl 的 seq 回执 ack。payload_json "" = null。
    Result<void> emergency_stop(const class Control& ctl, const std::string& label,
                                uint64_t seq, const std::string& payload_json);

    /// 关闭会话并释放 handle（幂等；join 泵 + C 侧逐条 user_free 状态回调后，
    /// 本侧再释放 ⊘ consume_video 的 cbs_ 堆对象）。
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

/// 单路视频消费者（move-only RAII；析构自动 close；默认构造 = 已关闭）。
class Consumer {
public:
    Consumer() noexcept = default;
    ~Consumer() { (void)close(); }

    Consumer(const Consumer&) = delete;
    Consumer& operator=(const Consumer&) = delete;
    Consumer(Consumer&& other) noexcept : h_(other.release_()) {}
    Consumer& operator=(Consumer&& other) noexcept {
        if (this != &other) {
            (void)close();
            h_ = other.release_();
        }
        return *this;
    }

    /// 是否持有有效消费者。
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// 本路 producer id（建立期常量；已关闭/读取失败 = 空串）。
    std::string id() const {
        if (!h_) return std::string();
        auto r = detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_consumer_id(h_, b, c, n);
        });
        return r ? r.value() : std::string();
    }

    /// 本路视频统计 JSON（单路读数，K4；键表 = Session::video_stats 同形）。
    Result<std::string> stats() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_consumer_stats(h_, b, c, n);
        });
    }

    /// 关闭本路并释放 handle（幂等；C 侧 join 泵 + user_free 帧回调堆对象）。
    Result<void> close() noexcept {
        if (!h_) return Result<void>();
        int rc = mediaservo_client_consumer_close(h_);
        h_ = nullptr;
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
};

/// 出程控制通道集（move-only RAII；析构自动 close）。
class Control {
public:
    /// 默认构造 = 已关闭通道。
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

    /// 是否持有有效通道。
    explicit operator bool() const noexcept { return h_ != nullptr; }

    /// 注册 ack 回调（累积形，批1b：多次注册 = 每条 ack 逐个回调；首次启动泵）。
    /// 返回注销凭据 token（Control::off_ack）；回调堆对象由 C 侧在 off_ack 泵
    /// 回收轮或 close 时释放（注册失败本侧 delete 兜底）。
    Result<uint64_t> on_ack(std::function<void(const std::string&)> cb) {
        if (!h_) return Result<uint64_t>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        auto* f = new std::function<void(const std::string&)>(std::move(cb));
        uint64_t token = 0;
        int rc = mediaservo_client_control_on_ack(h_, &Control::ack_trampoline, f,
                                                  &Control::ack_user_free, &token);
        if (rc != MEDIASERVO_OK) {
            delete f; // C 未接管，本侧兜底（禁泄漏）
            return Result<uint64_t>(tl::unexpect, detail::make_error(rc));
        }
        return Result<uint64_t>(token);
    }

    /// 按 token 注销 ack 回调（泵下轮回收并释放堆对象；在途轮次可能仍触发一次）。
    Result<void> off_ack(uint64_t token) {
        if (!h_) return Result<void>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        int rc = mediaservo_client_control_off_ack(h_, token);
        if (rc != MEDIASERVO_OK) {
            return Result<void>(tl::unexpect, detail::make_error(rc));
        }
        return Result<void>();
    }

    /// 发送命令信封（payload_json "" = null payload；seq 调用方自增维护）。
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

    /// DC 就绪态（发送/急停前门禁用，K5）。
    Result<DcState> ready_state() const {
        if (!h_) return Result<DcState>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        uint8_t st = 0;
        int rc = mediaservo_client_control_ready_state(h_, &st);
        if (rc != MEDIASERVO_OK) {
            return Result<DcState>(tl::unexpect, detail::make_error(rc));
        }
        return Result<DcState>(static_cast<DcState>(st));
    }

    /// SCTP 背压水位（字节数；0 = 可安全追加，K5）。
    Result<uint64_t> buffered_amount() {
        if (!h_) return Result<uint64_t>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        uint64_t bytes = 0;
        int rc = mediaservo_client_control_buffered_amount(h_, &bytes);
        if (rc != MEDIASERVO_OK) {
            return Result<uint64_t>(tl::unexpect, detail::make_error(rc));
        }
        return Result<uint64_t>(bytes);
    }

    /// server 分配的 data producer id 的 JSON 数组（观测面）。溢出经 needed
    /// 反馈自动扩一次重试（批1b 升形）。
    Result<std::string> producer_ids() {
        if (!h_) return Result<std::string>(tl::unexpect, Error{MEDIASERVO_CLIENT_ERR_INVALID_ARG, "closed", 0, false});
        return detail::needed_read(nullptr, [&](char* b, size_t c, size_t* n) {
            return mediaservo_client_control_producer_ids(h_, b, c, n);
        });
    }

    /// 关闭控制通道并释放 handle（幂等；join ack 泵后 C 侧全量回收注册表——
    /// 堆 std::function 经 user_free 释放，C++ 面无残留）。
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
        delete f; // C 未接管，本侧兜底（禁泄漏）
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
