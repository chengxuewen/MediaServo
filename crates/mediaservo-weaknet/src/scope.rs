//! scope.rs -- lightweight stats client (std::net plaintext HTTP/1.1 mini client; design §Dependencies
//! dropped-reqwest ruling: ~40 lines of GET/POST, zero new dependencies) + stream/device targeting (§dir leg table rev-2.2:
//! Stream/Device = the ordered-pair (local, remote) set, Device flattened after owner grouping).
//!
//! Truth source: `scripts/weaknet.sh` (api_get/login flow, media_ports/stream_ports parsing,
//! exit2 wording) + the server `admin.rs::sfu_stats` list-mode wire shape (knife C: rows carry +owner).
//! Liveness predicate: list mode is already filtered server-side by ICE tuple ("in the table = alive", PIT-185); client-side
//! `live` = the transport_id field present (a tuple-derived always-present field) -- defends against non-list shapes/future drift.
//!
//! Exit codes: parse/empty target/unreachable URL = exit2 (Fail::env); mutually exclusive params = exit4 (bad_param).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rust_embed::Embed;
use serde::Deserialize;

use crate::engine::{Env, Fail, Wn, probe_channel, tc_exec};
use crate::spec::ScopeSel;

/// In-process login token cache window (the weaknet tool is short-lived, 5min suffices; re-login on expiry).
const TOKEN_TTL: Duration = Duration::from_secs(300);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// C20 exemption shape: host loopback for the yaml-probed default port (cross-machine uses --server-url/env).
const PROBE_HOST: &str = "127.0.0.1";

// ---------- wire shape ----------

/// One stats list-mode row (a transport). remote_ports folded from remote_port
/// (design §API /v1/streams aggregate shape; current server tuples are single-valued -> 0 or 1 elements).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    pub room: String,
    /// Knife C: room-registered device ID; old servers without the field / without an owner room = None.
    pub owner: Option<String>,
    pub peer_id: String,
    /// "producer" | "consumer"
    pub role: String,
    /// "video" | "audio"
    pub kind: String,
    /// The transport observation row exists (tuple-derived field) = alive.
    pub live: bool,
    pub local_port: Option<u16>,
    pub remote_ports: Vec<u16>,
    /// stats row byte counter (the serve state-frame kbps differential source; old servers/missing field = None -- never fake 0).
    pub byte_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawRow {
    #[serde(default)]
    room: String,
    /// Old servers (pre knife C) lack the key; new servers with no owner room are null -- both shapes map to None.
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    peer_id: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    transport_id: Option<String>,
    #[serde(default)]
    local_port: Option<u16>,
    #[serde(default)]
    remote_port: Option<u16>,
    #[serde(default)]
    byte_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct StatsBody {
    #[serde(default)]
    streams: Vec<RawRow>,
}

/// Response body -> typed rows (pure function, the unit-test fixture surface; decoupled from fetching).
pub fn parse_streams_body(body: &str) -> Wn<Vec<StreamInfo>> {
    let v: StatsBody = serde_json::from_str(body).map_err(|e| {
        Fail::env(format!(
            "stats JSON parse failed: {e} (first {} bytes of body)",
            prefix_excerpt(body)
        ))
    })?;
    Ok(v.streams
        .into_iter()
        .map(|r| StreamInfo {
            room: r.room,
            owner: r.owner.filter(|s| !s.is_empty()),
            peer_id: r.peer_id,
            role: r.role,
            kind: r.kind,
            live: r.transport_id.is_some(),
            local_port: r.local_port,
            remote_ports: r.remote_port.into_iter().collect(),
            byte_count: r.byte_count,
        })
        .collect())
}

// ---------- targeting (§dir table: pairs = ordered (local_port, remote_port) pairs) ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Targeting {
    pub scope: ScopeSel,
    /// scope=Media: the local media port set
    pub ports: Vec<u16>,
    /// scope=Stream/Device: the flattened ordered-pair set (deduped, ascending)
    pub pairs: Vec<(u16, u16)>,
    /// E2 (T8): targeted names -- Stream = room set / Device = device ID set / Media = empty.
    /// Enters state via ApplyRequest.names, feeding the SSE-frame checkbox echo (panel) and the apply event stream key (bash minor account).
    pub names: Vec<String>,
}

impl Targeting {
    /// crate::engine::ScopeNames mapping (Stream->rooms / Device->devices / Media->empty) -- shared source for CLI/serve/scenario.
    #[must_use]
    pub fn scope_names(&self) -> crate::engine::ScopeNames {
        match self.scope {
            ScopeSel::Stream => {
                crate::engine::ScopeNames { rooms: self.names.clone(), devices: vec![] }
            }
            ScopeSel::Device => {
                crate::engine::ScopeNames { rooms: vec![], devices: self.names.clone() }
            }
            ScopeSel::Media => crate::engine::ScopeNames::default(),
        }
    }
}

/// One row -> (L,R) pair (either port missing = not targetable, WARN on record C15).
fn row_pairs(row: &StreamInfo, out: &mut Vec<(u16, u16)>) {
    let Some(l) = row.local_port else {
        eprintln!(
            "weaknet(scope): WARN room {} {} row missing local_port (tuple without a local port? skipping)",
            row.room, row.role
        );
        return;
    };
    for &r in &row.remote_ports {
        out.push((l, r));
    }
}

fn finish_pairs(mut pairs: Vec<(u16, u16)>) -> Vec<(u16, u16)> {
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

/// List of usable live rooms (for the exit2 cause; same slot as bash stream_rooms_available).
pub fn live_rooms_csv(rows: &[StreamInfo]) -> String {
    let mut rooms: Vec<&str> = rows.iter().filter(|s| s.live).map(|s| s.room.as_str()).collect();
    rooms.sort_unstable();
    rooms.dedup();
    if rooms.is_empty() { "?".into() } else { rooms.join(",") }
}

/// --stream: comma-separated room names -> an (L,R) pair per live row. Empty set exit2 + the live-room list.
pub fn targeting_for_rooms(rows: &[StreamInfo], sel: &str) -> Wn<Targeting> {
    let want: Vec<&str> = sel.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if want.is_empty() {
        return Err(Fail::bad_param("--stream needs stream/room names (comma-separated)"));
    }
    let mut pairs = Vec::new();
    for row in rows.iter().filter(|s| s.live && want.contains(&s.room.as_str())) {
        row_pairs(row, &mut pairs);
    }
    let pairs = finish_pairs(pairs);
    if pairs.is_empty() {
        return Err(Fail::env(format!(
            "streams [{sel}] have no live transport (not connected/name wrong; available rooms: {})",
            live_rooms_csv(rows)
        )));
    }
    Ok(Targeting {
        scope: ScopeSel::Stream,
        ports: vec![],
        pairs,
        names: want.iter().map(|s| (*s).to_string()).collect(),
    })
}

/// --device: comma-separated owner devices -> the (L,R) union over all live rows (producer+consumer) of their rooms.
/// **peer_id fallback**: when the owner field is missing (a running server older than knife C), room attribution is decided by
/// role==producer AND peer_id==device (the signaling peer_id = device ID, same source as the signaling
/// room_owners) -- every fallback hit logs a WARN (degradation made visible, C15). Empty set exit2 + the room list.
pub fn targeting_for_devices(rows: &[StreamInfo], sel: &str) -> Wn<Targeting> {
    let want: Vec<&str> = sel.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if want.is_empty() {
        return Err(Fail::bad_param("--device needs device IDs (comma-separated)"));
    }
    let mut owned_rooms: Vec<String> = Vec::new();
    let mut fallback_hits = 0usize;
    for row in rows.iter().filter(|s| s.live) {
        let owner_match = row.owner.as_deref().is_some_and(|o| want.contains(&o));
        let fallback = !owner_match
            && row.owner.is_none()
            && row.role == "producer"
            && want.contains(&row.peer_id.as_str());
        if (owner_match || fallback) && !owned_rooms.contains(&row.room) {
            owned_rooms.push(row.room.clone());
        }
        if fallback {
            fallback_hits += 1;
        }
    }
    if fallback_hits > 0 {
        eprintln!(
            "weaknet(scope): WARN --device matched {fallback_hits} rows via peer_id fallback (owner field missing: server predates knife C or the room has no owner registered)"
        );
    }
    let mut pairs = Vec::new();
    for row in rows.iter().filter(|s| s.live && owned_rooms.contains(&s.room)) {
        row_pairs(row, &mut pairs);
    }
    let pairs = finish_pairs(pairs);
    if pairs.is_empty() {
        return Err(Fail::env(format!(
            "device [{sel}] has no live streams (both owner/peer_id matching came up empty; available rooms: {})",
            live_rooms_csv(rows)
        )));
    }
    Ok(Targeting {
        scope: ScopeSel::Device,
        ports: vec![],
        pairs,
        names: want.iter().map(|s| (*s).to_string()).collect(),
    })
}

/// room level: the union of live-row local_ports (bash media_ports synonym). Empty set exit2 + the bash L518 wording.
pub fn targeting_media(rows: &[StreamInfo]) -> Wn<Targeting> {
    let mut ports: Vec<u16> = rows.iter().filter(|s| s.live).filter_map(|s| s.local_port).collect();
    ports.sort_unstable();
    ports.dedup();
    if ports.is_empty() {
        return Err(Fail::env(
            "media port observation empty: server not up / no producing streams / stats unreachable (start the service or retry; escape hatch --rtp-port; or target with --stream)",
        ));
    }
    Ok(Targeting { scope: ScopeSel::Media, ports, pairs: vec![], names: vec![] })
}

/// Scope adjudication priority (bash resolve_ports order): explicit port set > stream/rooms > device/ids > stats media ports.
/// `stream="all"` = explicit back to room level (bash equivalent). CLI and serve REST share the core (extracted from main.rs at T6, no copies).
/// stats is touched only when needed (the explicit-port escape hatch stays usable with zero server -- offline/no-credential scenarios).
pub fn resolve_targeting(
    server_url: Option<&str>,
    stream: Option<&str>,
    device: Option<&str>,
    ports_flag: &[u16],
) -> Wn<Targeting> {
    if !ports_flag.is_empty() {
        if stream.is_some() || device.is_some() {
            eprintln!(
                "weaknet: WARN --rtp-port given together with --stream/--device: the explicit port set wins (targeting ignored)"
            );
        }
        return Ok(Targeting {
            scope: ScopeSel::Media,
            ports: ports_flag.to_vec(),
            pairs: vec![],
            names: vec![],
        });
    }
    let stream = stream.map(str::trim).filter(|s| !s.is_empty());
    let device = device.map(str::trim).filter(|s| !s.is_empty());
    if stream.is_some() && device.is_some() {
        return Err(Fail::bad_param(
            "--stream and --device are mutually exclusive (pick one targeting scope)",
        ));
    }
    let url = server_url_or_default(server_url)?;
    let mut client = StatsClient::from_env(&url)?;
    let rows = client.fetch_streams()?;
    if let Some(s) = stream.filter(|s| *s != "all") {
        return targeting_for_rooms(&rows, s);
    }
    if let Some(d) = device {
        return targeting_for_devices(&rows, d);
    }
    targeting_media(&rows)
}

/// profile/scenario asset resolution (§vehicle surface: weaknet.d next to the binary first; embedded fallback lands with the T7 embed).
/// Shared by CLI and serve /v1/profiles (T6 lifted from main.rs to the lib surface, pinned by one function).
#[must_use]
pub fn weaknet_d_root() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("WEAKNET_ASSETS_DIR").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(v);
        if p.is_dir() {
            return Some(p);
        }
    }
    let exe_dir = std::env::current_exe().ok().and_then(|x| x.parent().map(Path::to_path_buf));
    if let Some(d) = &exe_dir {
        let sib = d.join("weaknet.d");
        if sib.is_dir() {
            return Some(sib);
        }
    }
    for base in std::iter::once(std::env::current_dir().unwrap_or_default()).chain(exe_dir) {
        for anc in base.ancestors().take(6) {
            let c = anc.join("scripts").join("weaknet.d");
            if c.is_dir() {
                return Some(c);
            }
        }
    }
    None
}

// ---------- embedded asset fallback (§vehicle M4': weaknet.d next to the binary first, rust-embed compile-time fallback;
// pinned to one function shared by CLI and serve) -- scp a single file and go (`up smoke` needs no asset dir) ----------

/// profile/scenario yaml embedded at compile time (assets/weaknet.d/; debug-embed on = debug builds self-contained too).
#[derive(Embed)]
#[folder = "assets/weaknet.d/"]
struct EmbeddedAssets;

/// Read an asset relative path (like "profiles/smoke.yaml"): fs hit first, else embedded; both missing = Ok(None).
pub fn read_asset(rel: &str) -> Wn<Option<String>> {
    if let Some(root) = weaknet_d_root() {
        let p = root.join(rel);
        if p.is_file() {
            let raw = std::fs::read_to_string(&p)
                .map_err(|e| Fail::env(format!("failed to read asset {}: {e}", p.display())))?;
            return Ok(Some(raw));
        }
    }
    Ok(EmbeddedAssets::get(rel).map(|f| String::from_utf8_lossy(&f.data).into_owned()))
}

/// List usable asset names (stems) under a subdirectory ("profiles"/"scenarios"): fs union embedded, deduped ascending.
pub fn list_assets(sub: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Some(root) = weaknet_d_root()
        && let Ok(rd) = std::fs::read_dir(root.join(sub))
    {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file()
                && p.extension().is_some_and(|x| x == "yaml" || x == "yml")
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                && !names.iter().any(|n| n == stem)
            {
                names.push(stem.to_string());
            }
        }
    }
    let prefix = format!("{sub}/");
    for key in EmbeddedAssets::iter() {
        let Some(rest) = key.strip_prefix(&prefix) else {
            continue;
        };
        let Some(stem) = rest.strip_suffix(".yaml").or_else(|| rest.strip_suffix(".yml")) else {
            continue;
        };
        // the embedded tree only recognizes files directly under one level of subdirectory (nested keys must not leak into the flat list)
        if stem.is_empty() || stem.contains('/') {
            continue;
        }
        if !names.iter().any(|n| n == stem) {
            names.push(stem.to_string());
        }
    }
    names.sort();
    names
}

// ---------- server_url resolution chain (flag > env WEAKNET_SERVER_URL > probe server.yaml) ----------

/// Pure chain (all three sources injected -- tests touch zero environment). yaml probe = listen.port, host loopback default.
pub fn pick_server_url(
    flag: Option<&str>,
    env_url: Option<&str>,
    yaml: Option<&Path>,
) -> Wn<String> {
    for cand in [flag, env_url] {
        if let Some(u) = cand.map(str::trim).filter(|s| !s.is_empty()) {
            return normalize_url(u);
        }
    }
    if let Some(p) = yaml {
        let raw = std::fs::read_to_string(p)
            .map_err(|e| Fail::env(format!("failed to read {}: {e}", p.display())))?;
        let doc: serde_yaml::Value = serde_yaml::from_str(&raw)
            .map_err(|e| Fail::env(format!("invalid server.yaml {}: {e}", p.display())))?;
        let port = doc
            .get("listen")
            .and_then(|l| l.get("port"))
            .and_then(|p| p.as_u64())
            .ok_or_else(|| {
                Fail::env(format!("{} missing listen.port (cannot derive server_url)", p.display()))
            })?;
        let port = u16::try_from(port)
            .map_err(|_| Fail::env(format!("listen.port out of u16 range: {port}")))?;
        return normalize_url(&format!("http://{PROBE_HOST}:{port}"));
    }
    Err(Fail::env(
        "server_url all three sources empty: --server-url / env WEAKNET_SERVER_URL / out/server/etc/server.yaml (probe missing; the escape hatch --rtp-port needs no server_url)",
    ))
}

/// https -> direct cause report (out of scope, design §Dependencies: the mini client is plaintext only).
fn normalize_url(u: &str) -> Wn<String> {
    if u.starts_with("https://") {
        return Err(Fail::env(format!(
            "https is out of scope: needs plaintext http:// (this tool's stats client has no TLS stack), got {u}"
        )));
    }
    let s = u.strip_prefix("http://").unwrap_or(u);
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Err(Fail::env(format!("invalid server_url shape: {u}")));
    }
    Ok(format!("http://{}", s.trim_end_matches('/')))
}

fn host_port(url: &str) -> Wn<(String, u16)> {
    let s = url
        .strip_prefix("http://")
        .ok_or_else(|| Fail::env(format!("stats request internal error: not an http URL {url}")))?;
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|e| Fail::env(format!("invalid server_url port {p:?}: {e}")))?,
        ),
        None => (s.to_string(), 80),
    };
    Ok((host, port))
}

/// CLI-surface assembly: env injection + repo-root ancestor probing (the bash SERVER_YAML default slot).
pub fn server_url_or_default(flag: Option<&str>) -> Wn<String> {
    let env_url = std::env::var("WEAKNET_SERVER_URL").ok().filter(|s| !s.is_empty());
    let yaml = server_yaml_probe_path();
    pick_server_url(flag, env_url.as_deref(), yaml.as_deref())
}

fn server_yaml_probe_path() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("WEAKNET_SERVER_YAML").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(v);
        return p.is_file().then_some(p);
    }
    let exe_dir = std::env::current_exe().ok().and_then(|x| x.parent().map(Path::to_path_buf));
    for base in std::iter::once(std::env::current_dir().unwrap_or_default()).chain(exe_dir) {
        for anc in base.ancestors().take(6) {
            let cand = anc.join("out").join("server").join("etc").join("server.yaml");
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

// ---------- HTTP mini client (std::net, Connection: close read to EOF) ----------

pub struct StatsClient {
    url: String,
    user: String,
    pass: String,
    token: Option<(String, Instant)>,
}

impl StatsClient {
    /// credential env injection (C20/password discipline: never hardcoded, missing = exit2 with cause -- bash L122 synonym).
    pub fn from_env(server_url: &str) -> Wn<Self> {
        let pass = std::env::var("WEAKNET_ADMIN_PASS").map_err(|_| {
            Fail::env("admin password not injected: export WEAKNET_ADMIN_PASS=<dev password> (env WEAKNET_ADMIN_USER defaults to admin)")
        })?;
        Ok(Self {
            url: server_url.to_string(),
            user: std::env::var("WEAKNET_ADMIN_USER")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "admin".into()),
            pass,
            token: None,
        })
    }

    fn request(&mut self, method: &str, path: &str, json_body: Option<&str>) -> Wn<(u16, String)> {
        let (host, port) = host_port(&self.url)?;
        let addr = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| Fail::env(format!("failed to resolve {host}:{port}: {e}")))?
            .next()
            .ok_or_else(|| Fail::env(format!("{host}:{port} has no resolution result")))?;
        let mut sock = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| {
            Fail::env(format!("failed to connect to {host}:{port}: {e} (server not up?)"))
        })?;
        sock.set_read_timeout(Some(IO_TIMEOUT))
            .and_then(|()| sock.set_write_timeout(Some(IO_TIMEOUT)))
            .map_err(|e| Fail::env(format!("failed to set the socket timeout: {e}")))?;
        let auth = match &self.token {
            Some((t, _)) => format!("Authorization: Bearer {t}\r\n"),
            None => String::new(),
        };
        let body = json_body.unwrap_or("");
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: mediaservo-weaknet\r\n\
             {auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(req.as_bytes())
            .map_err(|e| Fail::env(format!("{method} {path} send failed: {e}")))?;
        let mut raw = String::new();
        sock.read_to_string(&mut raw)
            .map_err(|e| Fail::env(format!("{method} {path} read failed: {e}")))?;
        let (head, resp_body) = raw.split_once("\r\n\r\n").ok_or_else(|| {
            Fail::env(format!(
                "{method} {path} response has no header/body separator (not an HTTP response?)"
            ))
        })?;
        // ponytail: only consumes content-length/EOF body shapes (axum Json always has content-length); chunked reports cause instead of a bare parse.
        if head.lines().any(|l| l.to_ascii_lowercase().contains("transfer-encoding: chunked")) {
            return Err(Fail::env(format!(
                "{method} {path} response is chunked: unsupported by the mini client"
            )));
        }
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse::<u16>().ok())
            .ok_or_else(|| {
                Fail::env(format!("{method} {path} status line invalid: {:?}", head.lines().next()))
            })?;
        Ok((status, resp_body.to_string()))
    }

    fn login(&mut self) -> Wn<String> {
        let body = serde_json::json!({ "username": self.user, "password": self.pass }).to_string();
        let (status, resp) = self.request("POST", "/api/auth/login", Some(&body))?;
        if status != 200 {
            return Err(Fail::env(format!(
                "login failed: HTTP {status} (wrong password / server not up / forbidden outside dev environments): {}",
                prefix_excerpt(&resp)
            )));
        }
        let token = serde_json::from_str::<serde_json::Value>(&resp)
            .ok()
            .and_then(|v| v.get("token").and_then(|t| t.as_str().map(str::to_string)))
            .ok_or_else(|| {
                Fail::env(format!("login response has no token field: {}", prefix_excerpt(&resp)))
            })?;
        self.token = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    fn token_cached(&mut self) -> Wn<()> {
        if let Some((_, at)) = &self.token
            && at.elapsed() < TOKEN_TTL
        {
            return Ok(());
        }
        self.login().map(|_| ())
    }

    /// GET /api/admin/sfu/stats list mode -> typed rows. 401 retries once after re-login.
    pub fn fetch_streams(&mut self) -> Wn<Vec<StreamInfo>> {
        for attempt in 0..2 {
            self.token_cached()?;
            let (status, resp) = self.request("GET", "/api/admin/sfu/stats", None)?;
            match status {
                200 => return parse_streams_body(&resp),
                401 if attempt == 0 => {
                    eprintln!("weaknet(scope): WARN token invalid (401), re-login and retry once");
                    self.token = None;
                }
                other => {
                    return Err(Fail::env(format!(
                        "GET stats failed: HTTP {other}: {}",
                        prefix_excerpt(&resp)
                    )));
                }
            }
        }
        Err(Fail::env("GET stats 401: still rejected after re-login (account/role changed?)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Embedded fallback baseline (T13 vehicle surface): the embedded 4 profiles + 1 scenario are always reachable
    /// regardless of the fs probe chain (scp single-file semantics); unknown keys = None (no fake hits).
    #[test]
    fn embedded_assets_are_always_resolvable() {
        let smoke = read_asset("profiles/smoke.yaml").unwrap();
        assert!(smoke.is_some() && smoke.unwrap().contains("rtt_ms: 80"));
        let names = list_assets("profiles");
        for n in ["smoke", "cell-edge", "bandwidth-wall", "remote-burst", "degraded"] {
            assert!(names.contains(&n.to_string()), "{n} ∈ {names:?}");
        }
        assert!(list_assets("scenarios").contains(&"example-cell-edge-40s".to_string()));
        assert_eq!(read_asset("profiles/nonexistent.yaml").unwrap(), None);
        assert!(list_assets("profiles").iter().all(|n| !n.contains('/')));
    }
}

fn prefix_excerpt(s: &str) -> String {
    let cut = s.chars().take(160).collect::<String>();
    cut.replace('\n', " ")
}

// ---------- capabilities (design §engine capability table; ifb probe arrives with T9) ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// iproute2 >= 6.6 (the seed semantics gate; same predicate as engine version_gate_seed).
    pub seed: bool,
    /// lo scenarios are pure port legs, always true (§dir table note).
    pub dir_lo: bool,
    /// The ifb mirror path for physical-port dir=in -- implemented at T9 (read-before-probe chain).
    pub ifb_ingress: bool,
    pub ifb_reason: String,
}

/// tc -V seeds the version bit; T9 arrival: the ifb bit goes through [`crate::ifb::probe`] read-before-probe
/// (ifb0 present = directly true with zero create/delete; only when absent does the local channel modprobe -> create -> up -> del; sidecar is read-only).
pub fn capabilities(env: &Env, iface: &str) -> Wn<Capabilities> {
    let chan = probe_channel(env, iface)?;
    let v = tc_exec(&chan, &["-V"])?;
    let seed =
        crate::engine::parse_iproute2_semver(&v).is_some_and(|(maj, min)| (maj, min) >= (6, 6));
    let (ifb_ingress, ifb_reason) = crate::ifb::probe(&chan);
    Ok(Capabilities { seed, dir_lo: true, ifb_ingress, ifb_reason })
}
