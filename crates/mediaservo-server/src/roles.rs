//! G3 舱端角色授权矩阵（D-H11）— 纯函数授权层，无 I/O 无 tokio，全矩阵单测。
//!
//! 矩阵（D-H11 定稿）:
//! ```text
//! 能力 \ 角色        viewer   operator   admin   dispatcher
//! 拉流(视频+视觉)     ✅        ✅        ✅      ✅（任意车）
//! 音频会议           ✅        ✅        ✅      ✅（任意车）
//! 控制(底盘/云台)     ❌        ✅        ✅      ❌
//! 急停              ❌        ✅*       ✅      ❌      *强审计
//! 配置下发           ❌        ❌        ✅      ❌
//! 状态/告警          ❌        ✅        ✅      ✅
//! ```
//! 车×舱白名单: viewer/operator 仅授权车（accounts.yaml vehicles）；admin/dispatcher 任意车。
//! 车端（device 会话）: produce 自己的流 / 收配置 / 收控制转发 —— 由会话身份门控制。

/// 舱端账号角色（D-H11 四级）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CockpitRole {
    Viewer,
    Operator,
    Admin,
    Dispatcher,
}

impl CockpitRole {
    /// 解析 role claim / accounts.yaml 角色串；未知角色 → None（拒绝）。
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

    /// 拉流（视频+视觉+音频）: admin/dispatcher 任意车；viewer/operator 仅白名单车。
    pub fn can_pull(&self, vehicle: &str, allowed: &[String]) -> bool {
        match self {
            Self::Admin | Self::Dispatcher => true,
            Self::Viewer | Self::Operator => allowed.iter().any(|v| v == vehicle),
        }
    }

    /// 控制（底盘/云台; P2P DC 协商授权）。
    pub fn can_control(&self) -> bool {
        matches!(self, Self::Operator | Self::Admin)
    }

    /// 急停（operator+ 强审计）。
    pub fn can_emergency(&self) -> bool {
        matches!(self, Self::Operator | Self::Admin)
    }

    /// 配置下发（admin 专属）。
    pub fn can_config(&self) -> bool {
        matches!(self, Self::Admin)
    }

    /// 状态/告警查看。
    pub fn can_status(&self) -> bool {
        matches!(self, Self::Operator | Self::Admin | Self::Dispatcher)
    }

    /// 音频会议房间 = 拉流矩阵同语义（同白名单）。
    pub fn can_audio(&self, vehicle: &str, allowed: &[String]) -> bool {
        self.can_pull(vehicle, allowed)
    }
}

/// 舱端账号身份（登录签发 token 的 claims 解析结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub username: String,
    pub role: CockpitRole,
    /// 车×舱白名单 device_id（viewer/operator 生效；admin/dispatcher 忽略）。
    pub vehicles: Vec<String>,
}

impl AccountIdentity {
    pub fn can_access_vehicle(&self, vehicle: &str) -> bool {
        self.role.can_pull(vehicle, &self.vehicles)
    }
}

/// 会话身份（G3 连接级）— 授权决策的唯一输入。
///
/// G3 additive 原则: 仅 Account（JWT 带合法 role）与 Device（设备认证）会话
/// 启用矩阵强制；Legacy（PSK/无角色 JWT）保持既有行为（开发/内网路径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIdentity {
    /// 车端设备（device auth 成功绑定，D-H11 连接级身份）。
    Device(String),
    /// 舱端账号（JWT role claim 合法）。
    Account(AccountIdentity),
    /// PSK/无管理身份 — 不加矩阵限制。
    Legacy,
}

impl SessionIdentity {
    pub fn is_account(&self) -> bool {
        matches!(self, Self::Account(_))
    }

    /// 供审计的 (username, role) 可读串（账号会话）。
    pub fn who(&self) -> String {
        match self {
            Self::Account(a) => format!("{}[{}]", a.username, a.role.as_str()),
            Self::Device(d) => format!("device:{d}"),
            Self::Legacy => "legacy-psk".into(),
        }
    }

    /// RoomJoin 门（D-H11 租户隔离 + 授权矩阵; P2P 协商的授权点）。
    /// `room_owner` = 该房间所属车端 device_id（device 会话加入时记录）。
    /// 返回拒绝理由（None = 允许）。
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
                // 房间无主（车未上线）: 允许加入（媒体访问仍受 consume 门控 + DeviceStream 帧过滤）。
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

    /// 账号会话禁止以 Host 角色入房（Host = 车端位，防账号抢占房间使车无法上线）。
    pub fn host_join_denied(&self) -> Option<&'static str> {
        match self {
            Self::Account(_) => Some("cockpit accounts cannot join as host role"),
            Self::Device(_) | Self::Legacy => None,
        }
    }

    /// 控制能力（I1 review — P2P 路径控制列强制）: Remote 角色 join 与 P2P 房间
    /// SDP/ICE 中继的裁决依据。operator/admin 有控制; viewer/dispatcher 无;
    /// 设备（控制接收方）与 legacy（无管理身份）放行。
    pub fn can_control(&self) -> bool {
        match self {
            Self::Account(a) => a.role.can_control(),
            Self::Device(_) | Self::Legacy => true,
        }
    }

    /// Produce 门: 车端自动允许（自己的流）; 账号默认禁止（舱端只消费）——
    /// **audio-* 会议房豁免**（D-H11 修订/P1: 会议麦上行，C29 全互连语义）;
    /// Legacy 允许（dev 路径）。
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

    /// Consume 门（RoomJoin 已按房间主车过滤 — 此处为对具体 producer 的纵深防御）:
    /// 账号只能 consume 自己有权车的 producer；设备/legacy 放行。
    pub fn can_consume(&self, producer_owner: Option<&str>) -> Result<(), String> {
        match self {
            Self::Account(a) => match producer_owner {
                // 非设备 producer（legacy 会话/未知来源）— 无主可校验，放行（兼容既有部署）。
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

    /// 急停门: operator/admin + 对该车的访问权（D-H11 急停强审计 — 见 audit.rs）。
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
            Self::Legacy => Ok(()), // dev PSK 路径
        }
    }

    /// 配置下发门（REST / admin API 用; WS 入站 ConfigPush 一律拒绝 — server 单向）。
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
        SessionIdentity::Account(AccountIdentity {
            username: "u".into(),
            role,
            vehicles,
        })
    }

    /// D-H11 矩阵全组合（表驱动）: 能力 × 角色 × 车授权。
    #[test]
    fn matrix_pull_by_role_and_allowlist() {
        // (角色, 白名单, 目标车, 期望)
        let cases: &[(CockpitRole, &[&str], &str, bool)] = &[
            (CockpitRole::Viewer, &["ms-car1"], "ms-car1", true),
            (CockpitRole::Viewer, &["ms-car1"], "ms-car2", false),
            (CockpitRole::Viewer, &[], "ms-car1", false), // 空白名单 = 无车可看
            (CockpitRole::Operator, &["ms-car1"], "ms-car1", true),
            (CockpitRole::Operator, &["ms-car1"], "ms-car2", false),
            (CockpitRole::Operator, &[], "ms-car1", false),
            (CockpitRole::Admin, &[], "ms-car9", true), // 任意车
            (CockpitRole::Dispatcher, &[], "ms-car9", true), // 任意车
            (CockpitRole::Dispatcher, &["ms-car1"], "ms-car9", true), // 白名单不生效
        ];
        for (role, list, target, want) in cases {
            assert_eq!(
                role.can_pull(target, &allowed(list)),
                *want,
                "role={role:?} allowed={list:?} target={target}"
            );
            // 音频 = 拉流同矩阵
            assert_eq!(role.can_audio(target, &allowed(list)), *want);
        }
    }

    #[test]
    fn matrix_boolean_capabilities_by_role() {
        let cases: &[(CockpitRole, bool, bool, bool, bool)] = &[
            // (角色, control, emergency, config, status)
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
            assert_eq!(CockpitRole::parse(bad), None, "{bad:?} 必须拒绝");
        }
    }

    // ── SessionIdentity 门 ──────────────────────────────────────────────────

    #[test]
    fn join_gate_account_allowlist() {
        let op_car1 = account(CockpitRole::Operator, allowed(&["ms-car1"]));
        assert_eq!(op_car1.join_vehicle_room(Some("ms-car1")), None, "授权车放行");
        assert!(
            op_car1.join_vehicle_room(Some("ms-car2")).is_some(),
            "非白名单车拒绝（租户隔离）"
        );
        assert_eq!(op_car1.join_vehicle_room(None), None, "房间无主放行（车未上线）");

        let viewer = account(CockpitRole::Viewer, allowed(&["ms-car1"]));
        assert_eq!(viewer.join_vehicle_room(Some("ms-car1")), None, "viewer 可拉流");
        assert!(viewer.join_vehicle_room(Some("ms-car2")).is_some());

        let admin = account(CockpitRole::Admin, vec![]);
        assert_eq!(admin.join_vehicle_room(Some("ms-car9")), None, "admin 任意车");

        let disp = account(CockpitRole::Dispatcher, vec![]);
        assert_eq!(disp.join_vehicle_room(Some("ms-car9")), None, "dispatcher 任意车");
    }

    #[test]
    fn join_gate_device_tenant_isolation() {
        let veh_a = SessionIdentity::Device("ms-car-a".into());
        assert_eq!(veh_a.join_vehicle_room(None), None, "车加入自己的空房间");
        assert_eq!(veh_a.join_vehicle_room(Some("ms-car-a")), None, "车加入自己的房间");
        assert!(
            veh_a.join_vehicle_room(Some("ms-car-b")).is_some(),
            "车 A 不可加入车 B 房间（租户隔离）"
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
        // I1 review: P2P 路径控制门 — 矩阵 + 身份
        assert!(!account(CockpitRole::Viewer, vec![]).can_control());
        assert!(!account(CockpitRole::Dispatcher, vec![]).can_control());
        assert!(account(CockpitRole::Operator, vec![]).can_control());
        assert!(account(CockpitRole::Admin, vec![]).can_control());
        assert!(SessionIdentity::Device("d".into()).can_control(), "设备是控制接收方");
        assert!(SessionIdentity::Legacy.can_control(), "legacy 不受矩阵限制");
    }

    #[test]
    fn produce_gate_device_and_legacy_allowed_account_denied() {
        assert_eq!(SessionIdentity::Device("d".into()).can_produce("vehicle_x"), Ok(()), "车端 produce 自动允许");
        assert_eq!(SessionIdentity::Legacy.can_produce("vehicle_x"), Ok(()), "dev 路径放行");
        assert!(
            account(CockpitRole::Viewer, vec![]).can_produce("vehicle_x").is_err(),
            "账号禁止 produce（视频房）"
        );
        assert!(account(CockpitRole::Admin, vec![]).can_produce("vehicle_x").is_err());
    }

    #[test]
    fn produce_gate_audio_room_account_exempt() {
        // P1/D-H11 修订: audio-* 会议房账号上行豁免（C29 麦语义）；视频房与近前缀房仍禁。
        assert_eq!(
            account(CockpitRole::Operator, vec![]).can_produce("audio-car1"),
            Ok(()),
            "audio 会议房账号上行豁免"
        );
        assert_eq!(account(CockpitRole::Viewer, vec![]).can_produce("audio-car1"), Ok(()));
        assert!(
            account(CockpitRole::Operator, vec![]).can_produce("vehicle_car1").is_err(),
            "视频房账号仍禁"
        );
        assert!(
            account(CockpitRole::Operator, vec![]).can_produce("audiox").is_err(),
            "audio 前缀必须是 audio-（近前缀不豁免）"
        );
    }

    #[test]
    fn consume_gate_account_allowlist_device_legacy_allow() {
        let op_car1 = account(CockpitRole::Operator, allowed(&["ms-car1"]));
        assert_eq!(op_car1.can_consume(Some("ms-car1")), Ok(()));
        assert!(op_car1.can_consume(Some("ms-car2")).is_err(), "非授权车 producer 拒绝");
        assert_eq!(op_car1.can_consume(None), Ok(()), "无主 producer 放行（兼容）");
        assert_eq!(SessionIdentity::Device("d".into()).can_consume(Some("ms-car2")), Ok(()));
        assert_eq!(SessionIdentity::Legacy.can_consume(Some("ms-car2")), Ok(()));
    }

    #[test]
    fn emergency_gate_matrix() {
        let op_car1 = account(CockpitRole::Operator, allowed(&["ms-car1"]));
        assert_eq!(op_car1.can_emergency(Some("ms-car1")), Ok(()), "operator 授权车急停");
        assert!(op_car1.can_emergency(Some("ms-car2")).is_err(), "非授权车拒绝");
        assert!(op_car1.can_emergency(None).is_err(), "车离线拒绝");
        let viewer = account(CockpitRole::Viewer, allowed(&["ms-car1"]));
        assert!(viewer.can_emergency(Some("ms-car1")).is_err(), "viewer 无急停");
        let dispatcher = account(CockpitRole::Dispatcher, vec![]);
        assert!(dispatcher.can_emergency(Some("ms-car9")).is_err(), "dispatcher 无急停");
        let admin = account(CockpitRole::Admin, vec![]);
        assert_eq!(admin.can_emergency(Some("ms-car9")), Ok(()), "admin 任意车急停");
        assert!(
            SessionIdentity::Device("d".into()).can_emergency(Some("ms-car-d")).is_err(),
            "车端是接收方"
        );
        assert_eq!(SessionIdentity::Legacy.can_emergency(Some("ms-car1")), Ok(()), "dev 路径");
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
        assert_eq!(
            account(CockpitRole::Operator, vec![]).who(),
            "u[operator]"
        );
    }
}
