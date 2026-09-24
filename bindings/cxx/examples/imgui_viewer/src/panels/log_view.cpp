// panels/log_view — 实现。
#include "log_view.hpp"

#include <imgui.h>

#include "../app_log.hpp"
#include "../dock_layout.hpp"

namespace viewer {

void render_log_view() {
    ImGui::Begin(dock::kLog);
    static bool follow = true;  // 自动滚底（IDE console 惯例；手动上翻= ImGui 滚动条，勾选关掉跟随）
    ImGui::Checkbox("follow", &follow);
    ImGui::SameLine();
    if (ImGui::Button("reset layout")) dock::request_reset_layout();  // 清 ini + 下帧重建（要素⑤）
    ImGui::Separator();

    const auto lines = log().snapshot();
    ImGui::BeginChild("scroll", ImVec2(0, 0), false);
    for (const LogLine& l : lines) {
        if (l.level == "warn") ImGui::TextColored({1, .45f, .45f, 1}, "[%6.1fs] %s", l.t_s, l.msg.c_str());
        else                   ImGui::TextColored({.55f, .85f, 1, 1},  "[%6.1fs] %s", l.t_s, l.msg.c_str());
    }
    if (follow && ImGui::GetScrollY() >= ImGui::GetScrollMaxY() - 4.0f)
        ImGui::SetScrollHereY(1.0f);  // 阈值内才滚——用户上翻阅读时不强拉回底部
    ImGui::EndChild();
    ImGui::End();
}

} // namespace viewer
