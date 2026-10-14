#!/usr/bin/env python3
"""deck closed loop — capture → record → playback (enumerate → CameraSource → Recorder → Player).

Run (the deck library needs FFmpeg built + MEDIASERVO_LIB_DIR):
  export MEDIASERVO_LIB_DIR=$PWD/target/debug
  python3 examples/record_playback.py
"""

import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))  # dev mode

import mediaservo
from mediaservo.deck import (
    CameraSource, CaptureOptions, DeviceKind, Player, Recorder, enumerate_devices,
)


def main() -> int:
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "recorded.mp4")
    print("mediaservo-deck %s" % mediaservo.deck.version())

        # 1. Enumerate devices (two-call wrapper) → stub:test-camera
    devs = enumerate_devices(DeviceKind.Camera)
    if not devs:
        print("no camera devices")
        return 1
    print("devices: %s" % devs)

        # 2. Capture 320x240@30 (stub color bars)
    frames = []

    def on_frame(f) -> None:
                frames.append(f)  # Frame.data is copied to bytes and safe to keep

    cam = CameraSource.open(devs[0], CaptureOptions(320, 240, 30))
    cam.on_frame(on_frame)
    cam.start()

        # 3. Record for 2 seconds (close order: recorder stop/close precedes camera stop/close)
    rec = Recorder.open(out)
    rec.record(cam)
    time.sleep(2.0)
    rec.stop()
    rec.close()
    cam.stop()
    cam.close()
    print("recorded %d frames -> %s (%d bytes)" % (len(frames), out, os.path.getsize(out)))

        # 4. Playback (the on_frame decode pump runs until EOF; close blocks and joins)
    played = [0]

    def on_playback(f) -> None:
        played[0] += 1

    player = Player.open(out)
    player.on_frame(on_playback)
    player.close()
    print("played back %d frames" % played[0])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
