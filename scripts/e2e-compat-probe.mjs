// e2e-compat P1 方言探针：legacy(缺字段)→1 / claim2→2 / claim99→钳3 / claim0→4101。
// 用法: MS_PSK=<psk> [MS_WS=ws://...] node --experimental-websocket e2e-compat-probe.mjs
const PSK = process.env.MS_PSK || process.exit(2);
const URL = process.env.MS_WS || "ws://127.0.0.1:9800/ws";

async function probe(name, joinPatch) {
  const ws = new WebSocket(URL);
  const room = "compat-" + name;
  const out = [];
  await new Promise((res) => {
    ws.onopen = () => ws.send(PSK);
    let stage = 0;
    ws.onmessage = (ev) => {
      const m = JSON.parse(ev.data);
      out.push(m);
      if (stage === 0 && m.type === "error" && m.code === 0) {
        stage = 1;
        ws.send(JSON.stringify({ type: "room_join", room_id: room, peer_role: "consumer", ...joinPatch }));
      } else if (stage === 1) {
        stage = 2;
        ws.close();
        res();
      }
    };
    setTimeout(res, 4000);
  });
  const joined = out.find((m) => m.type === "room_joined");
  const err = out.find((m) => m.type === "error" && m.code !== 0);
  console.log(
    `${name.padEnd(8)} → ${joined ? "room_joined protocol=" + JSON.stringify(joined.protocol ?? "ABSENT") : err ? "Error " + err.code + " (" + err.message.slice(0, 48) + ")" : "TIMEOUT " + JSON.stringify(out.map((x) => x.type))}`
  );
}

await probe("legacy-v1", {});
await probe("v2", { protocol: 2 });
await probe("claim99", { protocol: 99 });
await probe("claim0", { protocol: 0 });
