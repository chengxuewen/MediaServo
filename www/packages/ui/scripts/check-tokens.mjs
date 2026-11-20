#!/usr/bin/env node
/* Token existence check (PIT-258 guard) — @mediaservo/ui, talk-app T1.
 *
 * Why: an undefined CSS custom property silently zeroes the WHOLE declaration
 * (PIT-258: undefined --spN dropped entire rules with no error). This check
 * fails the lane instead: every var(--ms-*) referenced under www/ must be
 * defined in @mediaservo/ui (tokens.css or themes/*.css).
 *
 * Run: node scripts/check-tokens.mjs   (wired as the ui package `lint`)
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const uiRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const wwwRoot = join(uiRoot, '..', '..');

const defFiles = [
  join(uiRoot, 'tokens.css'),
  join(uiRoot, 'themes', 'fluent.css'),
  join(uiRoot, 'themes', 'material.css'),
];
const defined = new Set();
for (const f of defFiles) {
  for (const m of readFileSync(f, 'utf8').matchAll(/(--ms-[a-z0-9-]+)\s*:/g)) defined.add(m[1]);
}

const EXTS = new Set(['.css', '.ts', '.tsx', '.html']);
const SKIP = new Set(['node_modules', 'dist', '.turbo', '.harness']);
const refRe = /var\(\s*(--ms-[a-z0-9-]+)/g;
const refs = [];

function walk(dir) {
  let entries;
  try { entries = readdirSync(dir); } catch { return; }
  for (const e of entries) {
    const p = join(dir, e);
    let st;
    try { st = statSync(p); } catch { continue; }
    if (st.isDirectory()) {
      if (!SKIP.has(e)) walk(p);
    } else if (EXTS.has(e.slice(e.lastIndexOf('.')))) {
      const lines = readFileSync(p, 'utf8').split('\n');
      lines.forEach((line, i) => {
        for (const m of line.matchAll(refRe)) refs.push({ file: p, line: i + 1, name: m[1] });
      });
    }
  }
}

for (const area of ['apps', 'packages', 'tests']) walk(join(wwwRoot, area));

const missing = refs.filter((r) => !defined.has(r.name));
const files = new Set(refs.map((r) => r.file)).size;

if (missing.length) {
  console.error(`token check: FAIL — ${missing.length} undefined --ms-* reference(s):`);
  for (const r of missing) console.error(`  ${r.file.replace(wwwRoot + '/', '')}:${r.line}  ${r.name}`);
  process.exit(1);
}
console.log(`token check: OK — ${refs.length} var(--ms-*) refs across ${files} files, ${defined.size} tokens defined.`);
