// mediaservo-client-cxx 控制闭环 demo（S3 批；live e2e 载体，CI 门 = 编译+链接）。
//
// 流程镜像 crates/mediaservo-client/examples/basic.rs：
//   login → Session::connect → open_control({"chassis"}) → on_ack → 12× send(同 seq 重发)
//   任一 ack 到达 → exit 0；全部重试无 ack → exit 1。
//
// 编译示例（单行）:
//   g++ -std=c++17 -I bindings/c/include -I bindings/cxx/include -I bindings/c/mediaservo-client-c/include -I bindings/cxx/mediaservo-client-cxx/include bindings/cxx/mediaservo-client-cxx/examples/control_demo.cpp -L target/debug -lmediaservo_client -Wl,-rpath,$PWD/target/debug -o /tmp/opencode/control_demo
//
// 环境变量（无默认端点/凭证纪律，同 basic.rs）：
//   MSRTC_HTTP_BASE  http://<server>:9800        （必需）
//   MSRTC_WS_URL     ws://<server>:9800/ws       （必需）
//   MSRTC_ROOM       房间 ID                      （必需）
//   MSRTC_USER / MSRTC_PASS  账号口令             （必需）
//   MSRTC_LABEL      控制通道 label（可选，默认 "chassis"）

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

    // 1. 登录换 JWT
    auto token = ms::login(http_base, user, pass);
    if (!token) { fail("login", token.error()); return 1; }
    std::printf("login ok\n");

    // 2. 入房（Client 角色 = 消费形，控制权限由 server 账号 can_control 裁决）
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

    // 3. 控制出程（车端 consumer attach 有 ~20s 事件链时延——同 seq 重发幂等安全）
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
    return 0; // RAII: ctl → session 逆序析构即正序 close
}
