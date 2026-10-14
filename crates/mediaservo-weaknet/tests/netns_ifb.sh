#!/usr/bin/env bash
# tests/netns_ifb.sh -- W4 (design §ifb three-case dual proof) executed inside an unshare -rn network namespace.
#
# Safety: all kernel touching happens inside the new netns (user-ns root) -- the host's ens32/lo is never touched; the
# veth/ifb0 built inside the namespace dies automatically when the last process exits. ifb is a kernel-module-backed netdev: the module table is global,
# netns doesn't help -- when the host hasn't loaded ifb.ko, `ip link add ... type ifb` must fail (this script
# branches on that: can create = all three cases; cannot = a negative-evidence transcript, exit 77 = BLOCKED-env on the ledger).
#
# Usage: WEAKNET_BIN=/path/to/mediaservo-weaknet bash tests/netns_ifb.sh
# Exit: 0 = all three W4 cases pass, 77 = environment cannot be constructed (negative evidence already printed), otherwise = case failure
set -u -o pipefail

BIN="${WEAKNET_BIN:-}"
[[ -z "$BIN" || ! -x "$BIN" ]] && { echo "FAIL: WEAKNET_BIN unset or not executable: '$BIN'"; exit 2; }
export BIN

command -v tc >/dev/null || { echo "FAIL: no tc (iproute2) on the host"; exit 2; }
command -v unshare >/dev/null || { echo "FAIL: no unshare"; exit 2; }
unshare -rn true || { echo "SKIP: unshare -rn unavailable (user ns disabled?) -- W4 BLOCKED-env"; exit 77; }

echo "== environment: tc=$(tc -V) bin=$BIN =="
echo "== host /proc/modules ifb line (precheck, the global module table) =="
grep '^ifb ' /proc/modules || echo "(no ifb module line)"

unshare -rn bash -eu -o pipefail <<'NETNS'
echo "== entering unshare -rn (new netns, user-ns root) =="
ip link set lo up

# ---- branch predicate: can a netns create an ifb device (= the host kernel has ifb.ko loaded) ----
if ! err=$(ip link add ifb0 type ifb 2>&1); then
    echo "### negative-evidence transcript (the W4 BLOCKED-env evidence artifact) ###"
    echo "\$ ip link add ifb0 type ifb"
    echo "$err"
    echo "--- re-proof: modprobe inside the user-ns also fails (module loading is not a namespace-capability) ---"
    echo "\$ modprobe ifb"
    modprobe ifb 2>&1 || true
    echo "### verdict: the host has not loaded the ifb module -> W4 not closed (tasks.md T9); re-trigger = re-run this script after host root modprobe ifb ###"
    exit 77
fi
echo "### ifb creatable (the host module present) -- delete the precheck half-product and run all three cases ###"
ip link del ifb0

# ---- fixture: vethA local side + vethB moved into the peer netns ----
# both veth ends in the same netns take the local short-circuit; vethA ingress never sees packets (the first round's 0->0 topology root cause);
# the peer netns's source address is non-local -> real ingress traffic reaches vethA ingress.
unshare -n sleep 400 & PEER=$!
sleep 0.3
ip link add vethA type veth peer name vethB
ip addr add 10.99.0.1/24 dev vethA
ip link set vethA up
ip link set vethB netns "$PEER"
nsenter -t "$PEER" -n ip addr add 10.99.0.2/24 dev vethB
nsenter -t "$PEER" -n ip link set vethB up

STATEDIR=$(mktemp -d /tmp/weaknet-t9.XXXXXX)
export WEAKNET_STATEDIR="$STATEDIR"
export WEAKNET_CHANNEL=local
FLOOD_PIDS=()
cleanup() {
    for pid in "${FLOOD_PIDS[@]:-}"; do
        [[ -n "${pid:-}" ]] && kill "$pid" 2>/dev/null || true
    done
    "$BIN" clear >/dev/null 2>&1 || true
    kill "${PEER:-0}" 2>/dev/null || true
    rm -rf "$STATEDIR"
}
trap cleanup EXIT

flood() { # flood <port> <secs> -- flood from the vethB side to vethA:dport (= vethA ingress traffic)
    timeout "$2" nsenter -t "$PEER" -n bash -c "while :; do echo weaknet-t9 > /dev/udp/10.99.0.1/$1 2>/dev/null || true; done" &
    FLOOD_PIDS+=("$!")
}
ifb0_stat() { # <field: pkt|dropped> -- "... 0 pkt (dropped 0, ...": the pkt value is the field before, the dropped value the field after
    tc -s qdisc show dev ifb0 | awk -v f="$1" '
        /netem/ { getline;
            if (f=="pkt") { for(i=1;i<=NF;i++) if($i=="pkt"){ v=$(i-1); gsub(/[^0-9]/,"",v); print v; exit } }
            else          { for(i=1;i<=NF;i++) if($i=="(dropped"){ v=$(i+1); gsub(/[^0-9]/,"",v); print v; exit } }
        }'
}

echo "== case one (hit + hard proof): apply --iface vethA --dir in --loss 50 --rtp-port 5000 =="
flood 5000 90
sleep 1
"$BIN" apply --iface vethA --dir in --loss 50 --rtp-port 5000 --duration 60
echo "--- apply succeeded (the Inline verify's ifb0 leaf 1s Sent growth is itself the hit's leading evidence) ---"
sleep 3
SENT1=$(ifb0_stat pkt)
DROP1=$(ifb0_stat dropped)
echo "ifb0 leaf: Sent pkt=${SENT1:-∅} dropped=${DROP1:-∅}"
(( ${SENT1:-0} > 0 )) || { echo "FAIL hard proof: the ifb0 leaf Sent is zero (packets never traversed the mirror netem)"; exit 1; }
(( ${DROP1:-0} > 0 )) || { echo "FAIL: loss 50% produced no dropped"; exit 1; }

echo "== case two (control / no cross-hit): targeting = the 5000 leg; 6000-port traffic must not count into ifb0 =="
# stop the 5000 flood (kill by pid; pkill -f forbidden = rule #16's self-kill hazard)
for pid in "${FLOOD_PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
FLOOD_PIDS=()
sleep 0.5
S1=$(ifb0_stat pkt); S1=${S1:-0}
flood 6000 3
sleep 3.5
S2=$(ifb0_stat pkt); S2=${S2:-0}
echo "control window 3s (only 6000-port traffic): ifb0 Sent $S1 -> $S2 (should stay ~ unchanged = targeted hit, not an all-ports cross-hit)"
(( S2 - S1 < 100 )) || { echo "FAIL control: 6000-port traffic entered the 5000-leg mirror (cross-hit)"; exit 1; }
flood 5000 3
sleep 3.5
S3=$(ifb0_stat pkt); S3=${S3:-0}
echo "re-loss window 3s (5000-port traffic restored): ifb0 Sent $S2 -> $S3 (should grow markedly)"
(( S3 > S2 + 100 )) || { echo "FAIL re-loss: the 5000 leg no longer hits?"; exit 1; }

echo "== case three (clean teardown / zero residue): clear -> no ingress/ffff: lines AND ifb0 gone =="
for pid in "${FLOOD_PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
FLOOD_PIDS=()
"$BIN" clear
if tc qdisc show dev vethA | grep -qE 'ingress|ffff:'; then
    echo "FAIL teardown: vethA residue ingress/ffff: lines"; tc qdisc show dev vethA; exit 1
fi
if ip link show ifb0 >/dev/null 2>&1; then
    echo "FAIL teardown: this session's ifb0 was not deleted (the §ifb owner contract breached)"; exit 1
fi
echo "--- vethA final state ---"; tc qdisc show dev vethA
echo "### W4 all three cases pass (hit hard proof + targeted control + clean teardown zero residue) ###"
NETNS
