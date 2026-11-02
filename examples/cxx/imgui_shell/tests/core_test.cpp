// viewer_core unit tests (the PLAN W3 exit criterion: ctest -R core picks up this file).
// Pins = bitrate Δt=0 / reset regression / normal window + JSON extraction hits/rejections. assert-based, no framework.

#include <cassert>
#include <cstdio>

#include "imgui_shell/core.hpp"

using viewer_core::json_f64;
using viewer_core::json_u64;
using viewer_core::RateEstimator;

static void test_rate_estimator_normal_window() {
    RateEstimator r;
    r.update(1000, 0);            // anchor the baseline
    assert(r.per_sec() == 0.0);
    r.update(2000, 1000);         // 1000 bytes/1000ms = 1000 B/s = 8 kbps
    assert(r.per_sec() == 1000.0);
    assert(r.kbps() == 8.0);
    r.update(3000, 3000);         // Δ=2000/1000ms
    assert(r.per_sec() == 2000.0);
    assert(r.kbps() == 16.0);
}

static void test_rate_estimator_zero_dt_keeps_last() {
    RateEstimator r;
    r.update(1000, 100);
    r.update(2000, 1100); // Δ=1000/1s
    const double held = r.per_sec();
    r.update(2000, 1100); // fed again in the same millisecond: Δt=0 → keep the previous window (no divide-by-zero, no settling)
    assert(r.per_sec() == held);
    r.update(1500, 1200); // clock regression: dt<0 keeps the value too
    assert(r.per_sec() == held);
}

static void test_rate_estimator_counter_reset_reanchors() {
    RateEstimator r;
    r.update(1000, 50000);
    r.update(2000, 60000);
    assert(r.per_sec() > 0.0);
    r.update(3000, 100); // regression (re-subscription / stream restart) → re-anchor + current window 0; never outputs a negative rate
    assert(r.per_sec() == 0.0);
    assert(r.kbps() == 0.0);
    r.update(4000, 1100); // a normal window against the new baseline
    assert(r.per_sec() == 1000.0);
    r.reset();
    assert(r.per_sec() == 0.0);
    r.update(9000, 7); // after reset = unanchored
    assert(r.per_sec() == 0.0);
}

static void test_json_scalar_extraction() {
    const std::string j =
        R"({"bytes_received":123456,"packets_lost":7,"frames_per_second":29.97,"frame_width":1280})";
    uint64_t u = 0;
    double f = 0.0;
    assert(json_u64(j, "bytes_received", &u) && u == 123456);
    assert(json_u64(j, "packets_lost", &u) && u == 7);
    assert(json_f64(j, "frames_per_second", &f) && f > 29.96 && f < 29.98);
    assert(json_f64(j, "frame_width", &f) && f == 1280.0); // an integer is consumable as f64 too
    assert(!json_u64(j, "nope", &u));
    assert(!json_u64(j, "bytes_received", nullptr) == false); // the existence check
    // Rejected shapes: quoted numbers / decimals in the counter domain → u64 false (prefer missing over wrong)
    assert(!json_u64(R"({"a":"12"})", "a", &u));
    assert(!json_u64(R"({"a":1.5})", "a", &u));
    // Whitespace tolerance
    assert(json_u64(R"({ "a" :  42 })", "a", &u) && u == 42);
}

int main() {
    test_rate_estimator_normal_window();
    test_rate_estimator_zero_dt_keeps_last();
    test_rate_estimator_counter_reset_reanchors();
    test_json_scalar_extraction();
    std::printf("viewer_core: all assertions PASS\n");
    return 0;
}
