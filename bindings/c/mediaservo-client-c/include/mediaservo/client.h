/* MediaServo client C ABI (cockpit/consumer-side SDK) — login + SFU video consumption + control DataChannel.
 *
 * MAJOR = C ABI version (D241): additive-only within a MAJOR, binary compatible.
 * This header is hand-maintained (stable export surface, same discipline as cbindgen output).
 * Shared C type (mediaservo_err_t): see common.h.
 *
 * S6 batch 1b (⚠ breaking): the full prefix ms_client_* → mediaservo_client_* (zero aliases);
 * login moved to flat parameters; ⊘ old forms kept for one period (global last_error / first-consumer consume_video /
 * session-union video_stats); the new surface = multi-consumer + handle error slots (error_t machine-readable
 * code/wire_code/retryable) + session state observation + cumulative ack token registration.
 *
 * Usage (multi-consumer + control loop, mirroring crates/mediaservo-client/examples/basic.rs):
 *   char token[4096]; size_t need = 0;
 *   if (mediaservo_client_login("http://10.0.0.2:9800", "operator", "...",
 *                               token, sizeof(token), &need) != MEDIASERVO_OK) { read last_error }
 *   mediaservo_client_config_t cfg = MEDIASERVO_CLIENT_CONFIG_DEFAULT;
 *   cfg.signaling_url = "ws://10.0.0.2:9800/ws"; cfg.room = "vehicle_1"; cfg.jwt = token;
 *   mediaservo_client_session_t* s = NULL;
 *   mediaservo_client_session_create(&cfg, &s);
 *   char pid[256];
 *   mediaservo_client_session_wait_video(s, 30000, pid, sizeof(pid), NULL);
 *   mediaservo_client_consumer_t* cons = NULL;
 *   mediaservo_client_session_consume(s, pid, on_frame, user, on_free, &cons);
 *   const char* labels[] = {"chassis"};
 *   mediaservo_client_control_t* c = NULL;
 *   mediaservo_client_open_control(s, labels, 1, &c);
 *   uint64_t tok = 0;
 *   mediaservo_client_control_on_ack(c, on_ack, user, on_free, &tok);
 *   mediaservo_client_control_send(c, "chassis", 1, "steer", "{\"deg\":10}");
 *   ...
 *   mediaservo_client_control_off_ack(c, tok);            // or leave it to close's uniform reclamation
 *   mediaservo_client_control_close(c);
 *   mediaservo_client_consumer_close(cons);
 *   mediaservo_client_session_close(s);
 *
 * Lifecycle contract (inheriting link-c R2 discipline):
 * - Each handle is single-thread-owner; any API call after close is UB (close(NULL) is idempotent and returns OK).
 * - session_close = set the closed flag → close the session (wakes the ⊘ video pump; the watch sender dying wakes
 *   the state pump) → join the pumps → per-entry user_free → free memory. Does not implicitly close consumer/control.
 * - consumer_close = withdraw that consumer's receiver (K1 replay declares it dead) → join the pump (≤250ms) →
 *   user_free exactly once → free.
 * - The control handle closes independently (control_close joins its own ack pump); closing control before session
 *   is the normal order; in reverse order the ack pump converges naturally when the session's signaling surface dies.
 * - Frame/ack/state callbacks fire only on their own internal pump threads; no lock is held during a callback; callbacks must return
 *   quickly; calling any mediaservo_client_* API (including close) from within a callback is forbidden — undefined behavior.
 * - JSON strings and frame.data pointers inside callbacks are valid only during the callback (copy if you need to keep them).
 * - user_free contract (exactly once per registration): on_state → session_close;
 *   consume → consumer_close; on_ack → the pump reclamation round after off_ack (≤1s) or
 *   control_close (entries never off'ed are released at close).
 * - Every config struct starts with struct_size: fill sizeof(struct); the library validates >= the known size and
 *   ignores extra length — struct evolution does not break binary compatibility (R3).
 * - needed overflow contract (login/list_rooms/wait_video/consumer_id/consumer_stats/
 *   producer_ids/video_stats): insufficient cap → *needed receives the required byte count (including NUL) +
 *   ERR_INVALID_ARG (no partial write); success → *needed = the actual length. needed may be NULL.
 * - All blocking calls block_on a process-level shared multi_thread tokio runtime
 *   (the ack pump / WS read loop are tokio::spawn background tasks whose survival requires the runtime to outlive any single call).
 */
#ifndef MEDIASERVO_CLIENT_H
#define MEDIASERVO_CLIENT_H

#include <stddef.h>
#include <stdint.h>

#include <mediaservo/common.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -- Error codes (0 = ok, <0 = error; all ClientError variants mapped, pinned by exhaustive-match unit tests) -- */
#define MEDIASERVO_CLIENT_ERR_INVALID_ARG   (-1)
#define MEDIASERVO_CLIENT_ERR_LOGIN         (-2)   /* login request/parse failure */
#define MEDIASERVO_CLIENT_ERR_UNAUTHORIZED  (-3)   /* InvalidCredentials / AuthRejected(4003/4010/4011) /
                                                       RestRejected (a non-2xx on the REST discovery surface) */
#define MEDIASERVO_CLIENT_ERR_DENIED        (-4)   /* ControlDenied（server 4012） */
#define MEDIASERVO_CLIENT_ERR_TIMEOUT       (-5)   /* Timeout{what} */
#define MEDIASERVO_CLIENT_ERR_SIGNAL        (-6)   /* Signal/LinkError (WS connect/disconnect/send-recv) */
#define MEDIASERVO_CLIENT_ERR_PROTOCOL      (-7)   /* ProtocolTooLow / ProtocolUnsupported / 4101 */
#define MEDIASERVO_CLIENT_ERR_MALFORMED     (-8)   /* MalformedResponse / UnsupportedScheme */
#define MEDIASERVO_CLIENT_ERR_STATE         (-9)   /* InvalidState (channel/session not open or pumps already closed) */
#define MEDIASERVO_CLIENT_ERR_INTERNAL     (-10)   /* Server (unclassified) / WebRtc / Io / panic fallback */

/* Session connection-state wire values (session_state/out_state + the state callback's second parameter). */
#define MEDIASERVO_CLIENT_STATE_DISCONNECTED 0u /* auto_reconnect off and the link is down */
#define MEDIASERVO_CLIENT_STATE_CONNECTED    1u /* signaling connected (normal service state) */
#define MEDIASERVO_CLIENT_STATE_RECONNECTING 2u /* inside the reconnect loop after a disconnect */
#define MEDIASERVO_CLIENT_STATE_FAILED       3u /* non-retryable rejection (auth family/4101, terminal) */

/* DC ready-state wire values (control_ready_state; a stable mapping of RTCDataChannelState). */
#define MEDIASERVO_CLIENT_DC_CONNECTING 0u
#define MEDIASERVO_CLIENT_DC_OPEN       1u
#define MEDIASERVO_CLIENT_DC_CLOSING    2u
#define MEDIASERVO_CLIENT_DC_CLOSED     3u

/* -- Machine-readable error struct (K3; session_error out-param) -- */
typedef struct mediaservo_client_error_t {
    size_t   struct_size;  /* sizeof(mediaservo_client_error_t) */
    int      code;         /* MEDIASERVO_CLIENT_ERR_* (0 = this handle has no error) */
    uint32_t wire_code;    /* server wire code readback (0 = local / no code domain) */
    uint8_t  retryable;    /* 1 = retryable family (the C mirror of the D273 classification) */
} mediaservo_client_error_t;

/* -- Login config (⊘ kept for one period: since batch 1b login uses flat parameters, this struct is no longer consumed;
 *    batch 2 removes it with the ⊘ list) -- */
typedef struct mediaservo_client_login_config_t {
    size_t struct_size;      /* sizeof(mediaservo_client_login_config_t) */
    const char* http_base_url; /* "http://host:port" (v1 is plaintext http only; POST {base}/api/auth/login) */
    const char* username;
    const char* password;
} mediaservo_client_login_config_t;

#define MEDIASERVO_CLIENT_LOGIN_CONFIG_DEFAULT { sizeof(mediaservo_client_login_config_t), NULL, NULL, NULL }

/* -- Session config -- */
typedef struct mediaservo_client_config_t {
    size_t struct_size;      /* sizeof(mediaservo_client_config_t) */
    const char* signaling_url; /* "ws://host:9800/ws" */
    const char* room;          /* Room ID */
    const char* jwt;           /* account JWT (the mediaservo_client_login output); pick one of jwt/psk (exactly one non-empty) */
    const char* psk;           /* PSK passed directly (closed networks without accounts); pick one of jwt/psk */
    const char* role;          /* "Client"/"Viewer"→consumer role, "Remote"→cockpit peer role; NULL=Client.
                                *   Note: Client/Viewer are locally identical (PeerRole::Consumer) —
                                *   control-permission differences are adjudicated by the server account's can_control gate, not by the local role. */
    const char* hmac_key_file; /* Emergency-stop HMAC key file (W4b; must be 0600, non-empty, ≤4KiB, trailing newline auto-stripped).
                                *   NULL = estop is not signed (no key on the vehicle = the migration-allow form;
                                *   key present = the vehicle refusing to sign is the correct ruling, not silence). A bad path does not block
                                *   session setup — the estop call site reports INVALID_ARG. G13: keys never travel via argv/env. */
    /* Device identity instance directory (viewer-auth-matrix T1; identity.json + etc/link/signing.pem
     * in the same layout as msrtc-host init). Trailing additive: old callers (smaller struct_size) = the field
     * is absent; the library bound-checks by struct_size and does not read this slot. NULL/empty = disabled. Composes with jwt/psk
     * (server device auth takes priority); all three at once = INVALID_ARG. Missing files in the directory error at connect. */
    const char* identity_dir;
} mediaservo_client_config_t;

#define MEDIASERVO_CLIENT_CONFIG_DEFAULT \
    { sizeof(mediaservo_client_config_t), NULL, NULL, NULL, NULL, NULL, NULL, NULL }

/* ── opaque handle ── */
typedef struct mediaservo_client_session_t mediaservo_client_session_t;
typedef struct mediaservo_client_control_t mediaservo_client_control_t;
typedef struct mediaservo_client_consumer_t mediaservo_client_consumer_t;

/* -- Video frame description (I420; data is valid only inside the callback) -- */
typedef struct mediaservo_client_frame_t {
    uint32_t width;
    uint32_t height;
    int64_t  ts_us;         /* always 0 when the upstream has no clock source (consumed in arrival order) */
    const uint8_t* data;    /* I420 planes contiguous (len bytes); valid only inside the callback */
    size_t   len;
} mediaservo_client_frame_t;

/* -- Callbacks (fire on their own internal pump threads; strings/frames are valid only inside the callback) -- */
typedef void (*mediaservo_client_frame_cb)(mediaservo_client_session_t* s, const mediaservo_client_frame_t* frame, void* user);
typedef void (*mediaservo_client_ack_cb)(mediaservo_client_control_t* c, const char* ack_json, void* user);
typedef void (*mediaservo_client_state_cb)(mediaservo_client_session_t* s, uint8_t state, void* user);
/* user pointer deallocator: NULL = do not free; invoked exactly once per registration (timing in the file-header user_free contract). */
typedef void (*mediaservo_client_user_free)(void* user);

/* ── API ── */

/* Account login (POST {http_base}/api/auth/login, blocking; the batch-1b flat signature upgrade).
 * On success out_jwt = a NUL-terminated JWT; needed overflow contract in the file header (cap ≥ 4096 recommended for jwts). */
mediaservo_err_t mediaservo_client_login(const char* http_base, const char* username,
                                         const char* password, char* out_jwt, size_t cap,
                                         size_t* needed);

/* Exchange an API key for a short JWT (POST {http_base}/api/auth/exchange, blocking; a pre-session free function,
 * accountless-client-auth T2). On success out_jwt = a NUL-terminated JWT; fill it verbatim into config.jwt to use the
 * existing /ws gate (the credential source changes, the consumption surface does not). 401 (unknown key/wrong secret, identical message verbatim to prevent enumeration)
 * → ERR_UNAUTHORIZED; same needed overflow contract as login. */
mediaservo_err_t mediaservo_client_exchange(const char* http_base, const char* key_id,
                                            const char* secret, char* out_jwt, size_t cap,
                                            size_t* needed);

/* Room discovery (GET {http_base}/api/rooms, blocking; a pre-session free function — depends on no handle).
 * out_json = the JSON array `[{"room_id":..,"kind":"video"|"audio"},..]` (the server wire passed through,
 * this layer does not parse it). needed overflow contract in the file header.
 * An invalid token / wrong role (non-2xx) → ERR_UNAUTHORIZED; details in mediaservo_client_last_error. */
mediaservo_err_t mediaservo_client_list_rooms(const char* http_base, const char* jwt, char* out_json, size_t cap, size_t* needed);

/* PSK room discovery (psk-discover: Authorization: Psk <psk> → the Legacy full view,
 * the discovery surface for cross-room pulls). out_json has the same shape as list_rooms; 401 → ERR_UNAUTHORIZED. */
mediaservo_err_t mediaservo_client_list_rooms_psk(const char* http_base, const char* psk, char* out_json, size_t cap, size_t* needed);

/* Error code → static text (table source = the ClientError Display templates; unknown code → "unknown error code").
 * Copy semantics = truncate without erroring (same shape as last_error). */
mediaservo_err_t mediaservo_client_strerror(int code, char* buf, size_t cap);

/* Connect signaling and join the room (blocking). Exactly one of jwt/psk non-empty; on success *out points to the new handle (the caller closes). */
mediaservo_err_t mediaservo_client_session_create(const mediaservo_client_config_t* cfg, mediaservo_client_session_t** out);

/* The negotiated dialect version (S0; an old server = 1). */
mediaservo_err_t mediaservo_client_session_negotiated(const mediaservo_client_session_t* s, uint32_t* out_protocol);

/* The effective identity label (viewer-auth-matrix T4/F6; a static value derived locally at create, zero network).
 * Forms like "Device(ms-...)" / "jwt" / "legacy-psk" / "(no credential)"; same needed overflow
 * contract as list_rooms. Readable even on a closed session (a static snapshot). */
mediaservo_err_t mediaservo_client_session_identity(const mediaservo_client_session_t* s, char* out, size_t cap, size_t* needed);

/* Session connection-state snapshot (out_state = MEDIASERVO_CLIENT_STATE_*). A closed session → ERR_STATE. */
mediaservo_err_t mediaservo_client_session_state(const mediaservo_client_session_t* s, uint8_t* out_state);

/* Register a session connection-state callback (cumulative form: multiple registrations = every change invokes each registered entry; no unregister,
 * released together at session_close with per-entry user_free). The pump starts on first registration while the session is alive;
 * it reports only changes — fetch the initial state after registering via session_state. cb=NULL → ERR_INVALID_ARG. */
mediaservo_err_t mediaservo_client_session_on_state(mediaservo_client_session_t* s,
                                                    mediaservo_client_state_cb cb, void* user,
                                                    mediaservo_client_user_free user_free);

/* Purely wait for a video producer in the room (no side effects; the directed entry point for multi-consumer orchestration). On success out_pid = the producer
 * id; timeout → ERR_TIMEOUT; disconnect → ERR_STATE. needed overflow contract in the file header. */
mediaservo_err_t mediaservo_client_session_wait_video(mediaservo_client_session_t* s,
                                                      uint64_t timeout_ms, char* out_pid,
                                                      size_t cap, size_t* needed);

/* producer-label T2.2: wait directed by label (label NULL/empty = no filter; falls back to the first consumer).
 * All other contracts match wait_video (needed overflow/timeout/disconnect). additive: zero impact on existing consumers. */
mediaservo_err_t mediaservo_client_session_wait_video_labeled(
    const mediaservo_client_session_t* s, const char* label,
    uint64_t timeout_ms, char* out_pid, size_t cap, size_t* needed);

/* Subscribe to one video producer (the new multi-consumer form, K4): one independent frame pump + consumer handle per producer, no cross coupling.
 * producer_id comes from wait_video or the list/observation surface. On success **out_consumer = the new handle
 * (the caller owns consumer_close; session_close does not implicitly close it). user_free runs exactly once at consumer_close.
 * cb=NULL → ERR_INVALID_ARG (this form has no cancel semantics). */
mediaservo_err_t mediaservo_client_session_consume(mediaservo_client_session_t* s,
                                                   const char* producer_id,
                                                   mediaservo_client_frame_cb cb, void* user,
                                                   mediaservo_client_user_free user_free,
                                                   mediaservo_client_consumer_t** out_consumer);

/* ⊘ Consume the room's first video (first-consumer form, kept for one period; new code should use wait_video+consume):
 * internally = wait for the producer (≤30s) → consume → into_receiver, byte-for-byte identical to the old form.
 * The first call starts the frame pump thread; a repeat call → ERR_STATE (one-shot gate). */
mediaservo_err_t mediaservo_client_session_consume_video(mediaservo_client_session_t* s, mediaservo_client_frame_cb cb, void* user);

/* ⊘ video stats aggregate JSON (session-level union, kept for one period; use consumer_stats for precise per-consumer readings):
 * {"bytes_received","packets_received","packets_lost","frames_decoded",
 *  "frame_width","frame_height","frames_per_second"}, a fold over this session's inbound-rtp,
 * all zeros with no consumers. needed overflow contract in the file header. */
mediaservo_err_t mediaservo_client_session_video_stats(const mediaservo_client_session_t* s, char* out_json, size_t cap, size_t* needed);

/* K3 handle-level machine-readable error (this handle's most recent failed call; no error = code 0). Covers state/transport/mapping-class
 * failures; buffer overflow (the needed contract) only produces a return code + the global last_error. out.struct_size is required. */
mediaservo_err_t mediaservo_client_session_error(const mediaservo_client_session_t* s, mediaservo_client_error_t* out);

/* K3 handle-level error text (details of this handle's most recent failure; no error = empty string). Truncates without erroring. */
mediaservo_err_t mediaservo_client_session_last_error(const mediaservo_client_session_t* s, char* buf, size_t len);

/* Emergency-stop dual-path delivery (W4b): the DC fast path (HMAC-signed when hmac_key_file is configured) + the signaling audit copy.
 * Precondition: mediaservo_client_open_control has succeeded. payload_json NULL/empty = null.
 * OK = delivery succeeded (not "it executed" — the vehicle's adjudication is the seq receipt ack). */
mediaservo_err_t mediaservo_client_session_emergency_stop(mediaservo_client_session_t* s, mediaservo_client_control_t* c,
                                                          const char* label, uint64_t seq,
                                                          const char* payload_json);

/* Open the outbound control channel set (once per session — one ack pump per session; a second call → ERR_STATE).
 * labels is the array of channel names (e.g. "chassis"/"gimbal"). A dialect <2 is pre-rejected locally (ERR_PROTOCOL). */
mediaservo_err_t mediaservo_client_open_control(mediaservo_client_session_t* s, const char* const* labels, size_t labels_len, mediaservo_client_control_t** out);

/* Send one command envelope {seq, cmd, payload} (serde-built; payload_json NULL/"" = null;
 * invalid JSON → ERR_INVALID_ARG). The caller maintains seq as a monotonic counter (D-H3 makes re-sends idempotent-safe). */
mediaservo_err_t mediaservo_client_control_send(mediaservo_client_control_t* c, const char* label, uint64_t seq, const char* cmd, const char* payload_json);

/* Register an ack callback (cumulative form, the batch-1b signature upgrade: multiple registrations = each ack invokes every registered entry;
 * the first registration starts the pump). out_token returns the unregistration credential (may be NULL; 0 = invalid). cb=NULL → reject
 * (unregister via control_off_ack). user_free runs exactly once at the pump reclamation round after off_ack (≤1s) or at control_close.
 * ack_json = the serialized ControlAck ({"ack":..,"result":..}). */
mediaservo_err_t mediaservo_client_control_on_ack(mediaservo_client_control_t* c, mediaservo_client_ack_cb cb,
                                                  void* user, mediaservo_client_user_free user_free,
                                                  uint64_t* out_token);

/* Unregister an ack callback by token (marks it retired; the pump reclaims it next round and invokes its user_free; acks arriving after the next
 * round no longer trigger it — an in-flight round may still fire once, bounded grace). Unknown token → ERR_INVALID_ARG. */
mediaservo_err_t mediaservo_client_control_off_ack(mediaservo_client_control_t* c, uint64_t token);

/* DC ready state (out_state = MEDIASERVO_CLIENT_DC_*; used as the gate before sending/estop). */
mediaservo_err_t mediaservo_client_control_ready_state(const mediaservo_client_control_t* c, uint8_t* out_state);

/* SCTP backpressure water level (bytes; 0 = safe to append). Contends with the ack pump for the lock — acceptable within the single-thread-owner contract. */
mediaservo_err_t mediaservo_client_control_buffered_amount(mediaservo_client_control_t* c, uint64_t* out_bytes);

/* JSON array of the server-assigned data producer ids (observation surface). needed overflow contract in the file header
 * (the batch-1b upgrade: fixes the old fixed-buffer blind spot). */
mediaservo_err_t mediaservo_client_control_producer_ids(const mediaservo_client_control_t* c, char* out_json, size_t cap, size_t* needed);

/* Close the control channel and release the handle (idempotent; after joining the ack pump, reclaims the whole registry with per-entry user_free). */
mediaservo_err_t mediaservo_client_control_close(mediaservo_client_control_t* c);

/* -- Single consumer (produced by session_consume; K4 multi-consumer, no cross coupling) -- */

/* This consumer's producer id (needed overflow contract; ids are short, cap 256 is enough). */
mediaservo_err_t mediaservo_client_consumer_id(const mediaservo_client_consumer_t* c, char* out, size_t cap, size_t* needed);

/* This consumer's video stats JSON (per-consumer reading; the key table matches session_video_stats). Same needed contract as above. */
mediaservo_err_t mediaservo_client_consumer_stats(mediaservo_client_consumer_t* c, char* out_json, size_t cap, size_t* needed);

/* QOE periodic callback (consumer-qoe-callback, 2026-10-09; aligned with the TRRO OnMediaState shape):
 * the SDK handles tick/diff/conversion internally — cb receives ready-converted values. Idempotent: re-registration replaces the previous;
 * consumer_close stops it (the PIT-221 join discipline: after close/re-registration returns, the old cb physically cannot fire again —
 * a hard abort+join guarantee). cb fires on the SDK tick thread — **blocking is forbidden, re-entering close/on_qoe is forbidden**.
 * The field layout is frozen (ABI additive-only discipline): future fields may only append at the end. */
typedef struct mediaservo_client_qoe_snapshot {
    uint32_t struct_size;    /* sizeof this struct (caller-filled, the evolution guardrail) */
    uint32_t fps;
    uint32_t kbps;
    float    lost_percent;
    uint64_t decoded_total;
    uint32_t avg_decode_ms;  /* average decode time ms/frame (ΔtotalDecodeTime/Δframes; appended by qoe-full-metrics) */
    uint32_t jitter_ms;      /* jitter in ms (inbound jitter×1000) */
    uint32_t rtt_ms;         /* round-trip time ms (candidate-pair; 0 when the backend has no source) */
    uint32_t width;
    uint32_t height;
    uint32_t epoch;          /* renegotiation-breakpoint generation (for identification only, not for business decisions) */
} mediaservo_client_qoe_snapshot_t;

typedef void (*mediaservo_client_qoe_cb)(const mediaservo_client_qoe_snapshot_t* snap, void* user);

mediaservo_err_t mediaservo_client_consumer_on_qoe(mediaservo_client_consumer_t* c,
                                                   uint32_t period_ms,
                                                   mediaservo_client_qoe_cb cb, void* user);

/* Close this consumer and release the handle (idempotent; withdraw the receiver → join the pump (≤250ms) → user_free exactly once). */
mediaservo_err_t mediaservo_client_consumer_close(mediaservo_client_consumer_t* c);

/* Close the session and release the handle (idempotent; memory is freed only after joining the pumps + per-entry user_free of the state callbacks;
 * does not implicitly close consumer/control). */
mediaservo_err_t mediaservo_client_session_close(mediaservo_client_session_t* s);

/* -- Common -- */

/* ⊘ last error details (process-global, kept for one period — the K3 dual-write fallback; new code uses the handle-level
 * session_last_error + session_error). Thread-safe. */
mediaservo_err_t mediaservo_client_last_error(char* buf, size_t len);

/* ⊘ tracing log file target (MSRTC.log). Must be called once before the first runtime consumption point (any login/
 * consume API); repeat calls return ERR_STATE. The file opens in append mode, falling back to
 * stdout on failure. Levels: env MSRTC_LOG_LEVEL (default info); the libwebrtc noise layer defaults to warn. */
mediaservo_err_t mediaservo_client_set_log_file(const char* path);

/* SDK version (MAJOR.MINOR.PATCH). */
mediaservo_err_t mediaservo_client_version(char* buf, size_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MEDIASERVO_CLIENT_H */
