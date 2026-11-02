// mediaservo-deck-cxx capture→record→playback closed-loop example (skeleton; needs the deck backend dependencies ready to run).
// Build example: g++ -std=c++17 -I bindings/cxx/mediaservo-deck-cxx/include -I bindings/c/mediaservo-deck-c/include
//           -I bindings/c/include examples/vehicle_deck.cpp -L target/debug -lmediaservo_deck

#include <cstdlib>
#include <iostream>
#include <string>
#include <vector>

#include "mediaservo/deck.hpp"

int main() {
    using mediaservo::deck::CameraSource;
    using mediaservo::deck::CaptureOptions;
    using mediaservo::deck::DeviceKind;
    using mediaservo::deck::Player;
    using mediaservo::deck::Recorder;

    const char* out_path = std::getenv("MEDIASERVO_RECORD_PATH");
    if (!out_path) {
        std::cerr << "set MEDIASERVO_RECORD_PATH\n";
        return 1;
    }

    // Enumerate → open the first camera
    auto cams = mediaservo::deck::enumerate_devices(DeviceKind::Camera);
    if (cams.empty()) {
        std::cerr << "no camera devices\n";
        return 1;
    }
    std::cout << "camera: " << cams[0] << "\n";

    CaptureOptions opts; // default 1280x720@30
    auto cam_result = CameraSource::open(cams[0], opts);
    if (!cam_result) {
        std::cerr << "camera open failed: code=" << cam_result.error().code
                  << " msg=" << cam_result.error().message << "\n";
        return 1;
    }
    auto cam = std::move(cam_result).value();

    // Frame callback (pump thread; the frame pointer is valid only inside the callback — print only, never keep it)
    auto cb = cam.on_frame([](const mediaservo_frame_t& frame) {
        std::cout << "frame " << frame.width << "x" << frame.height << "\n";
    });
    if (!cb) {
        std::cerr << "on_frame failed: code=" << cb.error().code << "\n";
        return 1;
    }
    auto started = cam.start();
    if (!started) {
        std::cerr << "start failed: code=" << started.error().code << "\n";
        return 1;
    }

    // Record (the camera must be started and outlive the recording → close the recorder first)
    auto rec_result = Recorder::open(out_path);
    if (!rec_result) {
        std::cerr << "recorder open failed: code=" << rec_result.error().code << "\n";
        return 1;
    }
    auto rec = std::move(rec_result).value();
    auto recorded = rec.record(cam);
    if (!recorded) {
        std::cerr << "record failed: code=" << recorded.error().code << "\n";
        return 1;
    }

    std::cout << "recording to " << out_path << " (Ctrl-C to stop)...\n";
    std::cin.get();

    (void)rec.stop();   // request stop
    (void)rec.close();  // close the recorder first (flush + trailer)
    (void)cam.stop();   // then close the camera
    (void)cam.close();

    // Playback
    auto play_result = Player::open(out_path);
    if (!play_result) {
        std::cerr << "player open failed: code=" << play_result.error().code << "\n";
        return 1;
    }
    auto player = std::move(play_result).value();
    auto frames_cb = player.on_frame([](const mediaservo_frame_t& frame) {
        std::cout << "playback frame " << frame.width << "x" << frame.height << "\n";
    });
    if (!frames_cb) {
        std::cerr << "playback on_frame failed: code=" << frames_cb.error().code << "\n";
        return 1;
    }
    (void)player.close(); // blocking join of the decode pump until EOF
    std::cout << "playback done\n";
    return 0;
}
