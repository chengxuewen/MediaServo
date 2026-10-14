// panels/streams_tree — implementation (two-phase select→dispatch: checking/clicking a row = selection intent + highlight,
// the "Pull selected" button does the joins — a 09-24 user ruling; check-means-pull was the old tab-era semantics).
#include "streams_tree.hpp"

#include <cstdio>  // snprintf (the button's counted label)
#include <map>
#include <vector>

#include <imgui.h>

#include "../dock_layout.hpp"
#include "../sessions.hpp"

namespace viewer {

namespace {

/// Grouping key (room-grouping-semantics T3): the wire parent takes priority (a new server provides ownership directly);
/// the old wire falls back to the `_` prefix guess; a control whole-vehicle room / audio room is itself the group header.
std::string group_of(const RoomRow& r) {
    if (!r.parent.empty()) return r.parent;
    if (r.kind == "video") {
        const auto sep = r.room_id.rfind('_');
        if (sep != std::string::npos) return r.room_id.substr(0, sep);
    }
    return r.room_id;
}

} // namespace

void render_streams_tree(AppModel& m, const Env& env) {
    // Whether the row already has a session (duplicate-join protection; an already row cannot be selected again)
    auto is_live = [&m](const std::string& room) {
        for (auto& t : m.tiles)
            if (t->room == room) return true;
        return false;
    };

    ImGui::Begin(dock::kTree);
    int n_sel = 0;
    for (const auto& r : m.rooms)
        if (r.checked && !is_live(r.room_id)) ++n_sel;

    // -- Toolbar (pinned above the tree; survives scrolling): the two-phase select → dispatch --
    {
        char label[64];
        std::snprintf(label, sizeof(label), "Pull selected (%d) â¶", n_sel);
        ImGui::BeginDisabled(n_sel == 0);
        if (ImGui::Button(label, ImVec2(-1.0f, 0.0f))) {
            for (size_t i = 0; i < m.rooms.size(); ++i) {
                if (!m.rooms[i].checked) continue;
                if (!is_live(m.rooms[i].room_id))
                    join_room(m, env, m.rooms[i].room_id, m.rooms[i].video);
                m.rooms[i].checked = false;  // dispatching clears the selection (live rows turn grey)
            }
        }
        ImGui::EndDisabled();
        ImGui::SameLine();
        ImGui::TextDisabled("%d selected", n_sel);
    }
    ImGui::Separator();
    if (m.rooms.empty()) ImGui::TextDisabled("(no rooms listed)");

    // Order-stable grouping: first-appearance order (the UI does not jitter while the server's return order is stable)
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
        // TreeNodeEx pushes the ID stack only while expanded — when collapsed, do not render children and do not TreePop
        // (an unconditional Pop = an IDStack underflow assertion the moment it collapses; proven by a user's hands-on test on 09-24)
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
            // The stream name = the room id minus the base prefix (under `vehicle_test`, `test` is obvious; width-friendly)
            std::string leaf = row.room_id.size() > g.size() + 1 && row.room_id[g.size()] == '_'
                                   ? row.room_id.substr(g.size() + 1)
                                   : row.room_id;
            // Selected = yellow highlight + the whole row text clickable (a big hit area, fixing "the checkbox is not obvious"); live rows get a grey suffix and cannot be selected
            const bool sel = m.rooms[i].checked && !already;
            if (sel) ImGui::PushStyleColor(ImGuiCol_Text, {1, .9f, .4f, 1});
            ImGui::Text("%s  [%s]%s", leaf.c_str(), row.kind.c_str(), already ? "  live" : "");
            if (sel) ImGui::PopStyleColor();
            if (!already && ImGui::IsItemClicked()) m.rooms[i].checked = !m.rooms[i].checked;
            if (already) {  // a playing row → ✕ closes that session (a control room can be closed independently here)
                ImGui::SameLine();
                if (ImGui::SmallButton("x")) close_room(m, row.room_id);
                if (ImGui::IsItemHovered()) ImGui::SetTooltip("close this session");
            }
            ImGui::PopID();
        }
        ImGui::TreePop();
        }  // if(expanded) end
    }

    if (ImGui::Button("Reconnect")) { // the escape hatch for switching servers / re-login (the old semantics, as-is)
        m.logged_in = false;
        m.tiles.clear(); // close the sessions (the Session dtor)
    }
    ImGui::End();
}

} // namespace viewer
