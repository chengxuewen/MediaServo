//! ifb.rs -- T9 capability probing (design §engine capability table + the §ifb contract's "read-before-probe" chain).
//!
//! Verdict contract (the zero-kernel-touch pure function [`ifb_decision`], probe facts injected):
//! - **read-before-probe**: `ip link show ifb0` already exists -> directly true, **never delete an ifb0 not created by this probe**
//!   (otherwise refresh would tear down a live dir=in session's mirror = the black hole relapsing -- design §capability red line).
//! - local channel: only when absent, modprobe -> add -> link up -> [immediately del, only if created by this probe] -> true.
//!   module missing -> false + "kernel has no ifb module"; loaded but cannot create -> false + "no permission to create a netdev".
//! - sidecar channel: **read-only lsmod verdict** (container and host share the kernel -> /proc/modules IS the host module table;
//!   the image has no /lib/modules so it cannot modprobe itself, and no create/delete probing either -- avoids tearing down live host sessions).
//!   not loaded -> false + "needs modprobe ifb as host root (the sidecar channel has no /lib/modules)"; loaded -> true
//!   (NET_ADMIN + host netns, `ip link add` feasible at apply time).
//!
//! ifb is a kernel-module-backed netdev: `ip link add ... type ifb` requires **the host kernel to have ifb.ko loaded**
//! (the module table is global, netns doesn't help); request_module is disabled inside unprivileged user namespaces.

use crate::engine::{self, Channel};

/// §ifb contract fixed mirror device name (host-global singleton -- concurrent multi-NIC sharing of ifb0 is a documented simplification,
/// ponytail: per-iface ifb devices needed for simultaneous multi-NIC ingress; extend when a real need appears).
pub const IFB_DEV: &str = "ifb0";
/// Kernel module name (!= device name -- /proc/modules first column and the modprobe argument use this).
pub const IFB_MODULE: &str = "ifb";

pub const REASON_OK: &str = "";
pub const REASON_NO_MODULE: &str = "kernel has no ifb module";
/// sidecar-channel not-loaded dedicated wording (distinguishes "not loaded (host fix = modprobe)" vs "unavailable (permission)").
pub const REASON_NEED_HOST_ROOT: &str =
    "needs modprobe ifb as host root (the sidecar channel has no /lib/modules)";
pub const REASON_NO_PERM: &str = "no permission to create a netdev";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfbKind {
    /// local-root channel: the full modprobe -> create -> up -> del probe chain allowed.
    Local,
    /// docker NET_ADMIN sidecar: read-only verdict, zero create/delete side effects.
    Sidecar,
}

/// Probe facts (pure-function inputs -- the unit-test matrix touches zero environment; [`probe`] collects the real facts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfbFacts {
    pub kind: IfbKind,
    /// read result: `ip link show ifb0` succeeded (someone else's/the system's ifb0 present = the mirror channel ready-made).
    pub ifb0_exists: bool,
    /// the host kernel module table contains ifb (local reads this process's /proc/modules; sidecar reads the same table in the container = host).
    pub module_loaded: bool,
    /// the local probe chain add+up+del all succeeded; sidecar always false (no create/delete probing).
    pub create_ok: bool,
}

/// Pure verdict (the design §capability table behavior surface). Returns (ifb_ingress, reason).
/// Priority: read hit > create-probe success > channel x module facts.
#[must_use]
pub fn ifb_decision(f: IfbFacts) -> (bool, &'static str) {
    if f.ifb0_exists {
        return (true, REASON_OK);
    }
    match f.kind {
        // create-probe success is the final proof (even if the module-table read lags after a modprobe race, the kernel receipt wins).
        IfbKind::Local if f.create_ok => (true, REASON_OK),
        IfbKind::Local if !f.module_loaded => (false, REASON_NO_MODULE),
        IfbKind::Local => (false, REASON_NO_PERM),
        // sidecar: module present -> NET_ADMIN can create; absent -> only host root modprobe (no /lib/modules in the image).
        IfbKind::Sidecar if f.module_loaded => (true, REASON_OK),
        IfbKind::Sidecar => (false, REASON_NEED_HOST_ROOT),
    }
}

/// Read ifb from the host kernel module table (`/proc/modules` first column, exact word). netns/container visibility = global kernel state.
#[must_use]
pub fn module_loaded_from(proc_modules: &str) -> bool {
    proc_modules.lines().any(|l| l.split_whitespace().next() == Some(IFB_MODULE))
}

/// The real-probe entry (the design "read-before-probe" chain). The caller guarantees the transient write lock; this function has only
/// bounded create/delete side effects, and del is strictly limited to the ifb0 this add created.
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
            // best-effort modprobe (must fail as non-root -- the add receipt decides the cause, structurally same as bash "check loaded or not").
            let _ = engine::exec_prog(channel, &["modprobe", IFB_MODULE]);
            let create_ok = local_try_create(channel);
            IfbFacts { kind, ifb0_exists: false, module_loaded: local_module_loaded(), create_ok }
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
            eprintln!("weaknet(ifb): WARN /proc/modules unreadable (treating as not loaded): {e}");
            false
        }
    }
}

fn sidecar_module_loaded(channel: &Channel) -> bool {
    // the container's /proc/modules = the host kernel module table (modules are global, not a netns-isolated surface) -- documented in the module header.
    match engine::exec_prog(channel, &["cat", "/proc/modules"]) {
        Ok(s) => module_loaded_from(&s),
        Err(e) => {
            eprintln!(
                "weaknet(ifb): WARN sidecar failed to read /proc/modules (treating as not loaded): {}",
                e.msg
            );
            false
        }
    }
}

/// local create/delete probe: add -> up -> del (only what we created). Any step failing -> clean up this round's half-product and return false.
fn local_try_create(channel: &Channel) -> bool {
    if engine::exec_prog(channel, &["ip", "link", "add", IFB_DEV, "type", "ifb"]).is_err() {
        return false;
    }
    let up = engine::exec_prog(channel, &["ip", "link", "set", IFB_DEV, "up"]);
    let del = engine::exec_prog(channel, &["ip", "link", "del", IFB_DEV]);
    if let Err(e) = del {
        // probe residue must report cause (never silent -- the next apply reuses/overwrites, but ownership judgment depends on the site).
        eprintln!(
            "weaknet(ifb): WARN probe chain's closing del of {IFB_DEV} failed (residue awaits manual/apply reuse): {}",
            e.msg
        );
    }
    up.is_ok()
}

/// For the apply path's reuse: the "does this round need us to create ifb0" predicate beyond the probe chain.
pub fn link_present(channel: &Channel) -> bool {
    ifb0_exists(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(kind: IfbKind, exists: bool, loaded: bool, create: bool) -> IfbFacts {
        IfbFacts { kind, ifb0_exists: exists, module_loaded: loaded, create_ok: create }
    }

    #[test]
    fn read_first_hit_true_both_channels_empty_reason() {
        // read hit: both channels true + empty cause, never create/delete (a live session's mirror is not torn down).
        assert_eq!(ifb_decision(facts(IfbKind::Local, true, false, false)), (true, REASON_OK));
        assert_eq!(ifb_decision(facts(IfbKind::Sidecar, true, false, false)), (true, REASON_OK));
    }

    #[test]
    fn local_matrix() {
        // local x create-probe success = final proof (a lagging module-table read doesn't overturn it).
        assert_eq!(ifb_decision(facts(IfbKind::Local, false, false, true)), (true, REASON_OK));
        // local x module missing = "kernel has no ifb module".
        assert_eq!(
            ifb_decision(facts(IfbKind::Local, false, false, false)),
            (false, REASON_NO_MODULE)
        );
        // local x loaded but cannot create (the non-root netns case = request_module disabled + EPERM) = "no permission to create a netdev".
        assert_eq!(
            ifb_decision(facts(IfbKind::Local, false, true, false)),
            (false, REASON_NO_PERM)
        );
    }

    #[test]
    fn sidecar_matrix_distinguishes_not_loaded_vs_unavailable() {
        // sidecar read-only verdict: loaded -> true (NET_ADMIN can create at apply); not loaded -> the dedicated fix wording.
        assert_eq!(ifb_decision(facts(IfbKind::Sidecar, false, true, false)), (true, REASON_OK));
        assert_eq!(
            ifb_decision(facts(IfbKind::Sidecar, false, false, false)),
            (false, REASON_NEED_HOST_ROOT)
        );
    }

    #[test]
    fn module_table_parsing_exact_word_and_edges() {
        let table = "ifb 20480 0 - 0xffffffffffffffff\nsch_netem 20480 0 - 0xffffffffffffffff\n";
        assert!(module_loaded_from(table));
        // prefix words must not misjudge (ifb exact-column match, not substring).
        assert!(!module_loaded_from("dummy_ifb2 16384 0 -\nsch_foo 1 1 -\n"));
        assert!(!module_loaded_from(""));
        // an in-use module (refcount>0) matches too.
        assert!(module_loaded_from("ifb 20480 3 - 0xffffffffffffffff\n"));
        // the device name ifb0 appearing in another column != module loaded (module != dev name pinned, guarding against this family's confusion regressing).
        assert!(!module_loaded_from("dummy 20480 0 -\nfoo 1 1 ifb0 -\n"));
    }

    #[test]
    fn reason_strings_are_design_contract_text() {
        // the panel grey-tooltip consumes these two strings (design §UI "structurally unsupported" vs "not loaded on host" distinction).
        assert_eq!(REASON_NO_MODULE, "kernel has no ifb module");
        assert_eq!(
            REASON_NEED_HOST_ROOT,
            "needs modprobe ifb as host root (the sidecar channel has no /lib/modules)"
        );
        assert_eq!(REASON_NO_PERM, "no permission to create a netdev");
    }
}
