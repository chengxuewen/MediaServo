/* MediaServo shared C types — the common foundation for all SDK headers.
 *
 * MAJOR = C ABI version (D241): additive-only within a MAJOR, binary compatible.
 * This header is hand-maintained (stable export surface, same discipline as cbindgen output).
 */
#ifndef MEDIASERVO_COMMON_H
#define MEDIASERVO_COMMON_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -- Error codes --
 * Each SDK header expands its value space under the MEDIASERVO_<SDK>_ERR_* prefix (avoids macro collisions when multiple headers are included together):
 *   field: MEDIASERVO_FIELD_ERR_* (legacy MEDIASERVO_ERR_* aliases retained)
 *   link:  MEDIASERVO_LINK_ERR_*
 *   deck:  MEDIASERVO_DECK_ERR_*
 * 0 = ok, <0 = error code.
 */
typedef int mediaservo_err_t;
#define MEDIASERVO_OK 0

/* -- Frame metadata (link frame-bus wire format, fixed 36B LE, D243) --------
 * Layout is byte-for-byte identical to Rust FrameMeta::encode:
 *   seq(8) + width(4) + height(4) + format(1) + version(1)
 *   + is_keyframe(1) + reserved(1) + ts_mono_ns(8) + ts_epoch_ns(8)
 *
 * Note: #pragma pack(1) forces 36B — natural alignment would add padding
 * for the trailing u64 (40B total), which does not match the wire format. The C struct is just a field bag: the Rust side
 * reads it field-by-field to build FrameMeta; never reinterpret the whole block (padding/endianness risk).
 */
#pragma pack(push, 1)
typedef struct mediaservo_frame_meta_t {
    uint64_t seq;
    uint32_t width;
    uint32_t height;
    uint8_t  format;        /* 0=unknown, 1=I420, 2=NV12, 3=RGBA */
    uint8_t  version;       /* metadata version (for evolution, D243) */
    uint8_t  is_keyframe;   /* 0/1 */
    uint8_t  reserved;      /* must be 0 */
    uint64_t ts_mono_ns;    /* monotonic clock ns */
    uint64_t ts_epoch_ns;   /* wall-clock ns */
} mediaservo_frame_meta_t;
#pragma pack(pop)

/* Compile-time size assertions (C/C++ portable; avoids the C11 _Static_assert dependency) */
typedef char mediaservo_frame_meta_size_check[(sizeof(mediaservo_frame_meta_t) == 36) ? 1 : -1];

/* -- In-memory frame description (deck capture/playback callbacks, I420 three planes) --------
 * data_* pointers are valid only inside the callback — copy if you need to keep them (documented contract).
 */
typedef struct mediaservo_frame_t {
    uint32_t width;
    uint32_t height;
    uint64_t pts_us;        /* presentation timestamp µs */
    uint32_t stride_y;
    uint32_t stride_u;
    uint32_t stride_v;
    const uint8_t* data_y;
    const uint8_t* data_u;
    const uint8_t* data_v;
} mediaservo_frame_t;

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MEDIASERVO_COMMON_H */
