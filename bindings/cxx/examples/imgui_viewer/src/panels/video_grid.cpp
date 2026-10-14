// panels/video_grid — the center area: the video grid (the T4b root fix: legacy Columns called NextColumn once per whole row,
// so with 2/3 columns every cell piled back into the first = the root cause of always-one-column; after switching to a hand-computed SameLine grid, dragging the pane
// to a new width recomputes automatically; 1-6 columns).
#include "video_grid.hpp"

#include <algorithm>
#include <vector>

#include <imgui.h>

#include "../dock_layout.hpp"
#include "../sessions.hpp"

namespace viewer {

namespace {
constexpr int kMinCols = 1;
constexpr int kMaxCols = 6;
} // namespace

void render_grid(AppModel& m) {
    ImGui::Begin(dock::kGrid);
    ImGui::SetNextItemWidth(100);
    // The dropdown binds "cols-1" as the index (the W4 inherited off-by-one: using the value as an index showed cols=2 as "3")
    int idx = std::max(0, std::min(kMaxCols - 1, m.cols - 1));
    if (ImGui::Combo("cols", &idx, "1\0" "2\0" "3\0" "4\0" "5\0" "6\0"))
        m.cols = idx + 1;
    m.cols = std::max(kMinCols, std::min(kMaxCols, m.cols));

    // The grid draws **video sessions only** — the control room (vehicle) auto-paired by the dual-room convention has no media stream,
    // and once occupied a cell "waiting forever" = a phantom slot (caught by a user's desk test on 09-24). The pairing's existence is shown in the window-bottom note line.
    std::vector<Tile*> vids;
    for (auto& t : m.tiles)
        if (t->video) vids.push_back(t.get());
    if (vids.empty()) {
        ImGui::TextDisabled("no video streams - tick in Streams tree, then Pull selected");
        ImGui::End();
        return;
    }

    // The hand-computed SameLine grid: cell width = (available width - gap) / columns; height capped at 16:9 with the texture fit proportionally.
    const float spacing = ImGui::GetStyle().ItemSpacing.x;
    const int ncol = m.cols;
    const float cell_w =
        (ImGui::GetContentRegionAvail().x - spacing * (ncol - 1)) / static_cast<float>(ncol);

    std::string to_close;  // at most one close per frame: record inside the loop, erase outside (prevents dangling mid-iteration)
    for (size_t i = 0; i < vids.size(); ++i) {
        if (i % static_cast<size_t>(ncol) != 0) ImGui::SameLine(0.0f, spacing);
        Tile* t = vids[i];
        ImGui::PushID(t->room.c_str());
        ImGui::BeginGroup();
        ImGui::PushTextWrapPos(ImGui::GetCursorPos().x + cell_w); // narrow 6-column cells: room names wrap without spilling
        ImGui::Text("%s", t->room.c_str());
        ImGui::SameLine();
        if (ImGui::SmallButton("x")) to_close = t->room;
        if (ImGui::IsItemHovered()) ImGui::SetTooltip("close this stream");
        if (t->tex.texture_id()) {
            const float ar = static_cast<float>(t->tex.width()) / static_cast<float>(t->tex.height());
            ImGui::Image(static_cast<ImTextureID>(reinterpret_cast<uintptr_t>(t->tex.texture_id())),
                         ImVec2(cell_w, cell_w / ar));
        } else {
            // Slots without a picture use a 16:9 placeholder to keep the grid aligned (otherwise mixed rows have ragged heights)
            ImGui::TextDisabled("%s", t->err.empty() ? "waiting for video..." : t->err.c_str());
            const float ph = cell_w * 9.0f / 16.0f;
            ImGui::Dummy(ImVec2(cell_w, ph - ImGui::GetFrameHeight() - ImGui::GetTextLineHeight()));
        }
        // One mini-stats line (the W3a asset)
        ImGui::Text("%.0fk fps=%.0f %ux%u cb=%llu", t->bytes_rate.kbps(), t->fps,
                    t->w, t->h, static_cast<unsigned long long>(t->cb.load()));
        ImGui::PopTextWrapPos();
        ImGui::EndGroup();
        ImGui::PopID();
    }
    if (!to_close.empty()) close_room(m, to_close);
    // The control-pairing note line (making visible "vehicle sessions exist even though vehicle was never checked" — the teleop channel comes along with the stream room automatically)
    if (const size_t npair = m.tiles.size() - vids.size(); npair > 0)
        ImGui::TextDisabled("%zu control session%s (auto-paired with stream rooms)", npair, npair > 1 ? "s" : "");
    ImGui::End();
}

} // namespace viewer
