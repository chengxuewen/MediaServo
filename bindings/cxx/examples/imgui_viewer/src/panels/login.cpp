// panels/login — 实现（T3 自 main.cpp「登录面」分支逐字迁居）。
#include "login.hpp"

#include <cstring>

#include <imgui.h>

#include "../sessions.hpp"

namespace viewer {

void render_login(AppModel& m, const Env& env) {
    // 登录前浮动单窗（未 dock）；位置语义与拆分前一致
    ImGui::SetNextWindowPos(ImVec2(8, 40), ImGuiCond_FirstUseEver);
    ImGui::Begin("MSRTC Viewer");
    // server 地址登录前可改（env/缺省值预置进字段；登录后会话沿用建联时的值）
    ImGui::SetNextItemWidth(280);
    ImGui::InputText("server ws##url", m.url_ws, sizeof(m.url_ws));
    ImGui::InputText("server http##url", m.url_http, sizeof(m.url_http));
    ImGui::InputText("user", m.user, sizeof(m.user));
    ImGui::InputText("password", m.pass, sizeof(m.pass),
                     m.pass_shown ? ImGuiInputTextFlags_None
                                  : ImGuiInputTextFlags_Password);
    ImGui::SameLine();
    ImGui::Checkbox("show", &m.pass_shown);
    if (!m.status.empty()) ImGui::TextColored({1, .4f, .4f, 1}, "%s", m.status.c_str());
    bool do_login = ImGui::Button("Login") && m.pass[0] != '\0';
    if (m.auto_join) do_login = true;
    if (do_login) perform_login(m, env);   // 登录+列举+auto-join 全在 sessions 层
    ImGui::End();
}

} // namespace viewer
