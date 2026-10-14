// mediaservo-deck-cxx compile-and-run tests: version + device enumeration + error paths + move semantics + Result misuse.
// Build example: g++ -std=c++17 -I bindings/cxx/mediaservo-deck-cxx/include -I bindings/c/mediaservo-deck-c/include
//           -I bindings/c/include tests/test_deck.cpp -L target/debug -lmediaservo_deck
//           -o /tmp/opencode/test_deck_cxx

#include <cassert>
#include <stdexcept>
#include <string>
#include <vector>

#include "mediaservo/deck.hpp"

using mediaservo::deck::CameraSource;
using mediaservo::deck::CaptureOptions;
using mediaservo::deck::DeviceKind;
using mediaservo::deck::Player;
using mediaservo::deck::Recorder;

static void test_version() {
    auto v = mediaservo::deck::version();
    assert(v.has_value());
    assert(v.value().rfind("0.1.", 0) == 0); // "0.1.x"
}

static void test_enumerate_devices() {
    // the stub backend should have "stub:test-camera"; failure/empty list must not crash
    auto cams = mediaservo::deck::enumerate_devices(DeviceKind::Camera);
    for (const auto& id : cams) {
        assert(!id.empty());
    }
    (void)mediaservo::deck::enumerate_devices(DeviceKind::Audio);
    (void)mediaservo::deck::enumerate_devices(DeviceKind::Screen);
}

static void test_camera_error_path() {
    // Empty dev_id → the C ABI fails fast with INVALID_ARG; no hardware touched
    auto r = CameraSource::open("");
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_DECK_ERR_INVALID_ARG);
    assert(!r.error().message.empty());

    // Closed camera: start/on_frame → INVALID_ARG/"closed"
    CameraSource c;
    assert(!c);
    auto st = c.start();
    assert(!st.has_value());
    assert(st.error().code == MEDIASERVO_DECK_ERR_INVALID_ARG);
    assert(st.error().message == "closed");
    auto cb = c.on_frame([](const mediaservo_frame_t&) {});
    assert(!cb.has_value());
    assert(cb.error().code == MEDIASERVO_DECK_ERR_INVALID_ARG);
    assert(c.stop().has_value()); // idempotent
    assert(c.close().has_value());
    assert(c.close().has_value());
}

static void test_recorder_error_path() {
    // Empty path → fast INVALID_ARG
    auto r = Recorder::open("");
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_DECK_ERR_INVALID_ARG);

    // Closed recorder + unopened camera → INVALID_ARG/"closed"
    Recorder rec;
    CameraSource cam;
    assert(!rec && !cam);
    auto rd = rec.record(cam);
    assert(!rd.has_value());
    assert(rd.error().code == MEDIASERVO_DECK_ERR_INVALID_ARG);
    assert(rd.error().message == "closed"); // a closed recorder takes priority over the camera check
    assert(rec.stop().has_value());
    assert(rec.close().has_value());
}

static void test_player_error_path() {
    // Empty path → fast INVALID_ARG
    auto r = Player::open("");
    assert(!r.has_value());
    assert(r.error().code == MEDIASERVO_DECK_ERR_INVALID_ARG);

    // Closed player: on_frame → INVALID_ARG/"closed"
    Player p;
    assert(!p);
    auto cb = p.on_frame([](const mediaservo_frame_t&) {});
    assert(!cb.has_value());
    assert(cb.error().code == MEDIASERVO_DECK_ERR_INVALID_ARG);
    assert(p.close().has_value());
}

static void test_result_misuse_throws() {
    auto r = CameraSource::open("");
    assert(!r.has_value());
    bool threw = false;
    try {
        (void)r.value();
    } catch (const tl::bad_expected_access<mediaservo::Error>&) {
        threw = true;
    }
    assert(threw && "value() on error must throw bad_expected_access");

    auto ok_r = mediaservo::deck::version();
    assert(ok_r.has_value());
}

static void test_move_semantics() {
    CameraSource a;
    CameraSource b(std::move(a));
    assert(!a && !b);
    CameraSource c;
    c = std::move(b);
    assert(!b && !c);
    (void)c.close();

    Recorder x;
    Recorder y(std::move(x));
    assert(!x && !y);
    (void)y.close();

    Player p1;
    Player p2(std::move(p1));
    assert(!p1 && !p2);
    (void)p2.close();
}

int main() {
    test_version();
    test_enumerate_devices();
    test_camera_error_path();
    test_recorder_error_path();
    test_player_error_path();
    test_result_misuse_throws();
    test_move_semantics();
    return 0;
}
