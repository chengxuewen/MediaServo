// panels/log_view — implementation.
#include "log_view.hpp"

#include <imgui.h>

#include "../app_log.hpp"
#include "../dock_layout.hpp"

namespace viewer {

void render_log_view() {
    ImGui::Begin(dock::kLog);
    static bool follow = true;  // auto-scroll to bottom (the IDE console convention; scrolling up = the ImGui scrollbar; unchecking turns follow off)
    ImGui::Checkbox("follow", &follow);
    ImGui::SameLine();
    if (ImGui::Button("reset layout")) dock::request_reset_layout();  // clear the ini + rebuild next frame (element ⑤)
    ImGui::Separator();

    const auto lines = log().snapshot();
    ImGui::BeginChild("scroll", ImVec2(0, 0), false);
    for (const LogLine& l : lines) {
        if (l.level == "warn") ImGui::TextColored({1, .45f, .45f, 1}, "[%6.1fs] %s", l.t_s, l.msg.c_str());
        else                   ImGui::TextColored({.55f, .85f, 1, 1},  "[%6.1fs] %s", l.t_s, l.msg.c_str());
    }
    if (follow && ImGui::GetScrollY() >= ImGui::GetScrollMaxY() - 4.0f)
        ImGui::SetScrollHereY(1.0f);  // scroll only within the threshold — never yank the user back to the bottom while they read upward
    ImGui::EndChild();
    ImGui::End();
}

} // namespace viewer
