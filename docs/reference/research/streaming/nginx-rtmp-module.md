# nginx-rtmp-module Reference Study

> **Source**: [arut/nginx-rtmp-module](https://github.com/arut/nginx-rtmp-module)  
> **Last updated**: 2024-12-24 | **Stars**: 13k+ | **License**: BSD-2-Clause  
> **Author**: Roman Arutyunyan (arut)  
> **Created**: 2012-03-14 | **Mainline status**: maintenance mode (not active development)

---

## 1. Product Profile

nginx-rtmp-module is an nginx-based streaming-server extension that turns nginx from an HTTP server into a full-featured RTMP/HLS/MPEG-DASH streaming server. Leveraging nginx's mature C module-extension mechanism, it implements a complete RTMP protocol stack, live push/pull streaming, remuxing (HLS/DASH), recording, transcoding, and HTTP callbacks on top of nginx's event-driven architecture.

**Core positioning**: lightweight, high-performance streaming middleware suited to live distribution, video conferencing, and surveillance-stream ingest. It is the textbook case of nginx module development, fully demonstrating how to build an entirely new application-layer protocol framework on nginx.

**Module composition**:
- `ngx_rtmp_module` — core module (NGX_CORE_MODULE), managing RTMP connections, handshake, Chunk protocol, AMF codec
- `ngx_rtmp_live_module` — live module managing publishers/subscribers, GOP cache, multi-subscriber broadcast
- `ngx_rtmp_relay_module` — relay module implementing push/pull streaming, disconnect-reconnect, and static pull
- `ngx_rtmp_record_module` — recording module with FLV recording, keyframe-only mode, and timed splitting
- `ngx_rtmp_hls_module` — HLS remuxer turning RTMP streams into HTTP Live Streaming
- `ngx_rtmp_dash_module` — DASH remuxer turning RTMP streams into MPEG-DASH
- `ngx_rtmp_exec_module` — external-process module firing external commands (e.g. ffmpeg transcode) on stream events
- `ngx_rtmp_notify_module` — HTTP-callback module (on_publish/on_play/on_done/on_update)
- `ngx_rtmp_control_module` — HTTP control module offering recording control, client kicking, and other admin interfaces
- `ngx_rtmp_stat_module` — stats module exposing RTMP state information in XML
- `ngx_rtmp_codec_module` — codec module parsing H264/AAC/MP3 encoding information

---

## 2. Technical Characteristics

### 2.1 The nginx Module Architecture (event loop, module hooks, config directives)

nginx-rtmp-module is the best reference for understanding the nginx module-extension pattern. Beyond the HTTP protocol it defines a complete **programmatic module framework**, including its own module type, configuration hierarchy, event system, and lifecycle.

#### 2.1.1 Module Definition & Type Registration

Every nginx extension registers through the `ngx_module_t` struct. nginx-rtmp-module defines its own module type `NGX_RTMP_MODULE`, alongside `NGX_HTTP_MODULE`, `NGX_EVENT_MODULE`, etc.:

```c
// The core module — type NGX_CORE_MODULE, initializes the RTMP subsystem
ngx_module_t ngx_rtmp_module = {
    NGX_MODULE_V1,
    &ngx_rtmp_module_ctx,          // module context
    ngx_rtmp_commands,             // config directives (rtmp { }, server { }, listen, etc.)
    NGX_CORE_MODULE,               // core module type
    NULL,                           // init master
    NULL,                           // init module
    ngx_rtmp_init_process,         // init process — called when a worker process starts
    NULL, NULL, NULL, NULL,
    NGX_MODULE_V1_PADDING
};

// Feature modules — type NGX_RTMP_MODULE, implementing concrete features
ngx_module_t ngx_rtmp_relay_module = {
    NGX_MODULE_V1,
    &ngx_rtmp_relay_module_ctx,    // RTMP module context
    ngx_rtmp_relay_commands,       // module directives (push, pull, relay_buffer, etc.)
    NGX_RTMP_MODULE,               // RTMP module type
    NULL, NULL,
    ngx_rtmp_relay_init_process,   // init process
    NULL, NULL, NULL, NULL,
    NGX_MODULE_V1_PADDING
};
```

#### 2.1.2 The Three-Level Configuration System

Mirroring the HTTP module, nginx-rtmp-module defines a **Main → Server → Application** three-level configuration inheritance system:

| Level | HTTP analogue | Config block | Typical directives |
|-------|---------------|--------------|--------------------|
| `main_conf` | `http` block | `rtmp { }` | `rtmp_auto_push` |
| `srv_conf` | `server` block | `server { }` | `listen 1935`, `chunk_size` |
| `app_conf` | `location` block | `application { }` | `live on`, `push`, `pull` |

Config creation and merging follow the standard nginx pattern:

```c
// The RTMP module context — defines the config create/merge hook functions
static ngx_rtmp_module_t ngx_rtmp_relay_module_ctx = {
    NULL,                                    // preconfiguration
    ngx_rtmp_relay_postconfiguration,        // postconfiguration
    NULL,                                    // create main configuration
    NULL,                                    // init main configuration
    NULL,                                    // create server configuration
    NULL,                                    // merge server configuration
    ngx_rtmp_relay_create_app_conf,          // create app configuration
    ngx_rtmp_relay_merge_app_conf            // merge app configuration
};
```

In the `postconfiguration` stage, a module injects its functionality into the RTMP core's event-handling chain via **event-hook registration**:

```c
static ngx_int_t
ngx_rtmp_relay_postconfiguration(ngx_conf_t *cf)
{
    ngx_rtmp_core_main_conf_t *cmcf;
    ngx_rtmp_handler_pt *h;

    cmcf = ngx_rtmp_conf_get_module_main_conf(cf, ngx_rtmp_core_module);

    // Register the handshake-completed handler
    h = ngx_array_push(&cmcf->events[NGX_RTMP_HANDSHAKE_DONE]);
    *h = ngx_rtmp_relay_handshake_done;

    // Intercept publish/play/close-stream handling (chained hooks)
    next_publish = ngx_rtmp_publish;
    ngx_rtmp_publish = ngx_rtmp_relay_publish;

    next_play = ngx_rtmp_play;
    ngx_rtmp_play = ngx_rtmp_relay_play;

    return NGX_OK;
}
```

This **chained function-pointer replacement** pattern is the core mechanism of nginx module cooperation — multiple modules can independently register handlers for the same event without knowing each other.

#### 2.1.3 Integration with the nginx Event Loop

The core function of nginx's event loop is `ngx_process_events_and_timers()`, whose flow is:

1. Call `ngx_event_find_timer()` to find the nearest expiring timer
2. Invoke the event module for I/O events (e.g. `epoll_wait`), waiting for the timeout
3. Read from / write to ready connections
4. Call `ngx_event_expire_timers()` to handle expired timers
5. Call `ngx_event_process_posted()` to process the posted-event queue

nginx-rtmp-module slots entirely into this event loop. When a new connection arrives:

```c
// Register the connection handler in ngx_rtmp_optimize_servers
ls->handler = ngx_rtmp_init_connection;  // new-connection callback
ls->pool_size = 4096;

// After the connection is established, start the RTMP session loop
void ngx_rtmp_cycle(ngx_rtmp_session_t *s)
{
    ngx_connection_t *c = s->connection;

    // Register the RTMP read/write functions as the nginx connection's event handlers
    c->read->handler = ngx_rtmp_recv;
    c->write->handler = ngx_rtmp_send;

    // Set up the heartbeat timer
    s->ping_evt.handler = ngx_rtmp_ping;
    ngx_rtmp_reset_ping(s);

    // Start reading
    ngx_rtmp_recv(c->read);
}
```

RTMP protocol parsing happens in `ngx_rtmp_recv()`, executed on each read event. It parses the RTMP Chunk Header (fmt, csid, timestamp, message length, message type, message stream ID), then dispatches the complete message to registered handlers via `ngx_rtmp_fire_event()`:

```c
ngx_int_t
ngx_rtmp_fire_event(ngx_rtmp_session_t *s, ngx_uint_t evt,
    ngx_rtmp_header_t *h, ngx_chain_t *in)
{
    ngx_rtmp_core_main_conf_t *cmcf;
    ngx_array_t *ch;
    ngx_rtmp_handler_pt *hh;
    size_t n;

    cmcf = ngx_rtmp_get_module_main_conf(s, ngx_rtmp_core_module);
    ch = &cmcf->events[evt];
    hh = ch->elts;

    // Walk all registered handlers for this event type
    for (n = 0; n < ch->nelts; ++n, ++hh) {
        if (*hh && (*hh)(s, h, in) != NGX_OK) {
            return NGX_ERROR;
        }
    }

    return NGX_OK;
}
```

#### 2.1.4 Key Event Types

nginx-rtmp-module defines a rich set of RTMP event types; modules receive events by registering handlers into the `cmcf->events[]` arrays:

```c
// Protocol-level events — standard handlers auto-registered
NGX_RTMP_MSG_CHUNK_SIZE   // set chunk size
NGX_RTMP_MSG_ABORT        // abort message
NGX_RTMP_MSG_ACK          // acknowledgment
NGX_RTMP_MSG_ACK_SIZE     // set window ack size
NGX_RTMP_MSG_BANDWIDTH    // set peer bandwidth

// Application-level events — registered by feature modules
NGX_RTMP_CONNECT           // new connection
NGX_RTMP_HANDSHAKE_DONE    // handshake completed
NGX_RTMP_MSG_AMF_CMD       // AMF commands (connect, createStream, publish, play, etc.)
NGX_RTMP_MSG_AMF_META      // metadata
NGX_RTMP_MSG_AUDIO         // audio data
NGX_RTMP_MSG_VIDEO         // video data
NGX_RTMP_DISCONNECT        // disconnect
NGX_RTMP_STREAM_BEGIN      // stream begin
NGX_RTMP_STREAM_EOF        // stream end
```

AMF commands are further routed to concrete handlers via a hash table (`connect` → `ngx_rtmp_cmd_connect`, `publish` → `ngx_rtmp_cmd_publish`, `play` → `ngx_rtmp_cmd_play`, etc.).

### 2.2 RTMP Protocol Handling

nginx-rtmp-module implements the full RTMP protocol stack:

- **Handshake**: the C0/C1/C2 ↔ S0/S1/S2 four-step handshake, implemented in `ngx_rtmp_handshake()` and related functions
- **Chunk protocol**: variable-length Chunk Headers (1/3/11/15 bytes), multiplexing multiple Chunk Streams. The core parses fmt and csid byte by byte inside `ngx_rtmp_recv()`, recognizing Chunk Stream IDs (0-65599)
- **Message-type dispatch**: protocol-control messages (Set Chunk Size, Abort, ACK, Window ACK Size, Set Peer Bandwidth) go to `ngx_rtmp_protocol_message_handler`; user-control messages (Stream Begin, Stream EOF, Set Buffer Length, etc.) go to `ngx_rtmp_user_message_handler`
- **AMF codec**: AMF0 and AMF3 supported, serialized/deserialized via `ngx_rtmp_amf_read()`/`ngx_rtmp_amf_write()`. AMF command names route to handlers through a hash lookup table
- **Session management**: each connection maps to one `ngx_rtmp_session_t`, holding input/output Chunk Stream arrays (up to `max_streams`), an output-message ring queue (configurable `out_queue` size), and a Nagle-like coalescing mechanism (`out_cork`)

### 2.3 Multi-Worker Live Distribution

With the `rtmp_auto_push` directive on, the module automatically forwards streams between worker processes over a **Unix Domain Socket**. When one worker receives a publish, it pushes the stream to the other workers so subscribers can connect to any worker and get the same stream. This is an elegant way to achieve stateless horizontal scaling on top of nginx's multiprocess architecture.

### 2.4 Relay Modes

The Push/Pull model is implemented over the RTMP client, fully reusing the RTMP protocol stack:

- **Push**: on the publish event, automatically connects to the configured remote RTMP servers and pushes the stream. Disconnect-reconnect is supported (the `push_reconnect` interval)
- **Pull**: on the play event, automatically pulls from the remote RTMP server. Static pull is supported (the `static` parameter starts pulling at nginx startup)
- **Session Relay**: relay operations run in independent sessions and can cross different applications

### 2.5 HLS/DASH Remuxing

The HLS and DASH modules extract codec information from the RTMP stream's audio/video data, splitting on keyframes into `.m3u8`/`.mpd` playlists and `.ts`/`.m4s` segment files. They require a memory filesystem (tmpfs) to cut disk I/O. The HTTP side is served by the standard nginx `http { }` block for static files.

---

## 3. Key Capabilities

| Capability | Description |
|------------|-------------|
| **RTMP live** | A complete RTMP protocol stack; push/pull, one-to-many (TV mode), many-to-many (conference mode) |
| **HLS output** | Real-time HLS segmentation (m3u8 + ts), directly playable by iOS/Safari natively |
| **MPEG-DASH output** | Real-time DASH segmentation (mpd + m4s), compatible with modern browsers and players |
| **Relay push/pull** | The Push/Pull model; distribution across homogeneous nginx-rtmp clusters with disconnect-reconnect |
| **Multi-worker distribution** | Streams auto-forward between workers over Unix sockets — stateless horizontal scaling |
| **VOD** | FLV/MP4 files playable as RTMP streams |
| **Stream recording** | Record to FLV; keyframe-only mode, timed splitting, time/size caps |
| **Online transcode** | exec directives invoke ffmpeg, dynamically transcoding/scaling/remuxing on publish |
| **HTTP callbacks** | on_publish / on_play / on_done / on_update / on_record_done, enabling auth and business logic |
| **HTTP control** | HTTP interfaces for recording control (record/stop) and client disconnection |
| **RTMP stats** | Real-time XML statistics (connections, streams, bitrates, etc.) |
| **Access control** | IP-based allow/deny rules (publish/play controlled separately) |
| **Buffer control** | Configurable GOP cache, play buffer size, and timeouts |
| **Log level** | A dedicated `NGX_LOG_DEBUG_RTMP` level for debugging |

**Configuration example**:

```nginx
rtmp {
    server {
        listen 1935;
        chunk_size 4096;
        timeout 60s;

        application live {
            live on;
            record all;
            record_path /tmp/av;
            record_unique on;

            allow publish 127.0.0.1;
            deny publish all;

            hls on;
            hls_path /tmp/hls;

            dash on;
            dash_path /tmp/dash;

            on_publish http://localhost:8080/auth;
            on_done http://localhost:8080/done;

            push rtmp://cdn1.example.com/live;
            pull rtmp://origin.example.com/live name=camera1 static;

            exec ffmpeg -i rtmp://localhost/$app/$name -vcodec flv
                -acodec copy -s 640x360 -f flv
                rtmp://localhost/small/$name;
        }
    }
}
```

---

## 4. Deployment & Operations

### 4.1 Build Method

nginx-rtmp-module uses nginx's `--add-module` build; it is not built separately:

```bash
./configure --add-module=/path/to/nginx-rtmp-module
make && make install
```

This compiles the module into the nginx binary — no dynamic loading. A limitation of nginx's historic module system: dynamic module loading (`load_module`) arrived only in nginx 1.9.11+, and most third-party RTMP modules have not adapted to it.

### 4.2 Runtime Dependencies

- **nginx 1.6.x ~ 1.24.x** (specific versions need compatibility verification)
- **HLS/DASH**: tmpfs or high-performance storage needed (frequent segment writes)
- **Transcoding (exec)**: ffmpeg must be installed
- **Recording**: adequate disk space required

### 4.3 Performance Characteristics

- **10,000+ concurrent connections** per process (bounded by the `worker_connections` setting)
- Latency: typically < 1 second on LAN, < 3 seconds on the public internet
- HLS latency: 3~30 seconds (depending on segment length)
- Memory: roughly 64KB~256KB per connection (depending on buffers and stream count)

### 4.4 Common Operational Issues

- **Inactive maintenance**: the author stopped active development in 2018; a large backlog of unhandled Issues and PRs remains
- **HLS/DASH compatibility**: some players have trouble with newer HLS-spec features
- **Memory leaks**: small leaks in certain scenarios (e.g. frequent disconnect/reconnect)
- **Static pull**: starts pulling at nginx startup even with no subscribers

---

## 5. Ecosystem & Market

### 5.1 Primary Users

nginx-rtmp-module was once the de facto standard of the live-streaming industry, widely used for:

- **Streaming platforms**: Twitch and YouTube Live architecture references
- **CDN edge nodes**: the RTMP ingest layer, remuxed to HLS and distributed over the HTTP CDN
- **Surveillance**: IP-camera streams ingested over RTMP
- **Video conferencing**: the underlying stream-ingest layer for SFU/MCUs
- **Private streaming**: corporate intranets and education

### 5.2 Alternatives / Evolution Paths

| Solution | Characteristics | Comparison |
|----------|-----------------|------------|
| **SRS (Simple-RTMP-Server)** | C++, purpose-built for streaming | More active community, richer features, but less extensible than the nginx module system |
| **Janus (WebRTC Gateway)** | C, WebRTC-first | Targets WebRTC; no native traditional RTMP ingest (needs conversion) |
| **MediaMTX (rtsp-simple-server)** | Go, multi-protocol | Supports RTSP/RTMP/HLS/WebRTC; a more modern architecture |
| **nginx + SRTP/WebRTC** | nginx native WebRTC (NGINX Plus) | Paid edition, limited features |
| **OBS Studio + SRS** | Push client + server | A combination, not a single product |

### 5.3 Market Position

nginx-rtmp-module embodies the technical route of **building streaming capability atop a general reverse proxy via module extension**. The "one binary, many protocols" philosophy has clear operational advantages (unified deployment, monitoring, logging) but is constrained by the complexity of nginx C module development and its maintenance bottleneck.

---

## 6. Highlights & Limitations

### Highlights

1. **A showcase of architecture reuse**: exploiting nginx's mature event-driven, memory-pool, connection-management, and config-parsing infrastructure, a full streaming protocol stack is implemented with minimal code
2. **The event-hook system**: `ngx_rtmp_fire_event()` + handler arrays achieve loosely coupled module collaboration — new features just register handlers, touching no core code
3. **Seamless multi-worker distribution**: `rtmp_auto_push`'s stateless Unix-socket distribution shows a deep understanding and clever exploitation of nginx's multiprocess architecture
4. **Config-driven flexibility**: every feature is declarative in `nginx.conf`; no extra orchestration tooling needed
5. **A complete RTMP implementation**: AMF0/AMF3, the Chunk protocol, multi-stream multiplexing, window ACK, bandwidth control — all supported
6. **Production-proven**: stability validated across a large body of production deployments (in scenarios that don't hit known bugs)

### Limitations

1. **Stalled maintenance**: the core maintainer is inactive; unmerged PRs and unfixed Issues pile up
2. **No WebRTC**: born before WebRTC; cannot natively serve modern real-time protocols
3. **Single-machine architecture**: no native cluster coordination; relays are hardcoded in config, with no service discovery or auto-load-balancing
4. **High HLS/DASH latency**: segmented delivery carries inherent latency — unsuited to sub-second ultra-low-latency cases
5. **The C-language barrier**: module development demands deep knowledge of nginx internals; a steep learning curve
6. **Compile-time coupling**: modules must be built into the nginx binary — no hot loading or independent upgrade
7. **No SRT/RIST**: no support for streaming protocols designed for unreliable networks

---

## 7. Reference Value for MediaServo

### 7.1 The nginx Module Pattern vs Rust Trait Extension

nginx-rtmp-module demonstrates the **event-driven + module-hook + config-inheritance** architecture pattern. MediaServo, a Rust project, can borrow the design philosophy and realize the equivalent extension mechanism with Rust's trait system:

| nginx C pattern | Rust equivalent | Notes |
|-----------------|-----------------|-------|
| `ngx_module_t` + `NGX_MODULE_V1` | `trait Module` + derive macros | Define the module lifecycle (init_process, exit_process, etc.) via a trait |
| `ngx_rtmp_module_t` (create/merge conf) | `trait ModuleConfig<T>` | Specify config structs via associated types; the `Default` trait supplies merge semantics |
| `ngx_command_t` arrays | Attribute macro `#[rtmp_directive]` | Mark config parsers with attribute macros; directive registration code generated at compile time |
| Event queue `cmcf->events[]` | `EventBus<T>` or the observer pattern | Store handlers in `Vec<Box<dyn FnMut(...)>>`, dynamic registration supported |
| Chained function-pointer replacement (next_xxx) | decorator/wrapper pattern | Wrap original handlers with Rust closures or `Box<dyn Fn>` |
| The RTMP session state machine | `enum` + `match` | Rust ADTs express protocol state more safely; the compiler enforces exhaustive branches |
| `ngx_pool_t` memory pools | `bumpalo` / `typed_arena` crates | Rust RAII + lifetimes manage memory automatically; explicit pools often unnecessary |
| Config merge `ngx_conf_merge_*` | `Option::or()` / `unwrap_or()` chains | Rust's Option naturally fits config-override semantics |

### 7.2 Architecture-Decision Borrowings

1. **A protocol-agnostic application framework**: nginx-rtmp proves nginx is not merely an HTTP server — its core is a **configurable event loop + module loader**. HTTP and RTMP are both "applications" built on it. MediaServo can analogize this: a Rust-trait-based application framework where RTMP, WebRTC, SRT, and other protocols plug in as trait implementors.

2. **Config-driven routing**: RTMP routes streams by `application` name, the equivalent of HTTP's `location` routing. This fits streaming well — different `app`s can have entirely different processing pipelines (record, transcode, push to CDN).

3. **The generality of the relay model**: Push/Pull is a fundamental streaming-distribution primitive. MediaServo should design these as first-class citizens, not mere application-layer afterthoughts.

4. **Declarative pipeline composition**: the exec directive embodies the declarative "stream event → external process → stream output" pipeline. MediaServo can achieve similar but more reliable pipeline composition with Rust's type safety and compile-time checks.

### 7.3 Technical Evolution Directions

nginx-rtmp-module's historic limitations point the way for MediaServo:

- **From compile-time to runtime extension**: support dynamically loaded protocol modules (Rust `libloading` or embedded WASM)
- **From single protocol to multi-protocol**: RTMP + WebRTC + SRT + RIST unified in one framework
- **From hardcoded config to service discovery**: relay endpoints should support dynamic registration and health checks
- **From C to Rust**: memory safety and concurrency safety are vital at high-throughput streaming

> **Summary**: nginx-rtmp-module is the classic case of "how to build a streaming application on mature infrastructure." MediaServo should take its architectural essence (modularity, event-driven design, layered configuration, pipeline composition), reimplement it with modern Rust features, and avoid its maintenance traps.

**Related decisions**: D152 (recording hooks), D-STREAM-TOPOLOGY
