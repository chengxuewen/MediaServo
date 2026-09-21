// mediaservo-client-cxx 编译运行测试：version + 错误路径 + move 语义 + 默认构造。
// 免网络（仅回环 connect 失败断言 typed error；无任何真 server 依赖）。
// 编译示例: g++ -std=c++17 -I bindings/cxx/include -I bindings/c/include
//           -I bindings/c/mediaservo-client-c/include
//           -I bindings/cxx/mediaservo-client-cxx/include
//           bindings/cxx/mediaservo-client-cxx/tests/test_client.cpp
//           -L target/debug -lmediaservo_client -o /tmp/opencode/test_client_cxx

#include <cassert>
#include <cstdio>
#include <string>
#include <utility>

#include <cstdint>

#include "mediaservo/client.hpp"

using mediaservo::client::Config;
using mediaservo::client::Control;
using mediaservo::client::Session;

static void test_version() {
    auto v = mediaservo::client::version();
    assert(v.has_value());
    assert(v.value().rfind("0.1.", 0) == 0);
}

static void test_login_invalid_arg() {
    // 空凭证 → 本地校验即拒（不触网）
    auto r = mediaservo::client::login("http://127.0.0.1:9", "", "");
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(!r.error().message.empty()); // last_error 有详情
}

static void test_connect_requires_one_credential() {
    Config cfg;
    cfg.signaling_url = "ws://127.0.0.1:9/ws";
    cfg.room = "test"; // 无 jwt 无 psk → INVALID_ARG（不触网）
    auto s = Session::connect(cfg);
    assert(!s.has_value());
    assert(s.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
}

static void test_connect_bogus_url_typed_error() {
    // 回环死端口（discard=9 无监听）→ connect 失败必须为带 code+message 的 typed error。
    // 不断言精确 code（Signal/Internal 取决于 OS 拒绝形态），断言契约面：非 OK + 详情可读。
    Config cfg;
    cfg.signaling_url = "ws://127.0.0.1:9/ws";
    cfg.room = "test";
    cfg.jwt = "bogus.jwt.token";
    auto s = Session::connect(cfg);
    assert(!s.has_value());
    assert(s.error().code < 0);
    assert(!s.error().message.empty());
}

static void test_default_constructed_closed() {
    Session s;
    assert(!static_cast<bool>(s));
    auto n = s.negotiated();
    assert(!n.has_value());
    assert(n.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(n.error().message == "closed");

    Control c;
    assert(!static_cast<bool>(c));
    auto sent = c.send("chassis", 1, "steer", "");
    assert(!sent.has_value());
    assert(sent.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
}

static void test_list_rooms_guards_no_network() {
    char buf[256];
    size_t need = 0;
    // null/零参全部本地拒（不出网——守卫先于请求构造）。
    assert(mediaservo_client_list_rooms(nullptr, "jwt", buf, sizeof(buf), &need) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(mediaservo_client_list_rooms("http://127.0.0.1:9", nullptr, buf, sizeof(buf), &need) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(mediaservo_client_list_rooms("http://127.0.0.1:9", "jwt", nullptr, sizeof(buf), &need) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(mediaservo_client_list_rooms("http://127.0.0.1:9", "jwt", buf, 0, &need) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
}

static void test_list_rooms_cxx_typed_error() {
    // 回环死端口 → typed error（code 不精确断言，与 bogus-url 案同纪律）。
    auto r = mediaservo::client::list_rooms("http://127.0.0.1:9", "bogus.jwt");
    assert(!r.has_value());
    assert(!r.error().message.empty());
}

static void test_move_semantics() {
    // 默认构造移动保持关闭态；移动赋值自动 close 旧值（幂等 null close 无副作用）
    Session a;
    Session b(std::move(a));
    assert(!static_cast<bool>(b));
    Session c;
    c = std::move(b);
    assert(!static_cast<bool>(c));
    Control x;
    Control y(std::move(x));
    assert(!static_cast<bool>(y));
}

// ── S6 批1b 新形（closed 对象守卫面确定性钉；活体转换/多路/ack 扇出 = 1c）──

static void test_state_line_values() {
    // 线值合同（header/C 宏钉）：改动即红。
    using mediaservo::client::State;
    using mediaservo::client::DcState;
    assert(static_cast<uint8_t>(State::Disconnected) == MEDIASERVO_CLIENT_STATE_DISCONNECTED);
    assert(static_cast<uint8_t>(State::Connected) == MEDIASERVO_CLIENT_STATE_CONNECTED);
    assert(static_cast<uint8_t>(State::Reconnecting) == MEDIASERVO_CLIENT_STATE_RECONNECTING);
    assert(static_cast<uint8_t>(State::Failed) == MEDIASERVO_CLIENT_STATE_FAILED);
    assert(static_cast<uint8_t>(DcState::Connecting) == MEDIASERVO_CLIENT_DC_CONNECTING);
    assert(static_cast<uint8_t>(DcState::Closed) == MEDIASERVO_CLIENT_DC_CLOSED);
}

static void test_session_new_apis_on_closed() {
    Session s;
    auto st = s.state();
    assert(!st.has_value());
    assert(st.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    auto hooked = s.on_state([](mediaservo::client::State) {});
    assert(!hooked.has_value());
    assert(hooked.error().message == "closed");
    auto pid = s.wait_video(1000);
    assert(!pid.has_value());
    auto cons = s.consume("p", [](const mediaservo_client_frame_t&) {});
    assert(!cons.has_value());
    assert(cons.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
}

static void test_consumer_closed_semantics() {
    // 默认构造 = 已关闭；id 空串、stats 错、close 幂等 OK；移动保持关闭态。
    mediaservo::client::Consumer c;
    assert(!static_cast<bool>(c));
    assert(c.id().empty());
    auto st = c.stats();
    assert(!st.has_value());
    assert(st.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    auto cl = c.close();
    assert(cl.has_value());
    mediaservo::client::Consumer d(std::move(c));
    assert(!static_cast<bool>(d));
}

static void test_control_new_apis_on_closed() {
    Control c;
    auto tok = c.on_ack([](const std::string&) {});
    assert(!tok.has_value());
    assert(c.off_ack(tok ? *tok : 0).error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(!c.ready_state().has_value());
    assert(!c.buffered_amount().has_value());
    assert(!c.producer_ids().has_value());
}

static void test_error_machine_readable_fields() {
    // Error 机读位存在且默认 0/false（link/deck/field 恒此形；client 会话形填充归 1c）。
    mediaservo::Error e{MEDIASERVO_CLIENT_ERR_DENIED, "x"};
    assert(e.wire_code == 0);
    assert(!e.retryable);
    mediaservo::Error f{MEDIASERVO_CLIENT_ERR_DENIED, "y", 4012, false};
    assert(f.wire_code == 4012);
}

static void test_strerror_c_surface() {
    char buf[128];
    assert(mediaservo_client_strerror(MEDIASERVO_CLIENT_ERR_STATE, buf, sizeof(buf)) == MEDIASERVO_OK);
    assert(std::string(buf).find("invalid state") != std::string::npos);
    // 会话级 error/last_error 的 null 守卫（无句柄不崩）。
    assert(mediaservo_client_session_error(nullptr, nullptr) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(mediaservo_client_session_last_error(nullptr, buf, sizeof(buf)) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
}

int main() {
    test_version();
    test_login_invalid_arg();
    test_list_rooms_guards_no_network();
    test_list_rooms_cxx_typed_error();
    test_connect_requires_one_credential();
    test_connect_bogus_url_typed_error();
    test_default_constructed_closed();
    test_move_semantics();
    test_state_line_values();
    test_session_new_apis_on_closed();
    test_consumer_closed_semantics();
    test_control_new_apis_on_closed();
    test_error_machine_readable_fields();
    test_strerror_c_surface();
    std::printf("test_client_cxx: all assertions PASS\n");
    return 0;
}
