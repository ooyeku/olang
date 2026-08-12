// End-to-end proof of the dom bridge: a fake DOM implemented over the
// host imports, an olang program registering a click handler, two
// dispatched events, and assertions on the mutations.
import { readFile } from "node:fs/promises";

const fakeDom = {
  "#count": { text: "0", value: "" },
  "#btn": { text: "click me", value: "" },
  "#log": { text: "", value: "" },
};
const handles = ["", "#count", "#btn", "#log"]; // handle = index, 0 reserved
const listeners = {}; // handle -> {event -> callbackId}
const fetchLog = [];
function dispatchWith(cb, payload) {
  const bytes = new TextEncoder().encode(payload);
  const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
  mem().set(bytes, ptr);
  const r = result(ex.olang_dispatch_event_with(BigInt(cb), ptr, bytes.length));
  ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
  return r;
}

let ex; // wasm exports
const mem = () => new Uint8Array(ex.memory.buffer);
const readStr = (ptr, len) => new TextDecoder().decode(mem().slice(ptr, ptr + len));
function giveStr(s) {
  const bytes = new TextEncoder().encode(s);
  const ptr = ex.olang_alloc(4 + bytes.length);
  new DataView(ex.memory.buffer).setUint32(ptr, bytes.length, true);
  mem().set(bytes, ptr + 4);
  return ptr;
}

const imports = {
  env: {
    host_now_ms: () => performance.now(),
    host_epoch_ms: () => Date.now(),
    host_random_bytes: (ptr, len) =>
      crypto.getRandomValues(new Uint8Array(ex.memory.buffer, ptr, len)),
    host_dom_query: (ptr, len) => {
      const sel = readStr(ptr, len);
      const h = handles.indexOf(sel);
      return BigInt(h > 0 ? h : 0);
    },
    host_dom_set_text: (h, ptr, len) => { fakeDom[handles[Number(h)]].text = readStr(ptr, len); },
    host_dom_get_text: (h) => giveStr(fakeDom[handles[Number(h)]].text),
    host_dom_set_html: (h, ptr, len) => { fakeDom[handles[Number(h)]].html = readStr(ptr, len); },
    host_dom_get_value: (h) => giveStr(fakeDom[handles[Number(h)]].value),
    host_dom_set_value: (h, ptr, len) => { fakeDom[handles[Number(h)]].value = readStr(ptr, len); },
    host_dom_on: (h, ptr, len, id) => {
      (listeners[Number(h)] ??= {})[readStr(ptr, len)] = Number(id);
    },
    host_dom_focus: (_h) => {},
    host_dom_set_class: (h, ptr, len) => { fakeDom[handles[Number(h)]].className = readStr(ptr, len); },
    host_dom_fetch: (mp, ml, pp, pl, bp, bl, id) => {
      fetchLog.push({ method: readStr(mp, ml), path: readStr(pp, pl), body: readStr(bp, bl), cb: Number(id) });
    },
  },
};

const bytes = await readFile(process.argv[2]);
({ instance: { exports: ex } } = await WebAssembly.instantiate(bytes, imports));

function result(res) {
  const view = new DataView(ex.memory.buffer);
  const len = view.getUint32(res, true);
  const json = JSON.parse(new TextDecoder().decode(mem().slice(res + 4, res + 4 + len)));
  ex.olang_result_free(res);
  return json;
}

const program = `
let count = dom.query("#count")
let btn = dom.query("#btn")
let log = dom.query("#log")
dom.on(btn, "click", () => {
    let n = unwrap(str.parse_int(dom.get_text(count))) + 1
    dom.set_text(count, show(n))
    dom.set_text(log, "clicked " + show(n) + " times")
    println("handler ran, n=" + show(n))
})
dom.set_text(log, "ready")
println("session up")
`;
const enc = new TextEncoder().encode(program);
const ptr = ex.olang_alloc(enc.length);
mem().set(enc, ptr);
const startRes = result(ex.olang_session_start(ptr, enc.length));
ex.olang_dealloc(ptr, enc.length);
console.log("session:", JSON.stringify(startRes));
if (startRes.error) throw new Error(startRes.error);
if (fakeDom["#log"].text !== "ready") throw new Error("initial mutation missing");

// Click twice.
for (const expected of [1, 2]) {
  const cbId = listeners[2]["click"];
  const r = result(ex.olang_dispatch_event(BigInt(cbId)));
  console.log("event:", JSON.stringify(r));
  if (r.error) throw new Error(r.error);
  if (fakeDom["#count"].text !== String(expected))
    throw new Error(`count is ${fakeDom["#count"].text}, wanted ${expected}`);
  if (fakeDom["#log"].text !== `clicked ${expected} times`)
    throw new Error(`log is ${fakeDom["#log"].text}`);
}
// ── the fetch path: 1-arg handlers receive the response payload ──
const prog2 = `
dom.fetch("GET", "/api/things", "", (resp) => {
    let items = map_get(unwrap(json.parse(resp)), "items")
    dom.set_text(dom.query("#log"), "fetched " + show(len(items)) + " things")
})
println("fetch session up")
`;
{
  const enc2 = new TextEncoder().encode(prog2);
  const p2 = ex.olang_alloc(enc2.length);
  mem().set(enc2, p2);
  const r = result(ex.olang_session_start(p2, enc2.length));
  ex.olang_dealloc(p2, enc2.length);
  if (r.error) throw new Error("fetch session: " + r.error);
  if (fetchLog.length !== 1) throw new Error("host_dom_fetch not called");
  const resp = dispatchWith(fetchLog[0].cb, JSON.stringify({ items: [1, 2, 3] }));
  console.log("fetch event:", JSON.stringify(resp));
  if (resp.error) throw new Error("fetch dispatch: " + resp.error);
  if (fakeDom["#log"].text !== "fetched 3 things")
    throw new Error("fetch mutation missing: " + fakeDom["#log"].text);
}
// ── entropy: the random module must work through host_random_bytes ──
// (getrandom's wasm backend is the custom hook fed by that import.)
const prog3 = `
let rolls = map(range(0, 20), (i) => random.randint(1, 6))
let ok = len(filter(rolls, (r) => r >= 1 && r <= 6)) == 20
dom.set_text(dom.query("#log"), "random ok: " + show(ok) + " sample: " + show(random.random()))
`;
{
  const enc3 = new TextEncoder().encode(prog3);
  const p3 = ex.olang_alloc(enc3.length);
  mem().set(enc3, p3);
  const r = result(ex.olang_session_start(p3, enc3.length));
  ex.olang_dealloc(p3, enc3.length);
  if (r.error) throw new Error("random session: " + r.error);
  if (!fakeDom["#log"].text.startsWith("random ok: true"))
    throw new Error("random check failed: " + fakeDom["#log"].text);
  console.log("random:", fakeDom["#log"].text);
}
console.log("final dom:", JSON.stringify(fakeDom));
console.log("DOM BRIDGE END-TO-END PASSED (incl. fetch payloads + random)");
