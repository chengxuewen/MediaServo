// mediaservo-link-cxx compile-and-run tests: version + error paths + move semantics + Result misuse + callback lifetime.
// Build example: g++ -std=c++17 -I bindings/cxx/mediaservo-link-cxx/include -I bindings/c/mediaservo-link-c/include
//           -I bindings/c/include tests/test_link.cpp -L target/debug -lmediaservo_link
//           -o /tmp/opencode/test_link_cxx

#include <cassert>
#include <stdexcept>
#include <string>
#include <vector>

#include "mediaservo/link.hpp"

using mediaservo::link::Bus;
using mediaservo::link::SignalConfig;
using mediaservo::link::SignalSession;

static void test_version() {
    auto v = mediaservo::link::version();
    assert(v.has_value());
    assert(v.value().rfind("0.1.", 0) == 0); // "0.1.x"
}

static void test_signal_connect_error_path() {
    // Empty config → the C ABI fails fast with INVALID_ARG (url/psk/room required); no network touched
    auto r = SignalSession::connect(SignalConfig{});
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_LINK_ERR_INVALID_ARG);
    assert(!r.error().message.empty());
}

static void test_result_misuse_throws() {
    auto r = SignalSession::connect(SignalConfig{});
    assert(!r.has_value());
    bool threw = false;
    try {
        (void)r.value();
    } catch (const tl::bad_expected_access<mediaservo::Error>&) {
        threw = true;
    }
    assert(threw && "value() on error must throw bad_expected_access");

    auto ok_r = mediaservo::link::version();
    assert(ok_r.has_value());
}

static void test_closed_signal_error_path() {
    SignalSession s; // default construction = closed
    assert(!s);

    auto send = s.send("{\"type\":\"ping\"}");
    assert(!send.has_value());
    assert(send.error().code == MEDIASERVO_LINK_ERR_INVALID_ARG);
    assert(send.error().message == "closed");

    s.on_event([](const std::string&) {}); // closed no-op, no crash
    assert(s.close().has_value()); // idempotent
    assert(s.close().has_value());
}

static void test_bus_attach_error_path() {
    // Empty token/vk → JWT verification fails fast (BUS error; no iceoryx node created)
    auto r = Bus::attach("", "", "");
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_LINK_ERR_BUS);
    assert(!r.error().message.empty());
}

static void test_closed_bus_error_path() {
    Bus b; // default construction = closed
    assert(!b);

    mediaservo_frame_meta_t meta{};
    auto p = b.publish("camera/0", std::vector<uint8_t>{}, meta);
    assert(!p.has_value());
    assert(p.error().code == MEDIASERVO_LINK_ERR_INVALID_ARG);
    assert(p.error().message == "closed");

    auto sub = b.subscribe("camera/0");
    assert(!sub.has_value());
    assert(sub.error().code == MEDIASERVO_LINK_ERR_INVALID_ARG);

    assert(b.close().has_value());
    assert(b.close().has_value());
}

static void test_closed_stream_error_path() {
    mediaservo::link::Stream st; // default construction = closed
    assert(!st); // Stream is a handle class (operator bool)

    auto f = st.recv();
    assert(!f.has_value());
    assert(f.error().code == MEDIASERVO_LINK_ERR_INVALID_ARG);
    assert(f.error().message == "closed");

    assert(st.close().has_value());
    assert(st.close().has_value());
}

static void test_move_semantics() {
    SignalSession a;
    SignalSession b(std::move(a));
    assert(!a && !b);

    SignalSession c;
    c = std::move(b);
    assert(!b && !c);
    (void)c.close();

    Bus x;
    Bus y(std::move(x));
    assert(!x && !y);
    y = std::move(x);
    assert(!x && !y);
    (void)y.close();

    mediaservo::link::Stream s1;
    mediaservo::link::Stream s2(std::move(s1));
    assert(!s1 && !s2);
    (void)s2.close();
}

int main() {
    test_version();
    test_signal_connect_error_path();
    test_result_misuse_throws();
    test_closed_signal_error_path();
    test_bus_attach_error_path();
    test_closed_bus_error_path();
    test_closed_stream_error_path();
    test_move_semantics();
    return 0;
}
