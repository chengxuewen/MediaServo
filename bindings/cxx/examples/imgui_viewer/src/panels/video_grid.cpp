// panels/video_grid — 实现（逐字迁居自 main.cpp Tiles 页签体，含旧行为怪癖不顺手改）。
#include "video_grid.hpp"

#include <algorithm>

#include <imgui.h>

namespace viewer {

void render_grid_body(AppModel& m) {
    ImGui::SameLine(ImGui::GetWindowWidth() - 220);
    int old_cols = m.cols;
    ImGui::SetNextItemWidth(100);
    ImGui::Combo("cols", &m.cols, "1\0"
            "2\0"
            "3\0");
    m.cols = std::max(1, std::min(3, m.cols));
    (void)old_cols;
    const int ncol = m.cols;
    size_t i = 0;
    for (int row = 0; row * ncol < m.tiles.size(); ++row) {
        ImGui::Columns(ncol, nullptr, false);
        for (int c = 0; c < ncol && i < m.tiles.size(); ++c, ++i) {
            Tile* t = m.tiles[i].get();
            ImGui::BeginGroup();
            ImGui::Text("%s", t->room.c_str());
            if (t->tex.texture_id()) {
                const ImVec2 avail = ImGui::GetContentRegionAvail();
                float dw = avail.x;
                const float ar = static_cast<float>(t->tex.width()) / static_cast<float>(t->tex.height());
                float dh = dw / ar;
                ImGui::Image(static_cast<ImTextureID>(reinterpret_cast<uintptr_t>(t->tex.texture_id())), ImVec2(dw, dh));
            } else {
                ImGui::TextDisabled("%s", t->err.empty() ? "waiting for video..." : t->err.c_str());
            }
            // mini-stats 一行（W3a 资产）
            ImGui::Text("%.0fk fps=%.0f %ux%u cb=%llu", t->bytes_rate.kbps(), t->fps,
                        t->w, t->h, static_cast<unsigned long long>(t->cb.load()));
            ImGui::EndGroup();
        }
        ImGui::NextColumn();
        ImGui::Columns(1);
    }
}

} // namespace viewer
