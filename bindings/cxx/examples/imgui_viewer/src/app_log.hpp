// app_log — viewer 进程内日志环（底部 Log 面板的数据源；T4 新增件）。
//
// 线程模型（模板要点）：写方 = 泵/回调线程与主线程混用（Session 回调来自 WebRTC 线程），
// 读方 = 仅渲染帧。一把 mutex + 500 行 deque 封顶——低频 UI 日志，别上无锁队列。
// 与 stdout 的分工：`[frame]/[tile]` 机器判据行走 stdout（dummy 验收依赖），
// 人读的里程碑/错误行进本环（面板可见）。
#ifndef MSRTC_VIEWER_APP_LOG_HPP
#define MSRTC_VIEWER_APP_LOG_HPP

#include <chrono>
#include <deque>
#include <mutex>
#include <string>

namespace viewer {

struct LogLine {
    double t_s;          // 相对进程启动秒（main 设原点）
    std::string level;   // "info" | "warn"
    std::string msg;
};

class AppLog {
public:
    static constexpr size_t kMaxLines = 500;  // 封顶=内存上界；UI 滚动条天然限渲染量

    void add(const std::string& level, std::string msg);
    /// printf 形变参——SDK 回调点（C 风格上下文）用；C++17 无 std::format，snprintf 即正解。
    void add_fmt(const std::string& level, const char* fmt, ...)
#if defined(__GNUC__)
        __attribute__((format(printf, 3, 4)))
#endif
        ;
    /// 渲染侧快照（整拷 ≤500 行=60fps 下依然便宜，ponytail: 若日志风暴再改增量读）。
    std::deque<LogLine> snapshot() const;
    void set_time_origin(std::chrono::steady_clock::time_point t0);

private:
    mutable std::mutex mu_;
    std::deque<LogLine> lines_;
    std::chrono::steady_clock::time_point t0_{};
};

/// 全应用单例（example 级别刻意简单：一个 viewer 进程一份日志，不做注入缝）。
AppLog& log();

} // namespace viewer

#endif
