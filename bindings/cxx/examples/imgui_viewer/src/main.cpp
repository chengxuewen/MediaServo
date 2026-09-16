// imgui_viewer — 舱端验收例子（p3-gui-viewer 本批唯一 GUI 例）。
//
// W0 骨架：SDL3/ImGui 窗口 + SDK 链接面冒烟（version 显示）。
// W3-W4 逐块长出：房间勾选（list_rooms）· 多路 tile（consume_video 纹理）·
// 遥控面板（open_control/steer/ack·RTT）· 急停态机（§4 IN）· 401/resuming 态机。
//
// 凭据纪律（G13）：口令 stdin 交互读取，永不 argv——W4 接登录面时实装。

#include <imgui.h>
#include <imgui_shell/shell.hpp>
#include <mediaservo/client.hpp>

int main() {
    imgui_shell::WindowSpec spec;
    spec.title = "MSRTC Viewer";
    imgui_shell::App app(spec);

    // SDK 链接面冒烟：版本来自 libmediaservo_client（W0 判据之一）。
    const mediaservo::Result<std::string> ver = mediaservo::client::version();

    while (app.running()) {
        if (!app.pump_events()) break;
        app.begin_frame();

        ImGui::Begin("MSRTC Viewer (skeleton)");
        if (ver.has_value()) {
            ImGui::Text("sdk: %s", ver.value().c_str());
        } else {
            ImGui::TextColored(ImVec4(1.0f, 0.4f, 0.4f, 1.0f), "sdk version unavailable: %s",
                               ver.error().message.c_str());
        }
        ImGui::SeparatorText("W3+");
        ImGui::TextDisabled("rooms / tiles / control / estop — per PLAN \xc2\xa74/\xc2\xa77");
        ImGui::End();

        app.end_frame();
    }
    return 0;
}
