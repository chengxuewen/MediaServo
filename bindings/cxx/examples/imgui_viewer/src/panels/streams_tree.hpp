// panels/streams_tree — the left area: the device▸stream two-level checkbox tree (T4 replaced the old flat Rooms tab).
// Grouping rules (derived from the W4d discovery face; no new data): a kind=video stream room `<base>_<stream>` belongs to base;
// for control/audio rooms base = the room name itself. Under the one-room-one-stream model the "room layer" merges into the group header = two levels suffice (F5).
#ifndef MSRTC_VIEWER_PANELS_STREAMS_TREE_HPP
#define MSRTC_VIEWER_PANELS_STREAMS_TREE_HPP

#include "../app_model.hpp"

namespace viewer {

void render_streams_tree(AppModel& m, const Env& env);

} // namespace viewer

#endif
