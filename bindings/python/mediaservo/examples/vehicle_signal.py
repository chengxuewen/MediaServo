#!/usr/bin/env python3
"""link signaling + events — device-side signaling session skeleton (connect → events → send → close).

Run (requires a signaling server + MEDIASERVO_LIB_DIR):
  export MEDIASERVO_LIB_DIR=$PWD/target/debug
  python3 examples/vehicle_signal.py ws://127.0.0.1:9800/ws mediaservo-dev room-1
"""

import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))  # dev mode

import mediaservo
from mediaservo.link import SignalConfig, SignalSession


def on_event(event_json: str) -> None:
    # fires on the pump thread; the event JSON is copied to str and safe to keep
    print("event: %s" % event_json)


def main() -> int:
    if len(sys.argv) < 4:
        print("usage: vehicle_signal.py <ws_url> <psk> <room> [seconds]")
        return 1
    url, psk, room = sys.argv[1], sys.argv[2], sys.argv[3]
    seconds = float(sys.argv[4]) if len(sys.argv) > 4 else 5.0

    print("mediaservo-link %s" % mediaservo.link.version())
    cfg = SignalConfig(url=url, psk=psk, room=room, role="Pusher")  # empty role = Host

    # Connect signaling and create the session (blocking; raises LinkError on failure)
    session = SignalSession.connect(cfg)
    try:
        session.on_event(on_event)  # register the event callback (first registration makes the pump re-synthesize {"type":"connected"})
        time.sleep(0.5)
        session.send('{"type":"ping"}')
        time.sleep(seconds)
    finally:
        session.close()  # C close joins the event pump
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
