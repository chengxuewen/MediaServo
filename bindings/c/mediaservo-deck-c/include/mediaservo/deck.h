/* MediaServo Deck C ABI (capture/record/playback plane) — C consumption for local monitoring/NVR scenarios.
 *
 * MAJOR = C ABI version (D241): additive-only within a MAJOR, binary compatible.
 * This header is hand-maintained (stable export surface, same discipline as cbindgen output).
 * Shared C types (mediaservo_err_t/mediaservo_frame_t): see common.h.
 *
 * Usage:
 *   mediaservo_deck_camera_t* cam = NULL;
 *   mediaservo_deck_capture_options_t copts = MEDIASERVO_DECK_CAPTURE_OPTIONS_DEFAULT;  // 1280x720@30
 *   mediaservo_deck_camera_open("stub:test-camera", &copts, &cam);
 *   mediaservo_deck_camera_start(cam);
 *   mediaservo_deck_camera_frames_cb(cam, on_frame, NULL);  // per-frame callback on the pump thread
 *   ...
 *   mediaservo_deck_camera_stop(cam);
 *   mediaservo_deck_camera_close(cam);
 *
 * Lifecycle contract:
 *   - Each handle is single-thread-owner; after close the pointer is invalid and any API call is UB
 *     (closing the same pointer twice is also UB — C convention: set it to NULL after close).
 *   - The mediaservo_frame_t data_* pointers are valid only inside the callback — copy if you need to keep them.
 *   - Frame callbacks fire only on the pump thread; calling any mediaservo_deck_* API (including close) from within a callback is forbidden.
 *   - mediaservo_deck_recorder_record(rec, cam) requires the camera to be started and to outlive the recording;
 *     the close order must be recorder_stop/close before camera_stop/close.
 */
#ifndef MEDIASERVO_DECK_H
#define MEDIASERVO_DECK_H

#include <stddef.h>
#include <stdint.h>

#include <mediaservo/common.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -- Error codes (0 = ok, <0 = error) -- */
#define MEDIASERVO_DECK_ERR_INVALID_ARG  (-1)
#define MEDIASERVO_DECK_ERR_DEVICE       (-2)
#define MEDIASERVO_DECK_ERR_RECORDER     (-3)
#define MEDIASERVO_DECK_ERR_PLAYER       (-4)
#define MEDIASERVO_DECK_ERR_STATE        (-5)
#define MEDIASERVO_DECK_ERR_INTERNAL     (-6)

/* -- Capture options --
 * struct_size first field (R3): the caller must fill sizeof(mediaservo_deck_capture_options_t);
 * the library validates >= sizeof(known struct) and ignores extra length — struct evolution does not break binary compatibility.
 * width/height/framerate all 0 = default 1280x720@30.
 */
typedef struct mediaservo_deck_capture_options_t {
    size_t struct_size;           /* sizeof(mediaservo_deck_capture_options_t) */
        uint32_t width;               /* video width (0 = default 1280) */
        uint32_t height;              /* video height (0 = default 720) */
        uint32_t framerate;           /* framerate (0 = default 30) */
} mediaservo_deck_capture_options_t;

#define MEDIASERVO_DECK_CAPTURE_OPTIONS_DEFAULT { sizeof(mediaservo_deck_capture_options_t), 0, 0, 0 }

/* ── opaque handle ── */
typedef struct mediaservo_deck_camera_t mediaservo_deck_camera_t;
typedef struct mediaservo_deck_recorder_t mediaservo_deck_recorder_t;
typedef struct mediaservo_deck_player_t mediaservo_deck_player_t;

/* -- Frame callback (fires on the pump thread; the frame pointer is valid only inside the callback) -- */
typedef void (*mediaservo_deck_frame_cb)(const mediaservo_frame_t* frame, void* user);

/* -- Device enumeration -- */

/* Two-call pattern: the first call with out_ids=NULL returns the required length (excluding NUL; negative on error),
 * the second call fills the buffer (also returns the required length on truncation, snprintf convention).
 * kind: 0=Camera 1=Audio 2=Screen; multiple devices are separated by '\n'.
 * When out_len is non-NULL both calls write back the required length. */
mediaservo_err_t mediaservo_deck_devices_enumerate(int kind, char* out_ids, size_t cap, size_t* out_len);

/* -- camera (capture) -- */

/* Open the camera (local initialization only). dev_id must exist in the enumeration result. */
mediaservo_err_t mediaservo_deck_camera_open(const char* dev_id,
                             const mediaservo_deck_capture_options_t* opts,
                             mediaservo_deck_camera_t** out);

/* Start producing frames (uses the opts from open; only allowed once, repeat calls → STATE). */
mediaservo_err_t mediaservo_deck_camera_start(mediaservo_deck_camera_t* c);

/* Register the frame callback (fires per frame on the pump thread; a repeat call replaces the old callback). */
mediaservo_err_t mediaservo_deck_camera_frames_cb(mediaservo_deck_camera_t* c, mediaservo_deck_frame_cb cb, void* user);

/* Stop producing frames (idempotent). */
mediaservo_err_t mediaservo_deck_camera_stop(mediaservo_deck_camera_t* c);

/* Close the camera and release the handle (idempotent; calling during a frame callback is UB). */
mediaservo_err_t mediaservo_deck_camera_close(mediaservo_deck_camera_t* c);

/* -- recorder (recording) -- */

/* Create the recorder (default h264/mp4; the parent directory must already exist). */
mediaservo_err_t mediaservo_deck_recorder_new(const char* path, mediaservo_deck_recorder_t** out);

/* Bridge recording: camera frame pump → recorder. The camera must be started and outlive the recording. */
mediaservo_err_t mediaservo_deck_recorder_record(mediaservo_deck_recorder_t* r, mediaservo_deck_camera_t* c);

/* Request stop recording (idempotent; flush + trailer finalize on close). */
mediaservo_err_t mediaservo_deck_recorder_stop(mediaservo_deck_recorder_t* r);

/* Close the recorder and release the handle (idempotent; joins the recording task to complete the flush). */
mediaservo_err_t mediaservo_deck_recorder_close(mediaservo_deck_recorder_t* r);

/* -- player (playback) -- */

/* Open a media file (demux + decoder ready). */
mediaservo_err_t mediaservo_deck_player_open(const char* path, mediaservo_deck_player_t** out);

/* Per-frame decode callback pump (runs until EOF ends naturally; only allowed once).
 * close is a blocking join (waits for decoding to finish) — long files require waiting; no mid-flight abort (YAGNI). */
mediaservo_err_t mediaservo_deck_player_frames_cb(mediaservo_deck_player_t* p, mediaservo_deck_frame_cb cb, void* user);

/* Close the player and release the handle (idempotent; joins the decode pump until done). */
mediaservo_err_t mediaservo_deck_player_close(mediaservo_deck_player_t* p);

/* -- Common -- */

/* Details of the last error (thread-safe). */
mediaservo_err_t mediaservo_deck_last_error(char* buf, size_t len);

/* SDK version (MAJOR.MINOR.PATCH). */
mediaservo_err_t mediaservo_deck_version(char* buf, size_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MEDIASERVO_DECK_H */
