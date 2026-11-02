// panels/control — the Control pane body: tile selection + steer/e-stop/ack·RTT (relocated in T3).
// Channel setup (open_control/on_ack registration) is a session action, now owned by sessions::ensure_control.
#ifndef MSRTC_VIEWER_PANELS_CONTROL_HPP
#define MSRTC_VIEWER_PANELS_CONTROL_HPP

#include "../app_model.hpp"

namespace viewer {

void render_control(AppModel& m);

} // namespace viewer

#endif
