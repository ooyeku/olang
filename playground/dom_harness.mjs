// End-to-end proof of the dom bridge: a fake DOM implemented over the
// host imports, an olang program registering a click handler, two
// dispatched events, and assertions on the mutations.
import { readFile } from "node:fs/promises";

const fakeDom = {
  "#count": { text: "0", value: "" },
  "#btn": { text: "click me", value: "" },
  "#log": { text: "", value: "" },
  "body": { text: "", value: "" },
};
const handles = ["", "#count", "#btn", "#log", "body"]; // handle = index, 0 reserved
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
const fakeStorage = {};
const fakeState = {};
const fakeHistory = ["/"];
const routeHandlers = [];
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
  return dispatchRaw(cb, JSON.stringify(obj));
}
function dispatchRaw(cb, payload) {
  const bytes = new TextEncoder().encode(payload);
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

// ── fake workers: a second wasm instance per worker, message queues ──
// drained only between wasm frames (mirroring async postMessage — the
// page and worker never re-enter each other mid-execution).
const JSON_CALLBACK_BIT = 2 ** 40;
const fakeWorkerSources = {}; // path -> olang source (tests fill this)
const workers = [null];
let workerModule = null; // compiled lazily from the same bytes

function bootWorker(source) {
  workerModule ??= new WebAssembly.Module(bytes);
  const entry = { cb: null, pageCb: null, toWorker: [], toPage: [] };
  let wex;
  const wmem = () => new Uint8Array(wex.memory.buffer);
  const wread = (ptr, len) => new TextDecoder().decode(wmem().slice(ptr, ptr + len));
  const stub = () => {};
  const wimports = {
    env: new Proxy(
      {
        host_now_ms: () => performance.now(),
        host_epoch_ms: () => Date.now(),
        host_random_bytes: (ptr, len) =>
          crypto.getRandomValues(new Uint8Array(wex.memory.buffer, ptr, len)),
        host_dom_query: () => 0n,
        host_dom_query_all: () => 0,
        host_dom_fetch_with: () => {},
        host_dom_morph: () => {},
        host_dom_checked: () => 0n,
        host_dom_selection: () => 0,
        host_dom_set_selection: () => {},
        host_dom_values: () => 0,
        host_dom_active_id: () => 0,
        host_dom_prefers_dark: () => 0n,
        host_dom_confirm: () => 1n,
        host_dom_read_file: () => {},
        host_dom_get_text: () => 0,
        host_dom_get_value: () => 0,
        host_dom_get_attr: () => 0,
        host_dom_measure: () => 0,
        host_dom_location: () => 0,
        host_dom_storage_get: () => 0,
        host_dom_create: () => 0n,
        host_dom_set_interval: () => 0n,
        host_dom_worker_spawn: () => 0n,
        host_dom_post: (ptr, len) => { entry.toPage.push(wread(ptr, len)); },
        host_dom_on_message: (id) => { entry.cb = Number(id); },
      },
      { get: (t, name) => t[name] ?? stub }
    ),
  };
  wex = new WebAssembly.Instance(workerModule, wimports).exports;
  const enc = new TextEncoder().encode(source);
  const ptr = wex.olang_alloc(enc.length);
  wmem().set(enc, ptr);
  const view = () => new DataView(wex.memory.buffer);
  const res = wex.olang_session_start(ptr, enc.length);
  const len = view().getUint32(res, true);
  const boot = JSON.parse(new TextDecoder().decode(wmem().slice(res + 4, res + 4 + len)));
  wex.olang_result_free(res);
  wex.olang_dealloc(ptr, enc.length);
  if (boot.error) throw new Error("worker boot: " + boot.error);
  entry.deliver = (payload) => {
    const b = new TextEncoder().encode(payload);
    const p = wex.olang_alloc(Math.max(b.length, 1));
    wmem().set(b, p);
    const r = wex.olang_dispatch_event_json(BigInt(entry.cb), p, b.length);
    const l = view().getUint32(r, true);
    const json = JSON.parse(new TextDecoder().decode(wmem().slice(r + 4, r + 4 + l)));
    wex.olang_result_free(r);
    wex.olang_dealloc(p, Math.max(b.length, 1));
    if (json.error) throw new Error("worker dispatch: " + json.error);
  };
  return entry;
}

// Drain both directions until quiet; called only from top-level test
// code, so no wasm instance is ever on the stack when another dispatches.
function pumpWorkers() {
  let moved = true;
  while (moved) {
    moved = false;
    for (const w of workers) {
      if (!w) continue;
      while (w.cb != null && w.toWorker.length) { w.deliver(w.toWorker.shift()); moved = true; }
      while (w.pageCb != null && w.toPage.length) { dispatchRaw(w.pageCb, w.toPage.shift()); moved = true; }
    }
  }
}

const imports = {
  env: {
    host_now_ms: () => performance.now(),
    host_epoch_ms: () => Date.now(),
    host_random_bytes: (ptr, len) =>
      crypto.getRandomValues(new Uint8Array(ex.memory.buffer, ptr, len)),
    host_dom_query_all: (ptr, len) => giveStr("[]"),
  host_dom_morph: (h, ptr, len) => { node(h).html = readStr(ptr, len); },
  host_dom_checked: (h) => (node(h).checked ? 1n : 0n),
  host_dom_selection: (h) => giveStr("[0,0]"),
  host_dom_set_selection: () => {},
  host_dom_values: (h) => giveStr("[]"),
  host_dom_fetch_with: () => {},
  host_dom_query: (ptr, len) => {
      const sel = readStr(ptr, len);
      let h = handles.indexOf(sel);
      // Created elements are findable by their id attribute, like a
      // real querySelector.
      if (h <= 0 && sel.startsWith("#")) {
        const id = sel.slice(1);
        h = handles.findIndex((k, i) => i > 0 && fakeDom[k]?.attrs?.id === id);
      }
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
    host_dom_get_attr: (h, ptr, len) => {
      const name = readStr(ptr, len);
      const n = node(h);
      // Seed elements carry their id implicitly in the handle key,
      // like a real DOM element's id attribute.
      if (name === "id" && n.attrs[name] == null) {
        const key = handles[Number(h)];
        return giveStr(key.startsWith("#") ? key.slice(1) : "");
      }
      return giveStr(n.attrs[name] ?? "");
    },
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
    host_dom_remove: (h) => {
      node(h).removed = true;
      const key = handles[Number(h)];
      for (const k of handles) {
        const kids = fakeDom[k]?.children;
        const i = kids ? kids.indexOf(key) : -1;
        if (i >= 0) kids.splice(i, 1);
      }
    },
    host_dom_scroll_into_view: (_h) => {},
    host_dom_set_timeout: (ms, id) => { timers.push({ ms, cb: Number(id), kind: "timeout" }); },
    host_dom_set_interval: (ms, id) => {
      timers.push({ ms, cb: Number(id), kind: "interval" });
      return BigInt(timers.length);
    },
    host_dom_clear_interval: (_t) => {},
    host_dom_request_frame: (id) => { frames.push(Number(id)); },
    host_dom_on_frame: (id) => { frames.push(Number(id)); },
    host_dom_insert_before: (p, c, b) => {
      const kids = node(p).children;
      const child = handles[Number(c)];
      const i = kids.indexOf(child);
      if (i >= 0) kids.splice(i, 1);
      const before = Number(b) ? handles[Number(b)] : null;
      const j = before ? kids.indexOf(before) : -1;
      if (j >= 0) kids.splice(j, 0, child);
      else kids.push(child);
    },
    host_dom_push_state: (ptr, len) => { fakeHistory.push(readStr(ptr, len)); },
    host_dom_location: () => {
      // Split pathname from search, like a real Location.
      const raw = fakeHistory[fakeHistory.length - 1] ?? "/";
      return giveStr(JSON.stringify({ path: raw.split("?")[0], query: { tab: "x" } }));
    },
    host_dom_on_route: (id) => { routeHandlers.push(Number(id)); },
    host_dom_storage_get: (ptr, len) => giveStr(fakeStorage[readStr(ptr, len)] ?? ""),
    host_dom_storage_set: (kp, kl, vp, vl) => { fakeStorage[readStr(kp, kl)] = readStr(vp, vl); },
    host_dom_storage_remove: (ptr, len) => { delete fakeStorage[readStr(ptr, len)]; },
    host_dom_state_get: (ptr, len) => giveStr(fakeState[readStr(ptr, len)] ?? ""),
    host_dom_state_set: (kp, kl, vp, vl) => { fakeState[readStr(kp, kl)] = readStr(vp, vl); },
    host_dom_draw: (h, ptr, len) => {
      (node(h).drawn ??= []).push(JSON.parse(readStr(ptr, len)));
    },
    host_dom_draw_points: (h, ptr, n, sp, sl) => {
      const pts = new Float64Array(ex.memory.buffer, Number(ptr), n * 2);
      (node(h).points ??= []).push({
        n,
        style: JSON.parse(readStr(sp, sl)),
        head: Array.from(pts.slice(0, 6)),
      });
    },
    host_dom_worker_spawn: (ptr, len) => {
      const source = fakeWorkerSources[readStr(ptr, len)];
      if (source == null) return 0n;
      return BigInt(workers.push(bootWorker(source)) - 1);
    },
    host_dom_worker_send: (h, ptr, len) => { workers[Number(h)].toWorker.push(readStr(ptr, len)); },
    host_dom_worker_on: (h, id) => { workers[Number(h)].pageCb = Number(id); },
    host_dom_worker_close: (h) => { workers[Number(h)] = null; },
    host_dom_post: () => {},
    host_dom_active_id: () => giveStr(""),
    host_dom_prefers_dark: () => 0n,
    host_dom_confirm: () => 1n,
    host_dom_read_file: () => {},
    host_dom_on_message: () => {},
  },
};

const bytes = await readFile(process.argv[2]);
({ instance: { exports: ex } } = await WebAssembly.instantiate(bytes, imports));

// `node dom_harness.mjs <wasm> --boot <bundle.ol> <bundle.olb>`: the boot
// pin. Starts a session from the source and from the program image and
// prints both load times as JSON — the image must beat the parser.
if (process.argv[3] === "--boot") {
  const src = await readFile(process.argv[4]);
  const img = await readFile(process.argv[5]);
  const start = (buf, entry) => {
    const p = ex.olang_alloc(Math.max(buf.length, 1));
    mem().set(buf, p);
    const res = entry(p, buf.length);
    // After the call: the memory may have grown, detaching any earlier view.
    const view = new DataView(ex.memory.buffer);
    const len = view.getUint32(res, true);
    const json = JSON.parse(new TextDecoder().decode(mem().slice(res + 4, res + 4 + len)));
    ex.olang_result_free(res);
    ex.olang_dealloc(p, Math.max(buf.length, 1));
    return json;
  };
  const fromSource = start(new Uint8Array(src), ex.olang_session_start);
  const fromImage = start(new Uint8Array(img), ex.olang_session_start_bin);
  console.log(JSON.stringify({
    source_load_ms: fromSource.load_ms,
    image_load_ms: fromImage.load_ms,
    image_retry_with_source: fromImage.retry_with_source,
  }));
  process.exit(0);
}

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
// ── stage 3: routing, storage, and the ui layer's reconciliation ──
const prog6 = `
use ui { h, hk, render }
dom.storage_set("notes", "persisted!")
dom.push_state("/notes?tab=all")
let loc = dom.location()
println("path=" + map_get(loc, "path") + " tab=" + map_get(map_get(loc, "query"), "tab"))
dom.on_route((r) => { dom.set_text(dom.query("#count"), "routed " + map_get(r, "path")) })

let mount = dom.query("#log")
fn item(id, label) = hk(id, "p", #{ "data-id": id }, [label])
render(mount, [item("a", "alpha"), item("b", "beta"), item("c", "gamma")])
render(mount, [item("c", "gamma"), item("a", "alpha CHANGED")])
println("stored=" + dom.storage_get("notes"))
`;
{
  const enc6 = new TextEncoder().encode(prog6);
  const p6 = ex.olang_alloc(enc6.length);
  mem().set(enc6, p6);
  const r = result(ex.olang_session_start(p6, enc6.length));
  ex.olang_dealloc(p6, enc6.length);
  if (r.error) throw new Error("stage3 session: " + r.error);
  if (!r.output.includes("path=/notes tab=x")) throw new Error("location wrong: " + r.output);
  if (!r.output.includes("stored=persisted!")) throw new Error("storage wrong: " + r.output);
  if (fakeStorage["notes"] !== "persisted!") throw new Error("storage_set missing");
  if (fakeHistory[fakeHistory.length - 1] !== "/notes?tab=all") throw new Error("push_state missing");

  // Reconciliation: after two renders, "b" is gone, order is [c, a],
  // "a" was re-rendered in place, "c" untouched.
  const log = node(3);
  const aKey = handles.find((k) => fakeDom[k]?.attrs?.id === "ui-log-a");
  const cKey = handles.find((k) => fakeDom[k]?.attrs?.id === "ui-log-c");
  const bKey = handles.find((k) => fakeDom[k]?.attrs?.id === "ui-log-b");
  if (!aKey || !cKey) throw new Error("ui wrappers missing");
  // Only the ui-owned wrappers count — stage 1 parked its own card here.
  const order = log.children.filter((k) => (fakeDom[k]?.attrs?.id ?? "").startsWith("ui-log-"));
  if (order.join(",") !== [cKey, aKey].join(","))
    throw new Error("reconciled order wrong: " + order.join(","));
  if (!fakeDom[aKey].html.includes("alpha CHANGED")) throw new Error("update-in-place missing");
  if (!fakeDom[cKey].html.includes("gamma")) throw new Error("survivor content wrong");
  if (bKey && !fakeDom[bKey].removed) throw new Error("removed key not removed");

  // The route event round-trips.
  const rr = dispatchJson(routeHandlers[0], { type: "route", path: "/back", query: {} });
  if (rr.error) throw new Error("route dispatch: " + rr.error);
  if (fakeDom["#count"].text !== "routed /back") throw new Error("route handler missing");
  console.log("stage 3: routing + storage + ui reconciliation ok");
}
// ── stage 4: workers (a second olang off the "main thread") + fetch_json ──
fakeWorkerSources["/sum.ol"] = `
dom.post(#{ "kind": "ready", "value": 0 })
dom.on_message((m) => {
    let n = map_get(m, "upto")
    let mut total = 0
    let mut i = 1
    while i <= n {
        total = total + i
        i = i + 1
    }
    dom.post(#{ "kind": "sum", "value": total })
})
`;
const prog7 = `
let log = dom.query("#log")
let w = dom.worker("/sum.ol")
dom.worker_on(w, (m) => {
    dom.set_text(log, dom.get_text(log) + "[" + map_get(m, "kind") + " " + show(map_get(m, "value")) + "]")
})
dom.worker_send(w, #{ "kind": "sum", "upto": 10 })
dom.fetch_json("GET", "/api/data", "", (v) => {
    dom.set_text(dom.query("#count"), "got " + show(len(map_get(v, "items"))) + " items")
})
println("worker handle=" + show(w))
`;
{
  fakeDom["#log"].text = "";
  const enc7 = new TextEncoder().encode(prog7);
  const p7 = ex.olang_alloc(enc7.length);
  mem().set(enc7, p7);
  const r = result(ex.olang_session_start(p7, enc7.length));
  ex.olang_dealloc(p7, enc7.length);
  if (r.error) throw new Error("stage4 session: " + r.error);
  if (!r.output.includes("worker handle=1")) throw new Error("worker spawn failed: " + r.output);

  // The pre-registration post ("ready") queued; the sum request queued;
  // one pump delivers both directions in order.
  pumpWorkers();
  if (fakeDom["#log"].text !== "[ready 0][sum 55]")
    throw new Error("worker round-trip wrong: " + fakeDom["#log"].text);

  // fetch_json: the callback id carries the JSON bit, and the response
  // text is delivered through the JSON dispatch as a parsed value.
  const f = fetchLog[fetchLog.length - 1];
  if (f.path !== "/api/data") throw new Error("fetch_json path wrong: " + f.path);
  if (f.cb < JSON_CALLBACK_BIT) throw new Error("fetch_json callback lacks JSON bit: " + f.cb);
  const fr = dispatchRaw(f.cb - JSON_CALLBACK_BIT, JSON.stringify({ items: [1, 2, 3, 4] }));
  if (fr.error) throw new Error("fetch_json dispatch: " + fr.error);
  if (fakeDom["#count"].text !== "got 4 items")
    throw new Error("fetch_json payload wrong: " + fakeDom["#count"].text);

  // A worker close survives a later pump.
  const w = workers[1];
  workers[1] = null;
  pumpWorkers();
  console.log("stage 4: workers + fetch_json ok");
}
// ── stage 5: viz interactivity — marks carry data, helpers ride events ──
const prog9 = `
use viz
let rows = [#{ "t": 1, "v": 2.0 }, #{ "t": 2, "v": 3.5 }]
dom.set_html(dom.query("#log"),
    viz.chart(#{ "data": rows, "mark": "point", "x": "t", "y": "v", "interactive": true }))
viz.brush(dom.query("#btn"), (b) => {
    dom.set_text(dom.query("#btn"),
        "brushed " + show(math.round(map_get(b, "from") * 10.0)) + "-" +
        show(math.round(map_get(b, "to") * 10.0)))
})
viz.tooltip(dom.query("#log"))
viz.on_mark(dom.query("#log"), "click", (d) => {
    dom.set_text(dom.query("#count"), "picked " + map_get(d, "x"))
})
println("viz wired")
`;
{
  const enc9 = new TextEncoder().encode(prog9);
  const p9 = ex.olang_alloc(enc9.length);
  mem().set(enc9, p9);
  const r = result(ex.olang_session_start(p9, enc9.length));
  ex.olang_dealloc(p9, enc9.length);
  if (r.error) throw new Error("stage5 session: " + r.error);
  if (!fakeDom["#log"].html.includes('data-x="2"'))
    throw new Error("interactive marks missing data attrs");

  // The tooltip div and the brush anchor were appended to body.
  const kids = node(handles.indexOf("body")).children;
  const tipKey = kids.find((k) => k.endsWith("-div"));
  const anchorKey = kids.find((k) => k.endsWith("-input"));
  if (!tipKey || !anchorKey) throw new Error("tooltip/brush elements missing: " + kids);

  const base = { type: "pointermove", id: "", value: "", key: "",
    x: 100, y: 80, alt: false, ctrl: false, shift: false, meta: false };
  // Hovering a mark shows its datum; leaving the marks hides it.
  dispatchJson(listeners[3]["pointermove"], { ...base, data: { s: "v", x: "2", y: "3.5" } });
  if (fakeDom[tipKey].text !== "v · (2, 3.5)")
    throw new Error("tooltip text wrong: " + fakeDom[tipKey].text);
  if (node(handles.indexOf(tipKey)).style["display"] !== "block")
    throw new Error("tooltip not shown");
  dispatchJson(listeners[3]["pointermove"], { ...base, data: {} });
  if (node(handles.indexOf(tipKey)).style["display"] !== "none")
    throw new Error("tooltip not hidden off-marks");

  // on_mark fires only for mark targets.
  dispatchJson(listeners[3]["click"], { ...base, type: "click", data: { s: "v", x: "2", y: "3.5" } });
  if (fakeDom["#count"].text !== "picked 2") throw new Error("on_mark missing: " + fakeDom["#count"].text);
  dispatchJson(listeners[3]["click"], { ...base, type: "click", data: {} });
  if (fakeDom["#count"].text !== "picked 2") throw new Error("on_mark fired off-mark");

  // Brush: down at 20% of the fake 300px-wide rect (x=1), up at 60%.
  dispatchJson(listeners[2]["pointerdown"], { ...base, type: "pointerdown", x: 61 });
  dispatchJson(listeners[2]["pointerup"], { ...base, type: "pointerup", x: 181 });
  if (fakeDom["#btn"].text !== "brushed 2.0-6.0")
    throw new Error("brush range wrong: " + fakeDom["#btn"].text);
  console.log("stage 5: viz interactivity ok");
}
// ── stage 6: the bulk point path — packed f64, zero-copy, null-safe ──
const prog10 = `
let missing = map_get(#{}, "absent")
dom.draw_points(dom.query("#btn"), ods.series([0.0, 1.0, 2.0]), ods.series([5.0, 6.0, 7.0]),
    #{ "mode": "points", "size": 2.0, "sx": 10.0, "tx": 3.0 })
dom.draw_points(dom.query("#btn"), [1.5, 2.5], [4.0, 8.0], #{ "mode": "path", "color": "#fff" })
dom.draw_points(dom.query("#btn"), ods.series([1.0, missing, 3.0]), ods.series([9.0, 9.5, 10.0]), #{})
println("points sent")
`;
{
  const enc10 = new TextEncoder().encode(prog10);
  const p10 = ex.olang_alloc(enc10.length);
  mem().set(enc10, p10);
  const r = result(ex.olang_session_start(p10, enc10.length));
  ex.olang_dealloc(p10, enc10.length);
  if (r.error) throw new Error("stage6 session: " + r.error);
  const calls = node(2).points;
  if (!calls || calls.length !== 3) throw new Error("draw_points calls missing");
  const [a, b, c] = calls;
  if (a.n !== 3 || JSON.stringify(a.head) !== "[0,5,1,6,2,7]")
    throw new Error("series pack wrong: " + JSON.stringify(a));
  if (a.style.sx !== 10 || a.style.tx !== 3 || a.style.mode !== "points")
    throw new Error("style lost: " + JSON.stringify(a.style));
  if (b.n !== 2 || JSON.stringify(b.head) !== "[1.5,4,2.5,8]" || b.style.mode !== "path")
    throw new Error("list pack wrong: " + JSON.stringify(b));
  // The null pair dropped: 3 in, 2 out.
  if (c.n !== 2 || JSON.stringify(c.head) !== "[1,9,3,10]")
    throw new Error("null drop wrong: " + JSON.stringify(c));
  console.log("stage 6: bulk point path ok");
}
// ── stage 7: session state — JSON-typed, page-lifetime store ──
const prog11 = `
dom.state_set("count", 3)
dom.state_set("filter", #{ "status": "open", "tags": ["a", "b"] })
let c = dom.state_get("count")
let f = dom.state_get("filter")
let missing = dom.state_get("nope")
dom.set_text(dom.query("#log"),
    show(c + 1) + " " + map_get(f, "status") + " " +
    show(len(map_get(f, "tags"))) + " " + show(typeof(missing) == "Unit"))
println("state ok")
`;
{
  const enc11 = new TextEncoder().encode(prog11);
  const p11 = ex.olang_alloc(enc11.length);
  mem().set(enc11, p11);
  const r = result(ex.olang_session_start(p11, enc11.length));
  ex.olang_dealloc(p11, enc11.length);
  if (r.error) throw new Error("stage7 session: " + r.error);
  // count round-trips as a number, the Map survives with its list, and a
  // missing key reads as Unit (()).
  if (fakeDom["#log"].text !== "4 open 2 true")
    throw new Error("state round-trip wrong: " + fakeDom["#log"].text);
  console.log("stage 7: session state ok");
}
console.log("final dom:", JSON.stringify(fakeDom));
console.log("DOM BRIDGE END-TO-END PASSED (incl. fetch payloads + random)");
