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
        // 计数器倒退（重订阅/进程侧重置）：重锚，当前窗归零。
        t0_ = now_ms;
        base_ = total;
        last_per_ms_ = 0.0;
        return;
    }
    if (dt <= 0) {
        return; // 同窗/回退：保持上一窗值（钉：不除零不假清零）
    }
    last_per_ms_ = static_cast<double>(total - base_) / static_cast<double>(dt);
    t0_ = now_ms;
    base_ = total;
}

double RateEstimator::per_sec() const { return last_per_ms_ * 1000.0; }

double RateEstimator::kbps() const { return last_per_ms_ * 8.0; } // bytes/ms → kbit/s

void RateEstimator::reset() { *this = RateEstimator{}; }

namespace {

// `"key"` 后扫过 `:` 与空白到数字起点；返回数字串起始下标，失败 npos。
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
    // 拒绝小数/科学计数（stats 计数域为整数；带小数点=形不符，宁缺毋错）。
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
