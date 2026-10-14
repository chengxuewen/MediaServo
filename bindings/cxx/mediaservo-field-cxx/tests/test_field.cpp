// mediaservo-field-cxx compile-and-run tests: version + error paths + move semantics + Result misuse.
// Build example: g++ -std=c++17 -I bindings/cxx/mediaservo-field-cxx/include -I bindings/c/mediaservo-field-c/include
//           -I bindings/c/include tests/test_field.cpp -L target/debug -lmediaservo_field
//           -o /tmp/opencode/test_field_cxx

#include <cassert>
#include <stdexcept>
#include <string>

#include "mediaservo/field.hpp"

using mediaservo::field::PushConfig;
using mediaservo::field::PushSession;

static void test_version() {
    auto v = mediaservo::field::version();
    assert(v.has_value());
    assert(v.value().rfind("0.1.", 0) == 0); // "0.1.x"
}

static void test_connect_error_path() {
    // Empty config → the C ABI fails fast with INVALID_ARG (url/psk/room required); no network touched
    auto r = PushSession::connect(PushConfig{});
    assert(!r.has_value());
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_FIELD_ERR_INVALID_ARG);
    assert(!r.error().message.empty());
}

static void test_result_misuse_throws() {
    auto r = PushSession::connect(PushConfig{});
    assert(!r.has_value());
    bool threw = false;
    try {
        (void)r.value();
    } catch (const tl::bad_expected_access<mediaservo::Error>&) {
        threw = true;
    }
    assert(threw && "value() on error must throw bad_expected_access");
}

static void test_closed_session_error_path() {
    PushSession s; // default construction = closed
    assert(!s);

    auto p = s.publish_video();
    assert(!p.has_value());
    assert(p.error().code == MEDIASERVO_FIELD_ERR_INVALID_ARG);
    assert(p.error().message == "closed");

    auto st = s.start_video_frames();
    assert(!st.has_value());
    assert(st.error().code == MEDIASERVO_FIELD_ERR_INVALID_ARG);

    s.stop_video_frames(); // closed no-op, no crash
    assert(s.close().has_value()); // idempotent
    assert(s.close().has_value());
}

static void test_move_semantics() {
    PushSession a;                       // null
    PushSession b(std::move(a));         // move construction
    assert(!a && !b);

    PushSession c;
    c = std::move(b);                    // move assignment
    assert(!b && !c);

    (void)c.close();                     // double-close safe (the destructor triggers once more)
}

int main() {
    test_version();
    test_connect_error_path();
    test_result_misuse_throws();
    test_closed_session_error_path();
    test_move_semantics();
    return 0;
}
