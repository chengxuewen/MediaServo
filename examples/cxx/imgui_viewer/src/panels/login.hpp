// panels/login — the login face (the only panel main renders while logged out).
// Panel convention (the template page): stateless render functions; inputs write into AppModel, actions call sessions — never import the SDK.
#ifndef MSRTC_VIEWER_PANELS_LOGIN_HPP
#define MSRTC_VIEWER_PANELS_LOGIN_HPP

#include "../app_model.hpp"

namespace viewer {

void render_login(AppModel& m, const Env& env);

} // namespace viewer

#endif // MSRTC_VIEWER_PANELS_LOGIN_HPP
