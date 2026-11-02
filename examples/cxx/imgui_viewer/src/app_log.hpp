// app_log — the viewer's in-process log ring (the data source for the bottom Log panel; added in T4).
//
// Threading model (a template takeaway): writers = a mix of pump/callback threads and the main thread (Session callbacks come from WebRTC threads),
// readers = the render frame only. One mutex + a 500-line deque cap — low-frequency UI logs; do not reach for a lock-free queue.
// Split of duties with stdout: the `[frame]/[tile]` machine-judgment lines go to stdout (the dummy acceptance depends on them),
// human-readable milestone/error lines go into this ring (visible in the panel).
#ifndef MSRTC_VIEWER_APP_LOG_HPP
#define MSRTC_VIEWER_APP_LOG_HPP

#include <chrono>
#include <deque>
#include <mutex>
#include <string>

namespace viewer {

struct LogLine {
    double t_s;          // seconds relative to process start (main sets the origin)
    std::string level;   // "info" | "warn"
    std::string msg;
};

class AppLog {
public:
    static constexpr size_t kMaxLines = 500;  // the cap = the memory bound; the UI scrollbar naturally limits the render volume

    void add(const std::string& level, std::string msg);
    /// printf-style variadic — for SDK callback sites (C-style contexts); C++17 has no std::format, snprintf is the right answer.
    void add_fmt(const std::string& level, const char* fmt, ...)
#if defined(__GNUC__)
        __attribute__((format(printf, 3, 4)))
#endif
        ;
    /// Render-side snapshot (a full copy of ≤500 lines is still cheap at 60fps; ponytail: switch to incremental reads if a log storm appears).
    std::deque<LogLine> snapshot() const;
    void set_time_origin(std::chrono::steady_clock::time_point t0);

private:
    mutable std::mutex mu_;
    std::deque<LogLine> lines_;
    std::chrono::steady_clock::time_point t0_{};
};

/// Application-wide singleton (deliberately simple for an example: one log per viewer process, no injection seam).
AppLog& log();

} // namespace viewer

#endif
