"""MediaServo Python 绑定 — ctypes over libmediaservo_{field,link,deck}（D227 首步）。

子模块按需加载（PEP 562）: 仅安装部分 SDK（install bindings --components）时，
未安装的 SDK 在显式 import 时才报错，不影响已装 SDK 使用。
"""

__version__ = "0.1.1"
__all__ = ["field", "link", "deck", "MediaServoError", "__version__"]

from . import _ffi  # noqa: F401  (共享 ctypes 层；MediaServoError 供顶层 re-export)

MediaServoError = _ffi.MediaServoError


def __getattr__(name):
    if name in ("field", "link", "deck"):
        import importlib

        return importlib.import_module("." + name, __name__)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
