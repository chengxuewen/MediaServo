"""MediaServo Python bindings — ctypes over libmediaservo_{field,link,deck} (first step of D227).

Lazy submodule loading (PEP 562): when only some SDKs are installed (install bindings --components),
uninstalled SDKs only error on explicit import, without affecting the installed SDKs.
"""

__version__ = "0.1.3"
__all__ = ["field", "link", "deck", "MediaServoError", "__version__"]

from . import _ffi  # noqa: F401  (shared ctypes layer; MediaServoError re-exported at top level)

MediaServoError = _ffi.MediaServoError


def __getattr__(name):
    if name in ("field", "link", "deck"):
        import importlib

        return importlib.import_module("." + name, __name__)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
