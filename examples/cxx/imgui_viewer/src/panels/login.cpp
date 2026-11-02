// panels/login — implementation (the "login face" branch relocated verbatim from main.cpp in T3).
#include "login.hpp"

#include <cstring>

#include <imgui.h>

#include "../sessions.hpp"

namespace viewer {

void render_login(AppModel& m, const Env& env) {
    // A single floating window before login (not docked); position semantics identical to before the split
    ImGui::SetNextWindowPos(ImVec2(8, 40), ImGuiCond_FirstUseEver);
    ImGui::Begin("MSRTC Viewer");
    // The server address is editable before login (preset from env/defaults into the field; after login the session keeps the connect-time value)
    ImGui::SetNextItemWidth(280);
    ImGui::InputText("server ws##url", m.url_ws, sizeof(m.url_ws));
    ImGui::InputText("server http##url", m.url_http, sizeof(m.url_http));
    // The five auth modes (viewer-auth-matrix T3/F1): a horizontal Radio row + on-demand input fields.
    // Irrelevant fields are greyed, not hidden (test screenshots show "what this mode does not need"); the secrets share one show-eyes toggle.
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
        ImGui::TextDisabled("connect = discover all rooms (cross-room pull in tree)");
    } else if (m.auth == AuthMode::Device) {
        ImGui::InputText("identity dir", m.identity_dir, sizeof(m.identity_dir));
        ImGui::TextDisabled("dir must contain identity.json + etc/link/signing.pem");
    } else { // JwtPaste
        ImGui::InputTextMultiline("jwt", m.jwt_paste, sizeof(m.jwt_paste), {0, 60});
        ImGui::SameLine();
        ImGui::Checkbox("show", &m.pass_shown);
        ImGui::TextDisabled("bad/expired token -> 4013 red banner (fail-closed probe)");
    }
    // Irrelevant fields greyed out (F1: "what this mode does not need" stays visible).
    if (m.auth != AuthMode::Account) { ImGui::BeginDisabled(); ImGui::InputText("user", m.user, sizeof(m.user)); ImGui::EndDisabled(); }
    if (!m.status.empty()) ImGui::TextColored({1, .4f, .4f, 1}, "%s", m.status.c_str());
    if (!m.identity_label.empty()) ImGui::TextDisabled("identity: %s", m.identity_label.c_str());
    bool cred_ready;
    const char* btn;
    switch (m.auth) {
    case AuthMode::ApiKey: cred_ready = m.key_id[0] && m.key_secret[0]; btn = "Login"; break;
    case AuthMode::Psk:    cred_ready = m.psk[0] != '\0';               btn = "Connect"; break; // room optional: empty = the discovery tree
    case AuthMode::Device: cred_ready = m.identity_dir[0];              btn = "Connect"; break;
    case AuthMode::JwtPaste: cred_ready = m.jwt_paste[0];               btn = "Connect"; break;
    default:               cred_ready = m.pass[0] != '\0';              btn = "Login"; break;
    }
    bool do_login = ImGui::Button(btn) && cred_ready;
    if (m.auto_join) do_login = true;
    if (do_login) perform_login(m, env);   // login + listing + auto-join all live in the sessions layer
    ImGui::End();
}

} // namespace viewer
