# Docker & Network Constraints

> Split from [constraints.md](constraints.md) per D202 (OpenCode config optimization).
> This file: Docker, network, and container-specific constraints for MediaServo development.

## Docker Constraints

### Docker Desktop Volume Mount Performance
- Docker Desktop on macOS uses **osxfs (legacy) or virtiofs (newer)** for bind mounts. Both are **3-5x slower** than native Linux filesystem access.
- The `cargo-cache` named volume in `docker-compose.yml` mitigates this for dependency downloads, but **workspace source code bind mounts are still slow**.
- **Mitigation**: Prefer `docker compose exec` for running cargo commands inside the container rather than relying on host-side tooling. Avoid running `cargo build` from a host-mounted volume for large builds — use the container's internal filesystem or a dedicated volume.
- First-time `cargo build` with `sfu-mediasoup` inside Docker can take **15-30 minutes** (vs. 3-5 minutes for native Linux).

## Network Constraints

### UDP Ports for mediasoup
- The delivery model uses **single-port mux on 20000/udp** (rtc single-port); the legacy 40000-40100/udp mapping was REMOVED from `docker-compose.yml` (revival recipe: deployment manual Appendix B).
- When deploying outside Docker, allow 20000/udp in the host firewall; a legacy `rtc_ports_range: (40000, 40100)` deployment still needs that range allowed.

### ICE/STUN Required for Local P2P
- WebRTC (even on localhost) requires **ICE negotiation** with STUN to discover candidate pairs. Without a STUN server, localhost WebRTC connections fail because no candidate pairs are formed.
- **Development setup**: Run a STUN server (e.g., `coturn` or `stuntman`) locally, or configure the WebRTC transport to use a host-loopback ICE candidate.
- **Common gotcha**: Host and Client on the same machine assume localhost WebRTC "just works" — it does not. ICE must be configured explicitly even for loopback connections.
- mediasoup's `WebRtcTransport` uses ICE-Lite (server-side) by default, which reduces the ICE handshake to one round trip but still requires the client to send a STUN binding request.

## See Also

- [constraints.md](constraints.md) — Git commit rules
- [platform.md](platform.md) — macOS + Linux platform constraints
- `docs/modules/development/docker-workflow.md` — Docker dev workflow details
