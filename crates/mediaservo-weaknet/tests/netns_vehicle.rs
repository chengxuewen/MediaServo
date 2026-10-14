//! W8 vehicle stand-in integration trigger surface (tasks.md T12/T13): when the environment is complete, launches `tests/netns_vehicle.sh`
//! (a veth topology inside an unshare -rn netns -- anti-deadlock negatives/hit-exclusion quantification/watch/down/embedded fallback/zero residue);
//! any missing precondition = SKIP with cause, passing (the "cannot construct = not closed" discipline, same as netns_ifb; W8 stays on the stand-in ledger).

use std::path::Path;
use std::process::Command;

#[test]
fn netns_vehicle_e2e_when_environment_allows() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let script = Path::new(manifest).join("tests/netns_vehicle.sh");
    assert!(script.is_file(), "missing {script:?}");
    let bin = env!("CARGO_BIN_EXE_mediaservo-weaknet");

    let have = |prog: &str| {
        Command::new("sh")
            .args(["-c", "command -v \"$1\"", "sh", prog])
            .status()
            .is_ok_and(|st| st.success())
    };
    let unshare_ok =
        Command::new("unshare").args(["-rn", "true"]).status().is_ok_and(|st| st.success());
    let (tc_ok, py_ok) = (have("tc"), have("python3"));
    if !tc_ok || !py_ok || !unshare_ok {
        eprintln!(
            "SKIP netns_vehicle: tc={tc_ok} python3={py_ok} unshare_ok={unshare_ok} —— \
             W8 stand-in BLOCKED-env (re-trigger = re-run this case after iproute2/python3/userns are in place)"
        );
        return;
    }

    let out = Command::new("bash")
        .arg(&script)
        .env("WEAKNET_BIN", bin)
        .output()
        .expect("netns_vehicle.sh execution failed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    print!("{stdout}");
    eprint!("{stderr}");
    // script-internal race verdicts cannot-construct (userns disabled etc.) -> 77, same SKIP-with-cause passing.
    if out.status.code() == Some(77) {
        eprintln!(
            "SKIP netns_vehicle: the script judged the environment cannot be constructed (negative evidence in the transcript above) -- W8 BLOCKED-env"
        );
        return;
    }
    assert!(
        out.status.success(),
        "netns_vehicle.sh failed rc={:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        out.status.code()
    );
}
