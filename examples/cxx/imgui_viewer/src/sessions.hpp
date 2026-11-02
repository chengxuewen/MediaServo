// sessions — the SDK action surface (login / join / consume / control setup).
// This file **never includes imgui.h**: session-setup failures become red lines = a model write, not drawn here.
#ifndef MSRTC_VIEWER_SESSIONS_HPP
#define MSRTC_VIEWER_SESSIONS_HPP

#include <string>

#include "app_model.hpp"

namespace viewer {

/// Login + list_rooms fills the room table; when auto_join is set it also checks the target room (relocated from main's old login-button branch).
void perform_login(AppModel& m, const Env& env);
/// T4/F6: back-fills the active session's identity label into m.identity_label (no session = cleared).
void refresh_identity_label(AppModel& m);

/// Checking one room = creating one session (including the W4d dual-room pairing recursion; see the in-function note).
void join_room(AppModel& m, const Env& env, const std::string& room_id, bool video);

/// Lazily sets up the control channel (one-shot; includes the ack→RTT hook). Failures are recorded as a red tile.err line.
void ensure_control(Tile& t);

/// Close a single session (the grid ✕ / tree-row ✕): removing it from the registry destroys it — the Tile RAII wraps up
/// (the ctl pump stops first, the Session closes after), the same mechanism as Reconnect's full clear; logged into the ring.
/// Close only the given room: closing a stream room does not drag down the paired base control room (each closes independently; the pairing is visible in the tree).
void close_room(AppModel& m, const std::string& room);

} // namespace viewer

#endif // MSRTC_VIEWER_SESSIONS_HPP
