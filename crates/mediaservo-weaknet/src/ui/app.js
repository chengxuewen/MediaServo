// weaknet console v1 (T7). Contract = docs/plans/weaknet-agent/design.md §UI information architecture + §serve security stack 5.
// Zero frameworks/zero bundler/zero router lib/zero localStorage. This file never prints any access credential to the console (W7 grep gate).
// Write-path protocol: control change -> debounce 400ms -> POST /v1/set (sliders fire on pointerup); no active spec -> a 409
// toast guides opening the master switch; all echo (including our own set) consumes only SSE state frames -- the panel never invents optimistic updates.

// ---------- credential lifecycle (design §serve security stack 5) ----------
// sessionStorage = the repo-wide sole storage exemption (noted explicitly): the credential is stripped from the banner URL and stored only in this tab,
// discarded on close; localStorage avoided (persisting would only widen the leak surface).
const AUTH_KEY = "wnet-auth";
const qs = new URLSearchParams(location.search);
if (qs.get("token")) sessionStorage.setItem(AUTH_KEY, qs.get("token"));
history.replaceState(null, "", location.pathname); // strip the query from the address bar (no credential left in history/screenshots)
const auth = sessionStorage.getItem(AUTH_KEY) || "";

// ---------- DOM helpers ----------
const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const el = (tag, cls, html) => {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (html !== undefined) n.innerHTML = html;
  return n;
};
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) =>
  ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

let toastTimer = null;
function toast(msg, bad) {
  const t = $("#toast");
  t.textContent = msg;
  t.hidden = false;
  t.classList.toggle("bad", !!bad);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.hidden = true; }, 4200);
}
function showGate(bad) {
  const g = $("#gate");
  g.hidden = false;
  $("#gate-err").hidden = !bad;
  if (bad) {
    const card = g.querySelector(".gate-card");
    card.classList.remove("shake");
    void card.offsetWidth; // restart the animation (every round of consecutive 401s shakes)
    card.classList.add("shake");
  }
}

// ---------- thin REST wrapper ----------
async function api(path, opts = {}) {
  const headers = Object.assign({}, opts.headers);
  if (auth) headers.Authorization = "Bearer " + auth;
  let res;
  try {
    res = await fetch(path, Object.assign({}, opts, { headers }));
  } catch {
    toast(`request failed: ${path} (serve exited/offline?)`, true);
    return { status: 0, body: null };
  }
  let body = null;
  try { body = await res.json(); } catch { /* non-JSON body: ignore the parse */ }
  if (res.status === 401) showGate(!!auth); // fullscreen onboarding: copy the whole banner line or paste in the gate input (still 401 with a token = shake)
  return { status: res.status, body };
}

// ---------- model (sole write source = this object; sole read source = SSE frames) ----------
const ui = {
  spec: {
    rtt_ms: 0, jitter_ms: 0, loss: "0", loss_mode: "simple",
    gemodel: ["25", "0.2", "0.05"], reorder: "0", rate_mbps: null,
    seed: null, limit: 100000, dir: "both",
  },
  iface: "lo",
  duration: 300,
  selRooms: new Set(),
  selDevices: new Set(),
};
let caps = null;         // /v1/capabilities
let frame = null;        // most recent frame (the oracle surface also goes through __wnet)
let streamsNote = "";    // the /v1/streams cause (material for degraded mode)
let scenarios = [];
let scenPick = "";
let es = null;           // EventSource

// test hook (constructed injection, the PIT-174 method): the Playwright oracle reads the same-frame data (incl. chart series), sidestepping WS interception.
window.__wnet = { ready: false, frameCount: 0, lastFrame: null, ui };

// ---------- left column: spec inputs <-> model <-> backend flat form ----------
const nz = (v) => { // CLI semantics alignment: 0/empty = unset (null) -- a "0" string would enter the fingerprint token set and make read-back always mismatch (proven by apply 500)
  const t = String(v ?? "").trim();
  return t === "" || Number(t) === 0 ? null : t;
};
function specFields() {
  return {
    rtt_ms: Math.round(Number($('[data-spec="rtt_ms"]').value) || 0),
    jitter_ms: Math.round(Number($('[data-spec="jitter_ms"]').value) || 0),
    loss: nz($('[data-spec="loss"][id="loss-num"]').value),
    loss_mode: $("#loss-mode").value,
    gemodel: $$("[data-gem]").map((i) => String(i.value).trim() || "0"),
    reorder: nz($('[data-spec="reorder"]').value),
    rate_mbps: (v => v === "" ? null : Number(v))($('[data-spec="rate_mbps"]').value),
    seed: (v => v === "" ? null : Math.round(Number(v)))($('[data-spec="seed"]').value),
    limit: Math.round(Number($('[data-spec="limit"]').value) || 100000),
    dir: ($('input[name="dir"]:checked') || {}).value || "both",
  };
}
function scopeFields() {
  if (ui.selDevices.size) return { device_scope: { ids: [...ui.selDevices] } };
  if (ui.selRooms.size) return { stream_scope: { rooms: [...ui.selRooms] } };
  return {};
}

// ---------- write path ----------
let setTimer = null;
function scheduleSet(immediate) { // debounce 400ms; sliders fire on pointerup / discrete checks fire immediately
  clearTimeout(setTimer);
  if (immediate) { sendSet(); return; }
  setTimer = setTimeout(sendSet, 400);
}
async function sendSet(retry = true) {
  const body = Object.assign(specFields(), scopeFields());
  const r = await api("v1/set", { method: "POST", body: JSON.stringify(body), headers: { "Content-Type": "application/json" } });
  const err = r.body?.error || "";
  if (r.status === 409 && err.includes("holds the lock")) {
// serve x CLI/bursty writes = a legitimate queued concurrent state (design §Error handling); re-send once deferred, report only if still busy
    if (retry) { setTimeout(() => sendSet(false), 1200); return; }
    toast("write lock busy (another operation in progress) -- one re-send still occupied, try again later", true);
  } else if (r.status === 409 && (err.includes("no active spec") || err.includes("apply first"))) {
    toast("no active impairment (409) -- open the master switch first; control values kept, they take effect once opened");
  } else if (r.status === 409) {
    toast(`409: ${err || "?"} (scenario in progress, stop first)`, true);
  } else if (r.status === 400) toast(`params rejected (400): ${err}`, true);
  else if (r.status !== 200 && r.status !== 0) toast(`set failed (${r.status}): ${err}`, true);
// 200 also only means "dispatched": field echo waits for the SSE frame, no optimistic update
}
async function toggleMaster(on) {
  const sw = $("#master");
  if (on) {
    const dur = Math.round(Number($("#duration").value) || 300);
    const body = Object.assign(specFields(), scopeFields(), { duration: dur, iface: ui.iface.trim() || "lo" });
    const r = await api("v1/apply", { method: "POST", body: JSON.stringify(body), headers: { "Content-Type": "application/json" } });
    if (r.status === 409) { toast("scenario in progress (409) -- stop first", true); sw.checked = false; }
    else if (!r.ok && r.status !== 0) {
      toast(`apply failed (${r.status}): ${r.body?.error ?? "?"} -- usually stats unreachable/empty media ports, see the mid-column cause`, true);
      sw.checked = false;
    }
  } else {
    const r = await api("v1/clear", { method: "POST" });
    if (r.status !== 200 && r.status !== 0) toast(`clear failed (${r.status}): ${r.body?.error ?? "?"}`, true);
  }
// the switch's final visual position is decided by the state frame (spec!=null <=> master on)
}

// ---------- direction truth table (§dir leg table x capabilities) ----------
const isPhysical = (iface) => iface && iface.toLowerCase() !== "lo";
function dirGateReason() {
  const why = caps?.ifb_reason || "ifb probe not yet available";
  return why.includes("not loaded")
    ? `physical-port ingress needs an ifb mirror: ${why}`
    : `structurally unsupported: ${why}`;
}
function refreshDirGate() {
  const blocked = isPhysical(ui.iface) && caps && !caps.ifb_ingress;
  for (const radio of $$('input[name="dir"]')) {
    const off = blocked && radio.value !== "out"; // the egress legs of in/both depend on ifb; out always usable; lo fully usable (dir_lo always true)
    radio.disabled = off;
    const lab = radio.closest("label");
    lab.classList.toggle("grey", off);
    lab.title = off ? dirGateReason() : "";
  }
  if (blocked && ($('input[name="dir"]:checked') || {}).value !== "out") {
    const out = $('input[name="dir"][value="out"]');
    out.checked = true;
    toast("physical-port ingress unavailable (ifb greyed) -- direction switched to out");
    scheduleSet(true);
  }
}
function renderCapsLine() {
  const c = $("#capsline");
  if (!caps) { c.textContent = "capabilities loading..."; return; }
  c.innerHTML = `caps: seed=${caps.seed ? "OK" : "X (tc -V<6.6 greyed)"} · dir_lo=${caps.dir_lo ? "OK" : "X"} · `;
    + `ifb_ingress=${caps.ifb_ingress ? "✓" : "✗"}`
    + (caps.ifb_ingress ? "" : ` <span class="note">${esc(caps.ifb_reason)}</span>`);
  const seed = $("#seed");
  seed.disabled = !caps.seed;
  seed.title = caps.seed ? "" : "seed needs iproute2 tc -V >= 6.6 (this host does not support it)";
}

// ---------- mid column: owner-grouped stream table ----------
const PLAY_PORT = 8080; // documented constant: the browser-surface entry (Caddy, deploy/caddy/Caddyfile.native); naturally works cross-port
// T6 grep exemption (wW9): the panel on :9810 playing server :8080 streams is a deliberate cross-origin absolute URL --
// relativizing it with the rest of this file's URLs would hit the panel itself and become a bug.
function playUrl(room) {
  return `http://${location.hostname}:${PLAY_PORT}/?room=${encodeURIComponent(room)}`;
}
function renderStreams() {
  const host = $("#streams");
  host.textContent = "";
  if (!frame) { host.append(el("p", "empty", "waiting for the first state frame...")); return; }
  const rows = frame.streams || [];
  if (rows.length === 0) {
    if (streamsNote) { // degraded mode: replace the whole column (backend rule = scope non-empty AND stats unreachable -> reject with cause, never silently all ports)
      const p = el("p", "warn", `stats unreachable: shaping only by <b>${esc(ui.iface)}</b>/the media port set. Cause: <span class="note">${esc(streamsNote)}</span>`);
      const retry = el("a", "play", "retry");
      retry.href = "#";
      retry.addEventListener("click", (ev) => { ev.preventDefault(); fetchStreamsNote(); });
      p.append(" ", retry);
      host.append(p);
    } else {
      host.append(el("p", "empty", "no live streams (server without producers? host not pushing?)"));
    }
    return;
  }
  const groups = new Map();
  for (const s of rows) {
    const k = s.owner || "(unowned)";
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(s);
  }
  for (const [owner, list] of [...groups.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
    const box = el("div", "grp");
    const head = el("header");
    const gcb = el("input");
    gcb.type = "checkbox";
    gcb.checked = ui.selDevices.has(owner) || (owner !== "(unowned)" && list.every((s) => ui.selRooms.has(s.room)));
    if (owner === "(unowned)") {
      gcb.disabled = true;
      gcb.title = "unowned rooms cannot be targeted by device (owner missing; old servers fall back to peer_id, CLI --device only)";
    } else {
      gcb.addEventListener("change", () => {
        if (gcb.checked) { ui.selDevices.add(owner); ui.selRooms.clear(); }
        else ui.selDevices.delete(owner);
        scheduleSet(false);
      });
    }
    head.append(gcb, el("span", "owner", esc(owner)), el("span", "cnt", `${list.length} streams`));
    box.append(head);
    for (const s of list) {
      const row = el("div", "srow" + (s.live ? "" : " old"));
      const cb = el("input");
      cb.type = "checkbox";
      cb.checked = ui.selRooms.has(s.room);
      cb.addEventListener("change", () => {
        if (cb.checked) { ui.selRooms.add(s.room); ui.selDevices.clear(); }
        else {
          ui.selRooms.delete(s.room);
          if (ui.selRooms.size === 0 && ui.selDevices.size === 0)
            toast("selection cleared: targeting keeps the last applied values (set takes no empty set); to reshape at room level close the switch and re-open with no selections");
        }
        scheduleSet(false);
      });
      const kb = s.stale ? "stale" : (s.kbps == null ? "—" : s.kbps + "k");
      row.append(
        cb,
        el("span", "room", esc(s.room)),
        el("span", "kb", kb),
        el("span", "badge " + (s.live ? "live" : "age"), s.live ? "live" : "in aging window"),
        s.stale ? el("span", "badge stale", "stale") : "",
      );
      const a = el("a", "play", "play");
      a.href = playUrl(s.room);
      a.target = "_blank";
      a.rel = "noopener";
      a.title = `open the play page in a new tab (:${PLAY_PORT} entry · room=${s.room})`;
      row.append(a);
      box.append(row);
    }
    host.append(box);
  }
}

// ---------- right column: uPlot (kbps disconnection gaps + dropped tiers + event verticals) ----------
const PALETTE = ["#4f9cf9", "#3fb950", "#e3b341", "#f85149", "#bc8cff", "#39c5cf", "#ff7b72", "#d2a8ff", "#7ee787", "#ffa657"];
const chartState = { t: [], rooms: [], y: new Map(), drop: [], evs: [] }; // evs ring cap 200 (design §UI coding note)
let kbpsChart = null, dropChart = null;

window.__wnet.charts = chartState; // same-frame oracle data surface (series aligned with state)
function feedCharts(f) {
  const t = Date.now() / 1000;
  chartState.t.push(t);
  const seen = new Set();
  for (const s of f.streams || []) {
    seen.add(s.room);
    if (!chartState.y.has(s.room)) chartState.y.set(s.room, new Array(chartState.t.length - 1).fill(null));
// both semantics (first-sample null / stale) render as gaps; the inline badge distinguishes the source -- never conflated with 0 (design §API state frame)
    chartState.y.get(s.room).push(s.stale || s.kbps == null ? null : s.kbps);
  }
  for (const [room, arr] of chartState.y) if (!seen.has(room)) arr.push(null);
  chartState.drop.push(f.tc?.dropped ?? null);
  for (const e of f.ev || []) {
    chartState.evs.push(Object.assign({ _t: t }, e));
    if (chartState.evs.length > 200) chartState.evs.shift();
  }
// ponytail: setData each frame (2s cadence) + rebuild only when the rooms set changes; uPlot incremental addSeries unused -- <20 streams, rebuild cost negligible
  const rooms = [...chartState.y.keys()].sort();
  if (rooms.join("|") !== chartState.rooms.join("|")) {
    chartState.rooms = rooms;
    rebuildCharts();
  } else {
    pushData();
  }
  renderEvLog();
}
function chartData() {
  return [chartState.t, ...chartState.rooms.map((r) => chartState.y.get(r))];
}
function pushData() {
  if (kbpsChart) kbpsChart.setData(chartData());
  if (dropChart) dropChart.setData([chartState.t, chartState.drop]);
}
function eventLines(u) { // event verticals drawn by a draw hook (seconds on the time axis, same domain as x=time)
  const { ctx, bbox } = u;
  ctx.save();
  ctx.strokeStyle = "rgba(227,179,65,.55)";
  ctx.lineWidth = 1;
  for (const e of chartState.evs) {
    if (e._t < u.posToVal(bbox.left, "x") || e._t > u.posToVal(bbox.left + bbox.width, "x")) continue;
    const x = Math.round(u.valToPos(e._t, "x", true)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(x, bbox.top);
    ctx.lineTo(x, bbox.top + bbox.height);
    ctx.stroke();
  }
  ctx.restore();
}
function rightWidth() { return Math.max(320, $("#right").clientWidth - 26); }
function rebuildCharts() {
  if (kbpsChart) { kbpsChart.destroy(); kbpsChart = null; }
  kbpsChart = new uPlot({
    width: rightWidth(), height: 220,
    scales: { x: { time: true }, y: { label: "kbps" } },
    series: [{ label: "time" }, ...chartState.rooms.map((r, i) => ({
      label: r, stroke: PALETTE[i % PALETTE.length], width: 1.5, fill: "transparent",
    }))],
    axes: [{}, { size: 52, values: (u, v) => v.map(String) }],
    legend: { show: chartState.rooms.length > 1 },
    hooks: { draw: [eventLines] },
    cursor: { drag: { x: true, y: false } },
  }, chartData(), $("#kbps"));
  if (dropChart) { dropChart.destroy(); dropChart = null; }
  dropChart = new uPlot({
    width: rightWidth(), height: 130,
    scales: { x: { time: true }, y: {} },
    series: [{ label: "time" }, { label: "dropped", stroke: "#f85149", fill: "rgba(248,81,73,.12)", spanGaps: false }],
    hooks: { draw: [eventLines] },
  }, [chartState.t, chartState.drop], $("#drop"));
}
function renderEvLog() {
  const ul = $("#evlog");
  ul.textContent = "";
  for (const e of chartState.evs.slice(-5).reverse()) {
    const li = el("li");
    const detail = e.room || e.phase || e.stream || (e.err ? String(e.err).slice(0, 40) : "");
    const err = e.err ? ` <span class="err">${esc(String(e.err).slice(0, 80))}</span>` : "";
    const okv = e.ok === false ? ' <span class="err">FAIL</span>' : "";
    li.innerHTML = `<span class="evname">${esc(e.ev ?? "?")}</span> ${esc(detail)}${okv}${err}`;
    ul.append(li);
  }
}

// ---------- frame rendering (the sole echo channel) ----------
function renderRing(f) {
  const txt = $("#ring-txt"), fg = $("#ring-fg");
  const exp = f.expires_at;
  if (exp == null) { txt.textContent = "—"; fg.style.strokeDashoffset = 119.4; return; }
  const rem = Math.max(0, exp - Date.now());
  if (exp === 0) { txt.textContent = "∞"; fg.style.strokeDashoffset = 0; return; }
  txt.textContent = Math.ceil(rem / 1000) + "s";
// ponytail: ring ratio references the default 300s (the frame carries no total-duration field); purely visual, the text is the predicate
  fg.style.strokeDashoffset = 119.4 * Math.min(1, rem / 300000);
}
function renderHeader(f) {
  const h = $("#hstat");
  h.textContent = "";
  const chip = (k, v, cls) => h.append(el("span", "chip" + (cls ? " " + cls : ""), `${k} <b>${esc(v)}</b>`));
  if (f.spec) { chip("impairment", "ACTIVE", "act"); chip("scope", f.scope ?? "?"); chip("dir", f.dir ?? "?"); }
  if (f.job) chip("job", f.job.name ?? "?");
  if (f.tc?.dropped != null) chip("dropped", f.tc.dropped);
}
function echoSpecInputs(f) { // external CLI/other-surface writes -> frame echo (focused inputs are not interrupted)
  const s = f.spec;
  if (!s) return;
  const set = (sel, val) => {
    const i = $(sel);
    if (i && document.activeElement !== i) i.value = val;
  };
  set('[data-spec="rtt_ms"]', s.rtt_ms ?? 0);
  set('[data-spec="jitter_ms"]', s.jitter_ms ?? 0);
  let loss = "0", mode = "simple", gem = null;
  if (s.loss && typeof s.loss === "object") {
    if (s.loss.Simple != null) { loss = String(s.loss.Simple); mode = "simple"; }
    else if (s.loss.GeModel) { loss = String(s.loss.GeModel.loss); mode = "gemodel"; gem = [s.loss.GeModel.r, s.loss.GeModel.h, s.loss.GeModel.k]; }
  } else if (typeof s.loss === "string") loss = s.loss;
  set("#loss-num", loss.replace(/%$/, ""));
  set("#loss-range", loss.replace(/%$/, ""));
  if (document.activeElement !== $("#loss-mode")) $("#loss-mode").value = mode;
  $("#gem").hidden = $("#loss-mode").value !== "gemodel";
  if (gem) $$("[data-gem]").forEach((i, k) => { if (document.activeElement !== i) i.value = gem[k]; });
  set('[data-spec="reorder"]', (s.reorder_pct ?? "0").toString().replace(/%$/, ""));
  set('[data-spec="rate_mbps"]', s.rate_mbps ?? "");
  set('[data-spec="seed"]', s.seed ?? "");
  set('[data-spec="limit"]', s.limit ?? 100000);
  const dir = typeof s.dir === "string" ? s.dir : "both";
  const radio = $(`input[name="dir"][value="${dir}"]`);
  const cur = $('input[name="dir"]:checked');
// truth-table greying first: when the frame value points at a greyed (disabled) option, don't interrupt the local selection (e.g. a physical port awaiting a frame still carrying the old dir=both)
  if (radio && !radio.disabled && document.activeElement !== radio) radio.checked = true;
  else if (radio && radio.disabled && cur && !cur.disabled) void cur;
}
/// targeted-name echo (E2 ledger): state.names is the sole truth channel for checkboxes -- the panel forbids optimistic updates,
// frames are the authority; only while the user's 400ms debounce is in flight (setTimer pending) does a fresh local selection win, avoiding mutual race-erasure.
function echoScopeNames(f) {
  if (setTimer) return;
  ui.selRooms = new Set(f.names?.rooms ?? []);
  ui.selDevices = new Set(f.names?.devices ?? []);
}
function onFrame(f) {
  window.__wnet.frameCount++;
  window.__wnet.lastFrame = f;
  window.__wnet.ready = true;
  frame = f;
  if (f.frame_error) { toast(`state-frame error: ${f.frame_error}`, true); return; }
  $("#conn").className = "conn on";
  $("#conn").textContent = "● frames at 2s";
  $("#master").checked = f.spec != null;
  renderRing(f);
  renderHeader(f);
  echoSpecInputs(f);
  echoScopeNames(f);
  renderStreams();
  feedCharts(f);
  renderScenJob(f);
}
function renderScenJob(f) {
  $("#scen-job").textContent = f.job
    ? `running: ${f.job.name} (pid ${f.job.pid}) ${f.job.done ?? 0}/${f.job.total ?? "?"}`
    : "no active job";
}

// ---------- SSE ----------
function connectSSE() {
  es = new EventSource("v1/events?token=" + encodeURIComponent(auth)); // query-token is the sole exemption surface
  es.onmessage = (m) => {
    let f;
    try { f = JSON.parse(m.data); } catch { toast("state-frame JSON parse failed", true); return; }
    onFrame(f);
  };
  es.onerror = () => {
    $("#conn").className = "conn off";
    $("#conn").textContent = "● disconnected (EventSource auto-reconnecting)";
  };
}

// ---------- scenarios tab ----------
function renderScenList() {
  const ul = $("#scenlist");
  ul.textContent = "";
  if (scenarios.length === 0) { ul.append(el("li", "empty", "no scenarios (weaknet.d/scenarios or the /v1/scenarios cause)")); return; }
  for (const name of scenarios) {
    const li = el("li");
    const lab = el("label", name === scenPick ? "on" : "");
    const rb = el("input");
    rb.type = "radio";
    rb.name = "scen";
    rb.checked = name === scenPick;
    rb.addEventListener("change", () => { scenPick = name; renderScenList(); });
    lab.append(rb, el("span", "", esc(name)));
    li.append(lab);
    ul.append(li);
  }
}
async function loadScenarios() {
  const r = await api("v1/scenarios");
  scenarios = (r.body?.scenarios ?? []).slice();
  if (scenarios.length && !scenPick) scenPick = scenarios[0];
  renderScenList();
}
async function scenRun() {
  if (!scenPick) { toast("select a scenario first"); return; }
  const r = await api("v1/scenario/run", { method: "POST", body: JSON.stringify({ file: scenPick }), headers: { "Content-Type": "application/json" } });
  if (r.status === 501) toast(`scenario engine not yet available (501): ${r.body?.error ?? "arrives with T8"}`);
  else if (r.status !== 200 && r.status !== 0) toast(`run failed (${r.status}): ${r.body?.error ?? "?"}`, true);
}
async function scenStop() {
  const r = await api("v1/scenario/stop", { method: "POST", body: "{}", headers: { "Content-Type": "application/json" } });
  if (r.status === 501) toast(`stop likewise arrives with T8 (501): ${r.body?.error ?? ""}`);
  else if (r.status !== 200 && r.status !== 0) toast(`stop failed (${r.status}): ${r.body?.error ?? "?"}`, true);
}

// ---------- boot / onboarding ----------
async function fetchStreamsNote() {
  const r = await api("v1/streams");
  streamsNote = r.body?.note || "";
  renderStreams();
}
async function boot() {
  if (!auth) { showGate(); return; }
  $("#gate-clear").addEventListener("click", () => {
    sessionStorage.removeItem(AUTH_KEY);
    location.replace(location.pathname);
  });
  const [c, p] = await Promise.all([api("v1/capabilities"), api("v1/profiles")]);
  if (c.status === 401 || p.status === 401) return; // the gate is already presented by api()
  caps = c.body;
  renderCapsLine();
  refreshDirGate();
  for (const name of p.body?.profiles ?? []) {
    const o = el("option", "", esc(name));
    o.value = name;
    $("#profile").append(o);
  }
  await loadScenarios();
  await fetchStreamsNote();
  bind();
  connectSSE();
}
function bind() {
  $("#master").addEventListener("change", (e) => toggleMaster(e.target.checked));
// spec inputs: input -> debounce 400; range additionally fires on pointerup (the storm case)
  for (const i of $$("#spec-rows input, #spec-rows select")) {
    i.addEventListener("input", () => {
      if (i.type === "range") $("#loss-num").value = i.value;
      if (i.id === "loss-num") $("#loss-range").value = i.value;
      scheduleSet(false); // incl. sliders: 400ms convergence while dragging, pointerup fires as backstop
    });
    if (i.type === "range") i.addEventListener("pointerup", () => scheduleSet(true));
  }
  $("#loss-mode").addEventListener("change", () => {
    $("#gem").hidden = $("#loss-mode").value !== "gemodel";
    scheduleSet(true);
  });
// iface only changes the local truth-table panel greying; never enters set (accidentally pushing a physical port = real NIC impairment; switching iface goes through close-switch -> re-open apply)
  $("#iface").addEventListener("input", () => { ui.iface = $("#iface").value.trim(); refreshDirGate(); renderStreams(); });
  for (const r of $$('input[name="dir"]')) r.addEventListener("change", () => scheduleSet(true));
  $("#profile").addEventListener("change", () => {
    const v = $("#profile").value;
    if (v) toast(`profile "${v}" is a CLI preset (weaknet apply --profile ${v}); the panel apply channel is not wired (T7 boundary); manual editing below is equivalent`);
  });
  for (const b of $$("#tabs .tab")) b.addEventListener("click", () => {
    for (const t of $$("#tabs .tab")) t.classList.toggle("on", t === b);
    $("#tab-panel").hidden = b.dataset.tab !== "panel";
    $("#tab-scen").hidden = b.dataset.tab !== "scen";
  });
  $("#scen-run").addEventListener("click", scenRun);
  $("#scen-stop").addEventListener("click", scenStop);
  addEventListener("resize", () => { if (kbpsChart) rebuildCharts(); });
}
// ---------- gate token input (first-use onboarding UI) ----------
// reuses the credential lifecycle above (AUTH_KEY/sessionStorage); submit reloads with the credential,
// zero credential in the address bar -- coexists with the file-header replaceState strip-query policy. Binding at EOF: the $ helper is defined by then.
function applyGateToken() {
  const v = $("#gate-token").value.trim();
  if (!v) return;
  sessionStorage.setItem(AUTH_KEY, v);
  location.replace(location.pathname); // after reload boot() consumes the new token directly
}
$("#gate-apply").addEventListener("click", applyGateToken);
$("#gate-token").addEventListener("keydown", (e) => { if (e.key === "Enter") applyGateToken(); });
boot();
