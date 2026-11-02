// dock_layout — implementation (the five elements are in the header note; this file = a runnable textbook for the docking concepts).
#include "dock_layout.hpp"

#include <imgui.h>
#include <imgui_internal.h>  // the DockBuilder family is not exported by imgui.h (element ③) — an example project depending on the internal header is the same approach as the official demo

namespace viewer {
namespace dock {

namespace {

ImGuiID root_id() { return ImGui::GetID("msrtc_dock"); }  // a stable string = the same id across restarts (the key for element ⑤)

/// The first-run four-pane split: tree left / log bottom / right column (details top, control bottom) / center = the remaining grid.
void build_initial() {
    const ImGuiID root = root_id();
    ImGui::DockBuilderRemoveNode(root);  // on the reset path the old node exists — strip it clean before splitting (idempotent)
    ImGui::DockBuilderAddNode(root, ImGuiDockNodeFlags_DockSpace);
    // The official note at imgui_internal.h L3922: without SetNodeSize first, SplitNode ratios are imprecise — followed
    ImGui::DockBuilderSetNodeSize(root, ImGui::GetMainViewport()->WorkSize);

    ImGuiID left = 0, bottom = 0, right = 0, center = 0, info = 0, ctl = 0;
    ImGui::DockBuilderSplitNode(root,  ImGuiDir_Left,  0.20f, &left,   &center);
    ImGui::DockBuilderSplitNode(center, ImGuiDir_Down,  0.24f, &bottom, &center);
    ImGui::DockBuilderSplitNode(center, ImGuiDir_Right, 0.26f, &right,  &center);
    // center is now the middle video-grid area; the right column splits top/bottom next (the F6 ruling: tabs were the disease this round set out to cure — do not reintroduce them)
    ImGui::DockBuilderSplitNode(right, ImGuiDir_Down, 0.45f, &ctl, &info);

    ImGui::DockBuilderDockWindow(kTree, left);
    ImGui::DockBuilderDockWindow(kGrid, center);
    ImGui::DockBuilderDockWindow(kInfo, info);
    ImGui::DockBuilderDockWindow(kControl, ctl);
    ImGui::DockBuilderDockWindow(kLog, bottom);
    ImGui::DockBuilderFinish(root);
}

bool s_reset = false;

} // namespace

void request_reset_layout() { s_reset = true; }

void render_root(imgui_shell::App& app) {
    static bool s_first = true;
    if (s_first) {
        s_first = false;
        // element ④ first-run guard: never touch an ini with an archived layout (not the first run) — re-splitting every frame is the culprit that overwrites user drags.
        if (!app.had_saved_layout()) build_initial();
    } else if (s_reset) {
        s_reset = false;
        ImGui::ClearIniSettings();  // the public API: clears all window/dock archives (element ⑤'s "factory reset" form)
        build_initial();
    }
    // Element ②: lay the canvas every frame (the host window is managed inside the helper; the explicit id aligns with DockBuilder)
    ImGui::DockSpaceOverViewport(root_id(), nullptr, ImGuiDockNodeFlags_None);
}

} // namespace dock
} // namespace viewer
