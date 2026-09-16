/* viewer_core — GUI 无关纯函数层（p3 W3a：stats 差分推导 + JSON 标量提取）。
 *
 * 归属 imgui_shell 目录（PLAN §3 A+B 同库），命名空间独立=可被任意例子复用/单测。
 * 设计红线：不持 SDL/ImGui 类型；时间由调用方注入（now_ms 参数=可测形，无墙钟依赖）。
 */
#ifndef MEDIASERVO_VIEWER_CORE_HPP
#define MEDIASERVO_VIEWER_CORE_HPP

#include <cstdint>
#include <string>

namespace viewer_core {

/// 累计计数器 → 速率估计器（inbound-rtp 语义：喂 bytesReceived / framesDecoded 累计值）。
/// 钉（ctest -R core）：
/// - 首次 update 只锚基线，速率 0；
/// - Δt<=0（同毫秒/时钟回退）→ 保持上一窗值，不除零、不假清零；
/// - counter < base（会话重置/流重启）→ 重锚 + 当前窗 0——**绝不输出负速率**；
/// - 正常窗 = (Δcount/Δt) 滚动基线。
class RateEstimator {
public:
    void update(int64_t now_ms, uint64_t total);

    /// 每秒增量（帧计数用此；无数据/未成窗 = 0）。
    double per_sec() const;

    /// 字节计数专用：每秒字节 → kbit/s（bytes/ms 斜率 ×8）。
    double kbps() const;

    void reset();

private:
    bool primed_ = false;
    int64_t t0_ = 0;
    uint64_t base_ = 0;
    double last_per_ms_ = 0.0;
};

/// 扁平 JSON 对象里提取数值标量：扫描 `"key"` : <number>（无第三方库的壳层依赖）。
/// 支持整数与小数（webrtc stats 数值域；不支持带引号数字/科学计数=命中即 false
/// 由调用方走缺省）。`out` 可 NULL = 仅存在性判断。
bool json_u64(const std::string& json, const std::string& key, uint64_t* out);
bool json_f64(const std::string& json, const std::string& key, double* out);

} // namespace viewer_core

#endif /* MEDIASERVO_VIEWER_CORE_HPP */
