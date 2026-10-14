/* viewer_core — the GUI-agnostic pure-function layer (p3 W3a: stats diff derivation + JSON scalar extraction).
 *
 * Lives in the imgui_shell directory (the PLAN §3 A+B same-library plan); an independent namespace = reusable/unit-testable by any example.
 * Design red line: no SDL/ImGui types; time is injected by the caller (the now_ms parameter = the testable form, no wall-clock dependency).
 */
#ifndef MEDIASERVO_VIEWER_CORE_HPP
#define MEDIASERVO_VIEWER_CORE_HPP

#include <cstdint>
#include <string>

namespace viewer_core {

/// Cumulative counter → rate estimator (inbound-rtp semantics: feed the bytesReceived / framesDecoded cumulative values).
/// Pins (ctest -R core):
/// - the first update only anchors the baseline, rate 0;
/// - Δt<=0 (same millisecond / clock regression) → keep the previous window's value; no divide-by-zero, no fake zeroing;
/// - counter < base (session reset / stream restart) → re-anchor + current window 0 — **never outputs a negative rate**;
/// - a normal window = (Δcount/Δt) over a rolling baseline.
class RateEstimator {
public:
    void update(int64_t now_ms, uint64_t total);

    /// Per-second delta (use this for frame counts; no data / window not formed = 0).
    double per_sec() const;

    /// Bytes-specific: bytes per second → kbit/s (the bytes/ms slope ×8).
    double kbps() const;

    void reset();

private:
    bool primed_ = false;
    int64_t t0_ = 0;
    uint64_t base_ = 0;
    double last_per_ms_ = 0.0;
};

/// Extract a numeric scalar from a flat JSON object: scan `"key"` : <number> (no third-party library dependency in this shell layer).
/// Supports integers and decimals (the webrtc stats numeric domain; quoted numbers / scientific notation are unsupported = false on match,
/// the caller uses the default). `out` may be NULL = an existence check only.
bool json_u64(const std::string& json, const std::string& key, uint64_t* out);
bool json_f64(const std::string& json, const std::string& key, double* out);

} // namespace viewer_core

#endif /* MEDIASERVO_VIEWER_CORE_HPP */
