// dock_layout — the docking canvas host + the first-run four-pane split (this example's main carrier of docking concepts; all five elements are here).
//
// [The five docking elements at a glance] (verified against imgui v1.92.9b-docking; imgui_internal.h L3918-3933)
//  ① Master switch: ImGuiConfigFlags_DockingEnable (this example opts in via imgui_shell::ShellOptions.docking).
//     Plain windows with the flag on but no layout also get docking borders — hence per-app opt-in, not a blanket switch.
//  ② Canvas: ImGui::DockSpaceOverViewport(id, ...) once per frame; it manages the fullscreen host window internally
//     (NoTitleBar|NoMove|... handled by itself; no hand-written host Begin needed). Passing an explicit id is what lets it pair with DockBuilder.
//  ③ Programmatic pane splitting: DockBuilderAddNode(id, ImGuiDockNodeFlags_DockSpace) → several
//     DockBuilderSplitNode(parent, dir, ratio, &at, &opp) calls (dir = ImGuiDir_Left/Right/Up/Down;
//     the ratio says how much the side in `dir` gets) → DockBuilderDockWindow(window name, node) → DockBuilderFinish.
//     Note: the DockBuilder family lives in **imgui_internal.h** and imgui.h does not export it — hence this file's include.
//  ④ First-run guard (prevents "re-splitting every frame from overwriting the user's drags"): this example's mechanism = the shell probes at startup whether the ini already
//     has a saved layout (App::had_saved_layout) and builds once only on the run without an archive;
//     (erratum on record: imgui's WasActive belongs to ImGuiWindow, not DockNode — the folk-guard form does not hold in this version;
//     the ini-probe form is the one that meshes with the persistence semantics.)
//  ⑤ Persistence: layout/pane widths/window dock states all live in the ini (imgui_viewer.ini; writing disabled under the dummy driver).
//     "Restore default layout" = ClearIniSettings() + rebuild this frame (see the Log panel's reset button).
#ifndef MSRTC_VIEWER_DOCK_LAYOUT_HPP
#define MSRTC_VIEWER_DOCK_LAYOUT_HPP

#include <imgui_shell/shell.hpp>

#include "app_model.hpp"

namespace viewer {
namespace dock {

/// Docking window names = identities in the ini persistence. **Renaming invalidates every existing user's layout** — think ini before touching the names.
inline constexpr const char* kTree   = "Streams";     // left: the device▸stream checkbox tree
inline constexpr const char* kGrid   = "Video Grid";  // center: the video grid
inline constexpr const char* kInfo   = "Stream Info"; // right-top: the focused stream details
inline constexpr const char* kControl = "Control";    // right-bottom: teleop / e-stop
inline constexpr const char* kLog    = "Log";         // bottom: the process log ring

/// Called every frame (after begin_frame, before any panel Begin): the first-run layout (as needed) + laying the canvas.
void render_root(imgui_shell::App& app);

/// "Restore default layout" (ClearIniSettings + rebuild next frame). Called from other panels.
void request_reset_layout();

} // namespace dock
} // namespace viewer

#endif
