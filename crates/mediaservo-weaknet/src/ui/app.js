// weaknet 控制台 v1（T7）。契约 = docs/plans/weaknet-agent/design.md §UI 信息架构 + §serve 安全栈 5。
// 零框架/零打包器/零路由库/零 localStorage。本文件永不向控制台输出任何访问凭证（W7 grep 门）。
// 写路径协议：控件变更 → debounce 400ms → POST /v1/set（滑块 pointerup 即发）；无活跃 spec → 409
// toast 引导开总闸；一切回显（含本端 set）只吃 SSE 状态帧——面板不自造乐观更新。

// ---------- 凭证生命周期（design §serve 安全栈 5） ----------
// sessionStorage = 全仓 storage 唯一豁免（显式注记）：横幅 URL 的凭证剥出后只存本标签页，
// 关闭即弃；不用 localStorage（持久落盘反而扩大泄露面）。
const AUTH_KEY = "wnet-auth";
const qs = new URLSearchParams(location.search);
if (qs.get("token")) sessionStorage.setItem(AUTH_KEY, qs.get("token"));
history.replaceState(null, "", location.pathname); // 地址栏剥 query（进历史/截屏不留凭证）
const auth = sessionStorage.getItem(AUTH_KEY) || "";

// ---------- DOM 助手 ----------
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
function showGate() { $("#gate").hidden = false; }

// ---------- REST 小封装 ----------
async function api(path, opts = {}) {
  const headers = Object.assign({}, opts.headers);
  if (auth) headers.Authorization = "Bearer " + auth;
  let res;
  try {
    res = await fetch(path, Object.assign({}, opts, { headers }));
  } catch {
    toast(`请求失败：${path}（serve 已退出/断网？）`, true);
    return { status: 0, body: null };
  }
  let body = null;
  try { body = await res.json(); } catch { /* 非 JSON 响应体：忽略解析 */ }
  if (res.status === 401) showGate(); // 全屏引导：去横幅复制
  return { status: res.status, body };
}

// ---------- 模型（唯一写源=本对象；唯一读源=SSE 帧） ----------
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
let frame = null;        // 最近一帧（oracle 面也走 __wnet）
let streamsNote = "";    // /v1/streams 报因（降级模式素材）
let scenarios = [];
let scenPick = "";
let es = null;           // EventSource

// 测试钩（构造注入，PIT-174 法）：Playwright oracle 读同帧数据（含图表序列），消 WS 拦截不可达问题。
window.__wnet = { ready: false, frameCount: 0, lastFrame: null, ui };

// ---------- 左栏：spec 行 ↔ 模型 ↔ 后端 flat 形 ----------
const nz = (v) => { // CLI 语义对齐：0/空 = 未设（null）——"0" 串会进指纹 token 集致回读必不符（apply 500 实证）
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

// ---------- 写路径 ----------
let setTimer = null;
function scheduleSet(immediate) { // debounce 400ms；滑块 pointerup/离散勾选即发
  clearTimeout(setTimer);
  if (immediate) { sendSet(); return; }
  setTimer = setTimeout(sendSet, 400);
}
async function sendSet(retry = true) {
  const body = Object.assign(specFields(), scopeFields());
  const r = await api("/v1/set", { method: "POST", body: JSON.stringify(body), headers: { "Content-Type": "application/json" } });
  const err = r.body?.error || "";
  if (r.status === 409 && err.includes("锁持有")) {
    // serve×CLI/连发写 = 合法并态排队（design §Error handling）；一次性延后重发，仍忙才报
    if (retry) { setTimeout(() => sendSet(false), 1200); return; }
    toast("写锁忙（另一操作进行中）——重发一次仍被占，稍后再试", true);
  } else if (r.status === 409 && (err.includes("no active spec") || err.includes("先 apply"))) {
    toast("无活跃损伤（409）——先开总闸，控件值已保留、开闸即生效");
  } else if (r.status === 409) {
    toast(`409：${err || "?"}（scenario 进行中先 stop）`, true);
  } else if (r.status === 400) toast(`参数被拒（400）：${err}`, true);
  else if (r.status !== 200 && r.status !== 0) toast(`set 失败（${r.status}）：${err}`, true);
  // 200 也只当「已下发」：字段回显等 SSE 帧，不乐观更新
}
async function toggleMaster(on) {
  const sw = $("#master");
  if (on) {
    const dur = Math.round(Number($("#duration").value) || 300);
    const body = Object.assign(specFields(), scopeFields(), { duration: dur, iface: ui.iface.trim() || "lo" });
    const r = await api("/v1/apply", { method: "POST", body: JSON.stringify(body), headers: { "Content-Type": "application/json" } });
    if (r.status === 409) { toast("scenario 进行中（409）——先 stop", true); sw.checked = false; }
    else if (!r.ok && r.status !== 0) {
      toast(`apply 失败（${r.status}）：${r.body?.error ?? "?"}——多为 stats 不可达/媒体口为空，见中栏报因`, true);
      sw.checked = false;
    }
  } else {
    const r = await api("/v1/clear", { method: "POST" });
    if (r.status !== 200 && r.status !== 0) toast(`clear 失败（${r.status}）：${r.body?.error ?? "?"}`, true);
  }
  // 开关的最终视觉位置由状态帧决定（spec!=null ⇔ 总闸 on）
}

// ---------- 方向真值表（§dir 腿定义表 × capabilities） ----------
const isPhysical = (iface) => iface && iface.toLowerCase() !== "lo";
function dirGateReason() {
  const why = caps?.ifb_reason || "ifb 探测未到场";
  return why.includes("未加载")
    ? `物理口入向需 ifb 镜像：${why}`
    : `结构性不支持：${why}`;
}
function refreshDirGate() {
  const blocked = isPhysical(ui.iface) && caps && !caps.ifb_ingress;
  for (const radio of $$('input[name="dir"]')) {
    const off = blocked && radio.value !== "out"; // in/both 的下行腿依赖 ifb；out 恒可用；lo 全可用（dir_lo 恒真）
    radio.disabled = off;
    const lab = radio.closest("label");
    lab.classList.toggle("grey", off);
    lab.title = off ? dirGateReason() : "";
  }
  if (blocked && ($('input[name="dir"]:checked') || {}).value !== "out") {
    const out = $('input[name="dir"][value="out"]');
    out.checked = true;
    toast("物理口下行不可用（ifb 灰显）——方向已切 out");
    scheduleSet(true);
  }
}
function renderCapsLine() {
  const c = $("#capsline");
  if (!caps) { c.textContent = "capabilities 加载中…"; return; }
  c.innerHTML = `caps：seed=${caps.seed ? "✓" : "✗（tc -V<6.6 灰显）"} · dir_lo=${caps.dir_lo ? "✓" : "✗"} · `
    + `ifb_ingress=${caps.ifb_ingress ? "✓" : "✗"}`
    + (caps.ifb_ingress ? "" : ` <span class="note">${esc(caps.ifb_reason)}</span>`);
  const seed = $("#seed");
  seed.disabled = !caps.seed;
  seed.title = caps.seed ? "" : "seed 需 iproute2 tc -V ≥6.6（当前宿主不支持）";
}

// ---------- 中栏：owner 分组流表 ----------
const PLAY_PORT = 8080; // 文档化常量：浏览器面入口（Caddy，deploy/caddy/Caddyfile.native）；跨端口天然可用
function playUrl(room) {
  return `http://${location.hostname}:${PLAY_PORT}/?room=${encodeURIComponent(room)}`;
}
function renderStreams() {
  const host = $("#streams");
  host.textContent = "";
  if (!frame) { host.append(el("p", "empty", "等待首帧状态…")); return; }
  const rows = frame.streams || [];
  if (rows.length === 0) {
    if (streamsNote) { // 降级模式：整列替换（后端规则=scope 非空 ∧ stats 不可达 → 拒绝报因，禁静默转全口）
      const p = el("p", "warn", `stats 不可达：仅按 <b>${esc(ui.iface)}</b>/媒体口集整形。报因：<span class="note">${esc(streamsNote)}</span>`);
      const retry = el("a", "play", "↻重试");
      retry.href = "#";
      retry.addEventListener("click", (ev) => { ev.preventDefault(); fetchStreamsNote(); });
      p.append(" ", retry);
      host.append(p);
    } else {
      host.append(el("p", "empty", "无活性流（server 无 producer？host 未推流？）"));
    }
    return;
  }
  const groups = new Map();
  for (const s of rows) {
    const k = s.owner || "（无主）";
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(s);
  }
  for (const [owner, list] of [...groups.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
    const box = el("div", "grp");
    const head = el("header");
    const gcb = el("input");
    gcb.type = "checkbox";
    gcb.checked = ui.selDevices.has(owner) || (owner !== "（无主）" && list.every((s) => ui.selRooms.has(s.room)));
    if (owner === "（无主）") {
      gcb.disabled = true;
      gcb.title = "无主房间不可按设备定向（owner 缺失；旧 server 经 peer_id 兜底仅限 --device 命令行）";
    } else {
      gcb.addEventListener("change", () => {
        if (gcb.checked) { ui.selDevices.add(owner); ui.selRooms.clear(); }
        else ui.selDevices.delete(owner);
        scheduleSet(false);
      });
    }
    head.append(gcb, el("span", "owner", esc(owner)), el("span", "cnt", `${list.length} 流`));
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
            toast("已清空勾选：定向保持上次施加值（set 不收空集）；回段级整形请关闸后无勾选重开");
        }
        scheduleSet(false);
      });
      const kb = s.stale ? "stale" : (s.kbps == null ? "—" : s.kbps + "k");
      row.append(
        cb,
        el("span", "room", esc(s.room)),
        el("span", "kb", kb),
        el("span", "badge " + (s.live ? "live" : "age"), s.live ? "live" : "老化窗内"),
        s.stale ? el("span", "badge stale", "stale") : "",
      );
      const a = el("a", "play", "↗播放");
      a.href = playUrl(s.room);
      a.target = "_blank";
      a.rel = "noopener";
      a.title = `新标签打开播放页（:${PLAY_PORT} 入口 · room=${s.room}）`;
      row.append(a);
      box.append(row);
    }
    host.append(box);
  }
}

// ---------- 右栏：uPlot（kbps 断线缺口 + dropped 阶梯 + 事件竖线） ----------
const PALETTE = ["#4f9cf9", "#3fb950", "#e3b341", "#f85149", "#bc8cff", "#39c5cf", "#ff7b72", "#d2a8ff", "#7ee787", "#ffa657"];
const chartState = { t: [], rooms: [], y: new Map(), drop: [], evs: [] }; // evs 环上限 200（design §UI 落码注记）
let kbpsChart = null, dropChart = null;

window.__wnet.charts = chartState; // 同帧 oracle 数据面（序列对齐状态）
function feedCharts(f) {
  const t = Date.now() / 1000;
  chartState.t.push(t);
  const seen = new Set();
  for (const s of f.streams || []) {
    seen.add(s.room);
    if (!chartState.y.has(s.room)) chartState.y.set(s.room, new Array(chartState.t.length - 1).fill(null));
    // 两语义（首点 null / stale）都渲染为缺口；行内 badge 区分来源——禁混 0（design §API 状态帧）
    chartState.y.get(s.room).push(s.stale || s.kbps == null ? null : s.kbps);
  }
  for (const [room, arr] of chartState.y) if (!seen.has(room)) arr.push(null);
  chartState.drop.push(f.tc?.dropped ?? null);
  for (const e of f.ev || []) {
    chartState.evs.push(Object.assign({ _t: t }, e));
    if (chartState.evs.length > 200) chartState.evs.shift();
  }
  // ponytail: 每帧 setData（2s 节拍）+ rooms 集合变化才重建；uPlot 增量 addSeries 未用——流数 <20，重建成本可忽略
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
function eventLines(u) { // 事件竖线 draw hook 自绘（时间轴秒单位，与 x=time 同域）
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
    series: [{ label: "时间" }, ...chartState.rooms.map((r, i) => ({
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
    series: [{ label: "时间" }, { label: "dropped", stroke: "#f85149", fill: "rgba(248,81,73,.12)", spanGaps: false }],
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

// ---------- 帧渲染（一切回显的唯一通路） ----------
function renderRing(f) {
  const txt = $("#ring-txt"), fg = $("#ring-fg");
  const exp = f.expires_at;
  if (exp == null) { txt.textContent = "—"; fg.style.strokeDashoffset = 119.4; return; }
  const rem = Math.max(0, exp - Date.now());
  if (exp === 0) { txt.textContent = "∞"; fg.style.strokeDashoffset = 0; return; }
  txt.textContent = Math.ceil(rem / 1000) + "s";
  // ponytail: 环比例参照缺省 300s（帧不含总时长字段）；纯视觉，文字才是判据
  fg.style.strokeDashoffset = 119.4 * Math.min(1, rem / 300000);
}
function renderHeader(f) {
  const h = $("#hstat");
  h.textContent = "";
  const chip = (k, v, cls) => h.append(el("span", "chip" + (cls ? " " + cls : ""), `${k} <b>${esc(v)}</b>`));
  if (f.spec) { chip("损伤", "ACTIVE", "act"); chip("scope", f.scope ?? "?"); chip("dir", f.dir ?? "?"); }
  if (f.job) chip("job", f.job.name ?? "?");
  if (f.tc?.dropped != null) chip("dropped", f.tc.dropped);
}
function echoSpecInputs(f) { // 外部 CLI/其他端写 → 帧回显（聚焦控件不打断输入）
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
  // 真值表灰显优先：帧值指向被灰显(disabled)项时不打断本地选择（如物理口期帧仍带旧 dir=both）
  if (radio && !radio.disabled && document.activeElement !== radio) radio.checked = true;
  else if (radio && radio.disabled && cur && !cur.disabled) void cur;
}
function onFrame(f) {
  window.__wnet.frameCount++;
  window.__wnet.lastFrame = f;
  window.__wnet.ready = true;
  frame = f;
  if (f.frame_error) { toast(`状态帧错误：${f.frame_error}`, true); return; }
  $("#conn").className = "conn on";
  $("#conn").textContent = "● 帧流 2s";
  $("#master").checked = f.spec != null;
  renderRing(f);
  renderHeader(f);
  echoSpecInputs(f);
  renderStreams();
  feedCharts(f);
  renderScenJob(f);
}
function renderScenJob(f) {
  $("#scen-job").textContent = f.job
    ? `运行中：${f.job.name}（pid ${f.job.pid}）——done/total 随 T8`
    : "无活跃 job";
}

// ---------- SSE ----------
function connectSSE() {
  es = new EventSource("/v1/events?token=" + encodeURIComponent(auth)); // query-token 唯一豁免面
  es.onmessage = (m) => {
    let f;
    try { f = JSON.parse(m.data); } catch { toast("状态帧 JSON 解析失败", true); return; }
    onFrame(f);
  };
  es.onerror = () => {
    $("#conn").className = "conn off";
    $("#conn").textContent = "● 断线（EventSource 自动重连中）";
  };
}

// ---------- S 页签 ----------
function renderScenList() {
  const ul = $("#scenlist");
  ul.textContent = "";
  if (scenarios.length === 0) { ul.append(el("li", "empty", "无剧本（weaknet.d/scenarios 或 /v1/scenarios 报因）")); return; }
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
  const r = await api("/v1/scenarios");
  scenarios = (r.body?.scenarios ?? []).slice();
  if (scenarios.length && !scenPick) scenPick = scenarios[0];
  renderScenList();
}
async function scenRun() {
  if (!scenPick) { toast("先选一个剧本"); return; }
  const r = await api("/v1/scenario/run", { method: "POST", body: JSON.stringify({ file: scenPick }), headers: { "Content-Type": "application/json" } });
  if (r.status === 501) toast(`剧本引擎未到场（501）：${r.body?.error ?? "随 T8"}`);
  else if (r.status !== 200 && r.status !== 0) toast(`run 失败（${r.status}）：${r.body?.error ?? "?"}`, true);
}
async function scenStop() {
  const r = await api("/v1/scenario/stop", { method: "POST", body: "{}", headers: { "Content-Type": "application/json" } });
  if (r.status === 501) toast(`stop 同随 T8（501）：${r.body?.error ?? ""}`);
  else if (r.status !== 200 && r.status !== 0) toast(`stop 失败（${r.status}）：${r.body?.error ?? "?"}`, true);
}

// ---------- 引导加载 ----------
async function fetchStreamsNote() {
  const r = await api("/v1/streams");
  streamsNote = r.body?.note || "";
  renderStreams();
}
async function boot() {
  if (!auth) { showGate(); return; }
  $("#gate-clear").addEventListener("click", () => {
    sessionStorage.removeItem(AUTH_KEY);
    location.replace(location.pathname);
  });
  const [c, p] = await Promise.all([api("/v1/capabilities"), api("/v1/profiles")]);
  if (c.status === 401 || p.status === 401) return; // gate 已由 api() 呈现
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
  // spec 行：input → debounce 400；range 额外 pointerup 即发（风暴案）
  for (const i of $$("#spec-rows input, #spec-rows select")) {
    i.addEventListener("input", () => {
      if (i.type === "range") $("#loss-num").value = i.value;
      if (i.id === "loss-num") $("#loss-range").value = i.value;
      scheduleSet(false); // 含滑块：拖动期 400ms 收敛，pointerup 即发兜底
    });
    if (i.type === "range") i.addEventListener("pointerup", () => scheduleSet(true));
  }
  $("#loss-mode").addEventListener("change", () => {
    $("#gem").hidden = $("#loss-mode").value !== "gemodel";
    scheduleSet(true);
  });
  // iface 只改本地真值表面板灰显；不进 set（物理口误下发=真实网卡受损；换 iface 走关闸→重开 apply）
  $("#iface").addEventListener("input", () => { ui.iface = $("#iface").value.trim(); refreshDirGate(); renderStreams(); });
  for (const r of $$('input[name="dir"]')) r.addEventListener("change", () => scheduleSet(true));
  $("#profile").addEventListener("change", () => {
    const v = $("#profile").value;
    if (v) toast(`profile「${v}」是 CLI 预设（weaknet apply --profile ${v}）；面板施加通道未接（T7 边界），下方手工编辑等效`);
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
boot();
