// panels/streams_tree — 实现（两段式选择→下发：勾选/点行=选择意图并高亮，
// 「Pull selected」按钮统一 join——09-24 用户裁决，勾选即拉流是页签时代旧语义）。
#include "streams_tree.hpp"

#include <cstdio>  // snprintf（按钮计数标签）
#include <map>
#include <vector>

#include <imgui.h>

#include "../dock_layout.hpp"
#include "../sessions.hpp"

namespace viewer {

namespace {

/// 流房 `<base>_<stream>` → base；其它形态（control 整车房 / audio-*）自身即分组头。
std::string group_of(const RoomRow& r) {
    if (r.kind == "video") {
        const auto sep = r.room_id.rfind('_');
        if (sep != std::string::npos) return r.room_id.substr(0, sep);
    }
    return r.room_id;
}

} // namespace

void render_streams_tree(AppModel& m, const Env& env) {
    // 行是否已有会话（重复 join 防护；already 行不可再选）
    auto is_live = [&m](const std::string& room) {
        for (auto& t : m.tiles)
            if (t->room == room) return true;
        return false;
    };

    ImGui::Begin(dock::kTree);
    int n_sel = 0;
    for (const auto& r : m.rooms)
        if (r.checked && !is_live(r.room_id)) ++n_sel;

    // ── 工具条（钉在树上方，滚动不消失）：选择 → 确认下发 两段式 ──
    {
        char label[64];
        std::snprintf(label, sizeof(label), "Pull selected (%d) â¶", n_sel);
        ImGui::BeginDisabled(n_sel == 0);
        if (ImGui::Button(label, ImVec2(-1.0f, 0.0f))) {
            for (size_t i = 0; i < m.rooms.size(); ++i) {
                if (!m.rooms[i].checked) continue;
                if (!is_live(m.rooms[i].room_id))
                    join_room(m, env, m.rooms[i].room_id, m.rooms[i].video);
                m.rooms[i].checked = false;  // 下发即清选（live 行转灰显）
            }
        }
        ImGui::EndDisabled();
        ImGui::SameLine();
        ImGui::TextDisabled("%d selected", n_sel);
    }
    ImGui::Separator();
    if (m.rooms.empty()) ImGui::TextDisabled("(no rooms listed)");

    // 分组保序：首现顺序（服务器返回序稳定时 UI 不抖）
    std::vector<std::string> order;
    std::map<std::string, std::vector<size_t>> groups;
    for (size_t i = 0; i < m.rooms.size(); ++i) {
        const std::string g = group_of(m.rooms[i]);
        if (!groups.count(g)) order.push_back(g);
        groups[g].push_back(i);
    }

    for (const std::string& g : order) {
        bool any_live = false;
        for (size_t i : groups[g])
            for (auto& t : m.tiles)
                if (t->room == m.rooms[i].room_id) any_live = true;
        // TreeNodeEx 只在展开时压 ID 栈——折叠则不得渲染子项也不得 TreePop
        // （无条件 Pop=折叠瞬间 IDStack 下溢断言，09-24 用户手测实锤）
        if (ImGui::TreeNodeEx(g.c_str(),
                              any_live ? ImGuiTreeNodeFlags_DefaultOpen : 0,
                              "%s%s", g.c_str(), any_live ? "  ●" : "")) {
        for (size_t i : groups[g]) {
            const RoomRow& row = m.rooms[i];
            bool already = false;
            for (auto& t : m.tiles)
                if (t->room == row.room_id) already = true;
            ImGui::PushID(static_cast<int>(i));
            ImGui::BeginDisabled(already);
            ImGui::Checkbox("##ck", &m.rooms[i].checked);
            ImGui::EndDisabled();
            ImGui::SameLine();
            // 流名=房 id 去 base 前缀（`vehicle_test` 下 `test` 一目了然，宽度友好）
            std::string leaf = row.room_id.size() > g.size() + 1 && row.room_id[g.size()] == '_'
                                   ? row.room_id.substr(g.size() + 1)
                                   : row.room_id;
            // 选中=黄色高亮 + 整行文字可点（大命中区，修"勾选不明显"）；live 行灰缀不可选
            const bool sel = m.rooms[i].checked && !already;
            if (sel) ImGui::PushStyleColor(ImGuiCol_Text, {1, .9f, .4f, 1});
            ImGui::Text("%s  [%s]%s", leaf.c_str(), row.kind.c_str(), already ? "  live" : "");
            if (sel) ImGui::PopStyleColor();
            if (!already && ImGui::IsItemClicked()) m.rooms[i].checked = !m.rooms[i].checked;
            ImGui::PopID();
        }
        ImGui::TreePop();
        }  // if(expanded) end
    }

    if (ImGui::Button("Reconnect")) { // 换 server/重登录逃生舱（旧语义原样）
        m.logged_in = false;
        m.tiles.clear(); // close 会话（Session dtor）
    }
    ImGui::End();
}

} // namespace viewer
