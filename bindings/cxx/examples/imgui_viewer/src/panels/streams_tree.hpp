// panels/streams_tree — 左区：设备▸流两级勾选树（T4 替代旧 Rooms 平铺页签）。
// 分组规则（W4d 发现面派生，数据无新增）：kind=video 的流房 `<base>_<stream>` 归 base；
// control/audio 房 base=房名自身。一房一流口径下"房间层"并入分组头=两级即够（F5）。
#ifndef MSRTC_VIEWER_PANELS_STREAMS_TREE_HPP
#define MSRTC_VIEWER_PANELS_STREAMS_TREE_HPP

#include "../app_model.hpp"

namespace viewer {

void render_streams_tree(AppModel& m, const Env& env);

} // namespace viewer

#endif
