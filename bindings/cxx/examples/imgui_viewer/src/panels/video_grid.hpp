// panels/video_grid — the Tiles pane body: column-count selection + the tile grid + the mini-stats line (relocated verbatim in T3).
// Note: the Columns loop structure is as-relocated — layout polish belongs to T4 (after docking, the whole window is the grid host).
#ifndef MSRTC_VIEWER_PANELS_VIDEO_GRID_HPP
#define MSRTC_VIEWER_PANELS_VIDEO_GRID_HPP

#include "../app_model.hpp"

namespace viewer {

void render_grid(AppModel& m);

} // namespace viewer

#endif
