---
name: performance-optimization
description: "MediaServo performance profiling and optimization. WebRTC latency tracing, mediasoup SFU throughput, Admin UI React render profiling, cargo bench regression detection. Use when latency spikes, after media pipeline changes, or before release."
---

# performance-optimization — Performance Optimization

> WebRTC latency + mediasoup throughput + React rendering + cargo bench.
> Measure first, optimize second. Never optimize on guesses.

## Trigger Conditions

- latency >100ms (MediaServo target: <100ms E2E)
- throughput dropped >10%
- Admin UI rendering drops frames
- after media pipeline code changes
- user says "performance" / "latency" / "slow" / "优化性能" (optimize performance) <!-- c49:trigger -->
- pre-release benchmark regression

## Golden Rules

```
1. measure the baseline (cargo bench / Playwright trace)
2. locate the bottleneck (flamegraph / perf / Chrome DevTools)
3. optimize one variable at a time
4. verify against regression (re-measure, compare with baseline)
5. record the decision (decisions.md)
```

## Phase 1: Rust Layer — Benchmarking

### cargo bench

```bash
# run all benchmarks
cargo bench --workspace

# specific crate
cargo bench -p mediaservo-codec
cargo bench -p mediaservo-webrtc

# compare pre/post-change
cargo bench -- --save-baseline before
# ... make changes ...
cargo bench -- --baseline before
```

### Key Baseline Setup

```rust
// crates/mediaservo-media/benches/pipeline_bench.rs
use criterion::{black_box, Criterion};

fn bench_encode_1080p(c: &mut Criterion) {
    c.bench_function("encode_h264_1080p30", |b| {
        let frame = generate_test_frame(1920, 1080);
        let encoder = Encoder::new(CodecConfig::h264());
        b.iter(|| encoder.encode(black_box(&frame)))
    });
}
```

### Profiling

```bash
# flamegraph (Linux, requires perf)
cargo flamegraph --bin mediaservo-host -- --capture test-video

# valgrind / cachegrind
valgrind --tool=cachegrind cargo run -p mediaservo-host --release

# code-level timing (project already has a metrics module)
# crates/mediaservo-common/src/metrics.rs
use std::time::Instant;
let start = Instant::now();
// ... operation ...
metrics::record("encode_latency_us", start.elapsed().as_micros());
```

## Phase 2: WebRTC Latency Analysis

### Measurement Points

```
[Host capture] ──t1──> [Encode] ──t2──> [RTP packetize] ──t3──> [Network send]
                                                                        │
[Client render] <──t6── [Decode] <──t5── [RTP depacketize] <──t4── [Network recv]
```

```bash
# check existing metrics instrumentation
grep -rn 'metrics::record\|latency\|Instant::now' crates/mediaservo-webrtc/src/ --include='*.rs'
grep -rn 'metrics::record\|latency\|Instant::now' crates/mediaservo-media/src/ --include='*.rs'
```

### DataChannel Latency

```rust
// DataChannel echo test
// ponytail: reuse the existing E2E test script
// scripts/e2e/macos-e2e.sh already verifies DataChannel relay
// latency: 574 bytes, <1ms localhost
```

```bash
# run E2E DataChannel latency test
bash scripts/e2e/macos-e2e.sh  # macOS
pixi run test-sfu               # Linux/Docker SFU
```

### Optimization Patterns

```rust
// BEFORE: allocation per RTP packet
fn send_frame(&mut self, frame: &Frame) {
    let rtp_packet = self.packetizer.packetize(frame); // alloc
    self.transport.send(rtp_packet);
}

// AFTER: pre-allocated buffer pool
// chesterton: the buffer pool is a necessary trade-off from the PIT-01/PIT-03 analysis
fn send_frame(&mut self, frame: &Frame) {
    let buf = self.buffer_pool.acquire(frame.len());
    self.packetizer.packetize_into(frame, &mut buf);
    self.transport.send(&buf);
}
```

## Phase 3: mediasoup SFU Throughput

### Baseline Commands

```bash
# Docker environment
docker compose exec server cargo bench -p mediaservo-server -- --bench sfu_throughput

# or use mediasoup built-in stats
# check worker resource usage
grep -rn 'rtp_listener\|max_income_bitrate\|producer.*stats' crates/mediaservo-server/src/sfu/ --include='*.rs'
```

### mediasoup Tuning Parameters

```javascript
// mediasoup WebRtcTransport config reference
{
  initialAvailableOutgoingBitrate: 1_000_000,  // 1Mbps
  maxIncomingBitrate: 1_500_000,               // 1.5Mbps
  // lower these for lower latency:
  // initialAvailableOutgoingBitrate: 300_000,  // 300kbps
}
```

```bash
# check current MediaServo transport config
grep -rn 'availableOutgoingBitrate\|maxIncomingBitrate\|initial' crates/mediaservo-server/src/sfu/ --include='*.rs'
```

### SFU Health Monitoring

```bash
# runtime metrics (via Admin WebSocket)
# GET /api/admin/sfu/stats
curl -s http://localhost:9800/api/admin/sfu/stats | jq '.rooms[].peers[].transports[]'

# key metrics:
# - bytesReceived / bytesSent (bandwidth usage)
# - producerScore (encoder quality, 0-10)
# - packetLoss (loss rate, target <0.1%)
# - roundTripTime (RTT, target <50ms)
```

## Phase 4: Admin UI — React Render Optimization

### Playwright Performance Tracing

```bash
# launch Admin UI + Playwright trace
# use the local-playwright MCP tools:
# 1. browser_navigate → Admin UI
# 2. browser_evaluate → performance.mark('start')
# 3. perform interactions
# 4. browser_evaluate → performance.measure('render', 'start')

# code-level check
grep -rn 'useState\|useEffect\|useMemo\|useCallback\|React.memo' crates/mediaservo-server/src/admin/ --include='*.tsx' --include='*.ts'
```

### React Optimization Checklist

| Problem | Detection | Fix |
|------|------|------|
| cascading re-renders | React DevTools Profiler → flame chart | `React.memo` + `useCallback` |
| expensive compute in render | wrap render body with `console.time` | `useMemo` |
| large list without virtualization | items >100 and no `react-window` | `FixedSizeList` |
| WebSocket message floods | >60 updates per second | batch updates (requestAnimationFrame throttle) |
| uncleaned subscriptions | no useEffect cleanup | return `() => ws.close()` |
| unnecessary Context spread | Context value contains frequently-changing objects | split the Context / use a ref |

### Check Commands

```bash
# count heavy components in the React project
grep -rn 'export.*function.*Component\|export.*default function' crates/mediaservo-server/src/admin/ --include='*.tsx' | wc -l

# check missing memoization
grep -rn 'useState\|useEffect' crates/mediaservo-server/src/admin/ --include='*.tsx' | wc -l
grep -rn 'useMemo\|useCallback\|memo' crates/mediaservo-server/src/admin/ --include='*.tsx' | wc -l
# ponytail: ratio useMemo+useCallback+React.memo / useState+useEffect should be >0.5
```

### Playwright Verification Script

```javascript
// execute in Playwright MCP:
// 1. browser_navigate → http://localhost:9800/admin
// 2. browser_evaluate:
() => {
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) {
      if (entry.duration > 16) { // >1 frame (60fps)
        console.warn('Long task:', entry.duration.toFixed(1) + 'ms', entry.name);
      }
    }
  });
  observer.observe({ entryTypes: ['measure', 'longtask'] });
  performance.mark('monitoring-start');
}
// 3. operate the UI
// 4. browser_evaluate: () => performance.getEntriesByType('measure')
```

## Phase 5: CI Regression Detection

### Add a CI Performance Gate

```yaml
# add to .github/workflows/ci.yml:
perf-regression:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - run: cargo bench --workspace -- --output-format bencher | tee bench-output.txt
    - uses: benchmark-action/github-action-benchmark@v1
      with:
        tool: 'cargo'
        output-file-path: bench-output.txt
        github-token: ${{ secrets.GITHUB_TOKEN }}
        auto-push: true
        alert-threshold: '130%'  # alert on >30% regression
```

### Local Pre-Bench Checks

```bash
# quick check for obvious regressions
cargo check --workspace --all-features  # compile check
cargo clippy --workspace --all-features -- -D warnings  # no gratuitous alloc/clone
cargo bench --workspace -- --quick  # quick bench (under-sampled, sanity check only)
```

## Hotspot Reference (MediaServo Known)

| Hotspot | Location | Expectation | Monitoring |
|------|------|------|------|
| H.264 encoding | `mediaservo-codec/src/ffmpeg/encoder.rs` | <5ms 1080p | metrics: `encode_latency_us` |
| RTP packetize | `mediaservo-webrtc/src/backend/*/track.rs` | <1ms | metrics: `rtp_packetize_us` |
| WebSocket relay | `mediaservo-server/src/signaling/ws.rs` | <1ms per message | metrics: `ws_relay_us` |
| GStreamer appsink | `mediaservo-host/src/capture/gst.rs` | <3ms frame pull | dropped-frame counter |
| mediasoup transport | `mediaservo-server/src/sfu/transport.rs` | <50ms connect | room stats |

## Report Format

```
## Performance Analysis Report — [date]

### Baseline Comparison
| Benchmark | Before | After | Delta |
|------|--------|-------|-------|
| encode_h264_1080p30 | 3.2ms | 3.1ms | -3% |
| rtp_packetize | 0.8ms | 0.4ms | -50% ✅ |
| ws_relay_1kb | 0.3ms | 0.3ms | 0% |

### Findings
- [hotspot] rtp_packetize optimization: pre-allocated buffer pool, -50%
- [regression] none
- [bottleneck] mediasoup transport connect ~45ms (known PTH-07, deferred)

### Recommendations
- [P0] none
- [P1] SFU transport.connect actual call (see pitfall PIT-07)
```

## Forbidden

- optimizing without measuring (guess-optimizing usually introduces new problems)
- optimizing multiple variables at once (attribution becomes impossible)
- sacrificing readability for insignificant gains
- micro-benchmarks detached from real usage scenarios
- ignoring CI performance-regression alerts
- trading `unsafe` for performance without verification
