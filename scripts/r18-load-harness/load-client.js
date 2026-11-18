// R18 load peer — runs in a chromium page. Produces fake 720p H.264 ~2Mbps via
// mediasoup-client + the server WS signaling (PSK first-message auth), and
// consumes the other producer(s) in its room. Bundled by esbuild to an IIFE
// exposing window.__runPeer / window.__r18stats.
import * as mediasoupClient from 'mediasoup-client';

const mapIce = (p) => ({ usernameFragment: p.username_fragment, password: p.password, iceLite: true });
const mapDtls = (p) => ({ role: p.role, fingerprints: p.fingerprints.map((f) => ({ algorithm: f.algorithm, value: f.value })) });
const mapCands = (l) => (l || []).map((c) => ({
  foundation: c.foundation, priority: c.priority, ip: c.ip, port: c.port,
  protocol: c.protocol, type: c.candidateType ?? c.candidate_type ?? 'host',
}));

window.__r18stats = { ready: false, produced: false, producerId: null, consumers: 0, errors: [], outboundMbps: 0, inboundMbps: 0, videoWidth: 0 };

window.__runPeer = async function runPeer(opts) {
  const { wsUrl, psk, roomId, bitrate = 2000000, fps = 30, width = 1280, height = 720, consume = true, role = 'host', produce = true, produceDelayMs = 0 } = opts;
  const S = window.__r18stats;
  const ws = new WebSocket(wsUrl);
  const waiters = {};
  const send = (o) => ws.send(JSON.stringify(o));
  const wait = (type, timeout = 20000) => new Promise((res, rej) => {
    const t = setTimeout(() => rej(new Error('timeout waiting ' + type)), timeout);
    (waiters[type] ||= []).push((m) => { clearTimeout(t); res(m); });
  });
  let myPeerId = null;
  let device = null;
  let recvTransport = null;
  let recvTransportId = null;
  let deviceReady = false;
  const pendingProducers = [];
  const consumerObjs = [];
  let sendTransport = null;
  let producer = null;
  let track = null;

  async function ensureRecv() {
    if (recvTransport) return;
    send({ type: 'create_web_rtc_transport', room_id: roomId, peer_id: myPeerId, direction: 'recv' });
    const rt = await wait('web_rtc_transport_created');
    recvTransportId = rt.transport_id;
    recvTransport = device.createRecvTransport({
      id: rt.transport_id, iceParameters: mapIce(rt.ice_parameters),
      iceCandidates: mapCands(rt.ice_candidates), dtlsParameters: mapDtls(rt.dtls_parameters),
    });
    recvTransport.on('connect', ({ dtlsParameters }, ok) => {
      send({ type: 'connect_web_rtc_transport', room_id: roomId, peer_id: myPeerId, transport_id: recvTransportId,
        dtls_parameters: { fingerprints: dtlsParameters.fingerprints.map((f) => ({ algorithm: f.algorithm, value: f.value })), role: dtlsParameters.role } });
      ok();
    });
  }

  async function consumeProducer(m) {
    if (!consume || m.peer_id === myPeerId || m.kind !== 'video') return;
    try {
      if (!device || !device.loaded) { S.errors.push('consume: device not loaded'); return; }
      await ensureRecv();
      const cp = wait('consumed');
      send({ type: 'consume', room_id: roomId, peer_id: myPeerId, transport_id: recvTransportId,
        producer_id: m.producer_id, kind: m.kind, rtp_capabilities: device.rtpCapabilities });
      const c = await cp;
      const consumer = await recvTransport.consume({ id: c.consumer_id, producerId: c.producer_id, kind: c.kind, rtpParameters: c.rtp_parameters });
      consumerObjs.push(consumer);
      const v = document.createElement('video');
      v.autoplay = true; v.muted = true; v.playsInline = true;
      v.srcObject = new MediaStream([consumer.track]);
      document.body.appendChild(v);
      S.consumers++;
    } catch (e) { S.errors.push('consume: ' + e.message); }
  }

  function onNewProducer(m) {
    if (!consume || m.peer_id === myPeerId || m.kind !== 'video') return;
    if (!deviceReady) { pendingProducers.push(m); return; } // buffer until device loaded
    void consumeProducer(m);
  }

  ws.onmessage = (ev) => {
    let m; try { m = JSON.parse(ev.data); } catch { return; }
    const q = waiters[m.type];
    if (q && q.length) q.shift()(m);
    if (m.type === 'new_producer') onNewProducer(m);
    if (m.type === 'error' && m.code !== 0) S.errors.push('server error ' + m.code + ': ' + m.message);
  };

  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error('ws connect failed')); });
  ws.send(psk); // raw PSK first message (SimplePskAuth)
  const ack = await wait('error');
  if (ack.code !== 0) throw new Error('PSK auth failed: ' + ack.message);
  send({ type: 'room_join', room_id: roomId, peer_role: role, protocol: 4 });
  const joined = await wait('room_joined');
  myPeerId = joined.peer_id;
  send({ type: 'get_router_rtp_capabilities', room_id: roomId });
  const caps = await wait('router_rtp_capabilities');
  device = new mediasoupClient.Device();
  await device.load({ routerRtpCapabilities: caps.capabilities });
  S.deviceLoaded = device.loaded;
  S.ready = true; // joined + device loaded (remote peers are ready to consume)
  deviceReady = true;
  for (const m of pendingProducers.splice(0)) void consumeProducer(m); // late-joiner sync buffered before load

  // ---- produce fake video (host only) ----
  if (produce) {
  send({ type: 'create_web_rtc_transport', room_id: roomId, peer_id: myPeerId, direction: 'send' });
  const st = await wait('web_rtc_transport_created');
  sendTransport = device.createSendTransport({
    id: st.transport_id, iceParameters: mapIce(st.ice_parameters),
    iceCandidates: mapCands(st.ice_candidates), dtlsParameters: mapDtls(st.dtls_parameters),
  });
  sendTransport.on('connect', ({ dtlsParameters }, ok) => {
    send({ type: 'connect_web_rtc_transport', room_id: roomId, peer_id: myPeerId, transport_id: st.transport_id,
      dtls_parameters: { fingerprints: dtlsParameters.fingerprints.map((f) => ({ algorithm: f.algorithm, value: f.value })), role: dtlsParameters.role } });
    ok();
  });
  sendTransport.on('produce', ({ kind, rtpParameters }, ok, fail) => {
    const p = wait('produced');
    send({ type: 'produce', room_id: roomId, peer_id: myPeerId, transport_direction: 'send',
      kind, rtp_parameters: rtpParameters, transport_id: st.transport_id, label: 'load' });
    p.then((m) => ok({ id: m.producer_id })).catch(fail);
  });
  const stream = await navigator.mediaDevices.getUserMedia({
    video: { width: { ideal: width }, height: { ideal: height }, frameRate: { ideal: fps } }, audio: false,
  });
  track = stream.getVideoTracks()[0];
  if (produceDelayMs > 0) await new Promise((r) => setTimeout(r, produceDelayMs)); // let remotes join first
  producer = await sendTransport.produce({
    track, encodings: [{ maxBitrate: bitrate }],
    codecOptions: { videoGoogleStartBitrate: Math.round(bitrate / 1000) },
  });
  S.produced = true; S.producerId = producer.id;
  } // end if(produce)

  // ---- stats sampler (runs for hosts AND remotes) ----
  setInterval(async () => {
    try {
      if (producer) {
        const ps = await producer.getStats();
        let bytes = 0; ps.forEach((r) => { if (r.type === 'outbound-rtp' && r.kind === 'video') bytes = r.bytesSent || 0; });
        if (S._lastOut != null) S.outboundMbps = ((bytes - S._lastOut) * 8 / 1e6) / 2; // 2s interval
        S._lastOut = bytes;
        if (track) { const settings = track.getSettings(); S.videoWidth = settings.width || 0; }
      }
      S.sendConn = sendTransport ? sendTransport.connectionState : null;
      S.recvConn = recvTransport ? recvTransport.connectionState : null;
      S.trackMuted = consumerObjs[0] ? consumerObjs[0].track.muted : null;
      if (consumerObjs.length) {
        let ib = 0;
        for (const co of consumerObjs) {
          const cs = await co.getStats();
          cs.forEach((r) => { if (r.type === 'inbound-rtp' && r.kind === 'video') ib += r.bytesReceived || 0; });
        }
        if (S._lastIn != null) S.inboundMbps = ((ib - S._lastIn) * 8 / 1e6) / 2;
        S._lastIn = ib;
      }
    } catch (e) { /* stats transient */ }
  }, 2000);
};