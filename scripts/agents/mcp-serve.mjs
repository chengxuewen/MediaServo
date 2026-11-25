#!/usr/bin/env node
// mcp-serve.mjs — pinned MCP launcher (agent toolchain home: scripts/agents).
//
// Pins live in scripts/agents/mcp-servers.json (single source). npm servers are
// installed at the exact version into scripts/agents/node_modules/.mcp/<name>;
// pixi servers run their declared task in the declared environment. Pin drift
// fails loudly. PROJECT_ROOT (MEDIASERVO_AGENT_ROOT) is passed to the server so
// a parent workspace can point it at the workspace root instead of this repo.

import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, readdirSync } from 'node:fs'
import { dirname, isAbsolute, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..', '..')
// Setup writes this record (out/ is gitignored); the launcher only reads it.
const RECORD_PATH = join(ROOT, 'out', 'mcp-browser.json')
const PROJECT = process.env.MEDIASERVO_AGENT_ROOT
  || process.env.MEDIASERVO_AGENT_PROJECT_ROOT
  || ROOT
const isWin = process.platform === 'win32'

function arg(name) {
  const i = process.argv.indexOf(`--${name}`)
  if (i === -1 || i + 1 >= process.argv.length) return undefined
  return process.argv[i + 1]
}

const from = arg('from')
const name = arg('name')
if (!from || !name) {
  console.error('[mcp-serve] usage: --from <registry.json> --name <server>')
  process.exit(2)
}

const registryPath = isAbsolute(from) ? from : join(ROOT, from)
let spec
try {
  spec = JSON.parse(readFileSync(registryPath, 'utf8'))[name]
} catch (err) {
  console.error(`[mcp-serve] cannot read ${registryPath}: ${err.message}`)
  process.exit(2)
}
if (!spec) {
  console.error(`[mcp-serve] no server "${name}" in ${registryPath}`)
  process.exit(2)
}

function spawnAndExit(command, args, options) {
  const child = spawn(command, args, options)
  child.on('error', (err) => {
    console.error(`[mcp-serve] failed to start ${name}:`, err.message)
    process.exit(1)
  })
  child.on('exit', (code) => process.exit(code ?? 1))
}

// --- pixi-backed servers (serena): run the declared task in the declared env ---
if (spec.kind === 'pixi') {
  const envDir = join(ROOT, '.pixi', 'envs', spec.pixi_env)
  if (!existsSync(envDir)) {
    console.error(`[mcp-serve] pixi env "${spec.pixi_env}" missing; run: ./msrtc.sh mcp setup`)
    process.exit(1)
  }
  const pixi = process.env.PIXI_BIN || 'pixi'
  spawnAndExit(
    pixi,
    ['run', '--manifest-path', join(ROOT, 'pixi.toml'), '-e', spec.pixi_env, spec.task],
    {
      stdio: 'inherit',
      cwd: ROOT,
      env: { ...process.env, MEDIASERVO_AGENT_ROOT: PROJECT },
    },
  )
} else if (spec.kind === 'npm') {
  // One prefix per server: a shared prefix makes npm prune the other server's
  // packages on every install (observed: installing playwright removed codegraph).
  const PREFIX = join(HERE, 'node_modules', '.mcp', name)
  const binPath = join(PREFIX, 'node_modules', '.bin', isWin ? `${spec.bin}.cmd` : spec.bin)

  function installedVersion() {
    const parts = spec.pkg.startsWith('@') ? spec.pkg.split('/') : [spec.pkg]
    const manifest = join(PREFIX, 'node_modules', ...parts, 'package.json')
    if (!existsSync(manifest)) return null
    try {
      return JSON.parse(readFileSync(manifest, 'utf8')).version
    } catch {
      return null
    }
  }

  const have = installedVersion()
  if (!existsSync(binPath) || have !== spec.version) {
    if (have && have !== spec.version) {
      console.error(`[mcp-serve] pin drift: ${spec.pkg} installed @${have}, pin wants @${spec.version}`)
      console.error(`[mcp-serve] run: ./msrtc.sh mcp setup (or remove ${PREFIX})`)
      process.exit(1)
    }
    // Missing binary = first-run path. npm writes its summary to stdout, which IS
    // the MCP channel here, so capture and re-emit on stderr.
    console.error(`[mcp-serve] installing ${spec.pkg}@${spec.version} into ${PREFIX} ...`)
    mkdirSync(PREFIX, { recursive: true })
    const npmCache = join(HERE, 'node_modules', '.npm-cache')
    mkdirSync(npmCache, { recursive: true })
    const npm = process.env.NPM_BIN || join(dirname(process.execPath), isWin ? 'npm.cmd' : 'npm')
    const res = spawnSync(
      npm,
      ['install', '--no-save', '--no-package-lock', '--prefix', PREFIX, `${spec.pkg}@${spec.version}`],
      { cwd: HERE, env: { ...process.env, npm_config_cache: npmCache }, stdio: ['ignore', 'pipe', 'inherit'] },
    )
    if (res.stdout && res.stdout.length) process.stderr.write(res.stdout)
    if (res.error) {
      console.error('[mcp-serve] install failed:', res.error.message)
      process.exit(1)
    }
    if (res.status !== 0) process.exit(res.status ?? 1)
  }

  spawnNpm(name, spec, binPath)
} else {
  console.error(`[mcp-serve] unsupported kind "${spec.kind}" for ${name}`)
  process.exit(2)
}

// --- browser env handoff (record written by ./msrtc.sh mcp setup) ---------
// Setup resolves ONCE; the launcher only reads the record. Browser selection
// travels by env, never by CLI flags (flags outrank envs and would block every
// fallback). The opposite env must be unset symmetrically -- a stale shell
// value must never override the record.

function readResolution(name) {
  let data
  try { data = JSON.parse(readFileSync(RECORD_PATH, 'utf8')) } catch { return null }
  const entry = data && data.resolutions && data.resolutions[name]
  return entry && entry.mode ? entry : null
}

function spawnNpm(name, spec, binPath) {
  const args = [...(spec.args || [])]
  const env = { ...process.env }
  // npm bin shims use `#!/usr/bin/env node`; prepend THIS launcher's node so
  // an ancient GUI-inherited node can never win.
  env.PATH = `${dirname(process.execPath)}${isWin ? ';' : ':'}${env.PATH || ''}`

  if (spec.browsers && spec.browsers.length) {
    const res = readResolution(name)
    if (!res) {
      console.error(`[mcp-serve] ${name}: no browser resolution record (${RECORD_PATH}) -- run: ./msrtc.sh mcp setup`)
      process.exit(1)
    }
    if (res.mode === 'playwright') {
      env.PLAYWRIGHT_MCP_BROWSER = res.browser || spec.browsers[0]
      delete env.PLAYWRIGHT_MCP_EXECUTABLE_PATH
    } else if (res.mode === 'system') {
      if (!res.executable) {
        console.error(`[mcp-serve] ${name}: record mode=system but no executable -- run: ./msrtc.sh mcp setup`)
        process.exit(1)
      }
      env.PLAYWRIGHT_MCP_EXECUTABLE_PATH = res.executable
      delete env.PLAYWRIGHT_MCP_BROWSER
    } else {
      console.error(`[mcp-serve] ${name}: record mode "${res.mode}" unsupported -- run: ./msrtc.sh mcp setup`)
      process.exit(1)
    }
    // headless rule: explicit overrides win; else the display situation decides.
    // (A Wayland-only host needs an explicit 0 for a headed session.)
    if (process.env.MCP_FORCE_HEADED) env.PLAYWRIGHT_MCP_HEADLESS = '0'
    else if (process.env.MCP_FORCE_HEADLESS) env.PLAYWRIGHT_MCP_HEADLESS = '1'
    else if (!process.env.DISPLAY && !process.env.WAYLAND_DISPLAY) env.PLAYWRIGHT_MCP_HEADLESS = '1'
    else if (process.env.WAYLAND_DISPLAY && !process.env.DISPLAY) env.PLAYWRIGHT_MCP_HEADLESS = '0'
    // absolute scratch dir: the server defaults to ./.playwright-mcp in the cwd
    args.push('--output-dir', join(ROOT, 'out', name))
  }

  spawnAndExit(binPath, args, { stdio: 'inherit', cwd: PROJECT, env })
}
