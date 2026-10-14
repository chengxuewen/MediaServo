/* MediaServo Field C ABI (push plane) — vehicle-side SDK consumption.
 *
 * MAJOR = C ABI version (D241): additive-only within a MAJOR, binary compatible.
 * This header is hand-maintained (stable export surface, same discipline as cbindgen output).
 * Shared C types (mediaservo_err_t/mediaservo_frame_meta_t/mediaservo_frame_t): see common.h.
 *
 * Usage:
 *   mediaservo_push_config_t cfg = MEDIASERVO_PUSH_CONFIG_DEFAULT;
 *   cfg.url = "ws://host:9800/ws"; cfg.psk = "..."; cfg.room = "...";
 *   mediaservo_field_push_t* s = NULL;
 *   if (mediaservo_field_push_connect(&cfg, &s) != MEDIASERVO_OK) { read mediaservo_field_last_error }
 *   char track[64];
 *   mediaservo_field_push_publish_video(s, track, sizeof(track));
 *   mediaservo_field_push_start_video_frames(s);
 *   ...
 *   mediaservo_field_push_close(s);
 *
 * Lifecycle contract: each handle is single-thread-owner; any API call after close is UB (close is idempotent).
 */
#ifndef MEDIASERVO_FIELD_H
#define MEDIASERVO_FIELD_H

#include <stddef.h>
#include <stdint.h>

#include <mediaservo/common.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -- Error codes (0 = ok, <0 = error) -- */
#define MEDIASERVO_FIELD_ERR_INVALID_ARG  (-1)
#define MEDIASERVO_FIELD_ERR_CONNECT      (-2)
#define MEDIASERVO_FIELD_ERR_PUBLISH      (-3)
#define MEDIASERVO_FIELD_ERR_STATE        (-4)
#define MEDIASERVO_FIELD_ERR_INTERNAL     (-5)
/* Legacy aliases (kept additive-only; new code uses MEDIASERVO_FIELD_ERR_*) */
#define MEDIASERVO_ERR_INVALID_ARG        (-1)
#define MEDIASERVO_ERR_CONNECT            (-2)
#define MEDIASERVO_ERR_PUBLISH            (-3)
#define MEDIASERVO_ERR_STATE              (-4)
#define MEDIASERVO_ERR_INTERNAL           (-5)

/* -- Push config --
 * struct_size first field (R3): the caller must fill sizeof(mediaservo_push_config_t);
 * the library validates >= sizeof(known struct) and ignores extra length — struct evolution does not break binary compatibility.
 */
typedef struct mediaservo_push_config_t {
    size_t struct_size;           /* sizeof(mediaservo_push_config_t) */
        const char* url;              /* signaling WS address, e.g. "ws://host:9800/ws" */
        const char* psk;              /* PSK authentication key */
        const char* room;             /* Room ID */
        uint32_t width;               /* video width (default 1280) */
        uint32_t height;              /* video height (default 720) */
        uint32_t framerate;           /* framerate (default 30) */
        uint32_t bitrate_kbps;        /* encoder bitrate kbps (default 2000) */
        uint64_t keyframe_interval;   /* keyframe interval in seconds (default 2) */
} mediaservo_push_config_t;

#define MEDIASERVO_PUSH_CONFIG_DEFAULT { sizeof(mediaservo_push_config_t), NULL, NULL, NULL, 1280, 720, 30, 2000, 2 }

/* ── opaque handle ── */
typedef struct mediaservo_field_push_t mediaservo_field_push_t;

/* -- Push session API -- */

/* Connect signaling and create the session (blocking). Success: *out points to the new handle (caller closes). */
mediaservo_err_t mediaservo_field_push_connect(const mediaservo_push_config_t* cfg, mediaservo_field_push_t** out);

/* Publish the video track (blocking negotiation). The track id is written to out_track (at least 64 bytes). */
mediaservo_err_t mediaservo_field_push_publish_video(mediaservo_field_push_t* s, char* out_track, size_t out_track_len);

/* Start video frame generation (squares + timestamp watermark). */
mediaservo_err_t mediaservo_field_push_start_video_frames(mediaservo_field_push_t* s);

/* Stop video frame generation (idempotent). */
void mediaservo_field_push_stop_video_frames(mediaservo_field_push_t* s);

/* Close the session and release the handle (idempotent). */
mediaservo_err_t mediaservo_field_push_close(mediaservo_field_push_t* s);

/* -- Common -- */

/* Details of the last error (thread-safe). */
mediaservo_err_t mediaservo_field_last_error(char* buf, size_t len);

/* Details of the last error (deprecated alias, kept additive-only). */
mediaservo_err_t mediaservo_last_error(char* buf, size_t len);

/* SDK version (MAJOR.MINOR.PATCH). */
mediaservo_err_t mediaservo_field_version(char* buf, size_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MEDIASERVO_FIELD_H */
