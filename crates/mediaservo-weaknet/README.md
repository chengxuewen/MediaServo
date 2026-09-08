# mediaservo-weaknet

tc/netem 弱网模拟 agent（**产品仓 dev 工具**）：双端面单二进制——server 面（CLI + `serve` 控制面 + 内嵌 Web 面板）
与车端面（`up|down|watch` + weaknet.yaml + 反锁死守卫）。**lib 面仅供单测（tower::oneshot / fixture 驱动），
不进 bindings/package/oxmgr**（W5 三判据审计）。契约源 = 主仓 `docs/plans/weaknet-agent/`（rev-2.2）。
