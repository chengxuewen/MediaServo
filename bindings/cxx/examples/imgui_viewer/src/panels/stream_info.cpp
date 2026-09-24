// panels/stream_info — 实现。
#include "stream_info.hpp"

#include <vector>

#include <imgui.h>

#include "../dock_layout.hpp"

namespace viewer {

void render_stream_info(AppModel& m) {
    ImGui::Begin(dock::kInfo);
    if (m.tiles.empty()) {
        ImGui::TextDisabled("join a stream first");
        ImGui::End();
        return;
    }
    std::vector<const char*> names;
    for (auto& t : m.tiles) names.push_back(t->room.c_str());
    if (m.sel_info >= static_cast<int>(names.size())) m.sel_info = 0;
    ImGui::Combo("focus", &m.sel_info, names.data(), static_cast<int>(names.size()));
    Tile& t = *m.tiles[static_cast<size_t>(m.sel_info)];
    ImGui::SeparatorText(t.room.c_str());

    // 传输面（poll_stats 每 ~1s 刷新；这里只读展示）
    ImGui::Text("decoded fps     %.0f", t.fps);
    ImGui::Text("resolution      %ux%u", t.w, t.h);
    ImGui::Text("media bitrate   %.0f kbps", t.bytes_rate.kbps());
    ImGui::Text("frames cb/tex   %llu / %llu",
                static_cast<unsigned long long>(t.cb.load()),
                static_cast<unsigned long long>(t.tex.frames()));
    if (t.rtt_ms >= 0.0) ImGui::Text("control rtt     %.0f ms", t.rtt_ms);
    if (!t.err.empty()) {
        ImGui::PushStyleColor(ImGuiCol_Text, {1, .45f, .45f, 1});
        ImGui::TextWrapped("error: %s", t.err.c_str());
        ImGui::PopStyleColor();
    }
    ImGui::End();
}

} // namespace viewer
