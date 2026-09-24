// panels/video_grid — Tiles 页签体：列数选择 + tile 网格 + mini-stats 行（T3 逐字迁居）。
// 注意：Columns 循环结构为搬迁原样——布局精修归 T4（docking 后整窗即网格宿主）。
#ifndef MSRTC_VIEWER_PANELS_VIDEO_GRID_HPP
#define MSRTC_VIEWER_PANELS_VIDEO_GRID_HPP

#include "../app_model.hpp"

namespace viewer {

void render_grid(AppModel& m);

} // namespace viewer

#endif
