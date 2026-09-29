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
    // 鉴权五模式（viewer-auth-matrix T3/F1）：Radio 横排 + 按需输入域。
    // 不相关域灰显不隐藏（测试截图可见"该模式不需要什么"）；密语共用 show 眼睛。
    auto radio = [&](AuthMode mode, const char* label, const char* tip) {
        if (ImGui::RadioButton(label, m.auth == mode)) m.auth = mode;
        if (ImGui::IsItemHovered()) ImGui::SetTooltip("%s", tip);
    };
    radio(AuthMode::Account,  "Account",  "user + password login (role from account)");
    radio(AuthMode::ApiKey,   "API-Key",  "issued key pair -> exchange (permission baked at issue)");
    radio(AuthMode::Psk,      "PSK",      "shared secret LAN path (skips discovery, direct room)");
    radio(AuthMode::Device,   "Device",   "Ed25519 identity dir (D283 public-key challenge)");
    radio(AuthMode::JwtPaste, "JWT",      "paste token (bad/expired token -> 4013 red banner test)");
    ImGui::Separator();
    const bool secret_shown = m.pass_shown ? ImGuiInputTextFlags_None : ImGuiInputTextFlags_Password;
    if (m.auth == AuthMode::Account) {
        ImGui::InputText("user", m.user, sizeof(m.user));
        ImGui::InputText("password", m.pass, sizeof(m.pass), secret_shown);
        ImGui::SameLine();
        ImGui::Checkbox("show", &m.pass_shown);
    } else if (m.auth == AuthMode::ApiKey) {
        ImGui::InputText("key id", m.key_id, sizeof(m.key_id));
        ImGui::InputText("key secret", m.key_secret, sizeof(m.key_secret), secret_shown);
        ImGui::SameLine();
        ImGui::Checkbox("show", &m.pass_shown);
    } else if (m.auth == AuthMode::Psk) {
        ImGui::InputText("psk", m.psk, sizeof(m.psk), secret_shown);
        ImGui::SameLine();
        ImGui::Checkbox("show", &m.pass_shown);
        ImGui::InputText("room (direct join)", m.direct_room, sizeof(m.direct_room));
        ImGui::TextDisabled("discovery skipped: legacy identity cannot list rooms (401)");
    } else if (m.auth == AuthMode::Device) {
        ImGui::InputText("identity dir", m.identity_dir, sizeof(m.identity_dir));
        ImGui::TextDisabled("dir must contain identity.json + etc/link/signing.pem");
    } else { // JwtPaste
        ImGui::InputTextMultiline("jwt", m.jwt_paste, sizeof(m.jwt_paste), {0, 60});
        ImGui::SameLine();
        ImGui::Checkbox("show", &m.pass_shown);
        ImGui::TextDisabled("bad/expired token -> 4013 red banner (fail-closed probe)");
    }
    // 无关域灰显（F1：可见"该模式不需要什么"）。
    if (m.auth != AuthMode::Account) { ImGui::BeginDisabled(); ImGui::InputText("user", m.user, sizeof(m.user)); ImGui::EndDisabled(); }
    if (!m.status.empty()) ImGui::TextColored({1, .4f, .4f, 1}, "%s", m.status.c_str());
    if (!m.identity_label.empty()) ImGui::TextDisabled("identity: %s", m.identity_label.c_str());
    bool cred_ready;
    const char* btn;
    switch (m.auth) {
    case AuthMode::ApiKey: cred_ready = m.key_id[0] && m.key_secret[0]; btn = "Login"; break;
    case AuthMode::Psk:    cred_ready = m.psk[0] && m.direct_room[0];   btn = "Connect"; break; // F2 文案
    case AuthMode::Device: cred_ready = m.identity_dir[0];              btn = "Connect"; break;
    case AuthMode::JwtPaste: cred_ready = m.jwt_paste[0];               btn = "Connect"; break;
    default:               cred_ready = m.pass[0] != '\0';              btn = "Login"; break;
    }
    bool do_login = ImGui::Button(btn) && cred_ready;
    if (m.auto_join) do_login = true;
    if (do_login) perform_login(m, env);   // 登录+列举+auto-join 全在 sessions 层
    ImGui::End();
}

} // namespace viewer
