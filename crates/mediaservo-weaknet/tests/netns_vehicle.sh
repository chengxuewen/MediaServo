#!/usr/bin/env bash
# tests/netns_vehicle.sh -- the W8 vehicle-surface stand-in acceptance (tasks.md T12/T13): inside an unshare -rn netns,
# a veth topology exercises the full chain "weaknet.yaml -> apply (impact/anti-deadlock pins) -> watch -> down with zero residue".
#
# Safety: all kernel touching happens in a new netns (user-ns root) -- the host's ens32/lo is never touched; the topology dies with the process.
# WEAKNET_STATEDIR pinned to $WORK throughout (cleanup's clear can never fall back to the production out/.weaknet).
# Topology reuses the netns_ifb.sh proven shape (peer-netns outbound real traffic, not a local short-circuit).
#
# Anti-deadlock assertion stand-in method (note: the real surface = continuous ssh round-trips during apply + getent hosts resolution, to be added on vehicle day):
#   (1) protocol-leg exclusion (live evidence): during apply, `ping` over ICMP (protocol 1 != 17) with 0% loss and zero added delay --
#      the kernel-surface equivalent proof that TCP/SSH structurally never enters netem; the "every filter must carry the
#      protocol 17 conjunction" surface for TCP ephemeral-port collisions is already pinned by spec.rs::legs_anti_lockout_pins unit tests (double insurance).
#   (2) enumeration exclusion (quantified): UDP(53) traffic on a non-enumerated port counts zero at the leaf (delta ~ 0) --
#      live proof that "the port set comes from the weaknet.yaml enumeration" + L1b physical-port-without-ports direct exit2 cause (the rule-layer enumeration gate).
# Why the early TCP source-port probe scheme was dropped: socket handshake timing introduced flakes; its evidentiary strength equals (1)'s ping
# (both prove "non-UDP never reaches the leaf"), and the protocol dimension is separately pinned by unit tests -- the zero-construction scheme wins on equal strength.
#
# Usage: WEAKNET_BIN=/path/to/mediaservo-weaknet bash tests/netns_vehicle.sh
# Exit: 0 = all cases pass, 77 = environment cannot be constructed (no unshare/userns), otherwise = case failure
set -u -o pipefail

BIN="${WEAKNET_BIN:-}"
[[ -z "$BIN" || ! -x "$BIN" ]] && { echo "FAIL: WEAKNET_BIN unset or not executable: '$BIN'"; exit 2; }
export BIN
command -v tc >/dev/null || { echo "FAIL: no tc (iproute2) on the host"; exit 2; }
command -v unshare >/dev/null || { echo "FAIL: no unshare"; exit 2; }
command -v python3 >/dev/null || { echo "FAIL: no python3 (the UDP traffic source)"; exit 2; }
command -v ping >/dev/null || { echo "FAIL: no ping (the protocol-exclusion live-evidence probe)"; exit 2; }
unshare -rn true || { echo "SKIP: unshare -rn unavailable -- W8 BLOCKED-env"; exit 77; }

echo "== environment: tc=$(tc -V) $(python3 -V) bin=$BIN =="

# L0: --version (the scp-and-go chain's self-proof surface, zero kernel touching)
"$BIN" --version || { echo "FAIL L0: --version"; exit 1; }

unshare -rn bash -eu -o pipefail <<'NETNS'
echo "== entering unshare -rn (new netns, user-ns root) =="
ip link set lo up

# ---- fixture: vethA local side (10.99.0.1) + vethB moved into the peer netns (10.99.0.2, carries ARP/return traffic) ----
unshare -n sleep 400 & PEER=$!
sleep 0.3
ip link add vethA type veth peer name vethB
ip addr add 10.99.0.1/24 dev vethA
ip link set vethA up
ip link set vethB netns "$PEER"
nsenter -t "$PEER" -n ip addr add 10.99.0.2/24 dev vethB
nsenter -t "$PEER" -n ip link set vethB up

WORK=$(mktemp -d /tmp/wnet-vehicle.XXXXXX)
STATEDIR="$WORK/state"
mkdir -p "$STATEDIR"
export WEAKNET_STATEDIR="$STATEDIR"
export WEAKNET_CHANNEL=local
# targeting-chain disambiguation: stats/credentials/env-iface/asset dirs all cleared -> resolution can only come from --config/--rtp-port
export WEAKNET_SERVER_URL="" WEAKNET_ADMIN_PASS="" WEAKNET_DEV="" WEAKNET_ASSETS_DIR=""
PIDS=()
cleanup() {
    export WEAKNET_STATEDIR="$STATEDIR"
    for pid in "${PIDS[@]:-}"; do [[ -n "${pid:-}" ]] && kill "$pid" 2>/dev/null || true; done
    "$BIN" clear >/dev/null 2>&1 || true
    kill "${PEER:-0}" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

# traffic source (vethA egress, controllable source port) -- the dir=out legs match on sport in P
udp_sport() { # udp_sport <bindport> <secs>
    timeout "$2" python3 -c '
import socket, time, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("10.99.0.1", int(sys.argv[1])))
end = time.time() + float(sys.argv[2])
while time.time() < end:
    s.sendto(b"weaknet-vehicle", ("10.99.0.2", 47))
' "$1" "$2" &
    PIDS+=("$!")
}
leaf_stat() { # <field: pkt|dropped> -- the vethA parent 1:10 netem leaf (same parse shape as netns_ifb.sh)
    tc -s qdisc show dev vethA | awk -v f="$1" '
        /netem/ { getline;
            if (f=="pkt") { for(i=1;i<=NF;i++) if($i=="pkt"){ v=$(i-1); gsub(/[^0-9]/,"",v); print v; exit } }
            else          { for(i=1;i<=NF;i++) if($i=="(dropped"){ v=$(i+1); gsub(/[^0-9]/,"",v); print v; exit } }
        }'
}

# ---- L1 anti-deadlock rule-layer negatives (serde strictness + the physical-port enumeration gate) ----
printf 'portz: [1]\n' > "$WORK/badkey.yaml"
rc=$("$BIN" apply --config "$WORK/badkey.yaml" 2>&1; echo "RC:$?") || true
grep -q "RC:4" <<<"$rc" || { echo "FAIL L1a: unknown key should exit4, got: $rc"; exit 1; }
grep -q "iface.*ports.*duration.*spec" <<<"$rc" || { echo "FAIL L1a: the cause must list the legal key set, got: $rc"; exit 1; }
printf 'iface: vethA\nduration: 60\nspec:\n  rtt_ms: 80\n' > "$WORK/noports.yaml"
rc=$("$BIN" apply --config "$WORK/noports.yaml" 2>&1; echo "RC:$?") || true
grep -q "RC:2" <<<"$rc" || { echo "FAIL L1b: physical port without ports should exit2, got: $rc"; exit 1; }
grep -q "anti-deadlock" <<<"$rc" || { echo "FAIL L1b: the cause lacks enumeration guidance (anti-deadlock): $rc"; exit 1; }
echo "### L1 anti-deadlock negatives both pass (unknown key exit4 lists the legal set · physical port without ports exit2 cause) ###"

# ---- L2 main chain: yaml -> apply (dir=out port legs) + hit/exclusion quantified windows ----
cat > "$WORK/weaknet.yaml" <<'YAML'
iface: vethA
ports: [5000]
duration: 60
spec:
  rtt_ms: 80
  jitter_ms: 15
  loss: 10%
  dir: out
YAML
# two traffic paths first: 5000-UDP (hits a leg) / 53-UDP (the non-enumerated control)
udp_sport 5000 180
udp_sport 53 180
sleep 1
OUT=$("$BIN" apply --config "$WORK/weaknet.yaml" 2>&1) || { echo "FAIL L2: apply failed: $OUT"; exit 1; }
echo "$OUT"
grep -q "impact surface: vethA/UDP/5000/countdown 60s" <<<"$OUT" || { echo "FAIL L2: the impact-surface banner is missing"; exit 1; }
grep -q "read-back fingerprint passed" <<<"$OUT" || { echo "FAIL L2: the fingerprint assert is missing"; exit 1; }
sleep 2
H1=$(leaf_stat pkt); H1=${H1:-0}
sleep 3
H2=$(leaf_stat pkt); H2=${H2:-0}
D1=$(leaf_stat dropped); D1=${D1:-0}
echo "hit window 3s: vethA leaf Sent $H1 -> $H2 (delta=$((H2 - H1)), dropped=$D1)"
(( H2 - H1 > 1000 )) || { echo "FAIL L2 hit: the 5000-UDP leg never entered netem (delta=$((H2 - H1)))"; exit 1; }
(( D1 > 0 )) || { echo "FAIL L2 hit: loss 10% produced no dropped"; exit 1; }

echo "== exclusion window: stop 5000-UDP, only 53-UDP running + ping during apply (the ICMP protocol-exclusion live evidence) =="
kill "${PIDS[0]}" 2>/dev/null || true
wait "${PIDS[0]}" 2>/dev/null || true
sleep 0.5
C1=$(leaf_stat pkt); C1=${C1:-0}
PINGOUT=$(ping -c 3 -W 1 10.99.0.2 2>&1) || true
sleep 2
C2=$(leaf_stat pkt); C2=${C2:-0}
echo "exclusion window: leaf Sent $C1 -> $C2 (delta=$((C2 - C1)), expected ~ 0)"
echo "--- ping (during apply; the impairment has 80ms/10% loss yet ICMP untouched) ---"
tail -2 <<<"$PINGOUT"
grep -q " 0% packet loss" <<<"$PINGOUT" || { echo "FAIL L2(1): ping during apply shows loss/unreachable (protocol-leg exclusion breached?)"; exit 1; }
grep -q "rtt" <<<"$PINGOUT" || { echo "FAIL L2(1): ping did not get through (the RTT line missing) -- check the topology"; cat <<<"$PINGOUT"; exit 1; }
(( C2 - C1 < 100 )) || { echo "FAIL L2(2): non-enumerated/non-UDP traffic reached the leaf (delta=$((C2 - C1))) -- anti-deadlock breached"; exit 1; }
echo "### L2 three proofs pass (hit delta>1000+dropped>0 · ICMP exclusion live evidence 0% loss · non-enumerated UDP delta<100) ###"

# ---- L3 watch -> down (state cleared -> cleared -> exit0) ----
COLUMNS=100 timeout -s INT 12 "$BIN" watch > "$WORK/watch.out" 2>&1 &
WPID=$!
sleep 2.5
"$BIN" down
if wait "$WPID"; then WRC=0; else WRC=$?; fi
echo "--- watch frame (first 8 lines) ---"
head -8 "$WORK/watch.out"
(( WRC == 0 )) || { echo "FAIL L3: watch did not exit by itself after down (rc=$WRC)"; cat "$WORK/watch.out"; exit 1; }
grep -q "weaknet watch" "$WORK/watch.out" || { echo "FAIL L3: the watch frame header is missing"; exit 1; }
grep -qE "leaf sent=[1-9][0-9]*" "$WORK/watch.out" || { echo "FAIL L3: the counter line is missing or stuck at zero (leaf read broken)"; head -8 "$WORK/watch.out"; exit 1; }
grep -q "cleared" "$WORK/watch.out" || { echo "FAIL L3: the cleared predicate line is missing"; exit 1; }
tail -2 "$WORK/watch.out"
echo "### L3 pass (single-screen redraw rendering + state cleared -> cleared -> exit0) ###"

# ---- L4 embedded profile fallback (the scp single-file stand-in): binary copied out of the repo + only a weaknet.yaml next to the binary ----
EMBDIR="$WORK/embed"
mkdir -p "$EMBDIR" "$EMBDIR/empty.d"
cp "$BIN" "$EMBDIR/msrtc-weaknet"
printf 'ports: [5000]\n' > "$EMBDIR/weaknet.yaml"
timeout 30 python3 -c '
import socket, time
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("127.0.0.1", 5000))
end = time.time() + 29
while time.time() < end:
    s.sendto(b"x", ("127.0.0.1", 47))
' >/dev/null 2>&1 &
PIDS+=("$!")
sleep 0.5
export WEAKNET_STATEDIR="$WORK/state-lo"
EOUT=$( cd "$EMBDIR" && WEAKNET_ASSETS_DIR="$EMBDIR/empty.d" ./msrtc-weaknet up smoke --duration 30 2>&1 ) \
    && grep -q "applied: dev=lo" <<<"$EOUT" \
    || { echo "FAIL L4: the embedded up smoke failed: $EOUT"; exit 1; }
( cd "$EMBDIR" && WEAKNET_ASSETS_DIR="$EMBDIR/empty.d" ./msrtc-weaknet down ) >/dev/null
export WEAKNET_STATEDIR="$STATEDIR"
echo "### L4 pass (assets=empty dir still resolves embedded smoke + next-to-binary yaml probing + the up/down aliases) ###"

# ---- L5 teardown leaves zero residue ----
if tc qdisc show dev vethA | grep -qE 'ffff:|netem'; then
    echo "FAIL L5: vethA residue"; tc qdisc show dev vethA; exit 1
fi
if tc qdisc show dev lo | grep -qE 'ffff:|netem'; then
    echo "FAIL L5: lo residue"; tc qdisc show dev lo; exit 1
fi
[[ -f "$STATEDIR/state.json" || -f "$WORK/state-lo/state.json" ]] && { echo "FAIL L5: state residue"; exit 1; }
echo "### W8 stand-in full chain pass (L0 version · L1 anti-deadlock negatives · L2 quantified · L3 watch/down · L4 embedded · L5 zero residue) ###"
NETNS
