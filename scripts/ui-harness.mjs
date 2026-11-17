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
const PORT = 8199;

const CASES = [
  ['floor', `goto('intercom');enterChannel('talk-delta');document.getElementById('chReq').click();
    var D=GROUPS.find(g=>g.id==='talk-delta');var idleGrant=D.state==='held'&&D.holder==='dispatch.x';
    enterChannel('talk-bravo');pttDown();var B=GROUPS.find(g=>g.id==='talk-bravo');var noJump=B.holder!=='dispatch.x';
    return {idleGrant,noJump};`],
  ['queue', `goto('intercom');var B=GROUPS.find(g=>g.id==='talk-bravo');var n0=B.q.length;
    for(var i=0;i<12;i++)qPush(B,{n:'x'+i,prio:false,you:false});var cap=B.q.length<=8;
    floorAction(GROUPS.indexOf(B),'prio');var rew=B.q.every(x=>x.prio===B.priority);return {cap,rew};`],
  ['replay', `goto('intercom');var A=GROUPS[0],B=GROUPS[1],C=GROUPS[2];
    enterChannel(A.id);recConsent[A.id]=true;recPush(A,'f',5);enterChannel(B.id,true);var scanKeeps=(recBuf[A.id]||[]).length===1;
    recConsent[B.id]=true;recPush(B,'f',4);enterChannel(C.id);var manualClears=(recBuf[B.id]||[]).length===0;
    recConsent[C.id]=true;recPush(C,'f',3);goto('monitor');var xpageClears=(recBuf[C.id]||[]).length===0;
    return {scanKeeps,manualClears,xpageClears};`],
  ['emergency', `goto('intercom');enterChannel('talk-alpha');var e=document.getElementById('chEmerg');e.click();
    var l1=document.getElementById('chEmerg').textContent;document.querySelector('.ch-tab[data-tab=rec]').click();document.querySelector('.ch-tab[data-tab=floor]').click();
    var l2=document.getElementById('chEmerg').textContent;var armPersist=l1==='Confirm insert?'&&l2==='Confirm insert?';
    document.getElementById('chEmerg').click();var A=GROUPS[0];var emergNoPress=pttHeld===false&&A.holder==='dispatch.x';
    return {armPersist,emergNoPress};`],
  ['tabs-size', `goto('intercom');enterChannel('talk-delta');document.querySelector('.ch-tab[data-tab=diag]').click();
    var m=id=>{var r=document.getElementById(id).getBoundingClientRect();return Math.round(r.width)+'x'+Math.round(r.height);};
    var diag=m('chVox')==='56x30'&&m('chCodec')==='56x30'&&m('chLock')==='56x30';
    document.querySelector('.ch-tab[data-tab=comms]').click();var tabSwitch=!!document.getElementById('chMsgSend');
    return {diag,tabSwitch};`],
  ['nav', `var icos=[...document.querySelectorAll('.nav-ico')];var hrefs=icos.map(x=>x.querySelector('use').getAttribute('href'));
    return {navSvg:icos.every(x=>!!x.querySelector('svg')),unique:new Set(hrefs).size===hrefs.length,countOk:hrefs.length===16};`],
  ['notif', `goto('monitor');applyProto('sim','extemerg');var c=document.getElementById('floorNotif');var xpage=!c.hidden;
    c.click();var clickLand=current==='intercom'&&chanOpen!==null;goto('monitor');applyProto('sim','extemerg');
    goto('intercom');var inpageHide=c.hidden;return {xpage,clickLand,inpageHide};`],
  ['groups', `goto('intercom');var gm=document.getElementById('grpManage');gm.focus();gm.click();
    document.getElementById('grpNewName').value='Harness crew';document.getElementById('grpCreate').click();
    var T=GROUPS.find(g=>g.name==='Harness crew');var created=!!T;
    var A=GROUPS[0];evConsent[A.id]=true;evPush(A,'f',5);enterChannel(A.id);
    document.querySelector('[data-gd="'+A.id+'"]').click();document.getElementById('cfmOk').click();
    var cascade=evBuf[A.id]===undefined&&chanOpen===null;
    document.getElementById('grpClose').click();var focusRestore=document.activeElement===gm;
    myRole='field';gm.focus();gm.click();document.getElementById('grpNewName').value='X';document.getElementById('grpCreate').click();
    var roleGuard=!GROUPS.some(g=>g.name==='X');myRole='dispatcher';
    enterChannel(GROUPS[1].id);gm.focus();gm.click();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var escModal=chanOpen!==null&&!document.getElementById('grpScrim').classList.contains('show');
    return {created,cascade,focusRestore,roleGuard,escModal};`],
  ['evidence', `goto('intercom');var D=GROUPS.find(g=>g.id==='talk-delta');enterChannel(D.id);evConsent[D.id]=true;
    evBuf[D.id]=[{who:'a',dur:1,at:Date.now()-8*86400000,alert:'x',retain:7},{who:'b',dur:2,at:Date.now(),alert:'y',retain:7}];
    document.querySelector('.ch-tab[data-tab=rec]').click();
    var card=()=>[...document.querySelectorAll('.ch-card')].find(c=>c.textContent.includes('Event evidence'));
    var before=card().querySelectorAll('.replay-row').length;floorTick();var after=card().querySelectorAll('.replay-row').length;
    var purge=before===2&&after===1;
    document.getElementById('chEv').click();document.getElementById('cfmOk').click();var revoke=evBuf[D.id]===undefined;
    return {purge,revoke};`],
];

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
  for (const [name, script] of CASES) {
    await m.cmd('WebDriver:Navigate', { url: `http://127.0.0.1:${PORT}/portal-unified.html` });
    await new Promise(r => setTimeout(r, 600));
    let val, err = null;
    const inner = 'window.__result=(function(){' + script + '})();';
    const wrapped = "var s=document.createElement('script');s.textContent=" + JSON.stringify(inner) + ";document.documentElement.appendChild(s);var r=window.__result;window.__result=undefined;return r;";
    try { const r = await m.cmd('WebDriver:ExecuteScript', { script: wrapped, args: [] }); val = (r && typeof r === 'object' && 'value' in r) ? r.value : r; } catch (e) { err = String(e.message); val = {}; }
    const shotR = await m.cmd('WebDriver:TakeScreenshot', {}); const shot = typeof shotR === 'string' ? shotR : (shotR.value || shotR.screenshot || shotR.data);
    const png = path.join(OUT, name + '.png');
    fs.writeFileSync(png, Buffer.from(shot, 'base64'));
    const size = fs.statSync(png).size;
    const pass = !err && Object.values(val).every(v => v === true) && size > 0;
    if (!pass) fail++;
    results[name] = { pass, size, assertions: val, error: err };
    console.log(`${pass ? 'PASS' : 'FAIL'} ${name} size=${size} ${JSON.stringify(val)}${err ? ' err=' + err : ''}`);
  }
  fs.writeFileSync(path.join(OUT, 'results.json'), JSON.stringify({ when: new Date().toISOString(), fail, results }, null, 2));
  await m.cmd('WebDriver:DeleteSession', {}).catch(() => {});
  sock.end(); ff.kill(); srv.close();
  console.log(fail === 0 ? 'HARNESS ALL GREEN' : `HARNESS ${fail} FAIL`);
  process.exit(fail === 0 ? 0 : 1);
}
main().catch(e => { console.error('HARNESS ERROR', e); process.exit(1); });