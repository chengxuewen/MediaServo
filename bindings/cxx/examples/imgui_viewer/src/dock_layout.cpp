// dock_layout — 实现（五要素见头注；本文件 = docking 概念的可运行教材）。
#include "dock_layout.hpp"

#include <imgui.h>
#include <imgui_internal.h>  // DockBuilder 家族 imgui.h 不导出（要素③）——示例工程直依内部头是官方 demo 同款做法

namespace viewer {
namespace dock {

namespace {

ImGuiID root_id() { return ImGui::GetID("msrtc_dock"); }  // 稳定字符串=跨重启同 id（要素⑤的键）

/// 首版四区切分：左树 / 底日志 / 右列(上详情 下控制) / 中=剩余网格。
void build_initial() {
    const ImGuiID root = root_id();
    ImGui::DockBuilderRemoveNode(root);  // 复位路径下旧节点存在——先摘干净再切（幂等）
    ImGui::DockBuilderAddNode(root, ImGuiDockNodeFlags_DockSpace);
    // imgui_internal.h L3922 官方注：不先 SetNodeSize 则 SplitNode 比例不精确——照做
    ImGui::DockBuilderSetNodeSize(root, ImGui::GetMainViewport()->WorkSize);

    ImGuiID left = 0, bottom = 0, right = 0, center = 0, info = 0, ctl = 0;
    ImGui::DockBuilderSplitNode(root,  ImGuiDir_Left,  0.20f, &left,   &center);
    ImGui::DockBuilderSplitNode(center, ImGuiDir_Down,  0.24f, &bottom, &center);
    ImGui::DockBuilderSplitNode(center, ImGuiDir_Right, 0.26f, &right,  &center);
    // center 至此=中间视频网格区；右列再上下分（F6 裁决：页签=本次要治的病，不再引入）
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
        // 要素④ 首版守卫：ini 里已有存档布局（非首跑）就绝不动——每帧重切=覆盖用户拖拽的元凶
        if (!app.had_saved_layout()) build_initial();
    } else if (s_reset) {
        s_reset = false;
        ImGui::ClearIniSettings();  // 公开 API：清全部窗口/停靠存档（要素⑤的"恢复出厂"形）
        build_initial();
    }
    // 要素②：每帧铺画布（宿主窗由 helper 内部代管；显式 id 与 DockBuilder 对齐）
    ImGui::DockSpaceOverViewport(root_id(), nullptr, ImGuiDockNodeFlags_None);
}

} // namespace dock
} // namespace viewer
