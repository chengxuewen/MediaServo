// mediaservo-field-cxx push-flow example (skeleton; needs a running server for the full flow).
// Build example: g++ -std=c++17 -I bindings/cxx/mediaservo-field-cxx/include -I bindings/c/mediaservo-field-c/include
//           -I bindings/c/include examples/vehicle_field.cpp -L target/debug -lmediaservo_field

#include <iostream>
#include <string>

#include "mediaservo/field.hpp"

int main() {
    using mediaservo::field::PushConfig;
    using mediaservo::field::PushSession;

    // Configuration comes from environment variables (hardcoded keys/addresses are forbidden)
    const char* url = std::getenv("MEDIASERVO_SIGNAL_URL");
    const char* psk = std::getenv("MEDIASERVO_PSK");
    const char* room = std::getenv("MEDIASERVO_ROOM");
    if (!url || !psk || !room) {
        std::cerr << "set MEDIASERVO_SIGNAL_URL / MEDIASERVO_PSK / MEDIASERVO_ROOM\n";
        return 1;
    }

    PushConfig cfg;
    cfg.url = url;
    cfg.psk = psk;
    cfg.room = room;

    auto session = PushSession::connect(cfg);
    if (!session) {
        std::cerr << "connect failed: code=" << session.error().code
                  << " msg=" << session.error().message << "\n";
        return 1;
    }
    auto s = std::move(session).value();

    auto track = s.publish_video();
    if (!track) {
        std::cerr << "publish failed: code=" << track.error().code
                  << " msg=" << track.error().message << "\n";
        return 1;
    }
    std::cout << "published track: " << track.value() << "\n";

    auto started = s.start_video_frames();
    if (!started) {
        std::cerr << "start frames failed: code=" << started.error().code << "\n";
        return 1;
    }

    std::cout << "pushing frames (Ctrl-C to stop)...\n";
    std::cin.get(); // block until the user stops

    s.stop_video_frames();
    auto closed = s.close();
    if (!closed) {
        std::cerr << "close failed: code=" << closed.error().code << "\n";
        return 1;
    }
    return 0;
}
