"""MediaServo Field Python bindings — push session (mirrors the C++ mediaservo::field class structure).

API: PushConfig(dataclass) + PushSession（connect/publish_video/start_video_frames/
stop_video_frames/close). Errors raise FieldError (code + message), close is idempotent,
and __del__ closes as a safety net (never raises).
"""

import ctypes
from dataclasses import dataclass

from . import _ffi
from ._ffi import MediaServoError, cstr

__all__ = [
    "FieldError", "PushConfig", "PushSession", "version",
    "ERR_OK", "ERR_INVALID_ARG", "ERR_CONNECT", "ERR_PUBLISH", "ERR_STATE", "ERR_INTERNAL",
]

# Error codes (field.h)
ERR_OK = 0
ERR_INVALID_ARG = -1
ERR_CONNECT = -2
ERR_PUBLISH = -3
ERR_STATE = -4
ERR_INTERNAL = -5


class FieldError(MediaServoError):
    """field SDK call failed."""


_lib = _ffi.load("field")


def _last_error() -> str:
    buf = ctypes.create_string_buffer(512)
    _last_error_fn(buf, 512)
    return buf.value.decode("utf-8", errors="replace")


def _check(rc: int) -> None:
    if rc != ERR_OK:
        raise FieldError(rc, _last_error())


# -- FFI declarations (H3: restype + argtypes fully declared, 64-bit pointer truncation guard) ------

_API = {}  # name -> (fn, restype, argtypes); tests assert full coverage


def _api(name, restype, argtypes):
    fn = getattr(_lib, name)
    fn.restype = restype
    fn.argtypes = argtypes
    _API[name] = (fn, restype, argtypes)
    return fn


_push_connect = _api(
    "mediaservo_field_push_connect", ctypes.c_int,
    [ctypes.POINTER(_ffi.mediaservo_push_config_t), ctypes.POINTER(ctypes.c_void_p)],
)
_push_publish_video = _api(
    "mediaservo_field_push_publish_video", ctypes.c_int,
    [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t],
)
_push_start_video_frames = _api("mediaservo_field_push_start_video_frames", ctypes.c_int, [ctypes.c_void_p])
_push_stop_video_frames = _api("mediaservo_field_push_stop_video_frames", None, [ctypes.c_void_p])
_push_close = _api("mediaservo_field_push_close", ctypes.c_int, [ctypes.c_void_p])
_last_error_fn = _api("mediaservo_field_last_error", ctypes.c_int, [ctypes.c_char_p, ctypes.c_size_t])
_version_fn = _api("mediaservo_field_version", ctypes.c_int, [ctypes.c_char_p, ctypes.c_size_t])


def version() -> str:
    """SDK version (MAJOR.MINOR.PATCH), trailing NUL stripped."""
    buf = ctypes.create_string_buffer(64)
    rc = _version_fn(buf, 64)
    if rc != ERR_OK:
        raise FieldError(rc, _last_error())
    return buf.value.decode()


@dataclass
class PushConfig:
    """Push configuration (mirrors mediaservo_push_config_t; defaults match the C DEFAULT)."""

    url: str
    psk: str
    room: str
    width: int = 1280
    height: int = 720
    framerate: int = 30
    bitrate_kbps: int = 2000
    keyframe_interval: int = 2


class PushSession:
    """Push session (mirrors C++ mediaservo::field::PushSession). Default construction = closed."""

    def __init__(self, h=None):
        self._h = h

    @classmethod
    def connect(cls, cfg: PushConfig) -> "PushSession":
        """Connect signaling and create the session (blocking). Raises FieldError on failure; never returns a half-open session."""
        c = _ffi.mediaservo_push_config_t()
        c.struct_size = ctypes.sizeof(_ffi.mediaservo_push_config_t)  # R3: auto-fill
        c.url = cstr(cfg.url)
        c.psk = cstr(cfg.psk)
        c.room = cstr(cfg.room)
        c.width = cfg.width
        c.height = cfg.height
        c.framerate = cfg.framerate
        c.bitrate_kbps = cfg.bitrate_kbps
        c.keyframe_interval = cfg.keyframe_interval
        out = ctypes.c_void_p()
        rc = _push_connect(ctypes.byref(c), ctypes.byref(out))
        if rc != ERR_OK:
            raise FieldError(rc, _last_error())
        return cls(out.value)

    def _require_open(self) -> None:
        if not self._h:
            raise FieldError(ERR_STATE, "session not connected")

    def publish_video(self) -> str:
        """Publish the video track (blocking negotiation). Returns the track id."""
        self._require_open()
        track = ctypes.create_string_buffer(64)
        rc = _push_publish_video(self._h, track, 64)
        if rc != ERR_OK:
            raise FieldError(rc, _last_error())
        return track.value.decode()

    def start_video_frames(self) -> None:
        """Start video frame generation (squares + timestamp watermark)."""
        self._require_open()
        rc = _push_start_video_frames(self._h)
        if rc != ERR_OK:
            raise FieldError(rc, _last_error())

    def stop_video_frames(self) -> None:
        """Stop video frame generation (idempotent; the C ABI is void with no error channel)."""
        if self._h:
            _push_stop_video_frames(self._h)

    def close(self) -> None:
        """Close the session and release the handle (idempotent)."""
        h, self._h = self._h, None
        if h is None:
            return
        rc = _push_close(h)
        if rc != ERR_OK:
            raise FieldError(rc, _last_error())

    def __del__(self):
        try:  # safety-net close; __del__ must never raise (ctypes may already be unloaded during interpreter shutdown)
            self.close()
        except Exception:
            pass
