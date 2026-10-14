"""MediaServo ctypes shared loading layer — shared by the field/link/deck SDKs.

Load order (audit H3):
  1. Directory pointed to by the MEDIASERVO_LIB_DIR env var (dev mode: target/debug)
  2. In-package mediaservo/_libs/ (bundled with the wheel for packaged distribution)
  3. ctypes.util.find_library("mediaservo_<sdk>") (system install paths)

FFI safety (H3):
  - every exported function must declare restype + argtypes (64-bit pointer truncation guard);
    each SDK module registers into the _API table via _api(); tests assert full coverage.
  - CFUNCTYPE callback objects must be kept referenced by the handle-owning Python object (GC protection),
    released by each module only after the close() C call (join pump threads).
  - C structs are aligned field-by-field with the C headers (bindings/c/include/mediaservo/*.h);
    mediaservo_frame_meta_t must be _pack_ = 1 (36B wire format D243; natural alignment gives 40B and corrupts frames).
"""

import ctypes
import ctypes.util
import os
import sys

__all__ = [
    "MediaServoError",
    "load",
    "cstr",
    "mediaservo_frame_meta_t",
    "mediaservo_frame_t",
    "mediaservo_push_config_t",
    "mediaservo_link_signal_config_t",
    "mediaservo_deck_capture_options_t",
]


class MediaServoError(Exception):
    """SDK call failed (code is the corresponding header-declared MEDIASERVO_<SDK>_ERR_* value, message read from last_error)."""

    def __init__(self, code: int, message: str):
        super().__init__("%s (code %d)" % (message, code))
        self.code = code
        self.message = message


# -- Shared C types (common.h) ------------------------------------------------

# Fixed 36B wire format (D243): seq(8)+width(4)+height(4)+format(1)+version(1)
# +is_keyframe(1)+reserved(1)+ts_mono_ns(8)+ts_epoch_ns(8)
class mediaservo_frame_meta_t(ctypes.Structure):
    _pack_ = 1
    _fields_ = [
        ("seq", ctypes.c_uint64),
        ("width", ctypes.c_uint32),
        ("height", ctypes.c_uint32),
        ("format", ctypes.c_uint8),        # 0=unknown, 1=I420, 2=NV12, 3=RGBA
        ("version", ctypes.c_uint8),
        ("is_keyframe", ctypes.c_uint8),
        ("reserved", ctypes.c_uint8),      # must be 0
        ("ts_mono_ns", ctypes.c_uint64),
        ("ts_epoch_ns", ctypes.c_uint64),
    ]


# In-memory frame description (deck capture/playback callbacks, I420 three planes; data_* pointers valid only inside the callback)
class mediaservo_frame_t(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_uint32),
        ("height", ctypes.c_uint32),
        ("pts_us", ctypes.c_uint64),
        ("stride_y", ctypes.c_uint32),
        ("stride_u", ctypes.c_uint32),
        ("stride_v", ctypes.c_uint32),
        ("data_y", ctypes.POINTER(ctypes.c_uint8)),
        ("data_u", ctypes.POINTER(ctypes.c_uint8)),
        ("data_v", ctypes.POINTER(ctypes.c_uint8)),
    ]


# field.h — push config (struct_size first field per R3; callers must fill sizeof)
class mediaservo_push_config_t(ctypes.Structure):
    _fields_ = [
        ("struct_size", ctypes.c_size_t),
        ("url", ctypes.c_char_p),
        ("psk", ctypes.c_char_p),
        ("room", ctypes.c_char_p),
        ("width", ctypes.c_uint32),        # default 1280
        ("height", ctypes.c_uint32),       # default 720
        ("framerate", ctypes.c_uint32),    # default 30
        ("bitrate_kbps", ctypes.c_uint32), # default 2000
        ("keyframe_interval", ctypes.c_uint64),  # default 2
    ]


# link.h — signaling config
class mediaservo_link_signal_config_t(ctypes.Structure):
    _fields_ = [
        ("struct_size", ctypes.c_size_t),
        ("url", ctypes.c_char_p),
        ("psk", ctypes.c_char_p),
        ("room", ctypes.c_char_p),
        ("role", ctypes.c_char_p),  # "Host"/"Pusher"→Host, "Client"/"Puller"→Remote; NULL=Host
    ]


# deck.h — capture options (all-zero fields = C default 1280x720@30)
class mediaservo_deck_capture_options_t(ctypes.Structure):
    _fields_ = [
        ("struct_size", ctypes.c_size_t),
        ("width", ctypes.c_uint32),
        ("height", ctypes.c_uint32),
        ("framerate", ctypes.c_uint32),
    ]


# -- Loading ---------------------------------------------------------------------

_LIBS = {}  # sdk -> CDLL cache


def _lib_filename(sdk: str) -> str:
    if sys.platform == "darwin":
        return "libmediaservo_%s.dylib" % sdk
    if os.name == "nt":
        return "mediaservo_%s.dll" % sdk
    return "libmediaservo_%s.so" % sdk


def _find_library(sdk: str):
    """Locate libmediaservo_<sdk> in H3 order; returns the path or None."""
    name = _lib_filename(sdk)
    env_dir = os.environ.get("MEDIASERVO_LIB_DIR")
    if env_dir:
        cand = os.path.join(env_dir, name)
        if os.path.exists(cand):
            return cand
        cand_ver = os.path.join(env_dir, name + ".0")
        if os.path.exists(cand_ver):
            return cand_ver
    pkg_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "_libs")
    cand = os.path.join(pkg_dir, name)
    if os.path.exists(cand):
        return cand
    found = ctypes.util.find_library("mediaservo_%s" % sdk)
    if found:
        return found
    return None


def load(sdk: str):
    """Load and cache libmediaservo_<sdk> (field/link/deck). Raises ImportError with guidance when not found."""
    if sdk not in _LIBS:
        path = _find_library(sdk)
        if path is None:
            raise ImportError(
                "cannot load libmediaservo_%s: set MEDIASERVO_LIB_DIR to the build output "
                "dir (e.g. export MEDIASERVO_LIB_DIR=$PWD/target/debug) or add the dir "
                "containing it to LD_LIBRARY_PATH" % sdk
            )
        _LIBS[sdk] = ctypes.CDLL(path)
    return _LIBS[sdk]


def cstr(s) -> bytes:
    """str → bytes; empty string → None (C contract: optional/required empty strings are passed as NULL)."""
    return s.encode("utf-8") if s else None
