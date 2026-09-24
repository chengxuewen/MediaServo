// dock_layout — 停靠画布宿主 + 首版四区切分（本例 docking 概念的主注释载体，五要素都在这）。
//
// 【docking 五要素速查】（对 imgui v1.92.9b-docking 实测，imgui_internal.h L3918-3933）
//  ① 总开关：ImGuiConfigFlags_DockingEnable（本例经 imgui_shell::ShellOptions.docking opt-in）。
//     开着它但未做布局的普通窗口也会出现停靠边框——所以按 app 开，不一刀切。
//  ② 画布：ImGui::DockSpaceOverViewport(id, ...) 每帧一调，内部自带全屏宿主窗
//     （NoTitleBar|NoMove|... 它自己处理，无需手写宿主 Begin）。传显式 id 才能配 DockBuilder。
//  ③ 程序化切区：DockBuilderAddNode(id, ImGuiDockNodeFlags_DockSpace) → 若干
//     DockBuilderSplitNode(父, 方向, 比例, &at, &opp)（方向=ImGuiDir_Left/Right/Up/Down，
//     比例给「方向那一侧」占多少）→ DockBuilderDockWindow(窗口名, 节点) → DockBuilderFinish。
//     注意 DockBuilder 家族在 **imgui_internal.h**，imgui.h 不导出——本文件因此 include 它。
//  ④ 首版守卫（防"每帧重切覆盖用户拖拽"）：本例机制 = shell 启动时探测 ini 是否已有
//     存档布局（App::had_saved_layout），只在「无存档」的那次运行 build 一次；
//     （勘误在册：imgui 的 WasActive 属 ImGuiWindow 非 DockNode——网传守卫形在本版不成立，
//     ini 探测形才是与持久化语义咬合的写法。）
//  ⑤ 持久化：布局/分栏宽度/窗口停靠态全在 ini（imgui_viewer.ini，dummy 驱动下禁写）。
//     「恢复默认布局」= ClearIniSettings() + 本帧重建（见 log 面板复位钮）。
#ifndef MSRTC_VIEWER_DOCK_LAYOUT_HPP
#define MSRTC_VIEWER_DOCK_LAYOUT_HPP

#include <imgui_shell/shell.hpp>

#include "app_model.hpp"

namespace viewer {
namespace dock {

/// 停靠窗名 = ini 持久化里的身份。**改名等于所有老用户布局失效**——动名字前先想想 ini。
inline constexpr const char* kTree   = "Streams";     // 左：设备▸流勾选树
inline constexpr const char* kGrid   = "Video Grid";  // 中：视频网格
inline constexpr const char* kInfo   = "Stream Info"; // 右上：聚焦流详情
inline constexpr const char* kControl = "Control";    // 右下：遥控/急停
inline constexpr const char* kLog    = "Log";         // 底：进程日志环

/// 每帧调用（begin_frame 之后、任何面板 Begin 之前）：首版布局（按需）+ 铺画布。
void render_root(imgui_shell::App& app);

/// 「恢复默认布局」（ClearIniSettings + 下一帧重建）。从其它面板调用。
void request_reset_layout();

} // namespace dock
} // namespace viewer

#endif
