//! ifb.rs —— T9 capability 探测（design §引擎 capability 表 + §ifb 合同的「先读后探」链）。
//!
//! 判定合同（零内核触达的纯函数 [`ifb_decision`]，探测事实注入）：
//! - **先读后探**：`ip link show ifb0` 已存在 → 直接 true，**绝不删非本次探测所建的 ifb0**
//!   （否则 refresh 会拆掉活跃 dir=in 会话的镜像=黑洞复发——design §capability 红线）。
//! - local 通道：缺席才 modprobe → add → link up → 【立即 del，仅限本次探测所建】→ true。
//!   模块缺失 → false+「内核无 ifb 模块」；已加载但建不起来 → false+「无权创建 netdev」。
//! - sidecar 通道：**只读 lsmod 判定**（容器与宿主共享内核 → /proc/modules 即宿主模块表；
//!   镜像内无 /lib/modules 不能自行 modprobe，也不做建删探测——避免拆宿主活跃会话）。
//!   未加载 → false+「需宿主 root modprobe ifb（sidecar 通道无 /lib/modules）」；已加载 → true
//!   （NET_ADMIN + host netns，施加期 `ip link add` 可行）。
//!
//! ifb 是内核模块支持的 netdev：`ip link add ... type ifb` 需要**宿主内核已装载 ifb.ko**
//! （模块表全局，netns 帮不上忙）；非 root 用户命名空间内 request_module 被禁用。

use crate::engine::{self, Channel};

/// §ifb 合同固定镜像设备名（宿主全局单例——多物理口并发共享 ifb0 是已注记的简化，
/// ponytail: 多 NIC 同时上行需 per-iface ifb 设备，出现真实需求再扩）。
pub const IFB_DEV: &str = "ifb0";
/// 内核模块名（≠ 设备名——/proc/modules 首列与 modprobe 实参用此）。
pub const IFB_MODULE: &str = "ifb";

pub const REASON_OK: &str = "";
pub const REASON_NO_MODULE: &str = "内核无 ifb 模块";
/// sidecar 通道未加载专用文案（区分「未加载（宿主补救=modprobe）」vs「不可用（权限）」）。
pub const REASON_NEED_HOST_ROOT: &str = "需宿主 root modprobe ifb（sidecar 通道无 /lib/modules）";
pub const REASON_NO_PERM: &str = "无权创建 netdev";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfbKind {
    /// local-root 通道：允许完整 modprobe→建→up→删探测链。
    Local,
    /// docker NET_ADMIN sidecar：只读判定，零建删副作用。
    Sidecar,
}

/// 探测事实（纯函数入参——单测矩阵零环境触碰；实探由 [`probe`] 收集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfbFacts {
    pub kind: IfbKind,
    /// 先读结果：`ip link show ifb0` 成功（存在他人/系统建的 ifb0 = 镜像通道现成）。
    pub ifb0_exists: bool,
    /// 宿主内核模块表含 ifb（local 读本进程 /proc/modules；sidecar 读容器内同表=宿主）。
    pub module_loaded: bool,
    /// local 探测链 add+up+del 全成；sidecar 恒 false（不做建删探测）。
    pub create_ok: bool,
}

/// 纯判定（design §capability 表行为面）。返回 (ifb_ingress, reason)。
/// 优先级：先读命中 > 建探测成功 > 通道×模块事实。
#[must_use]
pub fn ifb_decision(f: IfbFacts) -> (bool, &'static str) {
    if f.ifb0_exists {
        return (true, REASON_OK);
    }
    match f.kind {
        // 建探测成功即终证（modprobe 竞态后模块表读数滞后也不翻案——事实以 kernel 回执为准）。
        IfbKind::Local if f.create_ok => (true, REASON_OK),
        IfbKind::Local if !f.module_loaded => (false, REASON_NO_MODULE),
        IfbKind::Local => (false, REASON_NO_PERM),
        // sidecar：模块在 → NET_ADMIN 可建；不在 → 只能宿主 root modprobe（镜像无 /lib/modules）。
        IfbKind::Sidecar if f.module_loaded => (true, REASON_OK),
        IfbKind::Sidecar => (false, REASON_NEED_HOST_ROOT),
    }
}

/// 宿主内核模块表读 ifb（`/proc/modules` 首列精确词）。netns/容器可见性=全局内核状态。
#[must_use]
pub fn module_loaded_from(proc_modules: &str) -> bool {
    proc_modules
        .lines()
        .any(|l| l.split_whitespace().next() == Some(IFB_MODULE))
}

/// 实探入口（design「先读后探」链）。写锁瞬持由调用方保证；本函数自身仅有界建删副作用，
/// 且 del 严格限于本次 add 成功的 ifb0。
pub fn probe(channel: &Channel) -> (bool, String) {
    let kind = match channel {
        Channel::LocalRoot => IfbKind::Local,
        Channel::Sidecar { .. } => IfbKind::Sidecar,
    };
    if ifb0_exists(channel) {
        return (true, REASON_OK.to_string());
    }
    let facts = match kind {
        IfbKind::Sidecar => IfbFacts {
            kind,
            ifb0_exists: false,
            module_loaded: sidecar_module_loaded(channel),
            create_ok: false,
        },
        IfbKind::Local => {
            // modprobe 尽力而为（非 root 必败——失败留给 add 回执定因，bash「载与不载都查」同构）。
            let _ = engine::exec_prog(channel, &["modprobe", IFB_MODULE]);
            let create_ok = local_try_create(channel);
            IfbFacts {
                kind,
                ifb0_exists: false,
                module_loaded: local_module_loaded(),
                create_ok,
            }
        }
    };
    let (ok, reason) = ifb_decision(facts);
    (ok, reason.to_string())
}

fn ifb0_exists(channel: &Channel) -> bool {
    engine::exec_prog(channel, &["ip", "link", "show", IFB_DEV]).is_ok()
}

fn local_module_loaded() -> bool {
    match std::fs::read_to_string("/proc/modules") {
        Ok(s) => module_loaded_from(&s),
        Err(e) => {
            eprintln!("weaknet(ifb): WARN /proc/modules 不可读（按未加载处理）: {e}");
            false
        }
    }
}

fn sidecar_module_loaded(channel: &Channel) -> bool {
    // 容器 /proc/modules = 宿主内核模块表（模块全局，非 netns 隔离面）——documented 于模块头注。
    match engine::exec_prog(channel, &["cat", "/proc/modules"]) {
        Ok(s) => module_loaded_from(&s),
        Err(e) => {
            eprintln!("weaknet(ifb): WARN sidecar 读 /proc/modules 失败（按未加载处理）: {}", e.msg);
            false
        }
    }
}

/// local 建删探测：add → up → del（仅限本次所建）。任一步败 → 清掉本次半成品后 false。
fn local_try_create(channel: &Channel) -> bool {
    if engine::exec_prog(channel, &["ip", "link", "add", IFB_DEV, "type", "ifb"]).is_err() {
        return false;
    }
    let up = engine::exec_prog(channel, &["ip", "link", "set", IFB_DEV, "up"]);
    let del = engine::exec_prog(channel, &["ip", "link", "del", IFB_DEV]);
    if let Err(e) = del {
        // 探测残留必须报因（不可静默——下一步 apply 会复用/覆盖，但所有权判断依赖现场）。
        eprintln!("weaknet(ifb): WARN 探测建删链收尾 del {IFB_DEV} 失败（残留待人工/apply 复用）: {}", e.msg);
    }
    up.is_ok()
}

/// 供 apply 路径复用：探测链之外的「本次是否需我方建 ifb0」判据。
pub fn link_present(channel: &Channel) -> bool {
    ifb0_exists(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(kind: IfbKind, exists: bool, loaded: bool, create: bool) -> IfbFacts {
        IfbFacts {
            kind,
            ifb0_exists: exists,
            module_loaded: loaded,
            create_ok: create,
        }
    }

    #[test]
    fn read_first_hit_true_both_channels_empty_reason() {
        // 先读命中：两通道一律 true+空因，绝不建删（活跃会话镜像不拆）。
        assert_eq!(ifb_decision(facts(IfbKind::Local, true, false, false)), (true, REASON_OK));
        assert_eq!(ifb_decision(facts(IfbKind::Sidecar, true, false, false)), (true, REASON_OK));
    }

    #[test]
    fn local_matrix() {
        // local × 建探测成功 = 终证（模块表滞后读数不翻案）。
        assert_eq!(ifb_decision(facts(IfbKind::Local, false, false, true)), (true, REASON_OK));
        // local × 模块缺失 = 「内核无 ifb 模块」。
        assert_eq!(ifb_decision(facts(IfbKind::Local, false, false, false)), (false, REASON_NO_MODULE));
        // local × 已加载但建不起（非 root netns 场景 = request_module 禁用+EPERM）= 「无权创建 netdev」。
        assert_eq!(ifb_decision(facts(IfbKind::Local, false, true, false)), (false, REASON_NO_PERM));
    }

    #[test]
    fn sidecar_matrix_distinguishes_not_loaded_vs_unavailable() {
        // sidecar 只读判定：已加载 → true（NET_ADMIN 施加期可建）；未加载 → 专属补救文案。
        assert_eq!(ifb_decision(facts(IfbKind::Sidecar, false, true, false)), (true, REASON_OK));
        assert_eq!(ifb_decision(facts(IfbKind::Sidecar, false, false, false)), (false, REASON_NEED_HOST_ROOT));
    }

    #[test]
    fn module_table_parsing_exact_word_and_edges() {
        let table = "ifb 20480 0 - 0xffffffffffffffff\nsch_netem 20480 0 - 0xffffffffffffffff\n";
        assert!(module_loaded_from(table));
        // 前缀词不误判（ifb 精确列匹配，非子串）。
        assert!(!module_loaded_from("dummy_ifb2 16384 0 -\nsch_foo 1 1 -\n"));
        assert!(!module_loaded_from(""));
        // 在用模块（refcount>0）同样命中。
        assert!(module_loaded_from("ifb 20480 3 - 0xffffffffffffffff\n"));
        // 设备名 ifb0 出现于他列 ≠ 模块已载（module≠dev 名钉死，防本族混淆回归）。
        assert!(!module_loaded_from("dummy 20480 0 -\nfoo 1 1 ifb0 -\n"));
    }

    #[test]
    fn reason_strings_are_design_contract_text() {
        // 面板灰显 tooltip 消费这两串（design §UI「结构性不支持」vs「宿主未加载」区分）。
        assert_eq!(REASON_NO_MODULE, "内核无 ifb 模块");
        assert_eq!(
            REASON_NEED_HOST_ROOT,
            "需宿主 root modprobe ifb（sidecar 通道无 /lib/modules）"
        );
        assert_eq!(REASON_NO_PERM, "无权创建 netdev");
    }
}
