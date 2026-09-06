// JS-side stress A/B for the websocket channel body change (Raw -> Json).
//
// Per frame, tauri delivers via evaluateJavaScript:
// - Raw (old): script contains `new Uint8Array([12,34,...]).buffer`; the JS side
//   then decodes it (old normalizeWebSocketMessage) and the consumer JSON.parses.
// - Json (new): script embeds the JSON text inline; the JS side receives a parsed
//   object, normalize re-serializes it (Text contract), consumer JSON.parses.
//
// eval() stands in for evaluateJavaScript so script-compile cost is included. This
// models tauri's direct-execute path (Raw < 1 KiB, Json < 8 KiB); Raw frames
// >= 1 KiB took the fetch queue instead (no number-array script). The base script
// is prebuilt; only a unique comment suffix is appended per iteration (busting
// V8's compilation cache) so harness cost stays negligible.
// Set AB_SCALE to multiply iteration counts.
// Run: node benchmarks/ws-channel-ab.mjs

const textDecoder = new TextDecoder();
const textEncoder = new TextEncoder();
const scale = Number(process.env.AB_SCALE ?? 1);

function buildConnectionsPayload(minLen) {
  const connection =
    '{"id":"bench-id","metadata":{"network":"tcp","type":"HTTP","sourceIP":"198.18.0.1","destinationIP":"93.184.216.34","host":"example.com","dnsMode":"normal","processPath":"/Applications/Example.app"},"chains":["Proxy","DIRECT"],"rule":"MATCH","rulePayload":"","upload":123456,"download":654321,"start":"2026-05-25T00:00:00Z"}';
  let payload = '{"downloadTotal":1,"uploadTotal":2,"connections":[';
  while (payload.length < minLen) {
    if (!payload.endsWith("[")) payload += ",";
    payload += connection;
  }
  return payload + "]}";
}

const rawConsume = (buf) => JSON.parse(textDecoder.decode(buf));
const jsonConsume = (obj) => JSON.parse(JSON.stringify(obj));

function warmup(baseScript, consume) {
  for (let i = 0; i < 200; i += 1) consume(eval(baseScript));
}

function timed(iterations, baseScript, consume) {
  let sink;
  const started = performance.now();
  for (let i = 0; i < iterations; i += 1) {
    sink = consume(eval(`${baseScript}/*${i}*/`));
  }
  if (sink === undefined) throw new Error("benchmark produced no result");
  return ((performance.now() - started) * 1000) / iterations;
}

const cases = [
  ["traffic", '{"up":69632,"down":3810304}', 300_000],
  ["memory", '{"inuse":536870912,"oslimit":0}', 300_000],
  [
    "logs",
    '{"type":"info","payload":"[TCP] 198.18.0.1:52311 --> example.com:443 match Match using Proxy[DIRECT]"}',
    300_000,
  ],
  ["connections-64k", buildConnectionsPayload(64 * 1024), 2_000],
  ["connections-256k", buildConnectionsPayload(256 * 1024), 300],
  ["connections-1m", buildConnectionsPayload(1024 * 1024), 50],
];

for (const [name, payload, baseIterations] of cases) {
  const iterations = Math.max(10, Math.round(baseIterations * scale));
  const bytes = textEncoder.encode(payload);
  const byteArray = Array.from(bytes);
  const rawBase = `new Uint8Array([${byteArray.join(",")}]).buffer`;
  const jsonBase = `(${payload})`;

  warmup(rawBase, rawConsume);
  warmup(jsonBase, jsonConsume);

  const rawUs = timed(iterations, rawBase, rawConsume);
  const jsonUs = timed(iterations, jsonBase, jsonConsume);

  const wireRaw = JSON.stringify(byteArray).length;
  const mbps = (us) => payload.length / 1024 / 1024 / (us / 1e6);
  console.log(
    `${name}: payload=${payload.length}B iters=${iterations} wire: raw=${wireRaw}B ` +
      `json=${payload.length}B (${(wireRaw / payload.length).toFixed(2)}x) | js per frame: ` +
      `raw=${rawUs.toFixed(2)}us json=${jsonUs.toFixed(2)}us speedup=${(rawUs / jsonUs).toFixed(2)}x | ` +
      `throughput: raw=${mbps(rawUs).toFixed(1)}MB/s json=${mbps(jsonUs).toFixed(1)}MB/s`,
  );
}
