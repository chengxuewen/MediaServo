// panels/stream_info — 右上区：聚焦流详情（mini-stats 的放大版 + 配对/错误全量）。
// 数据全来自模型（Tile 已算好的 fps/kbps/w/h/cb/tex/err/rtt）——本面板零新计算。
#ifndef MSRTC_VIEWER_PANELS_STREAM_INFO_HPP
#define MSRTC_VIEWER_PANELS_STREAM_INFO_HPP

#include "../app_model.hpp"

namespace viewer {

void render_stream_info(AppModel& m);

} // namespace viewer

#endif
