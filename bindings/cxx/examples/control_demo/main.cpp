// mediaservo-client-cxx control closed-loop demo (batch S3; the live-e2e vehicle; the CI gate = compile + link).
//
// The flow mirrors crates/mediaservo-client/examples/basic.rs:
//   login → Session::connect → open_control({"chassis"}) → on_ack → 12x send (same-seq re-sends)
//   Any ack arriving → exit 0; all retries exhausted without an ack → exit 1.
//
// Build example (single line):
//   g++ -std=c++17 -I bindings/c/include -I bindings/cxx/include -I bindings/c/mediaservo-client-c/include -I bindings/cxx/mediaservo-client-cxx/include bindings/cxx/mediaservo-client-cxx/examples/control_demo.cpp -L target/debug -lmediaservo_client -Wl,-rpath,$PWD/target/debug -o /tmp/opencode/control_demo
//
// Environment variables (no default endpoint/credentials discipline, same as basic.rs):
//   MSRTC_HTTP_BASE  http://<server>:9800        (required)
//   MSRTC_WS_URL     ws://<server>:9800/ws       (required)
//   MSRTC_ROOM       room ID                      (required)
//   MSRTC_USER / MSRTC_PASS  account password     (required)
//   MSRTC_LABEL      control channel label (optional, default "chassis")

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <thread>

#include "mediaservo/client.hpp"

namespace ms = mediaservo::client;

static std::atomic<bool> g_ack_received{false};

static void fail(const char* what, const mediaservo::Error& e) {
    std::fprintf(stderr, "%s FAILED: code=%d msg=%s\n", what, e.code, e.message.c_str());
}

static std::string required_env(const char* key) {
    const char* v = std::getenv(key);
    if (!v || !*v) {
        std::fprintf(stderr, "required env %s not set\n", key);
        std::exit(2);
    }
    return std::string(v);
}

int main() {
    const std::string http_base = required_env("MSRTC_HTTP_BASE");
    const std::string ws_url = required_env("MSRTC_WS_URL");
    const std::string room = required_env("MSRTC_ROOM");
    const std::string user = required_env("MSRTC_USER");
    const std::string pass = required_env("MSRTC_PASS");
    const char* label_env = std::getenv("MSRTC_LABEL");
    const std::string label = (label_env && *label_env) ? label_env : "chassis";

    // 1. Login for a JWT
    auto token = ms::login(http_base, user, pass);
    if (!token) { fail("login", token.error()); return 1; }
    std::printf("login ok\n");

    // 2. Join the room (the Client role = the consumer form; control permissions are adjudicated by the server account's can_control gate)
    ms::Config cfg;
    cfg.signaling_url = ws_url;
    cfg.room = room;
    cfg.jwt = token.value();
    cfg.role = "Client";
    auto session = ms::Session::connect(cfg);
    if (!session) { fail("session connect", session.error()); return 1; }

    auto neg = session->negotiated();
    if (!neg) { fail("negotiated", neg.error()); return 1; }
    std::printf("joined room=%s negotiated=%u\n", room.c_str(), neg.value());

    // 3. Send control outbound (the vehicle-side consumer attach has a ~20s event-chain delay — same-seq re-sends are idempotent-safe)
    auto ctl = session->open_control({label});
    if (!ctl) { fail("open_control", ctl.error()); return 1; }
    auto ids = ctl->producer_ids();
    if (ids) std::printf("control producers=%s\n", ids.value().c_str());

    auto hooked = ctl->on_ack([](const std::string& ack_json) {
        std::printf("ack: %s\n", ack_json.c_str());
        g_ack_received.store(true);
    });
    if (!hooked) { fail("on_ack", hooked.error()); return 1; }

    const uint64_t seq = 1;
    for (int attempt = 0; attempt < 12 && !g_ack_received.load(); ++attempt) {
        auto sent = ctl->send(label, seq, "steer", "{\"deg\":0.0}");
        if (!sent) { fail("send", sent.error()); return 1; }
        std::this_thread::sleep_for(std::chrono::seconds(5));
    }

    if (!g_ack_received.load()) {
        std::fprintf(stderr, "timed out waiting for ControlAck (12 retries)\n");
        return 1;
    }
    std::printf("control_demo OK\n");
    return 0; // RAII: ctl → session destroyed in reverse order = closed in the correct order
}
