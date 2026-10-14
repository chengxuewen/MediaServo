"""field SDK binding tests (unittest, zero dependencies).

Run (from the workspace root):
  export PATH="$HOME/.pixi/bin:$PATH"
  export MEDIASERVO_LIB_DIR=$(pwd)/target/debug
  python3 -m unittest discover -s bindings/python/mediaservo/tests
"""

import ctypes
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from mediaservo import _ffi, field  # noqa: E402


class FieldVersionTest(unittest.TestCase):
    def test_version(self):
        self.assertTrue(field.version().startswith("0.1."))


class FieldErrorPathTest(unittest.TestCase):
    def test_connect_empty_required_invalid_arg(self):
                # url/psk/room are required → C validation → INVALID_ARG
        with self.assertRaises(field.FieldError) as cm:
            field.PushSession.connect(field.PushConfig(url="", psk="", room=""))
        self.assertEqual(cm.exception.code, field.ERR_INVALID_ARG)
        self.assertIsInstance(cm.exception.message, str)

    def test_unconnected_call_state(self):
                # Calling a default-constructed (unconnected) session → STATE
        s = field.PushSession()
        with self.assertRaises(field.FieldError) as cm:
            s.publish_video()
        self.assertEqual(cm.exception.code, field.ERR_STATE)
        with self.assertRaises(field.FieldError) as cm:
            s.start_video_frames()
        self.assertEqual(cm.exception.code, field.ERR_STATE)

    def test_struct_size_invalid_arg(self):
                # R3: struct_size first field must be >= sizeof(known struct); setting 1 manually → INVALID_ARG
        cfg = _ffi.mediaservo_push_config_t()
        cfg.struct_size = 1
        out = ctypes.c_void_p()
        rc = field._push_connect(ctypes.byref(cfg), ctypes.byref(out))
        self.assertEqual(rc, field.ERR_INVALID_ARG)

    def test_close_idempotent_and_del_safe(self):
        s = field.PushSession()
        s.close()  # closing an already-closed session → OK, no raise
        s.close()
        del s  # __del__ safety net does not raise


class FieldFfiSafetyTest(unittest.TestCase):
    def test_all_functions_declared_restype_argtypes(self):
                # H3: every exported function must declare restype + argtypes (64-bit pointer truncation guard)
        self.assertGreaterEqual(len(field._API), 7)
        for name, (fn, restype, argtypes) in field._API.items():
            self.assertEqual(fn.restype, restype, name)
            self.assertEqual(fn.argtypes, argtypes, name)
            self.assertIsNotNone(argtypes, name + " missing argtypes")


if __name__ == "__main__":
    unittest.main()
