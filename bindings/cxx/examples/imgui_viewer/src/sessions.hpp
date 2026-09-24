// sessions — SDK 动作面（login / join / consume / control 建立）。
// 本文件**永不 include imgui.h**：会话建立失败要进红行=写模型，不由这里画。
#ifndef MSRTC_VIEWER_SESSIONS_HPP
#define MSRTC_VIEWER_SESSIONS_HPP

#include <string>

#include "app_model.hpp"

namespace viewer {

/// 登录 + list_rooms 填房间表；auto_join 置位时并勾入目标房（原 main 登录按钮分支迁居）。
void perform_login(AppModel& m, const Env& env);

/// 勾选一路房间 = 建一个会话（含 W4d 双房配对递归，见函数内注）。
void join_room(AppModel& m, const Env& env, const std::string& room_id, bool video);

/// 控制通道惰性建立（一次性；含 ack→RTT 挂点）。失败记 tile.err 红行。
void ensure_control(Tile& t);

} // namespace viewer

#endif // MSRTC_VIEWER_SESSIONS_HPP
