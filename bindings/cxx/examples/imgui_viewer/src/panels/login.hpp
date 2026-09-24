// panels/login — 登录面（未登录时 main 渲染的唯一面板）。
// 面板约定（模板页）：无状态渲染函数；输入写进 AppModel，动作调 sessions——不 import SDK。
#ifndef MSRTC_VIEWER_PANELS_LOGIN_HPP
#define MSRTC_VIEWER_PANELS_LOGIN_HPP

#include "../app_model.hpp"

namespace viewer {

void render_login(AppModel& m, const Env& env);

} // namespace viewer

#endif // MSRTC_VIEWER_PANELS_LOGIN_HPP
