// mediaservo-link-cxx signaling + events example (skeleton; needs a running server for the full flow).
// Build example: g++ -std=c++17 -I bindings/cxx/mediaservo-link-cxx/include -I bindings/c/mediaservo-link-c/include
//           -I bindings/c/include examples/vehicle_link.cpp -L target/debug -lmediaservo_link

#include <cstdlib>
#include <iostream>
#include <string>

#include "mediaservo/link.hpp"

int main() {
    using mediaservo::link::SignalConfig;
    using mediaservo::link::SignalSession;

    // Configuration comes from environment variables (hardcoded keys/addresses are forbidden)
    const char* url = std::getenv("MEDIASERVO_SIGNAL_URL");
    const char* psk = std::getenv("MEDIASERVO_PSK");
    const char* room = std::getenv("MEDIASERVO_ROOM");
    if (!url || !psk || !room) {
        std::cerr << "set MEDIASERVO_SIGNAL_URL / MEDIASERVO_PSK / MEDIASERVO_ROOM\n";
        return 1;
    }

    SignalConfig cfg;
    cfg.url = url;
    cfg.psk = psk;
    cfg.room = room;
    cfg.role = "Pusher"; // the vehicle-side push role

    // Register the event callback before connecting? — the callback can be registered any time after connect;
    // on first registration the pump re-synthesizes the {"type":"connected"} event (the C ABI contract).
    auto result = SignalSession::connect(cfg);
    if (!result) {
        std::cerr << "connect failed: code=" << result.error().code
                  << " msg=" << result.error().message << "\n";
        return 1;
    }
    auto session = std::move(result).value();

    session.on_event([](const std::string& event_json) {
        std::cout << "event: " << event_json << "\n"; // the event string is valid only inside the callback (already copied)
    });

    auto sent = session.send("{\"type\":\"get_status\"}");
    if (!sent) {
        std::cerr << "send failed: code=" << sent.error().code
                  << " msg=" << sent.error().message << "\n";
        return 1;
    }

    std::cout << "waiting for events (Ctrl-C to stop)...\n";
    std::cin.get();

    auto closed = session.close();
    if (!closed) {
        std::cerr << "close failed: code=" << closed.error().code << "\n";
        return 1;
    }
    return 0;
}
