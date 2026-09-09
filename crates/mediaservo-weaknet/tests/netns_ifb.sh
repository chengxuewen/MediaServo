#!/usr/bin/env bash
# tests/netns_ifb.sh —— W4（design §ifb 三案双证）在 unshare -rn 网络命名空间内执行。
#
# 安全面：一切内核触达都在新 netns（user-ns root）内——宿主 ens32/lo 零触碰；命名空间
# 内建的 veth/ifb0 随末进程退出自动消亡。ifb 是内核模块支撑的 netdev：模块表全局，
# netns 帮不了忙——宿主未装载 ifb.ko 时 `ip link add ... type ifb` 必挂（本脚本据此
# 分叉：可建=全三案；不可建=负证据 transcript，exit 77 = BLOCKED-env 挂账）。
#
# 用法：WEAKNET_BIN=/path/to/mediaservo-weaknet bash tests/netns_ifb.sh
# 退出：0=W4 三案全过 · 77=环境不可构（负证据已打印）· 其余=案失败
set -u -o pipefail

BIN="${WEAKNET_BIN:-}"
[[ -z "$BIN" || ! -x "$BIN" ]] && { echo "FAIL: WEAKNET_BIN 未设或不可执行: '$BIN'"; exit 2; }
export BIN

command -v tc >/dev/null || { echo "FAIL: 宿主无 tc（iproute2）"; exit 2; }
command -v unshare >/dev/null || { echo "FAIL: 无 unshare"; exit 2; }
unshare -rn true || { echo "SKIP: unshare -rn 不可用（user ns 被禁？）——W4 BLOCKED-env"; exit 77; }

echo "== 环境: tc=$(tc -V) bin=$BIN =="
echo "== 宿主 /proc/modules ifb 行（预检，全局模块表）=="
grep '^ifb ' /proc/modules || echo "(无 ifb 模块行)"

unshare -rn bash -eu -o pipefail <<'NETNS'
echo "== 进入 unshare -rn（新 netns，user-ns root）=="
ip link set lo up

# ---- 分支判据：netns 内能否建 ifb 设备（= 宿主内核已装载 ifb.ko）----
if ! err=$(ip link add ifb0 type ifb 2>&1); then
    echo "### 负证据 transcript（W4 BLOCKED-env 证据件）###"
    echo "\$ ip link add ifb0 type ifb"
    echo "$err"
    echo "--- 再证：user-ns 内 modprobe 亦不可（模块装载非 namespace 化能力）---"
    echo "\$ modprobe ifb"
    modprobe ifb 2>&1 || true
    echo "### 结论：宿主未装载 ifb 模块 → W4 不闭环（tasks.md T9），复验触发条件=宿主 root modprobe ifb 后重跑本脚本 ###"
    exit 77
fi
echo "### ifb 可建（宿主模块在场）——删除预检半成品，走全三案 ###"
ip link del ifb0

# ---- 夹具：vethA 本侧 + vethB 移入 peer netns ----
# 同一 netns 两端互打走 local 短路，vethA ingress 永远无包（首轮 0→0 拓扑根因）；
# peer netns 的源地址非本地 → 真入向流量达 vethA ingress。
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

flood() { # flood <port> <secs> —— 从 vethB 侧打到 vethA:dport（= vethA 入向流量）
    timeout "$2" nsenter -t "$PEER" -n bash -c "while :; do echo weaknet-t9 > /dev/udp/10.99.0.1/$1 2>/dev/null || true; done" &
    FLOOD_PIDS+=("$!")
}
ifb0_stat() { # <field: pkt|dropped> —— "…0 pkt (dropped 0,…"：pkt 值在前一字段，dropped 值在下一字段
    tc -s qdisc show dev ifb0 | awk -v f="$1" '
        /netem/ { getline;
            if (f=="pkt") { for(i=1;i<=NF;i++) if($i=="pkt"){ v=$(i-1); gsub(/[^0-9]/,"",v); print v; exit } }
            else          { for(i=1;i<=NF;i++) if($i=="(dropped"){ v=$(i+1); gsub(/[^0-9]/,"",v); print v; exit } }
        }'
}

echo "== 案一（命中 + 铁证）：apply --iface vethA --dir in --loss 50 --rtp-port 5000 =="
flood 5000 90
sleep 1
"$BIN" apply --iface vethA --dir in --loss 50 --rtp-port 5000 --duration 60
echo "--- apply 成功（Inline verify 的 ifb0 leaf 1s Sent 增量本身即命中前证）---"
sleep 3
SENT1=$(ifb0_stat pkt)
DROP1=$(ifb0_stat dropped)
echo "ifb0 leaf: Sent pkt=${SENT1:-∅} dropped=${DROP1:-∅}"
(( ${SENT1:-0} > 0 )) || { echo "FAIL 铁证：ifb0 leaf Sent 为零（包未穿过镜像 netem）"; exit 1; }
(( ${DROP1:-0} > 0 )) || { echo "FAIL：loss 50% 未产生 dropped"; exit 1; }

echo "== 案二（对照/防连坐）：定向=5000 腿；6000 口流量不得计入 ifb0 =="
# 停 5000 洪泛（按 pid kill；禁 pkill -f=规则#16 自杀面）
for pid in "${FLOOD_PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
FLOOD_PIDS=()
sleep 0.5
S1=$(ifb0_stat pkt); S1=${S1:-0}
flood 6000 3
sleep 3.5
S2=$(ifb0_stat pkt); S2=${S2:-0}
echo "对照窗 3s（仅 6000 口流量）：ifb0 Sent $S1 → $S2（应≈不变=定向命中非全口连坐）"
(( S2 - S1 < 100 )) || { echo "FAIL 对照：6000 口流量进了 5000 腿镜像（连坐）"; exit 1; }
flood 5000 3
sleep 3.5
S3=$(ifb0_stat pkt); S3=${S3:-0}
echo "复损窗 3s（5000 口流量恢复）：ifb0 Sent $S2 → $S3（应显著增长）"
(( S3 > S2 + 100 )) || { echo "FAIL 复损：5000 腿不再命中？"; exit 1; }

echo "== 案三（撤净/零残留）：clear → 无 ingress/ffff: 行 ∧ ifb0 消亡 =="
for pid in "${FLOOD_PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
FLOOD_PIDS=()
"$BIN" clear
if tc qdisc show dev vethA | grep -qE 'ingress|ffff:'; then
    echo "FAIL 撤净：vethA 残留 ingress/ffff: 行"; tc qdisc show dev vethA; exit 1
fi
if ip link show ifb0 >/dev/null 2>&1; then
    echo "FAIL 撤净：本次所建 ifb0 未被删除（§ifb owner 合同失守）"; exit 1
fi
echo "--- vethA 终态 ---"; tc qdisc show dev vethA
echo "### W4 三案全过（命中铁证 + 定向对照 + 撤净零残留）###"
NETNS
