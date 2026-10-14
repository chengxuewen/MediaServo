//! W4 integration trigger surface (tasks.md T9): when the environment is complete, launches `tests/netns_ifb.sh` (inside an unshare -rn
//! netns: the veth+ifb three cases -- hit hard proof/targeted control/clean teardown zero residue); any missing precondition = SKIP with cause
//! passing, W4 stays on the BLOCKED-env ledger ("cannot construct = not closed; a note never counts as green").
//!
//! The predicate source matches the script's internal branch: no ifb line in the host's /proc/modules => the netns can never create one (modules are global).
//! The script still re-proves inside the netns itself (the negative-evidence transcript is the script's own output).

use std::path::Path;
use std::process::Command;

#[test]
fn netns_ifb_e2e_when_environment_allows() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let script = Path::new(manifest).join("tests/netns_ifb.sh");
    assert!(script.is_file(), "missing {script:?}");
    let bin = env!("CARGO_BIN_EXE_mediaservo-weaknet");

    let mod_loaded = std::fs::read_to_string("/proc/modules")
        .map(|s| s.lines().any(|l| l.split_whitespace().next() == Some("ifb")))
        .unwrap_or(false);
    let unshare_ok =
        Command::new("unshare").args(["-rn", "true"]).status().is_ok_and(|st| st.success());
    if !mod_loaded || !unshare_ok {
        eprintln!(
            "SKIP netns_ifb: ifb_module={mod_loaded} unshare_ok={unshare_ok} —— \
             W4 = BLOCKED-env (re-trigger = re-run this case/netns_ifb.sh after host root modprobe ifb)"
        );
        return;
    }

    let out = Command::new("bash")
        .arg(&script)
        .env("WEAKNET_BIN", bin)
        .output()
        .expect("netns_ifb.sh execution failed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    print!("{stdout}");
    eprint!("{stderr}");
    // the script's in-netns re-test may still judge cannot-construct (a racing unload) -> 77 likewise passes as SKIP with cause.
    if out.status.code() == Some(77) {
        eprintln!(
            "SKIP netns_ifb: the script judged the environment cannot be constructed (negative evidence in the transcript above) -- W4 BLOCKED-env"
        );
        return;
    }
    assert!(
        out.status.success(),
        "netns_ifb.sh failed rc={:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        out.status.code()
    );
}
