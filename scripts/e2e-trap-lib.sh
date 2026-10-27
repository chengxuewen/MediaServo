#!/usr/bin/env bash
# e2e-trap-lib.sh — hardened EXIT-trap cleanup shared by the brand/deploy/package e2e scripts.
#
# Problem (PIT-192/PIT-235 family): the old traps did
#     rm -rf "$TMP"; rm -rf /tmp/iceoryx2 /dev/shm/iox2_*; pkill -x oxmgr; pkill -x cp-agent
# unconditionally. On a host running a LIVE cluster that meant killing foreign daemons
# and wiping shared-runtime state owned by other processes.
#
# This library makes cleanup ownership-scoped:
#   (a) the iceoryx2/SHM wipe runs ONLY when no foreign live nodes / msrtc-family
#       processes are detected (heuristic: node+service file contents reference paths
#       outside $TMP; SHM wipe additionally gated on no foreign family process);
#   (b) daemon reaping is per-pid: /proc/<pid>/cwd + OXMGR_HOME/OXMGR_DATA_DIR in
#       /proc/<pid>/environ must resolve inside $TMP, else the pid is skipped (WARN).
# Own tempdir removal ($TMP) is always unconditional.
#
# Usage: source this file from an e2e script, set TMP (mktemp -d), then
#     trap msrtc_e2e_cleanup EXIT
# The function is env-testable: the e2e script's own daemon/SHM live under $TMP by
# construction, so the production paths are the default and tests may override.
#
# Safe under `set -euo pipefail` (the e2e scripts run with it): every probe tolerates
# absence/permission denial and the function never fails on its own account.

# Root of the iceoryx2 runtime to wipe. Defaults to the production location.
: "${ICEORYX2_ROOT:=/tmp/iceoryx2}"
# Glob (quoted at the rm site, so expansion happens at call time) for SHM segments.
: "${SHM_GLOB:=/dev/shm/iox2_*}"
# Owned tempdir — the e2e script's mktemp -d. Paths under here are ours.
: "${TMP:=}"

# Shell words that name the product's process family (brand-agnostic core):
#   mediaservo-*  (default brand)   msrtc-*  (msrtc brand)   oxmgr  (daemon supervisor)
#   cp-* / host-* (legacy brands and their agents)
# Narrowed to a leading `^(mediaservo-|msrtc-|oxmgr|cp-|host-)` so foreign processes
# such as `1panel-agent` are never mistaken for the family.
_MSRTC_FAMILY_RE='^(mediaservo-|msrtc-|oxmgr|cp-|host-)'

# echo absolute, normalized paths found inside the iceoryx2 root that reference a
# location OUTSIDE $TMP. Exit 0 if at least one foreign path was found, 1 otherwise.
# A path is "owned" when it starts with $TMP/ (self-references to the root itself —
# e.g. /tmp/iceoryx2 inside service metadata — are treated as foreign: the runtime is
# a live system, never a fixture the e2e owns).
msrtc_e2e_foreign_paths() {
    local root="${1:-$ICEORYX2_ROOT}"
    local owned="${TMP%/}"
    [ -n "$owned" ] || owned='/'   # empty TMP => nothing is owned
    [ -d "$root" ] || return 1
    local found=0 p tok
    # -ah treats service/binary files as text; whitespace-delimited absolute paths only.
    while IFS= read -r tok; do
        case "$tok" in
            /*) ;;
            *) continue ;;
        esac
        # skip owned paths (exact prefix, normalized)
        case "$tok" in
            "$owned"/*) continue ;;
            *) ;;
        esac
        echo "$tok"
        found=1
    done < <(grep -RhaE -o '/[[:alnum:]_.+-]+(/[[:alnum:]_.+-]+)+' "$root" 2>/dev/null)
    [ "$found" = 1 ]
}

# Is there any live msrtc-family process at all (foreign or owned)?
msrtc_e2e_family_alive() {
    ps -eo comm= 2>/dev/null | grep -qE "$_MSRTC_FAMILY_RE"
}

# Reap daemons whose cwd or OXMGR_* state directory resolves inside $TMP. Foreign
# daemons are left untouched (WARN names each one skipped).
msrtc_e2e_reap_owned_daemons() {
    local owned="${TMP%/}" name pid cwd home data
    [ -n "$owned" ] || return 0
    for name in oxmgr cp-agent; do
        for pid in $(pgrep -x "$name" 2>/dev/null || true); do
            cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null || true)
            home=$(tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null \
                   | sed -n 's/^OXMGR_HOME=//p' | tail -1)
            data=$(tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null \
                   | sed -n 's/^OXMGR_DATA_DIR=//p' | tail -1)
            case "$cwd $home $data" in
                *"$owned/"*)
                    kill "$pid" 2>/dev/null || true
                    echo "reaped owned daemon: $name pid=$pid (cwd=$cwd)" ;;
                *)
                    echo "WARN: skipped foreign daemon: $name pid=$pid (cwd=$cwd home=$home data=$data)" ;;
            esac
        done
    done
}

# The EXIT trap: own-tempdir removal is unconditional; everything else is
# ownership-scoped. Returns 0 so a real script failure is never masked.
msrtc_e2e_cleanup() {
    local foreign="" skip_shm=0
    # (a) iceoryx2 wipe: only when the tree holds no foreign node/service paths.
    if [ -d "${ICEORYX2_ROOT}" ]; then
        foreign=$(msrtc_e2e_foreign_paths "$ICEORYX2_ROOT" || true)
        if [ -n "$foreign" ]; then
            echo "WARN: e2e cleanup skipped iceoryx2 wipe — foreign live nodes detected:"
            echo "$foreign" | sed 's/^/      /'
        else
            rm -rf "${ICEORYX2_ROOT}"
        fi
    fi
    # SHM wipe: only when no msrtc-family process is alive.
    if msrtc_e2e_family_alive; then
        echo "WARN: e2e cleanup skipped SHM wipe — live msrtc-family process detected"
    else
        rm -rf ${SHM_GLOB}   # quoted expansion is intentional (glob at call time)
    fi
    # (b) daemon reaping: per-pid ownership, foreign daemons untouched.
    msrtc_e2e_reap_owned_daemons || true
    # Own tempdir: always.
    [ -n "$TMP" ] && rm -rf "$TMP"
    return 0
}
