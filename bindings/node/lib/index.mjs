// MediaServo Node.js bindings — thin TS layer (livekit-rtc style wrapper over the napi .node binary).
// Usage: import { PushSession, SignalSession, CameraSource } from 'mediaservo';
// Loading: mediaservo.node from the same directory (build: cargo build -p mediaservo-node + cp .so → .node)

import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const binding = require('../mediaservo.node');

/**
 * Push stream configuration.
 * @typedef {Object} PushConfig
 * @property {string} url - Signaling WS URL (e.g. ws://host:9800/ws)
 * @property {string} psk - PSK authentication key
 * @property {string} room - Room ID
 * @property {number} [width] - Video width (default 1280)
 * @property {number} [height] - Video height (default 720)
 * @property {number} [framerate] - Framerate (default 30)
 * @property {number} [bitrateKbps] - Bitrate in kbps (default 2000)
 */
export class PushSession {
  /** @private */
  constructor(h) { this._h = h; }

  /** Connect signaling and create the session (async). */
  static async connect(cfg) {
    return new PushSession(await binding.JsPushSession.connect(cfg));
  }
  /** Publish the video track (SFU negotiation). @returns {Promise<string>} track id */
  publishVideo() { return this._h.publishVideo(); }
  startVideoFrames() { return this._h.startVideoFrames(); }
  stopVideoFrames() { return this._h.stopVideoFrames(); }
  close() { return this._h.close(); }
}

/**
 * Signaling configuration. role: "Host"|"Pusher"|"Client"|"Puller" (default Host).
 * @typedef {Object} SignalConfig
 * @property {string} url
 * @property {string} psk
 * @property {string} room
 * @property {string} [role]
 */
export class SignalSession {
  /** @private */
  constructor(h) { this._h = h; }

  static async connect(cfg) {
    return new SignalSession(await binding.JsSignalSession.connect(cfg));
  }
  /** Send a signaling message (SignalingMessage JSON). */
  send(json) { return this._h.send(json); }
  /** Subscribe to events (JSON string callback; event object {type, ...}). */
  onEvent(cb) { return this._h.onEvent((ev) => cb(JSON.parse(ev))); }
  close() { return this._h.close(); }
}

/**
 * Capture options.
 * @typedef {Object} CaptureOptions
 * @property {number} [width]
 * @property {number} [height]
 * @property {number} [framerate]
 */
export class CameraSource {
  /** @private */
  constructor(h) { this._h = h; }

  /** Open the camera (currently the stub device "stub:test-camera"). */
  static async open(devId, opts = {}) {
    return new CameraSource(await binding.JsCameraSource.open(devId, opts));
  }
  start() { return this._h.start(); }
  /**
   * Subscribe to frame callbacks. napi tuple → JS array [meta, data] → destructured into a frame object.
   * @param {(frame: {width:number,height:number,ptsUs:number,keyframe:boolean,data:Uint8Array}) => void} cb
   */
  onFrame(cb) {
    return this._h.onFrame(([meta, data]) => cb({ ...JSON.parse(meta), data }));
  }
  stop() { return this._h.stop(); }
  close() { return this._h.close(); }
}

/**
 * Recorder (FFmpeg mux; record(camera) runs as a background task, stop() triggers flush+trailer).
 */
export class Recorder {
  /** @private */
  constructor(h) { this._h = h; }
  static async open(path) { return new Recorder(await binding.JsRecorder.open(path)); }
  /** Start recording (returns immediately; the camera must already be started). */
  record(camera) { return this._h.record(camera._h); }
  stop() { return this._h.stop(); }
  close() { return this._h.close(); }
}

/**
 * Player (demux+decode; onFrame is called per frame on the pump thread).
 */
export class Player {
  /** @private */
  constructor(h) { this._h = h; }
  static async open(path) { return new Player(await binding.JsPlayer.open(path)); }
  onFrame(cb) {
    return this._h.onFrame(([meta, data]) => cb({ ...JSON.parse(meta), data }));
  }
  durationSecs() { return this._h.durationSecs(); }
  close() { return this._h.close(); }
}

export default { PushSession, SignalSession, CameraSource, Recorder, Player };
