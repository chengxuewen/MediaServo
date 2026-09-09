#!/usr/bin/env bash
# tests/netns_vehicle.sh —— W8 车端面替身验收（tasks.md T12/T13）：unshare -rn netns 内
# veth 拓扑跑通「weaknet.yaml → apply（影响面/反锁死钉）→ watch → down 零残留」全链。
#
# 安全面：一切内核触达在新 netns（user-ns root）——宿主 ens32/lo 零触碰；拓扑随进程退出消亡。
# WEAKNET_STATEDIR 全程钉 $WORK（cleanup 的 clear 永不回落生产 out/.weaknet）。
# 拓扑复用 tests/netns_ifb.sh 实证形（peer netns 出向真流量，非 local 短路）。
#
# 反锁死断言替身法（注记：真实面=apply 期 ssh 连续往返 + getent hosts 解析，需上车日补）：
#   ① 协议腿排除（活证）：施加期 `ping` ICMP（protocol 1≠17）0% 丢包零延迟——
#      TCP/SSH 结构性不进 netem 的内核面等价证；TCP 临时端口碰撞的「filter 必带
#      protocol 17 合取」面已由 spec.rs::legs_anti_lockout_pins 单元测试钉死（双保险）。
#   ② 枚举排除（定量）：非枚举口 UDP(53) 流量 leaf 计数零计入（Δ≈0）——「端口集来自
#      weaknet.yaml 枚举」的活证 + L1b 物理口无 ports 直接 exit2 报因（规则层枚举门）。
# 换掉早期 TCP 源口探针方案的原因：socket 握手时序引入 flake，证明力与 ① ping 等价
# （同为「非 UDP 不进叶」），协议维度另有单测钉——按等价强度取零构造方案。
#
# 用法：WEAKNET_BIN=/path/to/mediaservo-weaknet bash tests/netns_vehicle.sh
# 退出：0=全案过 · 77=环境不可构（无 unshare/userns）· 其余=案失败
set -u -o pipefail

BIN="${WEAKNET_BIN:-}"
[[ -z "$BIN" || ! -x "$BIN" ]] && { echo "FAIL: WEAKNET_BIN 未设或不可执行: '$BIN'"; exit 2; }
export BIN
command -v tc >/dev/null || { echo "FAIL: 宿主无 tc（iproute2）"; exit 2; }
command -v unshare >/dev/null || { echo "FAIL: 无 unshare"; exit 2; }
command -v python3 >/dev/null || { echo "FAIL: 无 python3（UDP 流量源）"; exit 2; }
command -v ping >/dev/null || { echo "FAIL: 无 ping（协议排除活证探针）"; exit 2; }
unshare -rn true || { echo "SKIP: unshare -rn 不可用——W8 BLOCKED-env"; exit 77; }

echo "== 环境: tc=$(tc -V) $(python3 -V) bin=$BIN =="

# L0：--version（scp 即用链的自证面，无内核触达）
"$BIN" --version || { echo "FAIL L0: --version"; exit 1; }

unshare -rn bash -eu -o pipefail <<'NETNS'
echo "== 进入 unshare -rn（新 netns，user-ns root）=="
ip link set lo up

# ---- 夹具：vethA 本侧（10.99.0.1）+ vethB 移入 peer netns（10.99.0.2，ARP/回程承载）----
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
# 定向链去歧义：stats/凭证/env-iface/资产目录全部清空 → 解析只可能来自 --config/--rtp-port
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

# 流量源（vethA egress，源口可控）——dir=out 腿按 sport∈P 匹配
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
leaf_stat() { # <field: pkt|dropped> —— vethA parent 1:10 netem 叶（解析形同 netns_ifb.sh）
    tc -s qdisc show dev vethA | awk -v f="$1" '
        /netem/ { getline;
            if (f=="pkt") { for(i=1;i<=NF;i++) if($i=="pkt"){ v=$(i-1); gsub(/[^0-9]/,"",v); print v; exit } }
            else          { for(i=1;i<=NF;i++) if($i=="(dropped"){ v=$(i+1); gsub(/[^0-9]/,"",v); print v; exit } }
        }'
}

# ---- L1 反锁死规则层负案（serde 严格 + 物理口枚举门）----
printf 'portz: [1]\n' > "$WORK/badkey.yaml"
rc=$("$BIN" apply --config "$WORK/badkey.yaml" 2>&1; echo "RC:$?") || true
grep -q "RC:4" <<<"$rc" || { echo "FAIL L1a: 未知键应 exit4，得: $rc"; exit 1; }
grep -q "iface.*ports.*duration.*spec" <<<"$rc" || { echo "FAIL L1a: 报因需列合法键集，得: $rc"; exit 1; }
printf 'iface: vethA\nduration: 60\nspec:\n  rtt_ms: 80\n' > "$WORK/noports.yaml"
rc=$("$BIN" apply --config "$WORK/noports.yaml" 2>&1; echo "RC:$?") || true
grep -q "RC:2" <<<"$rc" || { echo "FAIL L1b: 物理口无 ports 应 exit2，得: $rc"; exit 1; }
grep -q "反锁死" <<<"$rc" || { echo "FAIL L1b: 报因缺枚举指引（反锁死）：$rc"; exit 1; }
echo "### L1 反锁死负案双过（未知键 exit4 列合法集 · 物理口无 ports exit2 报因）###"

# ---- L2 主链：yaml→apply（dir=out 端口腿）+ 命中/排除定量窗 ----
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
# 两路流量先行：5000-UDP（命中腿）/ 53-UDP（枚举外对照）
udp_sport 5000 180
udp_sport 53 180
sleep 1
OUT=$("$BIN" apply --config "$WORK/weaknet.yaml" 2>&1) || { echo "FAIL L2: apply 失败: $OUT"; exit 1; }
echo "$OUT"
grep -q "影响面: vethA/UDP/5000/倒计时 60s" <<<"$OUT" || { echo "FAIL L2: 影响面横幅缺失"; exit 1; }
grep -q "回读指纹过" <<<"$OUT" || { echo "FAIL L2: 指纹断言缺失"; exit 1; }
sleep 2
H1=$(leaf_stat pkt); H1=${H1:-0}
sleep 3
H2=$(leaf_stat pkt); H2=${H2:-0}
D1=$(leaf_stat dropped); D1=${D1:-0}
echo "命中窗 3s：vethA leaf Sent $H1 → $H2（Δ=$((H2 - H1))，dropped=$D1）"
(( H2 - H1 > 1000 )) || { echo "FAIL L2 命中：5000-UDP 腿未进 netem（Δ=$((H2 - H1))）"; exit 1; }
(( D1 > 0 )) || { echo "FAIL L2 命中：loss 10% 未产生 dropped"; exit 1; }

echo "== 排除窗：停 5000-UDP，仅 53-UDP 在跑 + 施加期 ping（ICMP 协议排除活证）=="
kill "${PIDS[0]}" 2>/dev/null || true
wait "${PIDS[0]}" 2>/dev/null || true
sleep 0.5
C1=$(leaf_stat pkt); C1=${C1:-0}
PINGOUT=$(ping -c 3 -W 1 10.99.0.2 2>&1) || true
sleep 2
C2=$(leaf_stat pkt); C2=${C2:-0}
echo "排除窗：leaf Sent $C1 → $C2（Δ=$((C2 - C1))，期望≈0）"
echo "--- ping（施加期，损伤含 80ms/10% loss 却不沾 ICMP）---"
tail -2 <<<"$PINGOUT"
grep -q " 0% packet loss" <<<"$PINGOUT" || { echo "FAIL L2①：施加期 ping 出现丢包/不通（协议腿排除失守？）"; exit 1; }
grep -q "rtt" <<<"$PINGOUT" || { echo "FAIL L2①：ping 未达（RTT 行缺失）——检查拓扑"; cat <<<"$PINGOUT"; exit 1; }
(( C2 - C1 < 100 )) || { echo "FAIL L2②：枚举外/非 UDP 流量进叶（Δ=$((C2 - C1))）——反锁死失守"; exit 1; }
echo "### L2 三证过（命中 Δ>1000+dropped>0 · ICMP 排除活证 0% loss · 枚举外 UDP Δ<100）###"

# ---- L3 watch → down（state 清除 → cleared → exit0）----
COLUMNS=100 timeout -s INT 12 "$BIN" watch > "$WORK/watch.out" 2>&1 &
WPID=$!
sleep 2.5
"$BIN" down
if wait "$WPID"; then WRC=0; else WRC=$?; fi
echo "--- watch 帧（前 8 行）---"
head -8 "$WORK/watch.out"
(( WRC == 0 )) || { echo "FAIL L3: watch 未随 down 自退（rc=$WRC）"; cat "$WORK/watch.out"; exit 1; }
grep -q "weaknet watch" "$WORK/watch.out" || { echo "FAIL L3: watch 帧头缺失"; exit 1; }
grep -qE "leaf sent=[1-9][0-9]*" "$WORK/watch.out" || { echo "FAIL L3: 计数行缺失或恒零（叶读取坏）"; head -8 "$WORK/watch.out"; exit 1; }
grep -q "cleared" "$WORK/watch.out" || { echo "FAIL L3: cleared 判据行缺失"; exit 1; }
tail -2 "$WORK/watch.out"
echo "### L3 过（一屏重绘渲染 + state 清除→cleared→exit0）###"

# ---- L4 内嵌 profile 兜底（scp 单文件替身）：二进制离仓复制 + 仅二进制同级 weaknet.yaml ----
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
    || { echo "FAIL L4: 内嵌 up smoke 未成: $EOUT"; exit 1; }
( cd "$EMBDIR" && WEAKNET_ASSETS_DIR="$EMBDIR/empty.d" ./msrtc-weaknet down ) >/dev/null
export WEAKNET_STATEDIR="$STATEDIR"
echo "### L4 过（assets=空目录仍解析内嵌 smoke + 二进制同级 yaml 探测 + up/down 别名）###"

# ---- L5 撤净零残留 ----
if tc qdisc show dev vethA | grep -qE 'ffff:|netem'; then
    echo "FAIL L5: vethA 残留"; tc qdisc show dev vethA; exit 1
fi
if tc qdisc show dev lo | grep -qE 'ffff:|netem'; then
    echo "FAIL L5: lo 残留"; tc qdisc show dev lo; exit 1
fi
[[ -f "$STATEDIR/state.json" || -f "$WORK/state-lo/state.json" ]] && { echo "FAIL L5: state 残留"; exit 1; }
echo "### W8 替身全链过（L0 version · L1 反锁死负案 · L2 定量 · L3 watch/down · L4 内嵌 · L5 零残留）###"
NETNS
