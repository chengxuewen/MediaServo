---
name: observability-and-instrumentation
description: "Instrument MediaServo with useful logs, metrics, and traces: structured fields, RED metrics, spans across signaling/media, symptom-based alerts, and low-cardinality labels. Use when adding visibility or diagnosing production behavior."
---

# Observability and Instrumentation

> Instrument the question you will ask at 3 a.m.

## Trigger Conditions

- A new server path, transport, or media pipeline needs visibility.
- A production symptom cannot be explained by existing logs.
- You need latency/error/throughput metrics or a trace across signaling and media.
- 用户说“加日志 / 指标 / 追踪 / 告警 / 可观测性” <!-- c49:trigger -->

## Do Not Use

- Pure performance tuning: use `performance-optimization`.
- A one-off debug print: use `debugging-and-error-recovery`.
- Security audit logging: coordinate with `security-hardening`.

## Signal Selection

| Question | Signal |
|---|---|
| Is it failing? | error counter + structured error log |
| Is it slow? | latency histogram (p50/p95/p99) + span |
| Is it saturated? | queue depth, worker utilization, connection count |
| Where did it fail? | trace/span with correlation ID |
| Who is affected? | low-cardinality dimensions (route, transport, codec, region) |

## Workflow

### 1. State the question

Write the question first: "Why do consumers disconnect after 30 s?" The question decides the signal; a generic "add logs" does not.

### 2. Place the signal at the boundary

- Signaling: connect, join, publish, subscribe, disconnect.
- Media: producer/consumer creation, keyframe request, RTP timeout, ICE state.
- Server: request in/out, error class, queue depth.
- Dashboard: WS reconnect, playback stall, data freshness.

### 3. Use structured fields

```text
event=consumer_closed reason=rtp_timeout room=<id> peer=<id> transport=<id> duration_ms=<n>
```

- Stable event names; no prose-only messages.
- Include correlation/room/peer IDs where useful, but never secrets or raw media.
- One log per state transition, not per packet.

### 4. Metrics: RED + saturation

- **Rate**: requests or events per second.
- **Errors**: failures by class, not one generic counter.
- **Duration**: histogram with explicit buckets.
- **Saturation**: queue/worker/connection limits.

Keep labels low-cardinality; never put peer IDs, room IDs, or timestamps in metric labels.

### 5. Traces

- One span per boundary crossing; propagate the trace context through signaling and media setup.
- Record the outcome (ok/error) and the key attributes, not the payload.
- Sample deliberately; keep errors sampled at 100% where possible.

### 6. Alerts

- Alert on symptoms users feel: error rate, p95 latency, disconnect rate, stall rate.
- Do not alert on a single log line or a raw counter without a threshold/window.
- Every alert needs an owner, a runbook, and a rollback/ack path.

### 7. Verify

- Trigger the path and confirm the signal appears once, with the right fields.
- Check cardinality and volume; a metric that explodes is an outage risk.
- Run the relevant gates and a load/weaknet scenario where applicable.

## Anti-Patterns

| Smell | Fix |
|---|---|
| Logging every packet | Log state transitions; use metrics for volume. |
| `println!` in a hot path | Structured `tracing` with level control. |
| High-cardinality labels | Route/transport/codec/region only. |
| Alert on CPU alone | Alert on user-visible symptoms. |
| Secret or token in a log | Redact at the source; add a test. |
| Trace without outcome | Record status and error class. |

## Exit Criteria

- The question is answerable from the new signals.
- Logs are structured and redacted.
- Metrics are low-cardinality with a documented meaning.
- The alert (if any) has an owner and a runbook.
- Volume/cardinality checked; gates green.

## Boundaries

- `performance-optimization`: uses the signals to tune; this skill adds them.
- `debugging-and-error-recovery`: consumes the signals for one failure.
- `security-hardening`: reviews log redaction and audit requirements.
