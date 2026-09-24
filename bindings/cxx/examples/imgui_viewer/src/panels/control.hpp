// panels/control — Control 页签体：tile 选择 + steer/急停/ack·RTT（T3 迁居）。
// 通道建立（open_control/on_ack 注册）属会话动作，已归 sessions::ensure_control。
#ifndef MSRTC_VIEWER_PANELS_CONTROL_HPP
#define MSRTC_VIEWER_PANELS_CONTROL_HPP

#include "../app_model.hpp"

namespace viewer {

void render_control(AppModel& m);

} // namespace viewer

#endif
