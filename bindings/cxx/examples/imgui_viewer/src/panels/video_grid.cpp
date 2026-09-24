// panels/video_grid — 中区：视频网格（T4b 根修：legacy Columns 整行只 NextColumn 一次，
// 2/3 列设置下所有格堆回第一列=永远单列的根因；改 SameLine 手算网格后，窗格拖动
// 换宽自动重算，列数 1-6）。
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
    // 下拉绑定"列数-1"做索引（W4 遗传错位：值当索引时 cols=2 显示成 "3"）
    int idx = std::max(0, std::min(kMaxCols - 1, m.cols - 1));
    if (ImGui::Combo("cols", &idx, "1\0" "2\0" "3\0" "4\0" "5\0" "6\0"))
        m.cols = idx + 1;
    m.cols = std::max(kMinCols, std::min(kMaxCols, m.cols));

    // 网格只画**视频会话**——双房配对自动并入的控制房（vehicle）无媒体流，
    // 曾占一格"永远 waiting"=假位（09-24 用户桌测揪出）。控制配对存在性见窗底说明行。
    std::vector<Tile*> vids;
    for (auto& t : m.tiles)
        if (t->video) vids.push_back(t.get());
    if (vids.empty()) {
        ImGui::TextDisabled("no video streams - tick in Streams tree, then Pull selected");
        ImGui::End();
        return;
    }

    // SameLine 手算网格：格宽=(可用宽 - 间隙)/列数，高按 16:9 上限、纹理等比 fit。
    const float spacing = ImGui::GetStyle().ItemSpacing.x;
    const int ncol = m.cols;
    const float cell_w =
        (ImGui::GetContentRegionAvail().x - spacing * (ncol - 1)) / static_cast<float>(ncol);

    std::string to_close;  // 一帧最多关一路：环内只记录，环外 erase（防迭代中悬挂）
    for (size_t i = 0; i < vids.size(); ++i) {
        if (i % static_cast<size_t>(ncol) != 0) ImGui::SameLine(0.0f, spacing);
        Tile* t = vids[i];
        ImGui::PushID(t->room.c_str());
        ImGui::BeginGroup();
        ImGui::PushTextWrapPos(ImGui::GetCursorPos().x + cell_w); // 6 列窄格：房名换行不越格
        ImGui::Text("%s", t->room.c_str());
        ImGui::SameLine();
        if (ImGui::SmallButton("x")) to_close = t->room;
        if (ImGui::IsItemHovered()) ImGui::SetTooltip("close this stream");
        if (t->tex.texture_id()) {
            const float ar = static_cast<float>(t->tex.width()) / static_cast<float>(t->tex.height());
            ImGui::Image(static_cast<ImTextureID>(reinterpret_cast<uintptr_t>(t->tex.texture_id())),
                         ImVec2(cell_w, cell_w / ar));
        } else {
            // 无画面格用 16:9 占位保网格对齐（否则混排时高低参差）
            ImGui::TextDisabled("%s", t->err.empty() ? "waiting for video..." : t->err.c_str());
            const float ph = cell_w * 9.0f / 16.0f;
            ImGui::Dummy(ImVec2(cell_w, ph - ImGui::GetFrameHeight() - ImGui::GetTextLineHeight()));
        }
        // mini-stats 一行（W3a 资产）
        ImGui::Text("%.0fk fps=%.0f %ux%u cb=%llu", t->bytes_rate.kbps(), t->fps,
                    t->w, t->h, static_cast<unsigned long long>(t->cb.load()));
        ImGui::PopTextWrapPos();
        ImGui::EndGroup();
        ImGui::PopID();
    }
    if (!to_close.empty()) close_room(m, to_close);
    // 控制配对说明行（透明化"没勾 vehicle 却有 vehicle 会话"——遥控通道随流房自动并入）
    if (const size_t npair = m.tiles.size() - vids.size(); npair > 0)
        ImGui::TextDisabled("%zu control session%s (auto-paired with stream rooms)", npair, npair > 1 ? "s" : "");
    ImGui::End();
}

} // namespace viewer
