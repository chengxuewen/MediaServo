#!/usr/bin/env python3
"""field push skeleton — push vehicle video to the cloud (connect → publish track → generate frames → close).

Run (requires a signaling server + MEDIASERVO_LIB_DIR):
  export MEDIASERVO_LIB_DIR=$PWD/target/debug
  python3 examples/vehicle_push.py ws://127.0.0.1:9800/ws mediaservo-dev room-1 10
"""

import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))  # dev mode

import mediaservo
from mediaservo.field import PushConfig, PushSession


def main() -> int:
    if len(sys.argv) < 4:
        print("usage: vehicle_push.py <ws_url> <psk> <room> [seconds]")
        return 1
    url, psk, room = sys.argv[1], sys.argv[2], sys.argv[3]
    seconds = float(sys.argv[4]) if len(sys.argv) > 4 else 10.0

    print("mediaservo-field %s" % mediaservo.field.version())
    cfg = PushConfig(url=url, psk=psk, room=room,
                     width=1280, height=720, framerate=30,
                     bitrate_kbps=2000, keyframe_interval=2)

    # Connect signaling and create the session (blocking; raises FieldError on failure)
    session = PushSession.connect(cfg)
    try:
        # Publish the video track (blocking negotiation) → track id
        track = session.publish_video()
        print("track: %s" % track)

        # Start video frame generation (squares + timestamp watermark)
        session.start_video_frames()
        print("pushing for %.1fs ..." % seconds)
        time.sleep(seconds)

        session.stop_video_frames()  # idempotent
    finally:
        session.close()  # explicit close (__del__ is the safety net, but prefer explicit close)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
