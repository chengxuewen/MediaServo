"""MediaServo link Python bindings — signaling + frame bus (mirrors the C++ mediaservo::link class structure).

API: SignalConfig + SignalSession（connect/send/on_event/close）、Bus（attach/
publish/subscribe/close), Stream (recv/close), FrameMeta (9-field dataclass
↔ ctypes struct field-by-field, mediaservo_frame_meta_t _pack_=1 36B).

Threading semantics (C contract): the on_event callback fires on the internal pump thread; the event JSON string is valid only
inside the callback — the Python str is copied and safe to keep. Calling any method on this object from within the callback is forbidden.
"""

import ctypes
import traceback
from dataclasses import dataclass

from . import _ffi
from ._ffi import MediaServoError, cstr

__all__ = [
    "LinkError", "SignalConfig", "SignalSession", "Bus", "Stream", "FrameMeta", "version",
    "ERR_OK", "ERR_INVALID_ARG", "ERR_CONNECT", "ERR_SEND", "ERR_BUS",
    "ERR_STATE", "ERR_INTERNAL", "ERR_CLOSED",
]

# Error codes (link.h)
ERR_OK = 0
ERR_INVALID_ARG = -1
ERR_CONNECT = -2
ERR_SEND = -3
ERR_BUS = -4
ERR_STATE = -5
ERR_INTERNAL = -6
ERR_CLOSED = -7


class LinkError(MediaServoError):
    """link SDK call failed."""


_lib = _ffi.load("link")

# Event callback signature (link.h): void (*)(mediaservo_link_signal_t*, const char*, void*)
_EVENT_CB_TYPE = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_char_p, ctypes.c_void_p)


def _last_error() -> str:
    buf = ctypes.create_string_buffer(512)
    _last_error_fn(buf, 512)
    return buf.value.decode("utf-8", errors="replace")


def _check(rc: int) -> None:
    if rc != ERR_OK:
        raise LinkError(rc, _last_error())


# -- FFI declarations (H3: restype + argtypes fully declared, 64-bit pointer truncation guard) ------

_API = {}  # name -> (fn, restype, argtypes); tests assert full coverage


def _api(name, restype, argtypes):
    fn = getattr(_lib, name)
    fn.restype = restype
    fn.argtypes = argtypes
    _API[name] = (fn, restype, argtypes)
    return fn


_signal_connect = _api(
    "mediaservo_link_signal_connect", ctypes.c_int,
    [ctypes.POINTER(_ffi.mediaservo_link_signal_config_t), ctypes.POINTER(ctypes.c_void_p)],
)
_signal_send = _api(
    "mediaservo_link_signal_send", ctypes.c_int,
    [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t],
)
_signal_on_event = _api(
    "mediaservo_link_signal_on_event", None,
    [ctypes.c_void_p, _EVENT_CB_TYPE, ctypes.c_void_p],
)
_signal_close = _api("mediaservo_link_signal_close", ctypes.c_int, [ctypes.c_void_p])
_bus_attach = _api(
    "mediaservo_link_bus_attach", ctypes.c_int,
    [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)],
)
_bus_publish = _api(
    "mediaservo_link_bus_publish", ctypes.c_int,
    [ctypes.c_void_p, ctypes.c_char_p, ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t,
     ctypes.POINTER(_ffi.mediaservo_frame_meta_t)],
)
_bus_subscribe = _api(
    "mediaservo_link_bus_subscribe", ctypes.c_int,
    [ctypes.c_void_p, ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)],
)
_bus_recv = _api(
    "mediaservo_link_bus_recv", ctypes.c_int,
    [ctypes.c_void_p, ctypes.POINTER(_ffi.mediaservo_frame_meta_t),
     ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)],
)
_stream_close = _api("mediaservo_link_stream_close", ctypes.c_int, [ctypes.c_void_p])
_bus_close = _api("mediaservo_link_bus_close", ctypes.c_int, [ctypes.c_void_p])
_last_error_fn = _api("mediaservo_link_last_error", ctypes.c_int, [ctypes.c_char_p, ctypes.c_size_t])
_version_fn = _api("mediaservo_link_version", ctypes.c_int, [ctypes.c_char_p, ctypes.c_size_t])


def version() -> str:
    """SDK version (MAJOR.MINOR.PATCH), trailing NUL stripped."""
    buf = ctypes.create_string_buffer(64)
    rc = _version_fn(buf, 64)
    if rc != ERR_OK:
        raise LinkError(rc, _last_error())
    return buf.value.decode()


@dataclass
class SignalConfig:
    """Signaling configuration (mirrors mediaservo_link_signal_config_t)."""

    url: str
    psk: str
    room: str
    role: str = ""  # "Host"/"Pusher" → Host, "Client"/"Puller" → Remote; empty = Host


@dataclass
class FrameMeta:
    """Frame metadata (9 fields, 36B wire format D243; format: 0=unknown 1=I420 2=NV12 3=RGBA)."""

    seq: int = 0
    width: int = 0
    height: int = 0
    format: int = 0
    version: int = 0
    is_keyframe: int = 0
    reserved: int = 0
    ts_mono_ns: int = 0
    ts_epoch_ns: int = 0

    def to_c(self) -> _ffi.mediaservo_frame_meta_t:
        """→ ctypes struct (field-by-field assignment; no struct.pack guessing — the wire format is guaranteed by the _pack_=1 struct)."""
        c = _ffi.mediaservo_frame_meta_t()
        c.seq = self.seq
        c.width = self.width
        c.height = self.height
        c.format = self.format
        c.version = self.version
        c.is_keyframe = self.is_keyframe
        c.reserved = self.reserved
        c.ts_mono_ns = self.ts_mono_ns
        c.ts_epoch_ns = self.ts_epoch_ns
        return c

    @classmethod
    def from_c(cls, c: _ffi.mediaservo_frame_meta_t) -> "FrameMeta":
        return cls(
            seq=c.seq, width=c.width, height=c.height, format=c.format,
            version=c.version, is_keyframe=c.is_keyframe, reserved=c.reserved,
            ts_mono_ns=c.ts_mono_ns, ts_epoch_ns=c.ts_epoch_ns,
        )


class SignalSession:
    """Signaling session (mirrors C++ mediaservo::link::SignalSession). Default construction = closed."""

    def __init__(self, h=None):
        self._h = h
        self._cb_ref = None  # CFUNCTYPE reference (H3: prevents GC; released after C close joins the pump thread)

    @classmethod
    def connect(cls, cfg: SignalConfig) -> "SignalSession":
        """Connect signaling and create the session (blocking). Raises LinkError on failure; never returns a half-open session."""
        c = _ffi.mediaservo_link_signal_config_t()
        c.struct_size = ctypes.sizeof(_ffi.mediaservo_link_signal_config_t)  # R3: auto-fill
        c.url = cstr(cfg.url)
        c.psk = cstr(cfg.psk)
        c.room = cstr(cfg.room)
        c.role = cstr(cfg.role)  # empty → NULL = Host
        out = ctypes.c_void_p()
        rc = _signal_connect(ctypes.byref(c), ctypes.byref(out))
        if rc != ERR_OK:
            raise LinkError(rc, _last_error())
        return cls(out.value)

    def _require_open(self) -> None:
        if not self._h:
            raise LinkError(ERR_STATE, "session not connected")

    def send(self, json_str: str) -> None:
        """Send one signaling message (JSON; SignalingMessage type tag is snake_case)."""
        self._require_open()
        payload = json_str.encode("utf-8")
        rc = _signal_send(self._h, payload, len(payload))
        if rc != ERR_OK:
            raise LinkError(rc, _last_error())

    def on_event(self, callback) -> None:
        """Register the event callback (any time after connect; re-registration replaces; fires on the pump thread).

        callback(event_json: str) -> None; calling any method on this object from within the callback is forbidden.
        Closed session: no-op (C++ parity).
        """
        if not self._h:
            return

        def _trampoline(_h, event_json, _user):
            try:
                callback(event_json.decode("utf-8", errors="replace") if event_json else "")
            except Exception:
                traceback.print_exc()  # pump-thread exceptions must never cross the C boundary (UB)

        self._cb_ref = _EVENT_CB_TYPE(_trampoline)  # H3: keep the reference to prevent GC
        _signal_on_event(self._h, self._cb_ref, None)

    def close(self) -> None:
        """Close the session and release the handle (idempotent; the C close joins the event pump before the callback reference is released)."""
        h, self._h = self._h, None
        if h is None:
            return
        try:
            rc = _signal_close(h)
        finally:
            self._cb_ref = None
        if rc != ERR_OK:
            raise LinkError(rc, _last_error())

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass


class Bus:
    """Frame bus (mirrors C++ mediaservo::link::Bus). Default construction = closed."""

    def __init__(self, h=None):
        self._h = h

    @classmethod
    def attach(cls, endpoint: str, token_pem: str, vk_pem: str) -> "Bus":
        """Attach the frame bus (signature verification + ACL + iceoryx2 node, blocking). endpoint is reserved for Phase 1 (empty string is fine)."""
        out = ctypes.c_void_p()
        rc = _bus_attach(cstr(endpoint), cstr(token_pem), cstr(vk_pem), ctypes.byref(out))
        if rc != ERR_OK:
            raise LinkError(rc, _last_error())
        return cls(out.value)

    def _require_open(self) -> None:
        if not self._h:
            raise LinkError(ERR_STATE, "bus not attached")

    def publish(self, topic: str, payload: bytes, meta: FrameMeta) -> None:
        """Publish one frame (ACL check + SHM loan + send, blocking). Empty payload = metadata-only frame."""
        self._require_open()
        # C contract: payload NULL iff len == 0
        buf = ctypes.create_string_buffer(payload) if payload else None
        cmeta = meta.to_c()
        rc = _bus_publish(self._h, cstr(topic), buf, len(payload), ctypes.byref(cmeta))
        if rc != ERR_OK:
            raise LinkError(rc, _last_error())

    def subscribe(self, topic: str) -> "Stream":
        """Subscribe to a topic and create a frame stream (blocking)."""
        self._require_open()
        out = ctypes.c_void_p()
        rc = _bus_subscribe(self._h, cstr(topic), ctypes.byref(out))
        if rc != ERR_OK:
            raise LinkError(rc, _last_error())
        return Stream(out.value)

    def close(self) -> None:
        """Close the frame bus and release the handle (idempotent; shuts down all streams, stream recv returns CLOSED)."""
        h, self._h = self._h, None
        if h is None:
            return
        rc = _bus_close(h)
        if rc != ERR_OK:
            raise LinkError(rc, _last_error())

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass


class Stream:
    """Frame stream (mirrors C++ mediaservo::link::Stream). Default construction = closed."""

    # ponytail: single 16MiB buffer covers 4K I420 (12.4MiB); the C ABI cannot detect truncation,
    # larger frames need a bigger buffer — upgrade path: grow adaptively using the previous frame size from meta (same as C++).
    _CAP = 16 * 1024 * 1024

    def __init__(self, h=None):
        self._h = h
        self._buf = ctypes.create_string_buffer(self._CAP)

    def _require_open(self) -> None:
        if not self._h:
            raise LinkError(ERR_STATE, "stream not subscribed")

    def recv(self):
        """Blocking frame fetch → (FrameMeta, bytes). Shutdown (stream/bus close) raises LinkError CLOSED."""
        self._require_open()
        meta = _ffi.mediaservo_frame_meta_t()
        out_len = ctypes.c_size_t()
        rc = _bus_recv(self._h, ctypes.byref(meta), self._buf, self._CAP, ctypes.byref(out_len))
        if rc != ERR_OK:
            if rc == ERR_CLOSED:
                raise LinkError(rc, "stream closed")
            raise LinkError(rc, _last_error())
        return FrameMeta.from_c(meta), bytes(self._buf[: out_len.value])

    def close(self) -> None:
        """Close the frame stream and release the handle (idempotent; wakes a blocked recv so it returns CLOSED)."""
        h, self._h = self._h, None
        if h is None:
            return
        rc = _stream_close(h)
        if rc != ERR_OK:
            raise LinkError(rc, _last_error())

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass
