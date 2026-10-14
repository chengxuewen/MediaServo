// mediaservo-client-cxx compile-and-run tests: version + error paths + move semantics + default construction.
// Network-free (only loopback connect-failure typed-error assertions; no real-server dependency).
// Build example: g++ -std=c++17 -I bindings/cxx/include -I bindings/c/include
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

static void test_exchange_invalid_arg() {
    // accountless-client-auth T2: an empty key/secret is rejected locally (no network).
    auto r = mediaservo::client::exchange("http://127.0.0.1:9", "", "s");
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(!r.error().message.empty());
}

static void test_login_invalid_arg() {
    // Empty credentials → rejected by local validation (no network)
    auto r = mediaservo::client::login("http://127.0.0.1:9", "", "");
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(!r.error().message.empty()); // last_error carries the details
}

static void test_connect_requires_one_credential() {
    Config cfg;
    cfg.signaling_url = "ws://127.0.0.1:9/ws";
    cfg.room = "test"; // no jwt, no psk → INVALID_ARG (no network)
    auto s = Session::connect(cfg);
    assert(!s.has_value());
    assert(s.error().code == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
}

static void test_connect_bogus_url_typed_error() {
    // Loopback dead port (discard=9, nothing listening) → a connect failure must be a typed error carrying code+message.
    // No exact-code assertion (Signal/Internal depends on the OS rejection form); the contract asserted is: not OK + details readable.
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
    // null/zero params are all rejected locally (no network — guards precede request construction).
    assert(mediaservo_client_list_rooms(nullptr, "jwt", buf, sizeof(buf), &need) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(mediaservo_client_list_rooms("http://127.0.0.1:9", nullptr, buf, sizeof(buf), &need) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(mediaservo_client_list_rooms("http://127.0.0.1:9", "jwt", nullptr, sizeof(buf), &need) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(mediaservo_client_list_rooms("http://127.0.0.1:9", "jwt", buf, 0, &need) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
}

static void test_list_rooms_cxx_typed_error() {
    // Loopback dead port → typed error (no exact-code assertion; same discipline as the bogus-url case).
    auto r = mediaservo::client::list_rooms("http://127.0.0.1:9", "bogus.jwt");
    assert(!r.has_value());
    assert(!r.error().message.empty());
}

static void test_move_semantics() {
    // A default-constructed object stays closed through moves; move-assignment auto-closes the old value (idempotent null close, no side effects)
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

// -- S6 batch 1b new forms (deterministic pins on the closed-object guard surface; live conversion/multi-consumer/ack fan-out = 1c) --

static void test_state_line_values() {
    // Wire-value contract (pinned by header/C macros): any change turns it red.
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
    // Default construction = closed; empty id, stats errors, close idempotent OK; moves keep the closed state.
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
    // Error's machine-readable bits exist and default to 0/false (always this form for link/deck/field; filling them on the client session form belongs to 1c).
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
    // Null guards for session-level error/last_error (no crash without a handle).
    assert(mediaservo_client_session_error(nullptr, nullptr) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
    assert(mediaservo_client_session_last_error(nullptr, buf, sizeof(buf)) == MEDIASERVO_CLIENT_ERR_INVALID_ARG);
}

int main() {
    test_version();
    test_exchange_invalid_arg();
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
