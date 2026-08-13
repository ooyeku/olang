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
const node = (h) => {
  const el = fakeDom[handles[Number(h)]];
  el.attrs ??= {};
  el.classes ??= new Set();
  el.style ??= {};
  el.children ??= [];
  return el;
};
const listeners = {}; // handle -> {event -> callbackId}
const fetchLog = [];
const timers = []; // {ms, cb, kind}
const frames = []; // callback ids
let created = 0;
function dispatchWith(cb, payload) {
  const bytes = new TextEncoder().encode(payload);
  const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
  mem().set(bytes, ptr);
  const r = result(ex.olang_dispatch_event_with(BigInt(cb), ptr, bytes.length));
  ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
  return r;
}
function dispatchJson(cb, obj) {
  const bytes = new TextEncoder().encode(JSON.stringify(obj));
  const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
  mem().set(bytes, ptr);
  const r = result(ex.olang_dispatch_event_json(BigInt(cb), ptr, bytes.length));
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
    host_dom_get_attr: (h, ptr, len) => giveStr(node(h).attrs[readStr(ptr, len)] ?? ""),
    host_dom_set_attr: (h, np, nl, vp, vl) => { node(h).attrs[readStr(np, nl)] = readStr(vp, vl); },
    host_dom_remove_attr: (h, ptr, len) => { delete node(h).attrs[readStr(ptr, len)]; },
    host_dom_class_op: (h, op, ptr, len) => {
      const c = node(h).classes;
      const name = readStr(ptr, len);
      if (Number(op) === 0) c.add(name);
      else if (Number(op) === 1) c.delete(name);
      else c.has(name) ? c.delete(name) : c.add(name);
    },
    host_dom_set_style: (h, np, nl, vp, vl) => { node(h).style[readStr(np, nl)] = readStr(vp, vl); },
    host_dom_measure: (h) =>
      giveStr(JSON.stringify({ x: 1, y: 2, width: 300, height: 40 })),
    host_dom_create: (ptr, len) => {
      const key = `#created-${++created}-${readStr(ptr, len)}`;
      fakeDom[key] = { text: "", value: "" };
      return BigInt(handles.push(key) - 1);
    },
    host_dom_append: (p, c) => { node(p).children.push(handles[Number(c)]); },
    host_dom_remove: (h) => { node(h).removed = true; },
    host_dom_scroll_into_view: (_h) => {},
    host_dom_set_timeout: (ms, id) => { timers.push({ ms, cb: Number(id), kind: "timeout" }); },
    host_dom_set_interval: (ms, id) => {
      timers.push({ ms, cb: Number(id), kind: "interval" });
      return BigInt(timers.length);
    },
    host_dom_clear_interval: (_t) => {},
    host_dom_request_frame: (id) => { frames.push(Number(id)); },
    host_dom_on_frame: (id) => { frames.push(Number(id)); },
    host_dom_draw: (h, ptr, len) => {
      (node(h).drawn ??= []).push(JSON.parse(readStr(ptr, len)));
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
// ── stage 1: structured events, node ops, timers, frames ──
const prog4 = `
let log = dom.query("#log")
let btn = dom.query("#btn")
dom.on(btn, "click", (e) => {
    dom.set_text(log,
        map_get(e, "type") + " " + map_get(e, "id") + " " +
        show(map_get(e, "x")) + "," + show(map_get(e, "y")) + " shift=" +
        show(map_get(e, "shift")) + " row=" + map_get(map_get(e, "data"), "row"))
})
dom.set_attr(btn, "aria-label", "counter")
dom.class_add(btn, "primary")
dom.class_toggle(btn, "lit")
dom.set_style(btn, "color", "red")
let r = dom.measure(btn)
let card = dom.create("div")
dom.set_text(card, "made in olang")
dom.append(log, card)
dom.set_timeout(5, () => { dom.set_text(dom.query("#count"), "timed") })
dom.request_frame((f) => {
    dom.set_value(dom.query("#count"), "frame " + map_get(f, "type"))
})
println("attr=" + dom.get_attr(btn, "aria-label") + " w=" + show(map_get(r, "width")))
`;
{
  const enc4 = new TextEncoder().encode(prog4);
  const p4 = ex.olang_alloc(enc4.length);
  mem().set(enc4, p4);
  const r = result(ex.olang_session_start(p4, enc4.length));
  ex.olang_dealloc(p4, enc4.length);
  if (r.error) throw new Error("stage1 session: " + r.error);
  if (!r.output.includes("attr=counter w=300"))
    throw new Error("attr/measure round-trip failed: " + r.output);
  const btn = node(2);
  if (btn.attrs["aria-label"] !== "counter") throw new Error("set_attr missing");
  if (!btn.classes.has("primary") || !btn.classes.has("lit"))
    throw new Error("class ops missing: " + [...btn.classes]);
  if (btn.style["color"] !== "red") throw new Error("set_style missing");
  if (node(3).children.length !== 1) throw new Error("create/append missing");

  // Structured click: the handler reads type/id/coords/modifiers/data-*.
  const cb = listeners[2]["click"];
  const ev = dispatchJson(cb, {
    type: "click", id: "btn-7", value: "", key: "",
    x: 12, y: 34, alt: false, ctrl: false, shift: true, meta: false,
    data: { row: "r42" },
  });
  if (ev.error) throw new Error("structured click: " + ev.error);
  if (fakeDom["#log"].text !== "click btn-7 12,34 shift=true row=r42")
    throw new Error("structured payload wrong: " + fakeDom["#log"].text);

  // Fire the timer and the frame the program registered.
  const t = timers.find((t) => t.kind === "timeout");
  if (!t) throw new Error("set_timeout not registered");
  const tr = dispatchJson(t.cb, { type: "timeout" });
  if (tr.error) throw new Error("timeout dispatch: " + tr.error);
  if (fakeDom["#count"].text !== "timed") throw new Error("timeout mutation missing");
  if (frames.length !== 1) throw new Error("request_frame not registered");
  const fr = dispatchJson(frames[0], { type: "frame", delta: 16.6 });
  if (fr.error) throw new Error("frame dispatch: " + fr.error);
  if (fakeDom["#count"].value !== "frame frame")
    throw new Error("frame mutation missing: " + fakeDom["#count"].value);
  console.log("stage 1: structured events + node ops + timers ok");
}
// ── stage 2: the draw-list crosses once and replays faithfully ──
const prog5 = `
let sky = dom.query("#btn")
fn scene(t) = [
    #{ "op": "clear", "color": "#000" },
    #{ "op": "save" },
    #{ "op": "translate", "x": 100.0, "y": 50.0 },
    #{ "op": "rotate", "rad": t },
    #{ "op": "rect", "x": 0 - 10.0, "y": 0 - 10.0, "w": 20.0, "h": 20.0, "fill": "#7fd1b9" },
    #{ "op": "restore" },
    #{ "op": "circle", "x": 30.0, "y": 40.0, "r": 12.5, "stroke": "#f5c542", "line_width": 2 },
    #{ "op": "line", "x1": 0.0, "y1": 0.0, "x2": 60.0, "y2": 80.0, "stroke": "#fff" },
    #{ "op": "path", "points": [[0.0, 0.0], [10.0, 5.0], [20.0, 0.0]], "close": true, "fill": "#345" },
    #{ "op": "text", "x": 5.0, "y": 95.0, "text": "olang", "fill": "#9aa4b2" }
]
dom.on_frame((f) => {
    dom.draw(sky, scene(map_get(f, "delta") / 1000.0))
})
println("scene wired")
`;
{
  const enc5 = new TextEncoder().encode(prog5);
  const p5 = ex.olang_alloc(enc5.length);
  mem().set(enc5, p5);
  const r = result(ex.olang_session_start(p5, enc5.length));
  ex.olang_dealloc(p5, enc5.length);
  if (r.error) throw new Error("stage2 session: " + r.error);
  const frameCb = frames[frames.length - 1];
  const fr = dispatchJson(frameCb, { type: "frame", delta: 500.0 });
  if (fr.error) throw new Error("frame draw: " + fr.error);
  const drawn = node(2).drawn;
  if (!drawn || drawn.length !== 1) throw new Error("dom.draw not received");
  const ops = drawn[0];
  const kinds = ops.map((o) => o.op).join(",");
  if (kinds !== "clear,save,translate,rotate,rect,restore,circle,line,path,text")
    throw new Error("op order wrong: " + kinds);
  if (ops[3].rad !== 0.5) throw new Error("rotate lost precision: " + ops[3].rad);
  if (ops[4].fill !== "#7fd1b9" || ops[4].w !== 20) throw new Error("rect fields wrong");
  if (ops[6].line_width !== 2) throw new Error("stroke width missing");
  if (JSON.stringify(ops[8].points) !== "[[0,0],[10,5],[20,0]]")
    throw new Error("path points wrong: " + JSON.stringify(ops[8].points));
  console.log("stage 2: draw-list round-trip ok (" + ops.length + " ops)");
}
console.log("final dom:", JSON.stringify(fakeDom));
console.log("DOM BRIDGE END-TO-END PASSED (incl. fetch payloads + random)");
