#include "imgui_shell/core.hpp"

#include <cstdlib>

namespace viewer_core {

void RateEstimator::update(int64_t now_ms, uint64_t total) {
    if (!primed_) {
        primed_ = true;
        t0_ = now_ms;
        base_ = total;
        last_per_ms_ = 0.0;
        return;
    }
    const int64_t dt = now_ms - t0_;
    if (total < base_) {
        // Counter regression (re-subscription / process-side reset): re-anchor, zero the current window.
        t0_ = now_ms;
        base_ = total;
        last_per_ms_ = 0.0;
        return;
    }
    if (dt <= 0) {
        return; // same window / regression: keep the previous window's value (pin: no divide-by-zero, no fake zeroing)
    }
    last_per_ms_ = static_cast<double>(total - base_) / static_cast<double>(dt);
    t0_ = now_ms;
    base_ = total;
}

double RateEstimator::per_sec() const { return last_per_ms_ * 1000.0; }

double RateEstimator::kbps() const { return last_per_ms_ * 8.0; } // bytes/ms → kbit/s

void RateEstimator::reset() { *this = RateEstimator{}; }

namespace {

// After `"key"`, scan past `:` and whitespace to the number start; returns the index of the digit run, npos on failure.
std::string::size_type find_number(const std::string& json, const std::string& key) {
    const std::string pat = "\"" + key + "\"";
    std::string::size_type pos = json.find(pat);
    while (pos != std::string::npos) {
        pos += pat.size();
        while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) pos++;
        if (pos < json.size() && json[pos] == ':') {
            pos++;
            while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) pos++;
            if (pos < json.size() && (json[pos] == '-' || (json[pos] >= '0' && json[pos] <= '9'))) {
                return pos;
            }
        }
        pos = json.find(pat, pos);
    }
    return std::string::npos;
}

} // namespace

bool json_u64(const std::string& json, const std::string& key, uint64_t* out) {
    const auto pos = find_number(json, key);
    if (pos == std::string::npos) return false;
    // Reject decimals / scientific notation (the stats counter domain is integers; a decimal point = the wrong shape; prefer missing over wrong).
    auto end = pos;
    while (end < json.size() && ((json[end] >= '0' && json[end] <= '9'))) end++;
    if (end == pos || (end < json.size() && (json[end] == '.' || json[end] == 'e' || json[end] == 'E'))) {
        return false;
    }
    if (out) *out = std::strtoull(json.c_str() + pos, nullptr, 10);
    return true;
}

bool json_f64(const std::string& json, const std::string& key, double* out) {
    const auto pos = find_number(json, key);
    if (pos == std::string::npos) return false;
    if (out) *out = std::strtod(json.c_str() + pos, nullptr);
    return true;
}

} // namespace viewer_core
