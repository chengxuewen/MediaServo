/* MediaServo link C ABI (signaling + frame bus) — device-side SDK consumption.
 *
 * MAJOR = C ABI version (D241): additive-only within a MAJOR, binary compatible.
 * This header is hand-maintained (stable export surface, same discipline as cbindgen output).
 * Shared C types (mediaservo_err_t/mediaservo_frame_meta_t/mediaservo_frame_t): see common.h.
 *
 * Usage (signaling):
 *   mediaservo_link_signal_config_t cfg = MEDIASERVO_LINK_SIGNAL_CONFIG_DEFAULT;
 *   cfg.url = "ws://host:9800/ws"; cfg.psk = "..."; cfg.room = "...";
 *   mediaservo_link_signal_t* s = NULL;
 *   if (mediaservo_link_signal_connect(&cfg, &s) != MEDIASERVO_OK) { read mediaservo_link_last_error }
 *   mediaservo_link_signal_on_event(s, on_event, NULL);   -- register any time; re-registration replaces
 *   mediaservo_link_signal_send(s, "{\"type\":\"frame\",...}", len);
 *   ...
 *   mediaservo_link_signal_close(s);
 *
 * Lifecycle contract (R2):
 * - Each handle is single-thread-owner; any API call after close is UB (close(NULL) is idempotent and returns OK).
 * - close = set the closed flag → release the session (wakes the event pump) → join the pump thread → free memory.
 * - Event callbacks fire on a single internal pump thread and no lock is held during the callback; callbacks must return
 *   quickly; calling any mediaservo_link_signal_* API (including close) from within a callback is forbidden — undefined behavior.
 * - The event JSON string is valid only inside the callback (copy it if you need to keep it).
 * - Events that occurred before on_event registration may be lost (the broadcast keeps only post-subscription messages); on first
 *   registration the pump re-synthesizes the {"type":"connected"} event.
 *
 * Event JSON (opaque v1):
 *   {"type":"connected","room_id":...}
 *   {"type":"message","message":{...SignalingMessage}}
 *   {"type":"disconnected","reason":...}
 *   {"type":"error","error":...}
 */
#ifndef MEDIASERVO_LINK_H
#define MEDIASERVO_LINK_H

#include <stddef.h>
#include <stdint.h>

#include <mediaservo/common.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -- Error codes (0 = ok, <0 = error) -- */
#define MEDIASERVO_LINK_ERR_INVALID_ARG  (-1)
#define MEDIASERVO_LINK_ERR_CONNECT      (-2)
#define MEDIASERVO_LINK_ERR_SEND         (-3)
#define MEDIASERVO_LINK_ERR_BUS          (-4)
#define MEDIASERVO_LINK_ERR_STATE        (-5)
#define MEDIASERVO_LINK_ERR_INTERNAL     (-6)
#define MEDIASERVO_LINK_ERR_CLOSED       (-7)

/* -- Signaling config --
 * struct_size first field (R3): the caller must fill sizeof(mediaservo_link_signal_config_t);
 * the library validates >= sizeof(known struct) and ignores extra length — struct evolution does not break binary compatibility.
 */
typedef struct mediaservo_link_signal_config_t {
    size_t struct_size;      /* sizeof(mediaservo_link_signal_config_t) */
        const char* url;         /* signaling WS address, e.g. "ws://host:9800/ws" */
        const char* psk;         /* PSK authentication key */
        const char* room;        /* Room ID */
    const char* role;        /* "Host"/"Pusher"→Host, "Client"/"Puller"→Remote; NULL=Host */
} mediaservo_link_signal_config_t;

#define MEDIASERVO_LINK_SIGNAL_CONFIG_DEFAULT { sizeof(mediaservo_link_signal_config_t), NULL, NULL, NULL, NULL }

/* ── opaque handle ── */
typedef struct mediaservo_link_signal_t mediaservo_link_signal_t;
typedef struct mediaservo_link_bus_t mediaservo_link_bus_t;
typedef struct mediaservo_link_stream_t mediaservo_link_stream_t;

/* -- Signaling event callback (fires on a single internal pump thread; the event string is valid only inside the callback) -- */
typedef void (*mediaservo_link_event_cb)(mediaservo_link_signal_t* s, const char* event_json, void* user);

/* -- Signaling session API -- */

/* Connect signaling and create the session (blocking). Success: *out points to the new handle (caller closes). */
mediaservo_err_t mediaservo_link_signal_connect(const mediaservo_link_signal_config_t* cfg, mediaservo_link_signal_t** out);

/* Send one signaling message (JSON + byte length; SignalingMessage type tag is snake_case). */
mediaservo_err_t mediaservo_link_signal_send(mediaservo_link_signal_t* s, const char* msg_json, size_t len);

/* Register the event callback (any time after connect; re-registration replaces; the first registration starts the event pump). */
void mediaservo_link_signal_on_event(mediaservo_link_signal_t* s, mediaservo_link_event_cb cb, void* user);

/* Close the session and release the handle (idempotent; memory is freed only after joining the event pump). */
mediaservo_err_t mediaservo_link_signal_close(mediaservo_link_signal_t* s);

/* -- Frame bus API -- */

/* Attach the frame bus (signature verification + ACL + iceoryx2 node, blocking). endpoint is reserved for Phase 1
 * (an empty string is fine); token_pem/vk_pem are the Ed25519 capability token / verification key PEM strings. */
mediaservo_err_t mediaservo_link_bus_attach(const char* endpoint, const char* token_pem,
                            const char* vk_pem, mediaservo_link_bus_t** out);

/* Publish one frame (ACL check + SHM loan + send, blocking). meta is a 36B field bag
 * (read field-by-field); payload may be NULL if and only if len == 0 (metadata-only frame). */
mediaservo_err_t mediaservo_link_bus_publish(mediaservo_link_bus_t* b, const char* topic,
                             const uint8_t* payload, size_t len,
                             const mediaservo_frame_meta_t* meta);

/* Subscribe to a topic and create a frame-stream handle (blocking). Success: *out points to the new handle
 * (no transfer of ownership; release via mediaservo_link_stream_close). */
mediaservo_err_t mediaservo_link_bus_subscribe(mediaservo_link_bus_t* b, const char* topic, mediaservo_link_stream_t** out);

/* Blocking frame fetch: metadata is copied into out_meta, the payload into out_data (up to cap bytes),
 * *out_len = bytes actually copied (truncated when the frame exceeds cap — give large frames enough cap).
 * Shutdown (stream_close / bus_close) → MEDIASERVO_LINK_ERR_CLOSED. */
mediaservo_err_t mediaservo_link_bus_recv(mediaservo_link_stream_t* st, mediaservo_frame_meta_t* out_meta,
                          uint8_t* out_data, size_t cap, size_t* out_len);

/* Close the frame-stream handle (idempotent; wakes a blocked recv so it returns CLOSED). */
mediaservo_err_t mediaservo_link_stream_close(mediaservo_link_stream_t* st);

/* Close the frame-bus handle (idempotent; shuts down all streams, recv returns CLOSED). */
mediaservo_err_t mediaservo_link_bus_close(mediaservo_link_bus_t* b);

/* -- Common -- */

/* Details of the last error (thread-safe). */
mediaservo_err_t mediaservo_link_last_error(char* buf, size_t len);

/* SDK version (MAJOR.MINOR.PATCH). */
mediaservo_err_t mediaservo_link_version(char* buf, size_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MEDIASERVO_LINK_H */
