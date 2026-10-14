# mediaservo-weaknet

tc/netem weak-network emulation agent (**a product-repo dev tool**): dual-surface single binary -- server surface (CLI + the `serve` control plane + embedded web panel)
and vehicle surface (`up|down|watch` + weaknet.yaml + anti-deadlock guards). **The lib surface exists for unit tests only (tower::oneshot / fixture-driven),
stays out of bindings/package/oxmgr** (W5 three-predicate audit). Contract source = the parent repo `docs/plans/weaknet-agent/` (rev-2.2).
