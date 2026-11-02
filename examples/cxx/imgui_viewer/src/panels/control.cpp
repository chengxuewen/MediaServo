// panels/control — implementation (UI lines relocated verbatim; the setup block became sessions calls, semantics unchanged).
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
        ImGui::End();  // an early return must still close the window (missed by the T4 patch: Begin/End balancing requires counting every return path — caught by a user's second live run on 09-24)
        return;
    }
    std::vector<const char*> names;
    for (auto& t : m.tiles) names.push_back(t->room.c_str());
    if (m.sel_control >= static_cast<int>(names.size())) m.sel_control = -1;
    ImGui::Combo("tile", &m.sel_control, names.data(), static_cast<int>(names.size()));
    if (m.sel_control < 0) {
        ImGui::End();  // a second early-return leak of the same family (found by grepping all panels) — Begin/End balancing = counting every return path per file
        return;
    }
    Tile* t = m.tiles[m.sel_control].get();
    ensure_control(*t); // one-shot setup; the failure reason lands in the red line; reconnect = the Rooms page's Reconnect (the old inline block, call-ified)
    ImGui::SeparatorText(t->room.c_str());
    if (!t->err.empty()) ImGui::TextColored({1, .4f, .4f, 1}, "%s", t->err.c_str());
    ImGui::SliderFloat("steer deg", &m.steer_deg, -90.0f, 90.0f);
    if (ImGui::Button("send steer")) {
        t->send_cmd("chassis", ++m.seq, "steer",
                    (std::string("{\"deg\":") + std::to_string(m.steer_deg) + "}").c_str());
    }
    ImGui::SameLine();
    // W4b: the signed-estop compound surface (Config.hmac_key_file decides the signing state;
    // no key configured = the migration-allow form; a vehicle with a key refusing to sign = the correct ruling).
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
