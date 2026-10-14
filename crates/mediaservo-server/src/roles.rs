//! G3 cockpit role authorization matrix (D-H11) -- pure-function authorization layer, no I/O, no tokio, full-matrix unit tests.
//!
//! Matrix (D-H11 final):
//! ```text
//! capability \ role        viewer   operator   admin   dispatcher
//! pull (video+vision)      YES      YES        YES     YES (any vehicle)
//! audio conference         YES      YES        YES     YES (any vehicle)
//! control (chassis/gimbal) NO       YES        YES     NO
//! emergency stop           NO       YES*       YES     NO       * strong audit
//! config push              NO       NO         YES     NO
//! status/alarms            NO       YES        YES     YES
//! ```
//! Vehicle x cockpit allowlist: viewer/operator only for allowlisted vehicles (accounts.yaml vehicles); admin/dispatcher any vehicle.
//! Vehicle side (device session): produce its own streams / receive config / receive control forwarding -- gated by session identity.

/// Cockpit account roles (D-H11 four levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CockpitRole {
    Viewer,
    Operator,
    Admin,
    Dispatcher,
}

impl CockpitRole {
    /// Parse a role claim / accounts.yaml role string; unknown role -> None (reject).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(Self::Viewer),
            "operator" => Some(Self::Operator),
            "admin" => Some(Self::Admin),
            "dispatcher" => Some(Self::Dispatcher),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::Admin => "admin",
            Self::Dispatcher => "dispatcher",
        }
    }

    /// Pull streams (video+vision+audio): admin/dispatcher any vehicle; viewer/operator only allowlisted vehicles.
    pub fn can_pull(&self, vehicle: &str, allowed: &[String]) -> bool {
        match self {
            Self::Admin | Self::Dispatcher => true,
            Self::Viewer | Self::Operator => allowed.iter().any(|v| v == vehicle),
        }
    }

    /// Control (chassis/gimbal; P2P DC negotiation authorization).
    pub fn can_control(&self) -> bool {
        matches!(self, Self::Operator | Self::Admin)
    }

    /// Emergency stop (operator+ with strong audit).
    pub fn can_emergency(&self) -> bool {
        matches!(self, Self::Operator | Self::Admin)
    }

    /// Config push (admin only).
    pub fn can_config(&self) -> bool {
        matches!(self, Self::Admin)
    }

    /// Status/alarm viewing.
    pub fn can_status(&self) -> bool {
        matches!(self, Self::Operator | Self::Admin | Self::Dispatcher)
    }

    /// Audio conference rooms = same semantics as the pull matrix (same allowlist).
    pub fn can_audio(&self, vehicle: &str, allowed: &[String]) -> bool {
        self.can_pull(vehicle, allowed)
    }
}

/// Cockpit account identity (claims parsed from the token issued at login).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub username: String,
    pub role: CockpitRole,
    /// Vehicle x cockpit allowlist of device_ids (effective for viewer/operator; ignored for admin/dispatcher).
    pub vehicles: Vec<String>,
}

impl AccountIdentity {
    pub fn can_access_vehicle(&self, vehicle: &str) -> bool {
        self.role.can_pull(vehicle, &self.vehicles)
    }
}

/// Session identity (G3 connection-level) -- the sole input to authorization decisions.
///
/// G3 additive principle: only Account (JWT with a valid role) and Device (device-auth) sessions
/// enable matrix enforcement; Legacy (PSK/role-less JWT) keeps existing behavior (dev/intranet path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIdentity {
    /// Vehicle device (bound after device auth, D-H11 connection-level identity).
    Device(String),
    /// Cockpit account (JWT role claim valid).
    Account(AccountIdentity),
    /// PSK/no management identity -- not constrained by the matrix.
    Legacy,
}

impl SessionIdentity {
    pub fn is_account(&self) -> bool {
        matches!(self, Self::Account(_))
    }

    /// Auditable (username, role) readable string (account session).
    pub fn who(&self) -> String {
        match self {
            Self::Account(a) => format!("{}[{}]", a.username, a.role.as_str()),
            Self::Device(d) => format!("device:{d}"),
            Self::Legacy => "legacy-psk".into(),
        }
    }

    /// RoomJoin gate (D-H11 tenant isolation + authorization matrix; the authorization point for P2P negotiation).
    /// `room_owner` = the owning vehicle device_id of the room (recorded when a device session joins).
    /// Returns the denial reason (None = allowed).
    pub fn join_vehicle_room(&self, room_owner: Option<&str>) -> Option<String> {
        match self {
            Self::Device(d) => match room_owner {
                None => None,
                Some(owner) if owner == d => None,
                Some(owner) => {
                    Some(format!("device {d} cannot join room owned by another vehicle {owner}"))
                }
            },
            Self::Account(a) => match room_owner {
                // Room unowned (vehicle offline): joining allowed (media access still gated by consume + DeviceStream frame filtering).
                None => None,
                Some(owner) if a.can_access_vehicle(owner) => None,
                Some(owner) => Some(format!(
                    "account {} role {} has no access to vehicle {owner}",
                    a.username,
                    a.role.as_str()
                )),
            },
            Self::Legacy => None,
        }
    }

    /// Account sessions cannot join with the Host role (Host = the vehicle slot; prevents an account hijacking the room and blocking the vehicle from coming online).
    pub fn host_join_denied(&self) -> Option<&'static str> {
        match self {
            Self::Account(_) => Some("cockpit accounts cannot join as host role"),
            Self::Device(_) | Self::Legacy => None,
        }
    }

    /// Control capability (I1 review -- control-column enforcement on the P2P path): the deciding basis for Remote-role joins and
    /// P2P room SDP/ICE relaying. operator/admin have control; viewer/dispatcher do not;
    /// devices (control receivers) and legacy (no management identity) are allowed.
    pub fn can_control(&self) -> bool {
        match self {
            Self::Account(a) => a.role.can_control(),
            Self::Device(_) | Self::Legacy => true,
        }
    }

    /// Produce gate: vehicles auto-allowed (their own streams); accounts denied by default (cockpits only consume) --
    /// **audio-* conference-room exemption** (D-H11 revision/P1: conference mic uplink, C29 full-interconnect semantics);
    /// Legacy allowed (dev path).
    pub fn can_produce(&self, room_id: &str) -> Result<(), String> {
        match self {
            Self::Account(a) => {
                if crate::sfu::is_audio_room(room_id) {
                    Ok(())
                } else {
                    Err(format!(
                        "account {} role {} cannot produce media (cockpit is consume-only)",
                        a.username,
                        a.role.as_str()
                    ))
                }
            }
            Self::Device(_) | Self::Legacy => Ok(()),
        }
    }

    /// Consume gate (RoomJoin already filtered by room owner vehicle -- this is defense-in-depth per producer):
    /// accounts may only consume producers of vehicles they are authorized for; devices/legacy allowed.
    pub fn can_consume(&self, producer_owner: Option<&str>) -> Result<(), String> {
        match self {
            Self::Account(a) => match producer_owner {
                // Non-device producer (legacy session/unknown origin) -- no owner to check against, allowed (compat with existing deployments).
                None => Ok(()),
                Some(v) if a.can_access_vehicle(v) => Ok(()),
                Some(v) => Err(format!(
                    "account {} role {} has no access to vehicle {v}",
                    a.username,
                    a.role.as_str()
                )),
            },
            Self::Device(_) | Self::Legacy => Ok(()),
        }
    }

    /// Emergency-stop gate: operator/admin + access to that vehicle (D-H11 strong audit -- see audit.rs).
    pub fn can_emergency(&self, room_owner: Option<&str>) -> Result<(), String> {
        match self {
            Self::Account(a) if a.role.can_emergency() => match room_owner {
                Some(v) if a.can_access_vehicle(v) => Ok(()),
                Some(v) => Err(format!(
                    "account {} role {} has no access to vehicle {v}",
                    a.username,
                    a.role.as_str()
                )),
                None => Err("vehicle room has no owner (vehicle offline)".into()),
            },
            Self::Account(a) => Err(format!(
                "account {} role {} cannot send emergency (operator/admin only)",
                a.username,
                a.role.as_str()
            )),
            Self::Device(d) => Err(format!("device {d} cannot send emergency (receiver side)")),
            Self::Legacy => Ok(()), // dev PSK path
        }
    }

    /// Config-push gate (REST / admin API; inbound WS ConfigPush always denied -- server is one-way).
    pub fn can_config(&self) -> bool {
        matches!(self, Self::Account(a) if a.role.can_config())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn account(role: CockpitRole, vehicles: Vec<String>) -> SessionIdentity {
        SessionIdentity::Account(AccountIdentity { username: "u".into(), role, vehicles })
    }

    /// Full D-H11 matrix combination (table-driven): capability x role x vehicle authorization.
    #[test]
    fn matrix_pull_by_role_and_allowlist() {
        // (role, allowlist, target vehicle, expected)
        let cases: &[(CockpitRole, &[&str], &str, bool)] = &[
            (CockpitRole::Viewer, &["ms-car1"], "ms-car1", true),
            (CockpitRole::Viewer, &["ms-car1"], "ms-car2", false),
            (CockpitRole::Viewer, &[], "ms-car1", false), // empty allowlist = nothing to watch
            (CockpitRole::Operator, &["ms-car1"], "ms-car1", true),
            (CockpitRole::Operator, &["ms-car1"], "ms-car2", false),
            (CockpitRole::Operator, &[], "ms-car1", false),
            (CockpitRole::Admin, &[], "ms-car9", true), // any vehicle
            (CockpitRole::Dispatcher, &[], "ms-car9", true), // any vehicle
            (CockpitRole::Dispatcher, &["ms-car1"], "ms-car9", true), // allowlist not applied
        ];
        for (role, list, target, want) in cases {
            assert_eq!(
                role.can_pull(target, &allowed(list)),
                *want,
                "role={role:?} allowed={list:?} target={target}"
            );
            // audio = same matrix as pull
            assert_eq!(role.can_audio(target, &allowed(list)), *want);
        }
    }

    #[test]
    fn matrix_boolean_capabilities_by_role() {
        let cases: &[(CockpitRole, bool, bool, bool, bool)] = &[
            // (role, control, emergency, config, status)
            (CockpitRole::Viewer, false, false, false, false),
            (CockpitRole::Operator, true, true, false, true),
            (CockpitRole::Admin, true, true, true, true),
            (CockpitRole::Dispatcher, false, false, false, true),
        ];
        for (role, ctrl, emg, cfg, st) in cases {
            assert_eq!(role.can_control(), *ctrl, "{role:?} control");
            assert_eq!(role.can_emergency(), *emg, "{role:?} emergency");
            assert_eq!(role.can_config(), *cfg, "{role:?} config");
            assert_eq!(role.can_status(), *st, "{role:?} status");
        }
    }

    #[test]
    fn parse_role_accepts_four_and_rejects_unknown() {
        for (s, want) in [
            ("viewer", CockpitRole::Viewer),
            ("operator", CockpitRole::Operator),
            ("admin", CockpitRole::Admin),
            ("dispatcher", CockpitRole::Dispatcher),
        ] {
            assert_eq!(CockpitRole::parse(s), Some(want), "{s}");
            assert_eq!(want.as_str(), s);
        }
        for bad in ["", "superuser", "ADMIN", "viewer ", "null"] {
            assert_eq!(CockpitRole::parse(bad), None, "{bad:?} must be rejected");
        }
    }

    // -- SessionIdentity gates --------------------------------------------------------------

    #[test]
    fn join_gate_account_allowlist() {
        let op_car1 = account(CockpitRole::Operator, allowed(&["ms-car1"]));
        assert_eq!(op_car1.join_vehicle_room(Some("ms-car1")), None, "allowlisted vehicle allowed");
        assert!(
            op_car1.join_vehicle_room(Some("ms-car2")).is_some(),
            "non-allowlisted vehicle denied (tenant isolation)"
        );
        assert_eq!(op_car1.join_vehicle_room(None), None, "unowned room allowed (vehicle offline)");

        let viewer = account(CockpitRole::Viewer, allowed(&["ms-car1"]));
        assert_eq!(viewer.join_vehicle_room(Some("ms-car1")), None, "viewer can pull");
        assert!(viewer.join_vehicle_room(Some("ms-car2")).is_some());

        let admin = account(CockpitRole::Admin, vec![]);
        assert_eq!(admin.join_vehicle_room(Some("ms-car9")), None, "admin any vehicle");

        let disp = account(CockpitRole::Dispatcher, vec![]);
        assert_eq!(disp.join_vehicle_room(Some("ms-car9")), None, "dispatcher any vehicle");
    }

    #[test]
    fn join_gate_device_tenant_isolation() {
        let veh_a = SessionIdentity::Device("ms-car-a".into());
        assert_eq!(veh_a.join_vehicle_room(None), None, "vehicle joins its own empty room");
        assert_eq!(veh_a.join_vehicle_room(Some("ms-car-a")), None, "vehicle joins its own room");
        assert!(
            veh_a.join_vehicle_room(Some("ms-car-b")).is_some(),
            "vehicle A cannot join vehicle B's room (tenant isolation)"
        );
        assert_eq!(SessionIdentity::Legacy.join_vehicle_room(Some("ms-car-b")), None);
    }

    #[test]
    fn host_join_denied_for_accounts_only() {
        assert!(account(CockpitRole::Viewer, vec![]).host_join_denied().is_some());
        assert!(account(CockpitRole::Admin, vec![]).host_join_denied().is_some());
        assert_eq!(SessionIdentity::Device("d".into()).host_join_denied(), None);
        assert_eq!(SessionIdentity::Legacy.host_join_denied(), None);
    }

    #[test]
    fn control_capability_matrix() {
        // I1 review: P2P path control gate -- matrix + identity
        assert!(!account(CockpitRole::Viewer, vec![]).can_control());
        assert!(!account(CockpitRole::Dispatcher, vec![]).can_control());
        assert!(account(CockpitRole::Operator, vec![]).can_control());
        assert!(account(CockpitRole::Admin, vec![]).can_control());
        assert!(
            SessionIdentity::Device("d".into()).can_control(),
            "device is the control receiver"
        );
        assert!(SessionIdentity::Legacy.can_control(), "legacy is not constrained by the matrix");
    }

    #[test]
    fn produce_gate_device_and_legacy_allowed_account_denied() {
        assert_eq!(
            SessionIdentity::Device("d".into()).can_produce("vehicle_x"),
            Ok(()),
            "vehicle-side produce auto-allowed"
        );
        assert_eq!(SessionIdentity::Legacy.can_produce("vehicle_x"), Ok(()), "dev path allowed");
        assert!(
            account(CockpitRole::Viewer, vec![]).can_produce("vehicle_x").is_err(),
            "accounts denied produce (video room)"
        );
        assert!(account(CockpitRole::Admin, vec![]).can_produce("vehicle_x").is_err());
    }

    #[test]
    fn produce_gate_audio_room_account_exempt() {
        // P1/D-H11 revision: audio-* conference-room account uplink exemption (C29 mic semantics); video rooms and near-prefix rooms still denied.
        assert_eq!(
            account(CockpitRole::Operator, vec![]).can_produce("audio-car1"),
            Ok(()),
            "audio conference room account uplink exempt"
        );
        assert_eq!(account(CockpitRole::Viewer, vec![]).can_produce("audio-car1"), Ok(()));
        assert!(
            account(CockpitRole::Operator, vec![]).can_produce("vehicle_car1").is_err(),
            "video room account still denied"
        );
        assert!(
            account(CockpitRole::Operator, vec![]).can_produce("audiox").is_err(),
            "audio prefix must be audio- (near prefix not exempt)"
        );
    }

    #[test]
    fn consume_gate_account_allowlist_device_legacy_allow() {
        let op_car1 = account(CockpitRole::Operator, allowed(&["ms-car1"]));
        assert_eq!(op_car1.can_consume(Some("ms-car1")), Ok(()));
        assert!(
            op_car1.can_consume(Some("ms-car2")).is_err(),
            "non-authorized vehicle producer denied"
        );
        assert_eq!(op_car1.can_consume(None), Ok(()), "ownerless producer allowed (compat)");
        assert_eq!(SessionIdentity::Device("d".into()).can_consume(Some("ms-car2")), Ok(()));
        assert_eq!(SessionIdentity::Legacy.can_consume(Some("ms-car2")), Ok(()));
    }

    #[test]
    fn emergency_gate_matrix() {
        let op_car1 = account(CockpitRole::Operator, allowed(&["ms-car1"]));
        assert_eq!(
            op_car1.can_emergency(Some("ms-car1")),
            Ok(()),
            "operator emergency on allowlisted vehicle"
        );
        assert!(op_car1.can_emergency(Some("ms-car2")).is_err(), "non-authorized vehicle denied");
        assert!(op_car1.can_emergency(None).is_err(), "vehicle offline denied");
        let viewer = account(CockpitRole::Viewer, allowed(&["ms-car1"]));
        assert!(viewer.can_emergency(Some("ms-car1")).is_err(), "viewer has no emergency stop");
        let dispatcher = account(CockpitRole::Dispatcher, vec![]);
        assert!(
            dispatcher.can_emergency(Some("ms-car9")).is_err(),
            "dispatcher has no emergency stop"
        );
        let admin = account(CockpitRole::Admin, vec![]);
        assert_eq!(admin.can_emergency(Some("ms-car9")), Ok(()), "admin emergency on any vehicle");
        assert!(
            SessionIdentity::Device("d".into()).can_emergency(Some("ms-car-d")).is_err(),
            "vehicle side is the receiver"
        );
        assert_eq!(SessionIdentity::Legacy.can_emergency(Some("ms-car1")), Ok(()), "dev path");
    }

    #[test]
    fn config_gate_admin_only() {
        assert!(account(CockpitRole::Admin, vec![]).can_config());
        assert!(!account(CockpitRole::Operator, vec![]).can_config());
        assert!(!account(CockpitRole::Dispatcher, vec![]).can_config());
        assert!(!account(CockpitRole::Viewer, vec![]).can_config());
        assert!(!SessionIdentity::Device("d".into()).can_config());
        assert!(!SessionIdentity::Legacy.can_config());
    }

    #[test]
    fn who_labels_identity() {
        assert_eq!(SessionIdentity::Legacy.who(), "legacy-psk");
        assert_eq!(SessionIdentity::Device("ms-x".into()).who(), "device:ms-x");
        assert_eq!(account(CockpitRole::Operator, vec![]).who(), "u[operator]");
    }
}
