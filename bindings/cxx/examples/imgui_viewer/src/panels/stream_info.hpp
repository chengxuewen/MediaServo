// panels/stream_info — the right-top area: the focused stream's details (a magnified mini-stats + the full pairing/error picture).
// All data comes from the model (Tile's precomputed fps/kbps/w/h/cb/tex/err/rtt) — zero new computation in this panel.
#ifndef MSRTC_VIEWER_PANELS_STREAM_INFO_HPP
#define MSRTC_VIEWER_PANELS_STREAM_INFO_HPP

#include "../app_model.hpp"

namespace viewer {

void render_stream_info(AppModel& m);

} // namespace viewer

#endif
