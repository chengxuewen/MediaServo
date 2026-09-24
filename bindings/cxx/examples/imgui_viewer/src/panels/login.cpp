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
    ImGui::TextDisabled("ws=%s", env.ws.c_str());
    ImGui::TextDisabled("http=%s", env.http_base.c_str());
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
