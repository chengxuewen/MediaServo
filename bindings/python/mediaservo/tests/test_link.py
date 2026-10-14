"""link SDK binding tests (unittest, zero dependencies).

Run (from the workspace root):
  export PATH="$HOME/.pixi/bin:$PATH"
  export MEDIASERVO_LIB_DIR=$(pwd)/target/debug
  python3 -m unittest discover -s bindings/python/mediaservo/tests

Note: does not touch iceoryx2 SHM (no positive attach case); the FrameMeta 36B round-trip is pure-Python-level verification.
"""

import ctypes
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from mediaservo import _ffi, link  # noqa: E402


class LinkVersionTest(unittest.TestCase):
    def test_version(self):
        self.assertTrue(link.version().startswith("0.1."))


class LinkErrorPathTest(unittest.TestCase):
    def test_connect_empty_required_invalid_arg(self):
        with self.assertRaises(link.LinkError) as cm:
            link.SignalSession.connect(link.SignalConfig(url="", psk="", room=""))
        self.assertEqual(cm.exception.code, link.ERR_INVALID_ARG)

    def test_unconnected_call_state(self):
        s = link.SignalSession()
        with self.assertRaises(link.LinkError) as cm:
            s.send('{"type":"ping"}')
        self.assertEqual(cm.exception.code, link.ERR_STATE)
        b = link.Bus()
        with self.assertRaises(link.LinkError) as cm:
            b.publish("camera/0", b"", link.FrameMeta())
        self.assertEqual(cm.exception.code, link.ERR_STATE)
        st = link.Stream()
        with self.assertRaises(link.LinkError) as cm:
            st.recv()
        self.assertEqual(cm.exception.code, link.ERR_STATE)

    def test_struct_size_invalid_arg(self):
                # R3: struct_size first field must be >= sizeof(known struct); setting 1 manually → INVALID_ARG
        cfg = _ffi.mediaservo_link_signal_config_t()
        cfg.struct_size = 1
        out = ctypes.c_void_p()
        rc = link._signal_connect(ctypes.byref(cfg), ctypes.byref(out))
        self.assertEqual(rc, link.ERR_INVALID_ARG)

    def test_close_idempotent(self):
        s = link.SignalSession()
        s.close()
        s.close()
        b = link.Bus()
        b.close()
        st = link.Stream()
        st.close()


class LinkFrameMetaTest(unittest.TestCase):
    def test_frame_meta_36_bytes(self):
                # R4: _pack_=1 → 36B wire format; natural alignment is 40B and corrupts frames
        self.assertEqual(ctypes.sizeof(_ffi.mediaservo_frame_meta_t), 36)

    def test_frame_meta_roundtrip(self):
                # construct → to_c → from_c, all 9 fields identical
        meta = link.FrameMeta(
            seq=0x0102030405060708, width=1280, height=720, format=1,
            version=0, is_keyframe=1, reserved=0,
            ts_mono_ns=1234567890123, ts_epoch_ns=9876543210987,
        )
        back = link.FrameMeta.from_c(meta.to_c())
        self.assertEqual(meta, back)


class LinkFfiSafetyTest(unittest.TestCase):
    def test_all_functions_declared_restype_argtypes(self):
                # H3: every exported function must declare restype + argtypes (64-bit pointer truncation guard)
        self.assertGreaterEqual(len(link._API), 11)
        for name, (fn, restype, argtypes) in link._API.items():
            self.assertEqual(fn.restype, restype, name)
            self.assertEqual(fn.argtypes, argtypes, name)
            self.assertIsNotNone(argtypes, name + " missing argtypes")


if __name__ == "__main__":
    unittest.main()
