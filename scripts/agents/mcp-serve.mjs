#!/usr/bin/env node
// mcp-serve.mjs — pinned npm-MCP launcher (agent toolchain home: scripts/agents).
//
// Pins live in scripts/agents/mcp-servers.json (single source). The launcher
// installs at the exact version into scripts/agents/node_modules, fails loudly
// on pin drift, and spawns the server with cwd = MEDIASERVO_AGENT_PROJECT_ROOT
// (default: this repo root). Arguments are passed as an array (no shell); npm's
// stdout is redirected to stderr so the MCP JSON-RPC channel stays clean.

import { spawn, spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { dirname, isAbsolute, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..', '..')
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
if (spec.kind !== 'npm') {
  console.error(`[mcp-serve] unsupported kind "${spec.kind}" for ${name}`)
  process.exit(2)
}

const binPath = join(HERE, 'node_modules', '.bin', isWin ? `${spec.bin}.cmd` : spec.bin)

function installedVersion() {
  const parts = spec.pkg.startsWith('@') ? spec.pkg.split('/') : [spec.pkg]
  const manifest = join(HERE, 'node_modules', ...parts, 'package.json')
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
    console.error('[mcp-serve] run: ./msrtc.sh mcp setup   (or remove scripts/agents/node_modules)')
    process.exit(1)
  }
  // Missing binary = first-run path (clean clone / GUI first click). npm writes
  // its summary to stdout, which IS the MCP channel here, so capture and re-emit
  // on stderr.
  console.error(`[mcp-serve] installing ${spec.pkg}@${spec.version} into scripts/agents/node_modules ...`)
  const npm = process.env.NPM_BIN || join(dirname(process.execPath), isWin ? 'npm.cmd' : 'npm')
  const res = spawnSync(
    npm,
    ['install', '--no-save', '--no-package-lock', '--prefix', HERE, `${spec.pkg}@${spec.version}`],
    { cwd: HERE, stdio: ['ignore', 'pipe', 'inherit'] },
  )
  if (res.stdout && res.stdout.length) process.stderr.write(res.stdout)
  if (res.error) {
    console.error('[mcp-serve] install failed:', res.error.message)
    process.exit(1)
  }
  if (res.status !== 0) process.exit(res.status ?? 1)
}

const child = spawn(binPath, spec.args || [], { stdio: 'inherit', cwd: PROJECT })
child.on('error', (err) => {
  console.error(`[mcp-serve] failed to start ${spec.bin}:`, err.message)
  process.exit(1)
})
child.on('exit', (code) => process.exit(code ?? 1))
