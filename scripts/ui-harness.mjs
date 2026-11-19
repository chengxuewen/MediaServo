// P7.4 UI assertion harness (C61): Marionette-driven Firefox headless.
// Read-back channel = WebDriver:ExecuteScript returning JSON (no OCR / no exit-code trust).
// Products: docs/ui/prototypes/.harness/<case>.png (stat>0 enforced) + results.json.
// Run: node scripts/ui-harness.mjs   (requires firefox on PATH; serves prototype over local HTTP)
import net from 'node:net';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PROTO = path.join(ROOT, 'docs/ui/prototypes');
const OUT = path.join(PROTO, '.harness');
fs.mkdirSync(OUT, { recursive: true });
const PORT = Number(process.env.HARNESS_PORT || 8199);

/* C66: board alignment across width bands (fixed icon slots / card reflow / no overflow). */
const BOARD = `goto('intercom');var rows=[...document.querySelectorAll('.board-row')];
  var hs=rows.map(r=>Math.round(r.getBoundingClientRect().height));
  var acts=rows.map(r=>Math.round(r.querySelector('.row-actions').getBoundingClientRect().height));
  var rights=rows.map(r=>Math.round(r.querySelector('.row-actions').getBoundingClientRect().right));
  var inside=rows.every(r=>{var rr=r.getBoundingClientRect();var mr=r.querySelector('[data-a=mon]').getBoundingClientRect();return mr.right<=rr.right-1;});
  var holder=rows.every(r=>r.querySelector('.floor-holder').getBoundingClientRect().width>20);
  var noOv=rows.every(r=>r.scrollWidth<=r.clientWidth);
  return {uniformH:new Set(hs).size===1,singleLine:new Set(acts).size===1&&acts[0]<=30,rightEdge1:new Set(rights).size===1,inside,holder,noOv};`;

const CASES = [
  ['accounts-narrow', `myRole='admin';goto('accounts');
    var noOv=document.documentElement.scrollWidth<=innerWidth;
    var card=getComputedStyle(document.querySelector('#accBody td:nth-child(2)')).display==='flex';
    var tailHidden=getComputedStyle(document.querySelector('#modeInd .mi-tail')).display==='none';
    return {noOv,card,tailHidden};`, 600],
  ['admin-controls', `var ids=['devices','rooms','sessions','streams','config','keys','audit','settings','signaling'];var bad=[];
    ids.forEach(id=>{goto(id);document.querySelectorAll('#page-'+id+' .btn,#page-'+id+' .chip-btn,#page-'+id+' .seg,#page-'+id+' .search,#page-'+id+' .iconbtn,#page-'+id+' .msg-in').forEach(e=>{if(!e.getClientRects().length)return;var h=Math.round(e.getBoundingClientRect().height);if(h!==30)bad.push(id+':'+h);});});
    var h30=bad.length===0;
    goto('devices');var s0=Math.round(document.querySelector('#devSeg').getBoundingClientRect().height);
    document.querySelector('#devSeg [data-f=pending]').click();var s1=Math.round(document.querySelector('#devSeg').getBoundingClientRect().height);
    var stateStable=s0===30&&s1===30;
    var aw=[...document.querySelectorAll('#page-devices .tbl .actions .btn')].map(b=>Math.round(b.getBoundingClientRect().width));
    var actionsUniform=aw.length>0&&new Set(aw).size===1&&aw[0]===72;
    return {h30,stateStable,actionsUniform};`],
  ['admin-spoke', `goto('admin');document.querySelector('#adminHub [data-go=devices]').click();
    var back=!!document.querySelector('#page-devices [data-back]');
    var tw=document.querySelector('#page-devices .tbl-wrap');var noHScroll=tw.scrollWidth<=tw.clientWidth;
    var card=getComputedStyle(document.querySelector('#page-devices .tbl tbody td:nth-child(2)')).display==='flex';
    var crumbVisible=getComputedStyle(document.getElementById('crumb')).display!=='none';
    document.querySelector('#page-devices [data-back]').click();var backWorks=current==='admin';
    return {back,noHScroll,card,crumbVisible,backWorks};`, 600],
  ['board-1100', BOARD, 1100],
  ['board-1000', BOARD, 1000],
  ['board-1200', BOARD, 1200],
];

/* C65 negative self-check: sabotage that MUST turn the case red (run with --negative). */
const NEG = {
  floor: `GROUPS.find(g=>g.id==='talk-bravo').q=[];`,
  queue: `FLOOR_CFG.queueMax=99;`,
  'board-1100': `var st=document.createElement('style');st.textContent='.row-actions{display:flex;flex-wrap:wrap;width:120px}';document.head.appendChild(st);`,
  evidence: `evPurge=function(){};`,
  'device-drawer': `openDevDrawer=function(){};`,
  'device-config': `flushStaged=function(){};`,
  'device-role': `setGuard=function(){};`,
  accounts: `renderAccounts=function(){};`,
  psk: `doRotate=function(){};`,
  creds: `resetSecret=function(){};`,
  fleet: `renderFleetStrip=function(){};`,
  'monitor-link': `renderAvail=function(){};`,
  'audit-ring': `renderAudit=function(){};`,
  'nav-collapse': `syncNavCollapse=function(){};`,
  'accounts-narrow': `renderAccounts=function(){};`,
  notif: `renderNotif=function(){};`,
};

function serve() {
  return new Promise(res => {
    const srv = http.createServer((req, rq) => {
      const f = path.join(PROTO, req.url === '/' ? 'portal-unified.html' : req.url.split('?')[0]);
      fs.readFile(f, (e, d) => { if (e) { rq.statusCode = 404; rq.end(); } else { rq.setHeader('content-type', 'text/html'); rq.end(d); } });
    });
    srv.listen(PORT, '127.0.0.1', () => res(srv));
  });
}

class Marionette {
  constructor(sock) { this.sock = sock; this.buf = ''; this.id = 0; this.pending = new Map();
    sock.on('data', d => { this.buf += d.toString(); this.drain(); }); }
  drain() { for (;;) { const i = this.buf.indexOf(':'); if (i < 0) return;
      const len = parseInt(this.buf.slice(0, i), 10); if (isNaN(len)) { this.buf = ''; return; }
      if (this.buf.length < i + 1 + len) return;
      const msg = JSON.parse(this.buf.slice(i + 1, i + 1 + len)); this.buf = this.buf.slice(i + 1 + len);
      if (Array.isArray(msg) && msg[0] === 1) { const p = this.pending.get(msg[1]); if (p) { this.pending.delete(msg[1]); p(msg[2], msg[3]); } } } }
  cmd(name, params = {}) { const id = ++this.id; const payload = JSON.stringify([0, id, name, params]);
    return new Promise((res, rej) => { this.pending.set(id, (err, r) => err ? rej(new Error(JSON.stringify(err))) : res(r));
      this.sock.write(payload.length + ':' + payload); }); }
}

async function main() {
  const srv = await serve();
  const prof = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-prof-'));
  const ff = spawn('firefox', ['--headless', '--marionette', '--profile', prof, '--no-remote', 'about:blank'], { stdio: 'ignore' });
  await new Promise(r => setTimeout(r, 4000));
  const sock = await new Promise((res, rej) => { const s = net.connect(2828, '127.0.0.1', () => res(s)); s.on('error', rej); });
  await new Promise(r => sock.once('data', r)); // handshake
  const m = new Marionette(sock);
  await m.cmd('WebDriver:NewSession', {});
  const results = {}; let fail = 0;
  const NEG_MODE = process.argv.includes('--negative');
  let CURW = 1440;
  const run = async (src) => {
    const inner = 'try{window.__result=(function(){' + src + '})();}catch(e){window.__result={__err:String(e && e.message || e)+" @"+String(e && e.stack || "").slice(0,220)};}';
    const wrapped = "var s=document.createElement('script');s.textContent=" + JSON.stringify(inner) + ";document.documentElement.appendChild(s);var r=window.__result;window.__result=undefined;return r;";
    const exec = () => m.cmd('WebDriver:ExecuteScript', { script: wrapped, args: [] });
    const withTimeout = (p, ms) => Promise.race([p, new Promise((_, rej) => setTimeout(() => rej(new Error('execute-timeout')), ms))]);
    const unwrap = r => ({ val: (r && typeof r === 'object' && 'value' in r) ? r.value : r, err: null });
    try { return unwrap(await withTimeout(exec(), 25000)); }
    catch (e) {
      if (String(e.message) !== 'execute-timeout') return { val: {}, err: String(e.message) };
      // Marionette headless flake: re-navigate and retry once (single-part cases are idempotent reads/actions)
      try {
        await m.cmd('WebDriver:Navigate', { url: `http://127.0.0.1:${PORT}/portal-unified.html` });
        await new Promise(r => setTimeout(r, 700));
        await m.cmd('WebDriver:SetWindowRect', { width: CURW, height: 900 });
        await new Promise(r => setTimeout(r, 300));
        return unwrap(await withTimeout(exec(), 25000));
      } catch (e2) { return { val: {}, err: 'execute-timeout(retry): ' + String(e2.message) }; }
    }
  };
  const runMany = async (script, name) => {
    const parts = Array.isArray(script) ? script : [script];
    let val = {}, err = null;
    for (let i = 0; i < parts.length; i++) {
      if (process.env.H9DBG) console.log('PART', name, i, 'start');
      const r = await run(parts[i]);
      if (process.env.H9DBG) console.log('PART', name, i, 'done', JSON.stringify(r.val).slice(0,140));
      if (r.val && r.val.__err) { err = r.val.__err; console.log('PART-ERR', name, i, r.val.__err); }
      if (r.err) err = r.err;
      if (r.val && typeof r.val === 'object') val = Object.assign(val, r.val);
      if (i < parts.length - 1) await new Promise(res => setTimeout(res, 450));
    }
    return { val, err };
  };
  for (const [name, script, width] of CASES) {
    if (process.env.H9ONLY && name !== process.env.H9ONLY) continue;
    await m.cmd('WebDriver:Navigate', { url: `http://127.0.0.1:${PORT}/portal-unified.html` });
    await new Promise(r => setTimeout(r, 600));
    CURW = width || 1440;
    await m.cmd('WebDriver:SetWindowRect', { width: CURW, height: 900 });
    await new Promise(r => setTimeout(r, 300));
    if (NEG_MODE) {
      const neg = NEG[name];
      if (!neg) { console.log(`NEG-SKIP ${name}`); continue; }
      await run(neg);
      const { val, err } = await runMany(script, name);
      const got = (val && typeof val === 'object') ? val : null;
      const red = !!err || !got || Object.keys(got).length === 0 || Object.values(got).some(v => v !== true);
      if (!red) fail++;
      results[name] = { negative: true, red, assertions: val, error: err };
      console.log(`${red ? 'NEG-OK' : 'NEG-BAD'} ${name} ${JSON.stringify(val)}${err ? ' err=' + err : ''}`);
      continue;
    }
    const { val, err } = await runMany(script, name);
    const shotR = await m.cmd('WebDriver:TakeScreenshot', {}); const shot = typeof shotR === 'string' ? shotR : (shotR.value || shotR.screenshot || shotR.data);
    const png = path.join(OUT, name + '.png');
    fs.writeFileSync(png, Buffer.from(shot, 'base64'));
    const size = fs.statSync(png).size;
    const pass = !err && !!val && typeof val === 'object' && Object.keys(val).length > 0 && Object.values(val).every(v => v === true) && size > 0;
    if (!pass) fail++;
    results[name] = { pass, size, assertions: val, error: err };
    console.log(`${pass ? 'PASS' : 'FAIL'} ${name} size=${size} ${JSON.stringify(val)}${err ? ' err=' + err : ''}`);
  }
  fs.writeFileSync(path.join(OUT, NEG_MODE ? 'results-negative.json' : 'results.json'), JSON.stringify({ when: new Date().toISOString(), mode: NEG_MODE ? 'negative' : 'positive', fail, results }, null, 2));
  await m.cmd('WebDriver:DeleteSession', {}).catch(() => {});
  sock.end(); ff.kill(); srv.close();
  console.log(NEG_MODE ? (fail === 0 ? 'NEGATIVE SELF-CHECK: ALL SABOTAGED CASES RED AS EXPECTED' : `NEGATIVE SELF-CHECK ${fail} BAD`) : (fail === 0 ? 'HARNESS ALL GREEN' : `HARNESS ${fail} FAIL`));
  process.exit(fail === 0 ? 0 : 1);
}
main().catch(e => { console.error('HARNESS ERROR', e); process.exit(1); });