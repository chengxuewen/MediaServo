// panels/rooms — 实现（勾选=发 join 意图给 sessions；already 态灰显防重复会话）。
#include "rooms.hpp"

#include <imgui.h>

#include "../sessions.hpp"

namespace viewer {

void render_rooms_body(AppModel& m, const Env& env) {
    for (size_t i = 0; i < m.rooms.size(); ++i) {
        bool already = false;
        for (auto& t : m.tiles)
            if (t->room == m.rooms[i].room_id) already = true;
        ImGui::PushID(static_cast<int>(i));
        ImGui::BeginDisabled(already);
        ImGui::Checkbox("##ck", &m.rooms[i].checked);
        ImGui::EndDisabled();
        ImGui::SameLine();
        ImGui::Text("%s  [%s]", m.rooms[i].room_id.c_str(), m.rooms[i].kind.c_str());
        if (m.rooms[i].checked && !already) {
            join_room(m, env, m.rooms[i].room_id, m.rooms[i].video);
            m.rooms[i].checked = false;
        }
        ImGui::PopID();
    }
    if (ImGui::Button("Reconnect")) { // 换 server/重登录逃生舱
        m.logged_in = false;
        m.tiles.clear(); // close 会话（Session dtor）
    }
}

} // namespace viewer
