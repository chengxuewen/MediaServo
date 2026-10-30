// scripts/weaknet.d/sampler.mjs -- browser-side tile sampler
// (pattern source: e2e-bitrate-probe.cjs + the MiniStatsCard DOM)
// Output JSONL: {t_utc, src:"browser", room, st, fps, res, kbps, jitter_ms}
// Usage: node sampler.mjs <seconds> [interval_s]
import { chromium } from 'playwright';

const SECS = parseInt(process.argv[2] || '60', 10);
const IV = parseInt(process.argv[3] || '5', 10);
const b = await chromium.launch({
  args: ['--no-sandbox'],
  executablePath: process.env.CHROMIUM_PATH || undefined,
});
const p = await b.newPage();
await p.goto('http://127.0.0.1:8080/', { waitUntil: 'networkidle' });
await p.fill('input[type=text], input#username', 'admin');
await p.fill('input[type=password]', process.env.WEAKNET_ADMIN_PASS || '');
await p.click('button[type=submit]');
await p.waitForSelector('button:has-text("Play"), button[title*=播放]', { timeout: 40000 });
for (const btn of await p.$$('button:has-text("Play"), button[title*=播放]')) await btn.click().catch(() => {});
const num = (s, re) => { const m = s.match(re); return m ? m[1] : null; };
const t0 = Date.now();
while (Date.now() - t0 < SECS * 1000) {
  const tiles = await p.evaluate(() => [...document.querySelectorAll('.video-player')].map(t => t.innerText || ''));
  const tstr = new Date().toISOString().replace(/\.\d+Z/, 'Z');
  for (const txt of tiles) {
    const room = num(txt, /(vehicle_test\w+|audio-[\w-]+|\S*stream\S*)/);
    if (!room) continue;
    console.log(JSON.stringify({
      t_utc: tstr, src: 'browser', room,
      st: num(txt, /(LIVE|连接中|源离线|连接失败|连接已断开)/),
      fps: parseFloat(num(txt, /帧率\s*([\d.]+)/) || '0') || null,
      res: num(txt, /分辨率\s*([\dxX]+]+)/),
      kbps: (() => { const m = txt.match(/码率\s*([\d.]+)\s*(K|M)/); return m ? parseFloat(m[1]) * (m[2] === 'M' ? 1000 : 1) : null; })(),
      jitter_ms: parseFloat(num(txt, /抖动\s*([\d.]+)/) || '0') || null,
    }));
  }
  await new Promise(r => setTimeout(r, IV * 1000));
}
await b.close();
