// panels/streams_tree — 实现（勾选=发 join 意图给 sessions；面板约定同 rooms 时代）。
#include "streams_tree.hpp"

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
    ImGui::Begin(dock::kTree);
    ImGui::TextDisabled("device ▸ stream");
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
        ImGui::TreeNodeEx(g.c_str(),
                          any_live ? ImGuiTreeNodeFlags_DefaultOpen : 0,
                          "%s%s", g.c_str(), any_live ? "  ●" : "");
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
            ImGui::Text("%s  [%s]%s", leaf.c_str(), row.kind.c_str(), already ? "  live" : "");
            if (m.rooms[i].checked && !already) {
                join_room(m, env, row.room_id, row.video);
                m.rooms[i].checked = false;
            }
            ImGui::PopID();
        }
        ImGui::TreePop();
    }

    if (ImGui::Button("Reconnect")) { // 换 server/重登录逃生舱（旧语义原样）
        m.logged_in = false;
        m.tiles.clear(); // close 会话（Session dtor）
    }
    ImGui::End();
}

} // namespace viewer
