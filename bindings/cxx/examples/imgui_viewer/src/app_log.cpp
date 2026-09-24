// app_log — 实现。
#include "app_log.hpp"

#include <cstdarg>
#include <cstdio>
#include <utility>

namespace viewer {

void AppLog::add(const std::string& level, std::string msg) {
    LogLine l;
    l.t_s = std::chrono::duration<double>(std::chrono::steady_clock::now() - t0_).count();
    l.level = level;
    l.msg = std::move(msg);
    std::lock_guard<std::mutex> lk(mu_);
    lines_.push_back(std::move(l));
    if (lines_.size() > kMaxLines) lines_.pop_front();
}

void AppLog::add_fmt(const std::string& level, const char* fmt, ...) {
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    std::vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    add(level, buf);
}

std::deque<LogLine> AppLog::snapshot() const {
    std::lock_guard<std::mutex> lk(mu_);
    return lines_;
}

void AppLog::set_time_origin(std::chrono::steady_clock::time_point t0) { t0_ = t0; }

AppLog& log() {
    static AppLog instance;
    return instance;
}

} // namespace viewer
