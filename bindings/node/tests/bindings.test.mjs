// node:test — error paths + local frame verification (no server needed; for a real server see examples/).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PushSession, SignalSession, CameraSource, Recorder, Player } from '../lib/index.mjs';

test('PushSession.connect rejects on empty config', async () => {
  await assert.rejects(PushSession.connect({ url: '', psk: '', room: '' }));
});

test('SignalSession.connect rejects on unknown role', async () => {
  await assert.rejects(SignalSession.connect({ url: 'ws://x', psk: 'p', room: 'r', role: 'bogus' }));
});

test('CameraSource frames arrive via onFrame (I420)', async () => {
  const cam = await CameraSource.open('stub:test-camera', { width: 320, height: 180, framerate: 10 });
  await cam.start();
  let frame = null;
  await cam.onFrame((f) => { if (!frame) frame = f; });
  await new Promise((r) => setTimeout(r, 800));
  await cam.stop();
  await cam.close();
  assert.ok(frame, 'frame received');
  assert.equal(frame.width, 320);
  assert.equal(frame.height, 180);
  assert.equal(frame.data.length, 320 * 180 * 3 / 2, 'I420 size');
});

test('Recorder + Player closed loop (record camera → playback frames)', async () => {
  const path = `/tmp/opencode/node_test_${Date.now()}.mp4`;
  const cam = await CameraSource.open('stub:test-camera', { width: 320, height: 180, framerate: 10 });
  await cam.start();
  const rec = await Recorder.open(path);
  await rec.record(cam);           // background task, returns immediately
  await new Promise((r) => setTimeout(r, 1200));
  rec.stop();                       // stop_signal → flush + trailer
  await rec.close();
  await cam.stop(); await cam.close();

  const player = await Player.open(path);
  assert.ok(player.durationSecs() > 1.0, 'duration > 1s');
  let frames = 0;
  await player.onFrame(() => frames++);
  await new Promise((r) => setTimeout(r, 500));
  await player.close();
  assert.ok(frames > 5, `decoded ${frames} frames`);
});

test('CameraSource.open rejects unknown device', async () => {
  await assert.rejects(CameraSource.open('nope', {}));
});
