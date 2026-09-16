// viewer_core 单元测试（PLAN W3 出门判据：ctest -R core 命中本件）。
// 钉 = 码率 Δt=0/复位倒退/正常窗 + JSON 提取命中/拒形。assert-based，无框架。

#include <cassert>
#include <cstdio>

#include "imgui_shell/core.hpp"

using viewer_core::json_f64;
using viewer_core::json_u64;
using viewer_core::RateEstimator;

static void test_rate_estimator_normal_window() {
    RateEstimator r;
    r.update(1000, 0);            // 锚基线
    assert(r.per_sec() == 0.0);
    r.update(2000, 1000);         // 1000 字节/1000ms = 1000 B/s = 8 kbps
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
    r.update(2000, 1100); // 同毫秒再喂：Δt=0 → 保持上一窗（不除零、不清算）
    assert(r.per_sec() == held);
    r.update(1500, 1200); // 时钟回退：dt<0 同样保持
    assert(r.per_sec() == held);
}

static void test_rate_estimator_counter_reset_reanchors() {
    RateEstimator r;
    r.update(1000, 50000);
    r.update(2000, 60000);
    assert(r.per_sec() > 0.0);
    r.update(3000, 100); // 倒退（重订阅/流重启）→ 重锚 + 当前窗 0，绝不输出负速率
    assert(r.per_sec() == 0.0);
    assert(r.kbps() == 0.0);
    r.update(4000, 1100); // 新基线正常窗
    assert(r.per_sec() == 1000.0);
    r.reset();
    assert(r.per_sec() == 0.0);
    r.update(9000, 7); // reset 后 = 未锚定
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
    assert(json_f64(j, "frame_width", &f) && f == 1280.0); // 整数也可为 f64 消费
    assert(!json_u64(j, "nope", &u));
    assert(!json_u64(j, "bytes_received", nullptr) == false); // 存在性判断
    // 拒形：带引号数字 / 计数域小数 → u64 false（宁缺毋错）
    assert(!json_u64(R"({"a":"12"})", "a", &u));
    assert(!json_u64(R"({"a":1.5})", "a", &u));
    // 空白容忍
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
