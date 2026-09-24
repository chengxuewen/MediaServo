// panels/control — 实现（UI 行逐字迁居；建立块换成 sessions 调用，语义不变）。
#include "control.hpp"

#include <memory>
#include <vector>

#include <imgui.h>

#include "../app_log.hpp"
#include "../dock_layout.hpp"
#include "../sessions.hpp"

namespace viewer {

void render_control(AppModel& m) {
    ImGui::Begin(dock::kControl);
    if (m.tiles.empty()) {
        ImGui::TextDisabled("join a room first");
        ImGui::End();  // 早退也必须关窗（T4 补丁漏网：Begin/End 配平要数所有 return 路径——09-24 用户实跑第二次揪出）
        return;
    }
    std::vector<const char*> names;
    for (auto& t : m.tiles) names.push_back(t->room.c_str());
    if (m.sel_control >= static_cast<int>(names.size())) m.sel_control = -1;
    ImGui::Combo("tile", &m.sel_control, names.data(), static_cast<int>(names.size()));
    if (m.sel_control < 0) {
        ImGui::End();  // 同族早退二漏（grep 全面板扫出）——Begin/End 配平=每文件 return 路径逐条数
        return;
    }
    Tile* t = m.tiles[m.sel_control].get();
    ensure_control(*t); // 一次性建立；失败原因进红行，重连=Rooms 页 Reconnect（旧内联块的调用化）
    ImGui::SeparatorText(t->room.c_str());
    if (!t->err.empty()) ImGui::TextColored({1, .4f, .4f, 1}, "%s", t->err.c_str());
    ImGui::SliderFloat("steer deg", &m.steer_deg, -90.0f, 90.0f);
    if (ImGui::Button("send steer")) {
        t->send_cmd("chassis", ++m.seq, "steer",
                    (std::string("{\"deg\":") + std::to_string(m.steer_deg) + "}").c_str());
    }
    ImGui::SameLine();
    // W4b：签名急停组合面（Config.hmac_key_file 决定签名态；
    // 未配 key = 迁移放行形；车端有 key 则拒签=正确裁决）。
    if (ImGui::Button("ESTOP")) {
        auto es = t->sess.emergency_stop(*t->ctl, "chassis", 900,
                                         R"({"reason":"viewer-ui"})");
        m.estop_sent = es.has_value();
        log().add_fmt(es ? "info" : "warn", "ESTOP %s (%s)", es ? "sent+acked" : "FAILED",
                      m.key_signed ? "signed" : "unsigned");
        if (!es) {
            std::lock_guard<std::mutex> lk(t->ack_mu);
            t->ack_last = "estop: " + es.error().message;
        }
    }
    ImGui::SameLine();
    ImGui::TextDisabled("%s", m.key_signed ? "signed" : "unsigned (no key file)");
    std::string acks;
    {
        std::lock_guard<std::mutex> lk(t->ack_mu);
        acks = t->ack_last;
    }
    ImGui::Text("ack: %s", acks.empty() ? "-" : acks.c_str());
    if (t->rtt_ms >= 0.0) ImGui::SameLine(), ImGui::Text("(rtt=%.0fms)", t->rtt_ms);
    ImGui::Text("estop sent=%s", m.estop_sent ? "yes" : "no");
    ImGui::End();
}

} // namespace viewer
