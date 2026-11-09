# P0 audio proof harness

Minimal browser harness for the P0 gate: two Chromium contexts (fake microphone) publish and
consume in one `audio-*` room, proving the browser uplink/downlink path before any floor work.

## Run

```bash
# 1) server (from the MediaServo submodule root)
bash mediaservo.sh up --env dev server

# 2) install once (from www/)
cd www && pnpm install

# 3) proof (from this directory)
cd tests/p0-audio && pnpm test
```

Evidence is written to
`../../../../../docs/plans/talkservo-port/evidence/p0-browser-audio.json`
(override with `P0_EVIDENCE`). The runner exits 0 only when the publisher produced and the
consumer saw a track/playing status/non-zero bitrate.

Environment knobs: `CHROME_PATH`, `P0_ROOM` (default `audio-p0`), `P0_EVIDENCE`.

Notes:
- Accounts come from `config/accounts.yaml` (dispatcher/dispatch123 for publish, admin/admin123
  for consume). Change via the query string in `run.mjs` if the registry changes.
- Vite proxies `/api` and `/ws` to `127.0.0.1:9800` so both pages stay same-origin.
- This harness is a test asset; it does not change product code.
