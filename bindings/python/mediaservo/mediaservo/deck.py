"""MediaServo deck Python bindings — capture/record/playback (mirrors the C++ mediaservo::deck class structure).

API: DeviceKind(Enum) + enumerate_devices、CaptureOptions + CameraSource（open/
start/on_frame/stop/close）、Recorder（open/record/stop/close）、Player（open/
on_frame/close). Frame callbacks receive a Frame dataclass (data is copied to bytes — C pointers are
valid only inside the callback).

Threading semantics (C contract): frame callbacks fire on the pump thread; calling any deck API from within a callback is forbidden.
Close order (recording scenario): recorder stop/close precedes camera stop/close.
"""

import ctypes
import enum
import traceback
from dataclasses import dataclass

from . import _ffi
from ._ffi import MediaServoError, cstr

__all__ = [
    "DeckError", "DeviceKind", "CaptureOptions", "Frame",
    "CameraSource", "Recorder", "Player", "enumerate_devices", "version",
    "ERR_OK", "ERR_INVALID_ARG", "ERR_DEVICE", "ERR_RECORDER",
    "ERR_PLAYER", "ERR_STATE", "ERR_INTERNAL",
]

# Error codes (deck.h)
ERR_OK = 0
ERR_INVALID_ARG = -1
ERR_DEVICE = -2
ERR_RECORDER = -3
ERR_PLAYER = -4
ERR_STATE = -5
ERR_INTERNAL = -6


class DeckError(MediaServoError):
    """deck SDK call failed."""


_lib = _ffi.load("deck")

# Frame callback signature (deck.h): void (*)(const mediaservo_frame_t*, void*)
_FRAME_CB_TYPE = ctypes.CFUNCTYPE(None, ctypes.POINTER(_ffi.mediaservo_frame_t), ctypes.c_void_p)


def _last_error() -> str:
    buf = ctypes.create_string_buffer(512)
    _last_error_fn(buf, 512)
    return buf.value.decode("utf-8", errors="replace")


def _check(rc: int) -> None:
    if rc != ERR_OK:
        raise DeckError(rc, _last_error())


# -- FFI declarations (H3: restype + argtypes fully declared, 64-bit pointer truncation guard) ------

_API = {}  # name -> (fn, restype, argtypes); tests assert full coverage


def _api(name, restype, argtypes):
    fn = getattr(_lib, name)
    fn.restype = restype
    fn.argtypes = argtypes
    _API[name] = (fn, restype, argtypes)
    return fn


_devices_enumerate = _api(
    "mediaservo_deck_devices_enumerate", ctypes.c_int,
    [ctypes.c_int, ctypes.c_char_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)],
)
_camera_open = _api(
    "mediaservo_deck_camera_open", ctypes.c_int,
    [ctypes.c_char_p, ctypes.POINTER(_ffi.mediaservo_deck_capture_options_t),
     ctypes.POINTER(ctypes.c_void_p)],
)
_camera_start = _api("mediaservo_deck_camera_start", ctypes.c_int, [ctypes.c_void_p])
_camera_frames_cb = _api(
    "mediaservo_deck_camera_frames_cb", ctypes.c_int,
    [ctypes.c_void_p, _FRAME_CB_TYPE, ctypes.c_void_p],
)
_camera_stop = _api("mediaservo_deck_camera_stop", ctypes.c_int, [ctypes.c_void_p])
_camera_close = _api("mediaservo_deck_camera_close", ctypes.c_int, [ctypes.c_void_p])
_recorder_new = _api(
    "mediaservo_deck_recorder_new", ctypes.c_int,
    [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)],
)
_recorder_record = _api("mediaservo_deck_recorder_record", ctypes.c_int, [ctypes.c_void_p, ctypes.c_void_p])
_recorder_stop = _api("mediaservo_deck_recorder_stop", ctypes.c_int, [ctypes.c_void_p])
_recorder_close = _api("mediaservo_deck_recorder_close", ctypes.c_int, [ctypes.c_void_p])
_player_open = _api(
    "mediaservo_deck_player_open", ctypes.c_int,
    [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)],
)
_player_frames_cb = _api(
    "mediaservo_deck_player_frames_cb", ctypes.c_int,
    [ctypes.c_void_p, _FRAME_CB_TYPE, ctypes.c_void_p],
)
_player_close = _api("mediaservo_deck_player_close", ctypes.c_int, [ctypes.c_void_p])
_last_error_fn = _api("mediaservo_deck_last_error", ctypes.c_int, [ctypes.c_char_p, ctypes.c_size_t])
_version_fn = _api("mediaservo_deck_version", ctypes.c_int, [ctypes.c_char_p, ctypes.c_size_t])


def version() -> str:
    """SDK version (MAJOR.MINOR.PATCH), trailing NUL stripped."""
    buf = ctypes.create_string_buffer(64)
    rc = _version_fn(buf, 64)
    if rc != ERR_OK:
        raise DeckError(rc, _last_error())
    return buf.value.decode()


class DeviceKind(enum.Enum):
    """Device kind (matches the kind parameter of enumerate())."""

    Camera = 0
    Audio = 1
    Screen = 2


@dataclass
class CaptureOptions:
    """Capture options (mirrors mediaservo_deck_capture_options_t)."""

    width: int = 1280
    height: int = 720
    framerate: int = 30


@dataclass
class Frame:
    """One frame (I420 three planes concatenated: data = y + u + v, each plane row width = stride_*).

    data is copied to bytes — the C data_* pointers are valid only inside the callback.
    Plane slicing: y = data[:stride_y*height]; u = data[stride_y*height:...].
    """

    width: int
    height: int
    pts_us: int
    stride_y: int
    stride_u: int
    stride_v: int
    data: bytes


def enumerate_devices(kind: DeviceKind) -> list:
    """Enumerate device ids (two-call pattern; multiple devices separated by '\n'; failure = empty list, C++ parity)."""
    first = ctypes.c_size_t()
    rc = _devices_enumerate(int(kind.value), None, 0, ctypes.byref(first))
    if rc < 0 or first.value == 0:  # positive rc = required length (snprintf convention)
        return []
    buf = ctypes.create_string_buffer(first.value + 1)  # C writes cap-1 chars + NUL
    out_len = ctypes.c_size_t()
    rc = _devices_enumerate(int(kind.value), buf, first.value + 1, ctypes.byref(out_len))
    if rc < 0:
        return []
    ids = buf.value.decode("utf-8", errors="replace")
    return [d for d in ids.split("\n") if d] if ids else []


def _make_frame_trampoline(cb):
    """Python frame callback → CFUNCTYPE (H3: the returned object must be kept alive by the handle-owning Python object).

    Copy the I420 three planes to bytes (pointers are valid only inside the callback); pump-thread exceptions must never cross the C boundary (UB).
    """

    def _trampoline(frame_ptr, _user):
        try:
            f = frame_ptr.contents
            n_y = f.stride_y * f.height
            n_uv = f.stride_u * (f.height // 2)
            n_v = f.stride_v * (f.height // 2)

            def _copy(p, n):
                return ctypes.string_at(p, n) if p else b""

            cb(Frame(
                width=f.width, height=f.height, pts_us=f.pts_us,
                stride_y=f.stride_y, stride_u=f.stride_u, stride_v=f.stride_v,
                data=_copy(f.data_y, n_y) + _copy(f.data_u, n_uv) + _copy(f.data_v, n_v),
            ))
        except Exception:
            traceback.print_exc()

    return _FRAME_CB_TYPE(_trampoline)


class CameraSource:
    """Camera capture (mirrors C++ mediaservo::deck::CameraSource). Default construction = closed."""

    def __init__(self, h=None):
        self._h = h
        self._cb_ref = None  # CFUNCTYPE reference (H3: prevents GC; released after C close joins the frame pump)

    @classmethod
    def open(cls, dev_id: str, opts: CaptureOptions = None) -> "CameraSource":
        """Open the camera (local initialization only; dev_id must exist in the enumeration result)."""
        opts = opts or CaptureOptions()
        c = _ffi.mediaservo_deck_capture_options_t()
        c.struct_size = ctypes.sizeof(_ffi.mediaservo_deck_capture_options_t)  # R3: auto-fill
        c.width = opts.width
        c.height = opts.height
        c.framerate = opts.framerate
        out = ctypes.c_void_p()
        rc = _camera_open(cstr(dev_id), ctypes.byref(c), ctypes.byref(out))
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())
        return cls(out.value)

    def _require_open(self) -> None:
        if not self._h:
            raise DeckError(ERR_STATE, "camera not open")

    def start(self) -> None:
        """Start producing frames (uses the opts from open; only allowed once, repeat calls → STATE)."""
        self._require_open()
        rc = _camera_start(self._h)
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())

    def on_frame(self, callback) -> None:
        """Register the frame callback (fires per frame on the pump thread; re-registration replaces; the callback receives a Frame).

        Calling any deck API from within the callback is forbidden; frame data is copied and safe to keep.
        """
        self._require_open()
        self._cb_ref = _make_frame_trampoline(callback)  # H3: keep the reference to prevent GC
        rc = _camera_frames_cb(self._h, self._cb_ref, None)
        if rc != ERR_OK:
            self._cb_ref = None
            raise DeckError(rc, _last_error())

    def stop(self) -> None:
        """Stop producing frames (idempotent)."""
        self._require_open()
        rc = _camera_stop(self._h)
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())

    def close(self) -> None:
        """Close the camera and release the handle (idempotent; the C close joins the frame pump before the callback reference is released)."""
        h, self._h = self._h, None
        if h is None:
            return
        try:
            rc = _camera_close(h)
        finally:
            self._cb_ref = None
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass


class Recorder:
    """Recorder (mirrors C++ mediaservo::deck::Recorder). Default construction = closed."""

    def __init__(self, h=None):
        self._h = h

    @classmethod
    def open(cls, path: str) -> "Recorder":
        """Create the recorder (default h264/mp4; the parent directory must already exist)."""
        out = ctypes.c_void_p()
        rc = _recorder_new(cstr(path), ctypes.byref(out))
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())
        return cls(out.value)

    def _require_open(self) -> None:
        if not self._h:
            raise DeckError(ERR_STATE, "recorder not open")

    def record(self, camera: CameraSource) -> None:
        """Bridge recording: camera frame pump → recorder. The camera must be started and outlive the recording."""
        self._require_open()
        if not camera._h:
            raise DeckError(ERR_INVALID_ARG, "camera closed")
        rc = _recorder_record(self._h, camera._h)
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())

    def stop(self) -> None:
        """Request stop recording (idempotent; flush + trailer finalize on close)."""
        self._require_open()
        rc = _recorder_stop(self._h)
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())

    def close(self) -> None:
        """Close the recorder and release the handle (idempotent; joins the recording task to complete the flush)."""
        h, self._h = self._h, None
        if h is None:
            return
        rc = _recorder_close(h)
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass


class Player:
    """Player (mirrors C++ mediaservo::deck::Player). Default construction = closed."""

    def __init__(self, h=None):
        self._h = h
        self._cb_ref = None

    @classmethod
    def open(cls, path: str) -> "Player":
        """Open a media file (demux + decoder ready)."""
        out = ctypes.c_void_p()
        rc = _player_open(cstr(path), ctypes.byref(out))
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())
        return cls(out.value)

    def _require_open(self) -> None:
        if not self._h:
            raise DeckError(ERR_STATE, "player not open")

    def on_frame(self, callback) -> None:
        """Per-frame decode callback pump (runs until EOF ends naturally; only allowed once).

        close is a blocking join (waits for decoding to finish) — long files require waiting; no mid-flight abort (YAGNI).
        """
        self._require_open()
        self._cb_ref = _make_frame_trampoline(callback)  # H3: keep the reference to prevent GC
        rc = _player_frames_cb(self._h, self._cb_ref, None)
        if rc != ERR_OK:
            self._cb_ref = None
            raise DeckError(rc, _last_error())

    def close(self) -> None:
        """Close the player and release the handle (idempotent; joins the decode pump before the callback reference is released)."""
        h, self._h = self._h, None
        if h is None:
            return
        try:
            rc = _player_close(h)
        finally:
            self._cb_ref = None
        if rc != ERR_OK:
            raise DeckError(rc, _last_error())

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass
