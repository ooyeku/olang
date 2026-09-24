// End-to-end proof of the dom bridge: a fake DOM implemented over the
// host imports, an olang program registering a click handler, two
// dispatched events, and assertions on the mutations.
import { readFile } from "node:fs/promises";

const fakeDom = {
  "#count": { text: "0", value: "" },
  "#btn": { text: "click me", value: "" },
  "#log": { text: "", value: "" },
  "body": { text: "", value: "" },
  // The web SDK's mount point, so a real client bundle's `mount` runs
  // under `--boot` and the boot number covers the first render.
  "#app": { text: "", value: "", html: "" },
};
const handles = ["", "#count", "#btn", "#log", "body", "#app"]; // handle = index, 0 reserved
const node = (h) => {
  const el = fakeDom[handles[Number(h)]];
  el.attrs ??= {};
  el.classes ??= new Set();
  el.style ??= {};
  el.children ??= [];
  return el;
};
const listeners = {}; // handle -> {event -> callbackId}
let errorCallback = null; // dom.on_error's handler
const handleFor = (key) => {
  fakeDom[key] ??= { text: "", value: "" };
  const at = handles.indexOf(key);
  return at > 0 ? at : handles.push(key) - 1;
};
const fetchLog = [];
const timers = []; // {ms, cb, kind}
const frames = []; // callback ids
const fakeStorage = {};
const fakeState = {};
const fakeHistory = ["/"];
const routeHandlers = [];
let created = 0;
// The shim's rule, mirrored: one dispatch at a time; a nested one waits.
let dispatching = false;
const dispatchQueue = [];
function enter(run) {
  if (dispatching) { dispatchQueue.push(run); return; }
  dispatching = true;
  try { run(); } finally {
    dispatching = false;
    if (dispatchQueue.length) { const next = dispatchQueue.shift(); queueMicrotask(() => enter(next)); }
  }
}
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
  let r = null;
  enter(() => {
    const bytes = new TextEncoder().encode(payload);
    const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
    mem().set(bytes, ptr);
    r = result(ex.olang_dispatch_event_json(BigInt(cb), ptr, bytes.length));
    ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
  });
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
        host_dom_patch: () => {},
        host_take_error: () => 0,
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
  // Models what the shim's patcher answers: a `keep` (or a `keeps`
  // list) naming a key that was not on the page after the previous patch
  // cannot be honored, and is reported back as missing.
  host_dom_patch: (h, ptr, len) => {
    const n = node(h);
    n.tree = JSON.parse(readStr(ptr, len));
    n.patches = (n.patches ?? 0) + 1;
    const before = n.keys ?? new Map(); // key -> the keys beneath it
    const after = new Map();
    const missing = [];
    const walk = (v, under) => {
      if (v == null || typeof v === "string") return;
      if (Array.isArray(v)) { for (const c of v) walk(c, under); return; }
      const kept = v.keep !== undefined ? [String(v.keep)] : Array.isArray(v.keeps) ? v.keeps.map(String) : null;
      if (kept) {
        for (const k of kept) {
          if (!before.has(k)) { missing.push(k); continue; }
          const beneath = before.get(k);
          after.set(k, beneath);
          for (const u of under) u.add(k);
          for (const b of beneath) { after.set(b, before.get(b) ?? new Set()); for (const u of under) u.add(b); }
        }
        return;
      }
      const key = v.attrs && v.attrs["data-key"] != null ? String(v.attrs["data-key"]) : null;
      let inside = under;
      if (key) {
        const mine = new Set();
        after.set(key, mine);
        for (const u of under) u.add(key);
        inside = [...under, mine];
      }
      walk(v.children, inside);
    };
    walk(n.tree, []);
    n.keys = after;
    return missing.length ? giveStr(JSON.stringify(missing)) : 0;
  },
  host_take_error: () => {
    if (hostError == null) return 0;
    const m = hostError;
    hostError = null;
    return giveStr(m);
  },
  host_dom_checked: (h) => (node(h).checked == null ? 2n : node(h).checked ? 1n : 0n),
  host_dom_selection: (h) => giveStr("[0,0]"),
  host_dom_set_selection: () => {},
  host_dom_values: (h) => giveStr("[]"),
  host_dom_fetch_with: () => {},
  host_dom_query: (ptr, len) => {
      const sel = readStr(ptr, len);
      // What document.querySelector does with a malformed selector.
      if (sel.includes("[]")) throw new SyntaxError(`'${sel}' is not a valid selector`);
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
    host_dom_off: (h, ptr, len) => {
      const ev = readStr(ptr, len);
      const l = listeners[Number(h)] ?? {};
      const ids = [];
      for (const k of Object.keys(l)) if (ev === "*" || k === ev) { ids.push(l[k]); delete l[k]; }
      return giveStr(JSON.stringify(ids));
    },
    // A focus fires the element's focusin listener synchronously, as a
    // browser does — the nested dispatch the shim's queue exists for.
    host_dom_focus: (h) => {
      const l = listeners[Number(h)];
      if (l && l.focusin != null) dispatchJson(l.focusin, { type: "focusin", id: handles[Number(h)] });
      // A hidden control does not take focus, and the call says so.
      return node(h).attrs.hidden != null ? 0 : 1;
    },
    // `window` and `document` are handles like any element's.
    host_dom_window: () => BigInt(handleFor("window")),
    host_dom_document: () => BigInt(handleFor("document")),
    host_dom_on_error: (id) => { errorCallback = Number(id); },
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
    // A quota: a value over 64 characters is refused, and the refusal is
    // an answer.
    host_dom_storage_set: (kp, kl, vp, vl) => {
      const v = readStr(vp, vl);
      if (v.length > 64) return 0;
      fakeStorage[readStr(kp, kl)] = v;
      return 1;
    },
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

// The shim's rule, mirrored: a host import that throws reports through
// host_take_error instead of unwinding through the runtime.
let hostError = null;
{
  const HANDLE = new Set(["host_dom_query", "host_dom_create", "host_dom_set_interval", "host_dom_worker_spawn",
    "host_dom_prefers_dark", "host_dom_confirm", "host_dom_checked"]);
  const STRING = new Set(["host_dom_query_all", "host_dom_get_text", "host_dom_get_value", "host_dom_get_attr",
    "host_dom_measure", "host_dom_location", "host_dom_storage_get", "host_dom_state_get", "host_dom_active_id",
    "host_dom_selection", "host_dom_values", "host_dom_off"]);
  for (const name of Object.keys(imports.env)) {
    if (!name.startsWith("host_dom_")) continue;
    const f = imports.env[name];
    imports.env[name] = (...a) => {
      try { return f(...a); }
      catch (e) {
        hostError = e && e.message ? e.message : String(e);
        return HANDLE.has(name) ? 0n : STRING.has(name) ? giveStr("") : undefined;
      }
    };
  }
}

const bytes = await readFile(process.argv[2]);

// `node dom_harness.mjs <wasm> --high-memory`: the heap above 2 GiB.
// A pointer crosses the boundary as a wasm i32, which JavaScript reads as
// signed; past 2 GiB the shim once read selectors from the wrong bytes and
// failed on every result buffer after. This mode runs the REAL shim's
// boundary block (u32, unsignedExports, readStr, giveStr, takeResultText)
// against the runtime with the low 2 GiB held, so every allocation the
// session makes lands above 0x8000_0000, and drives dispatches that
// succeed, raise, and fire one-shot callbacks. Prints one JSON line.
if (process.argv[3] === "--high-memory") {
  const shimSrc = await readFile(new URL("../frameworks/web-sdk/static/olang-dom.js", import.meta.url), "utf8");
  const from = shimSrc.indexOf("  // ── the wasm boundary: pointers are unsigned ──");
  const to = shimSrc.indexOf("  // (end of the wasm boundary)");
  if (from < 0 || to < from) throw new Error("high-memory: cannot find the shim's wasm boundary block");
  const B = new Function(
    `let ex;\n${shimSrc.slice(from, to)}\n` +
    "return { bind: (raw) => (ex = unsignedExports(raw)), u32, mem, readStr, giveStr, takeResultText };"
  )();
  let hx; // the runtime's exports, bound through the shim's boundary
  let highest = 0; // the highest pointer the host was handed
  const seen = (p) => { highest = Math.max(highest, B.u32(p)); return p; };
  const names = [null, "#app"];
  const text = {};
  const on = {};
  const attached = {}; // event -> every callback id bound for it
  const intervals = new Map(); // timer id -> callback id
  let lastTimer = 0;
  const env = {
    host_now_ms: () => performance.now(),
    host_epoch_ms: () => Date.now(),
    host_random_bytes: (ptr, len) =>
      crypto.getRandomValues(new Uint8Array(hx.memory.buffer, B.u32(ptr), B.u32(len))),
    host_dom_query: (ptr, len) => {
      seen(ptr);
      const i = names.indexOf(B.readStr(ptr, len));
      return BigInt(i > 0 ? i : 0);
    },
    host_dom_set_text: (h, ptr, len) => { seen(ptr); text[names[Number(h)]] = B.readStr(ptr, len); },
    host_dom_on: (h, ptr, len, id) => {
      seen(ptr);
      if (!names[Number(h)]) throw new TypeError("no element for handle " + h);
      on[B.readStr(ptr, len)] = Number(id);
      (attached[B.readStr(ptr, len)] ??= []).push(Number(id));
    },
    // dom.off: detach every listener bound for the event; answer their ids.
    host_dom_off: (h, ptr, len) => {
      const ev = B.readStr(ptr, len);
      const ids = attached[ev] ?? [];
      delete attached[ev];
      return B.giveStr(JSON.stringify(ids));
    },
    host_dom_set_timeout: (ms, id) => { on.timeout = Number(id); },
    host_dom_set_interval: (ms, id) => { intervals.set(++lastTimer, Number(id)); return BigInt(lastTimer); },
    host_dom_clear_interval: (t) => { intervals.delete(Number(t)); },
    // What a real read_file does with a handle that names no file input:
    // its JavaScript throws, and the callback it was handed never runs.
    host_dom_read_file: (h, id) => { throw new TypeError("Cannot read properties of undefined (reading 'files')"); },
    host_take_error: () => {
      if (hostError == null) return 0;
      const m = hostError;
      hostError = null;
      return B.giveStr(m);
    },
  };
  // The shim's rule: a host import that throws reports through
  // host_take_error instead of unwinding through the runtime.
  let hostError = null;
  for (const name of Object.keys(env)) {
    if (!name.startsWith("host_dom_")) continue;
    const f = env[name];
    env[name] = (...a) => {
      try { return f(...a); }
      catch (e) {
        hostError = e && e.message ? e.message : String(e);
        return name === "host_dom_set_interval" ? 0n : name === "host_dom_off" ? B.giveStr("") : undefined;
      }
    };
  }
  const { instance } = await WebAssembly.instantiate(bytes, {
    env: new Proxy(env, { get: (t, name) => t[name] ?? (() => 0) }),
  });
  hx = B.bind(instance.exports);
  // Hold the low 2 GiB in 16 MiB blocks: nothing below is free after.
  const held = [];
  for (;;) {
    const p = hx.olang_alloc(16 << 20);
    if (p < 0) throw new Error("high-memory: the boundary answered a negative pointer: " + p);
    held.push(p);
    if (p >= 2 ** 31 + (32 << 20)) break;
    if (held.length > 200) throw new Error("high-memory: the heap never passed 2 GiB");
  }
  const program = `
let app = dom.query("#app")
dom.set_text(app, "booted")
dom.on(app, "click", (ev) => dom.set_text(dom.query("#app"), "clicked " + to_string(map_get(ev, "n"))))
dom.on(app, "boom", (ev) => dom.set_text(dom.query("#nope"), "never"))
dom.on(app, "notstr", (ev) => dom.find(42))
dom.on(app, "arm", (ev) => dom.set_timeout(5, (t) => dom.set_text(dom.query("#app"), "timeout " + to_string(map_get(ev, "n")))))
dom.on(app, "cycle", (ev) => {
    let n = map_get(ev, "n")
    let t = dom.set_interval(1000, (tick) => dom.set_text(dom.query("#app"), "tick " + to_string(n)))
    dom.clear_interval(t)
})
dom.on(app, "rebind", (ev) => {
    let n = map_get(ev, "n")
    let gone = dom.off(dom.query("#app"), "rebound")
    dom.on(dom.query("#app"), "rebound", (e) => dom.set_text(dom.query("#app"), "rebound " + to_string(n) + " off " + to_string(gone)))
})
dom.on(app, "throws", (ev) => {
    let n = map_get(ev, "n")
    dom.read_file(dom.query("#app"), (f) => dom.set_text(dom.query("#app"), "file " + to_string(n)))
})
dom.on(app, "badhandle", (ev) => dom.on(99, "click", (e) => ()))
dom.on(app, "badtype", (ev) => dom.on("app", "click", (e) => ()))
`;
  const src = new TextEncoder().encode(program);
  const sp = seen(hx.olang_alloc(src.length));
  B.mem().set(src, sp);
  const boot = JSON.parse(B.takeResultText(seen(hx.olang_session_start(sp, src.length))));
  hx.olang_dealloc(sp, src.length);
  if (boot.error) throw new Error("high-memory: boot: " + boot.error);
  if (text["#app"] !== "booted") throw new Error("high-memory: boot did not set the text: " + JSON.stringify(text));
  const fire = (id, obj) => {
    const b = new TextEncoder().encode(JSON.stringify(obj));
    const p = seen(hx.olang_alloc(Math.max(b.length, 1)));
    B.mem().set(b, p);
    const r = JSON.parse(B.takeResultText(seen(hx.olang_dispatch_event_json(BigInt(id), p, b.length))));
    hx.olang_dealloc(p, Math.max(b.length, 1));
    return r;
  };
  const live = () => JSON.parse(B.takeResultText(seen(hx.olang_handler_count())));
  // The rebind stage's first binding, made before the baseline: the
  // stage replaces it 200 times and must leave exactly it behind.
  fire(on.rebind, { type: "rebind", n: 0 });
  const liveBefore = live();
  let raised = 0;
  for (let i = 1; i <= 120; i++) {
    const r = fire(on.click, { type: "click", n: i, pad: "x".repeat(i * 997) });
    if (r.error || text["#app"] !== "clicked " + i)
      throw new Error(`high-memory: click ${i}: ${JSON.stringify(r)} / ${text["#app"]}`);
    if (i % 3 === 0) {
      // A handler that raises answers a readable result with the error
      // inside, and the next dispatch is unaffected.
      const b = fire(on.boom, { type: "boom" });
      if (!b.error || !b.error.includes('no element matches "#nope"'))
        throw new Error("high-memory: the raise was not reported: " + JSON.stringify(b));
      raised++;
    }
    if (i % 5 === 0) {
      const b = fire(on.notstr, { type: "notstr" });
      if (!b.error || !b.error.includes("dom.find: not a string"))
        throw new Error("high-memory: dom.find(42) did not refuse: " + JSON.stringify(b));
    }
    if (i % 4 === 0) {
      // A one-shot callback runs once and leaves the registry.
      fire(on.arm, { type: "arm", n: i });
      const t = fire(on.timeout, { type: "timeout" });
      if (t.error || text["#app"] !== "timeout " + i)
        throw new Error("high-memory: the timeout did not run: " + JSON.stringify(t));
      const again = fire(on.timeout, { type: "timeout" });
      if (!again.error || !again.error.includes("already ran"))
        throw new Error("high-memory: a spent one-shot ran again: " + JSON.stringify(again));
    }
  }
  // Handler lifetimes beyond the one-shots: each stage below runs 200
  // times and the registry must come back to where it started.
  const stage = (label, run) => {
    const before = live().live;
    for (let i = 1; i <= 200; i++) run(i);
    return live().live - before;
  };
  // (a) An interval created and cleared: the handler leaves with the timer.
  const intervalGrowth = stage("interval", (i) => {
    const r = fire(on.cycle, { type: "cycle", n: i });
    if (r.error) throw new Error("high-memory: interval cycle: " + JSON.stringify(r));
    if (intervals.size !== 0) throw new Error("high-memory: an interval outlived clear_interval");
  });
  // (b) A handler re-bound for the same element and event after dom.off:
  // the old one is detached and released, the new one fires.
  let offReported = true;
  const rebindGrowth = stage("rebind", (i) => {
    const r = fire(on.rebind, { type: "rebind", n: i });
    if (r.error) throw new Error("high-memory: rebind: " + JSON.stringify(r));
    if ((attached.rebound ?? []).length !== 1) throw new Error("high-memory: dom.off left listeners: " + JSON.stringify(attached.rebound));
    const f = fire(attached.rebound[0], { type: "rebound" });
    if (f.error) throw new Error("high-memory: the rebound handler: " + JSON.stringify(f));
    if (text["#app"] !== `rebound ${i} off 1`) offReported = false;
  });
  // (c) A host call that throws, and a registration on a handle that is
  // not an element: the call raises, and its callback is not kept.
  let throwsReported = 0;
  const throwGrowth = stage("throws", (i) => {
    const r = fire(on.throws, { type: "throws", n: i });
    if (r.error && r.error.includes("dom.read_file")) throwsReported++;
    const b = fire(on.badhandle, { type: "badhandle" });
    if (!b.error) throw new Error("high-memory: dom.on(99, ...) did not raise");
    const c = fire(on.badtype, { type: "badtype" });
    if (!c.error || !c.error.includes("expected an element handle"))
      throw new Error("high-memory: dom.on(\"app\", ...) did not refuse: " + JSON.stringify(c));
  });
  const liveAfter = live();
  for (const p of held) hx.olang_dealloc(p, 16 << 20);
  console.log(JSON.stringify({
    highest,
    memory_mb: Math.round(hx.memory.buffer.byteLength / 1048576),
    raised,
    live_before: liveBefore.live,
    live_after: liveAfter.live,
    registered: liveAfter.registered,
    interval_growth: intervalGrowth,
    rebind_growth: rebindGrowth,
    rebind_off_reported: offReported,
    throw_growth: throwGrowth,
    throws_reported: throwsReported,
  }));
  process.exit(0);
}

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
  const profile = process.argv.includes("--profile");
  const fromSource = start(new Uint8Array(src), ex.olang_session_start);
  if (profile) ex.olang_profile_start();
  const fromImage = start(new Uint8Array(img), ex.olang_session_start_bin);
  const report = profile ? result(ex.olang_profile_stop()) : null;
  console.log(JSON.stringify({
    source_load_ms: fromSource.load_ms,
    source_run_ms: fromSource.run_ms,
    image_load_ms: fromImage.load_ms,
    image_run_ms: fromImage.run_ms,
    image_retry_with_source: fromImage.retry_with_source,
    error: fromImage.error ?? fromSource.error ?? null,
  }));
  // `--profile`: where the image's run went, by olang function — the
  // declarations themselves are top-level statements and appear as the
  // gap between run_ms and the rows' total.
  if (report) {
    const shown = report.rows.slice(0, 15).map((r) => `${r.function} [${r.tier}] x${r.calls} self ${r.self_ms}ms total ${r.total_ms}ms`);
    console.log(shown.join("\n"));
    const inFunctions = report.rows.filter((r) => r.builtin === false).reduce((a, r) => a + r.self_ms, 0);
    console.log(`self time inside functions: ${inFunctions.toFixed(1)} ms of run ${fromImage.run_ms} ms`);
  }
  process.exit(0);
}

// `node dom_harness.mjs <wasm> --repaints <bundle.ol>`: the repaint pin.
// The bundle is a web SDK client (tests/w21_test.rs writes it) mounted on
// `#app`; each action is dispatched as a click through the delegated
// listener, and the patches that reach `#app` are counted: one for a
// handled action, none for a handler that changes nothing or a change
// confined to unwatched keys, two when a `keep` could not be honored.
if (process.argv[3] === "--repaints") {
  const src = new Uint8Array(await readFile(process.argv[4]));
  const p = ex.olang_alloc(src.length);
  mem().set(src, p);
  const boot = result(ex.olang_session_start(p, src.length));
  ex.olang_dealloc(p, src.length);
  if (boot.error) throw new Error("repaints session: " + boot.error);
  const app = node(handles.indexOf("#app"));
  const click = listeners[handles.indexOf("#app")]?.click;
  if (click == null) throw new Error("mount bound no click listener on #app");
  const fire = (action) => {
    const was = app.patches ?? 0;
    const r = dispatchJson(click, { type: "click", tag: "button", id: "", value: "", key: "", data: { action } });
    if (r.error) throw new Error(action + ": " + r.error);
    return (app.patches ?? 0) - was;
  };
  const flat = (v, out = []) => {
    if (v == null) return out;
    if (Array.isArray(v)) { for (const c of v) flat(c, out); return out; }
    out.push(v);
    if (v.children) flat(v.children, out);
    return out;
  };
  const expect = (what, got, want) => {
    if (got !== want) throw new Error(`${what}: ${got} patch(es), expected ${want}`);
  };
  if ((app.patches ?? 0) !== 1) throw new Error("mount painted " + app.patches + " times, expected 1");
  expect("a handled action", fire("inc"), 1);
  if (!flat(app.tree).some((v) => Array.isArray(v.keeps) && v.keeps.length === 3))
    throw new Error("an unmoved memo_list did not answer one marker for its rows: " + JSON.stringify(app.tree));
  expect("a handler that changes nothing", fire("idle"), 0);
  expect("a change confined to an unwatched key", fire("beat"), 0);
  expect("one row retitled", fire("retitle"), 1);
  const rows = flat(app.tree).filter((v) => v.keep !== undefined || (v.attrs && String(v.attrs["data-key"] ?? "").startsWith("rows:")));
  const built = rows.filter((v) => v.tag === "li").map((v) => v.attrs["data-key"]).join(",");
  const kept = rows.filter((v) => v.keep !== undefined && String(v.keep).startsWith("rows:")).map((v) => v.keep).join(",");
  if (built !== "rows:2" || kept !== "rows:1,rows:3")
    throw new Error(`memo_list rebuilt [${built}] and kept [${kept}]; expected rows:2 rebuilt, rows:1 and rows:3 kept`);
  expect("opening the drawer", fire("toggle"), 1);
  expect("closing it", fire("toggle"), 1);
  // The drawer's stamp outlived its element: the keep cannot be honored,
  // the runtime forgets the stamp and paints once more.
  expect("reopening it (a stale keep heals)", fire("toggle"), 2);
  if (!flat(app.tree).some((v) => v.tag === "aside" && v.attrs["data-key"] === "drawer"))
    throw new Error("the healed repaint did not build the drawer: " + JSON.stringify(app.tree));
  fire("stats");
  const stats = JSON.parse(fakeDom["#log"].text);
  console.log(JSON.stringify(stats));
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
// ── stage 8: dom.patch — the node tree crosses as data ──
const prog12 = `
let rows = map([3, 1, 2], (n) => #{ "tag": "li", "attrs": #{ "data-key": "r" + to_string(n) }, "children": [#{ "text": "row " + to_string(n) }] })
dom.patch(dom.query("#log"), #{ "tag": "ul", "attrs": #{ "class": "rows", "hidden": false }, "children": rows })
println("patched")
`;
{
  const enc12 = new TextEncoder().encode(prog12);
  const p12 = ex.olang_alloc(enc12.length);
  mem().set(enc12, p12);
  const r = result(ex.olang_session_start(p12, enc12.length));
  ex.olang_dealloc(p12, enc12.length);
  if (r.error) throw new Error("stage8 session: " + r.error);
  const tree = fakeDom["#log"].tree;
  // The tree arrives typed: attrs keep their booleans, children their
  // order and keys, text nodes their text — nothing was rendered to
  // markup on the way.
  if (!tree || tree.tag !== "ul" || tree.attrs.class !== "rows" || tree.attrs.hidden !== false)
    throw new Error("patch tree wrong: " + JSON.stringify(tree));
  const keys = tree.children.map((c) => c.attrs["data-key"]).join(",");
  if (keys !== "r3,r1,r2" || tree.children[0].children[0].text !== "row 3")
    throw new Error("patch children wrong: " + JSON.stringify(tree.children));
  console.log("stage 8: dom.patch tree ok");
}
// ── stage 9: the browser profiler, over a 100-row repaint loop ──
const prog13 = `
fn row(i) = #{ "tag": "tr", "attrs": #{ "data-key": "r" + to_string(i), "class": if i % 2 == 0 => "even" else => "odd" },
    "children": [#{ "tag": "td", "attrs": #{}, "children": [#{ "text": "Issue " + to_string(i) }] },
                 #{ "tag": "td", "attrs": #{ "title": "priority" }, "children": [#{ "text": to_string(i % 5) }] }] }
fn table(n, tick) = #{ "tag": "table", "attrs": #{ "data-tick": to_string(tick) }, "children": map(range(0, n), row) }
let root = dom.query("#log")
let t0 = time.monotonic_ms()
for tick in range(0, 20) { dom.patch(root, table(100, tick)) }
let per = (time.monotonic_ms() - t0) / 20
println("repaint of 100 rows, wasm side: " + to_string(per) + " ms each")
// What the tree's crossing costs, from dom.patch's own answer: the
// measurement behind leaving the JSON encoding in place.
let mut ser = 0.0
let mut nodes = 0
for tick in range(0, 20) {
    let a = dom.patch(root, table(200, tick))
    ser = ser + map_get(a, "serialize_ms")
    nodes = map_get(a, "nodes")
}
println("serialize: " + to_string(ser / 20.0) + " ms for " + to_string(nodes) + " nodes")
dom.patch(root, table(100, 0))
`;
{
  ex.olang_profile_start();
  const enc13 = new TextEncoder().encode(prog13);
  const p13 = ex.olang_alloc(enc13.length);
  mem().set(enc13, p13);
  const r = result(ex.olang_session_start(p13, enc13.length));
  ex.olang_dealloc(p13, enc13.length);
  if (r.error) throw new Error("stage9 session: " + r.error);
  const rep = result(ex.olang_profile_stop());
  if (!Array.isArray(rep.rows) || rep.rows.length === 0)
    throw new Error("profile report empty: " + JSON.stringify(rep));
  const top = rep.rows[0];
  if (typeof top.function !== "string" || typeof top.self_ms !== "number" || typeof top.tier !== "string")
    throw new Error("profile row shape wrong: " + JSON.stringify(top));
  if (!rep.rows.some((row) => row.function === "row" || row.function === "table"))
    throw new Error("profile did not see the view functions: " + JSON.stringify(rep.rows.slice(0, 5)));
  console.log("stage 9: browser profiler ok —", rep.rows.slice(0, 3).map((x) => `${x.function} ${x.self_ms}ms/${x.calls}`).join(", "));
  if (fakeDom["#log"].tree.children.length !== 100) throw new Error("repaint tree lost rows");
  const crossing = r.output.split("\n").find((l) => l.startsWith("serialize:"));
  if (!crossing) throw new Error("dom.patch did not answer its costs: " + r.output);
  console.log("stage 9: the tree's crossing —", crossing);
}
// ── stage 10: a focus inside a handler nests a dispatch — queued, not re-entered ──
const prog14 = `
let input = dom.query("#btn")
let log = dom.query("#log")
dom.on(input, "focusin", (ev) => dom.set_text(log, dom.get_text(log) + " focusin"))
dom.on(dom.query("#count"), "click", (ev) => {
    dom.focus(input)
    dom.set_text(log, dom.get_text(log) + " clicked")
})
dom.set_text(log, "start")
`;
{
  const enc14 = new TextEncoder().encode(prog14);
  const p14 = ex.olang_alloc(enc14.length);
  mem().set(enc14, p14);
  const r = result(ex.olang_session_start(p14, enc14.length));
  ex.olang_dealloc(p14, enc14.length);
  if (r.error) throw new Error("stage10 session: " + r.error);
  const click = listeners[handles.indexOf("#count")].click;
  const outer = dispatchJson(click, { type: "click", id: "count" });
  if (outer.error) throw new Error("stage10 outer dispatch: " + outer.error);
  // The focusin ran after the click handler finished, not inside it.
  if (fakeDom["#log"].text !== "start clicked") throw new Error("nested dispatch ran inline: " + fakeDom["#log"].text);
  await new Promise((resolve) => setTimeout(resolve, 0));
  if (fakeDom["#log"].text !== "start clicked focusin") throw new Error("queued dispatch lost: " + fakeDom["#log"].text);
  console.log("stage 10: nested dispatch queued ok");
}
// ── stage 11: a host call that throws is an olang error, and the session lives on ──
const prog15 = `
let r = attempt(() => dom.query("#wip-[]"))
dom.set_text(dom.query("#log"), show(r))
let again = dom.query("#count")
dom.set_text(dom.query("#count"), "still here")
`;
{
  const enc15 = new TextEncoder().encode(prog15);
  const p15 = ex.olang_alloc(enc15.length);
  mem().set(enc15, p15);
  const r = result(ex.olang_session_start(p15, enc15.length));
  ex.olang_dealloc(p15, enc15.length);
  if (r.error) throw new Error("stage11 session: " + r.error);
  const shown = fakeDom["#log"].text;
  if (!shown.startsWith("Err(") || !shown.includes("dom.query") || !shown.includes("not a valid selector"))
    throw new Error("host throw not raised as an olang error: " + shown);
  if (fakeDom["#count"].text !== "still here") throw new Error("session did not survive the host throw");
  console.log("stage 11: host throw raised, session alive ok");
}
// ── stage 11b: the window, the document, and host calls that answer ──
const prog16 = `
let log = dom.query("#log")
dom.on(dom.window(), "online", (ev) => dom.set_text(log, "online=" + show(map_get(ev, "online"))))
dom.on(dom.document(), "visibilitychange", (ev) => dom.set_text(log, "hidden=" + show(map_get(ev, "hidden"))))
dom.on_error((e) => dom.set_text(dom.query("#count"), "reported: " + map_get(e, "error")))
dom.on(dom.query("#btn"), "click", (ev) => map_get(1, "boom"))
dom.set_attr(dom.query("#btn"), "hidden", "")
println(show(dom.focus(dom.query("#btn"))) + " " + show(dom.focus(log)))
println(show(dom.storage_set("draft", "short")) + " " + show(dom.storage_set("draft", "${"x".repeat(80)}")))
println(dom.storage_get("draft"))
`;
{
  const enc16 = new TextEncoder().encode(prog16);
  const p16 = ex.olang_alloc(enc16.length);
  mem().set(enc16, p16);
  const r = result(ex.olang_session_start(p16, enc16.length));
  ex.olang_dealloc(p16, enc16.length);
  if (r.error) throw new Error("stage11b session: " + r.error);
  if (r.output !== "false true\ntrue false\nshort\n")
    throw new Error("stage 11b: focus and storage_set did not answer: " + JSON.stringify(r.output));
  const win = listeners[handles.indexOf("window")];
  const doc = listeners[handles.indexOf("document")];
  if (!win || win.online == null || !doc || doc.visibilitychange == null)
    throw new Error("stage 11b: window/document listeners not bound");
  dispatchJson(win.online, { type: "online", online: true });
  if (fakeDom["#log"].text !== "online=true") throw new Error("stage 11b: online payload: " + fakeDom["#log"].text);
  dispatchJson(doc.visibilitychange, { type: "visibilitychange", hidden: true });
  if (fakeDom["#log"].text !== "hidden=true") throw new Error("stage 11b: visibility payload: " + fakeDom["#log"].text);
  // A handler that raises: the dispatch answers the error, the shim's
  // part — mirrored here — reports it to the registered handler, and the
  // session goes on serving events.
  if (errorCallback == null) throw new Error("stage 11b: dom.on_error registered nothing");
  const failed = dispatchJson(listeners[handles.indexOf("#btn")].click, { type: "click", data: {} });
  if (!failed.error) throw new Error("stage 11b: the raising handler reported no error");
  const reported = dispatchJson(errorCallback, { error: failed.error, output: failed.output ?? "", trap: false });
  if (reported.error || !fakeDom["#count"].text.startsWith("reported: "))
    throw new Error("stage 11b: the error handler did not run: " + JSON.stringify(reported) + fakeDom["#count"].text);
  dispatchJson(win.online, { type: "online", online: false });
  if (fakeDom["#log"].text !== "online=false") throw new Error("stage 11b: the session did not survive the raise");
  console.log("stage 11b: window + document events, answering host calls, error handler ok");
}
console.log("final dom:", JSON.stringify(fakeDom));
// ── stage 12: the shim's morph keeps the focused control's live value ──
// The real shim's `morphInto` runs here against a small DOM model with a
// focus: the control someone is typing in keeps its value and identity
// across a repaint, every other control follows the markup, and a keyed
// row that moves keeps its node. (Pinned nowhere else: the browser is
// the only other DOM.)
{
  const shimSrc = await readFile(new URL("../frameworks/web-sdk/static/olang-dom.js", import.meta.url), "utf8");
  const start = shimSrc.indexOf("  function morphInto(el, html)");
  const end = shimSrc.indexOf("  function morphNode(from, to)");
  const nodeEnd = shimSrc.indexOf("\n  }\n", shimSrc.indexOf("morphChildren(from, to);", end)) + 4;
  if (start < 0 || end < 0 || nodeEnd < 4) throw new Error("stage 12: cannot find the shim's morph functions");
  const morphSrc = shimSrc.slice(start, nodeEnd);

  const VOID = new Set(["input", "br", "img", "hr", "meta", "link"]);
  class MNode {
    constructor(nodeType) { this.nodeType = nodeType; this.parentNode = null; this.childNodes = []; }
    contains(n) { for (let c = n; c; c = c.parentNode) if (c === this) return true; return false; }
    get lastChild() { return this.childNodes[this.childNodes.length - 1] || null; }
    appendChild(c) { return this.insertBefore(c, null); }
    insertBefore(c, ref) {
      // As a browser does: an element that leaves the document, even to
      // come straight back, loses the focus it held.
      if (c.parentNode && document.activeElement && c.contains(document.activeElement)) document.activeElement = null;
      if (c.parentNode) c.parentNode.removeChild(c);
      const i = ref ? this.childNodes.indexOf(ref) : this.childNodes.length;
      this.childNodes.splice(i < 0 ? this.childNodes.length : i, 0, c);
      c.parentNode = this;
      return c;
    }
    removeChild(c) { const i = this.childNodes.indexOf(c); if (i >= 0) this.childNodes.splice(i, 1); c.parentNode = null; return c; }
    replaceChild(n, old) { this.insertBefore(n, old); this.removeChild(old); return old; }
  }
  class MText extends MNode {
    constructor(data) { super(3); this.data = data; }
    get nodeName() { return "#text"; }
    cloneNode() { return new MText(this.data); }
  }
  class MElement extends MNode {
    constructor(tag) { super(1); this.tagName = tag.toUpperCase(); this.nodeName = this.tagName; this._attrs = new Map(); this._value = null; this.checked = false; }
    get attributes() { return [...this._attrs].map(([name, value]) => ({ name, value })); }
    getAttribute(n) { return this._attrs.has(n) ? this._attrs.get(n) : null; }
    setAttribute(n, v) { this._attrs.set(n, String(v)); }
    removeAttribute(n) { this._attrs.delete(n); }
    hasAttribute(n) { return this._attrs.has(n); }
    get dataset() { const o = {}; for (const [k, v] of this._attrs) if (k.startsWith("data-")) o[k.slice(5)] = v; return o; }
    get type() { return this.getAttribute("type") || "text"; }
    focus() { document.activeElement = this; }
    get value() { return this._value ?? this.getAttribute("value") ?? ""; }
    set value(v) { this._value = String(v); }
    cloneNode(deep) {
      const c = new MElement(this.tagName);
      for (const [k, v] of this._attrs) c._attrs.set(k, v);
      if (deep) for (const k of this.childNodes) c.appendChild(k.cloneNode(true));
      return c;
    }
  }
  function parseHtml(html, into) {
    const re = /<\/([a-zA-Z0-9-]+)\s*>|<([a-zA-Z0-9-]+)((?:\s+[a-zA-Z0-9:-]+(?:="[^"]*")?)*)\s*(\/?)>|([^<]+)/g;
    let cur = into; let m;
    while ((m = re.exec(html))) {
      if (m[1]) { if (cur.parentNode) cur = cur.parentNode; }
      else if (m[2]) {
        const el = new MElement(m[2]);
        for (const a of (m[3] || "").matchAll(/([a-zA-Z0-9:-]+)(?:="([^"]*)")?/g)) el.setAttribute(a[1], a[2] ?? "");
        cur.appendChild(el);
        if (!m[4] && !VOID.has(m[2].toLowerCase())) cur = el;
      } else if (m[5] && m[5].trim() !== "") cur.appendChild(new MText(m[5]));
    }
  }
  const document = {
    activeElement: null,
    createElement(tag) {
      if (tag === "template") {
        const t = { content: new MNode(11) };
        Object.defineProperty(t, "innerHTML", { set(html) { t.content.childNodes = []; parseHtml(html, t.content); } });
        return t;
      }
      return new MElement(tag);
    },
  };
  const moveAt = shimSrc.indexOf("  function moveNode(parent, node, before)");
  const moveSrc = shimSrc.slice(moveAt, shimSrc.indexOf("\n  }\n", moveAt) + 4);
  if (moveAt < 0) throw new Error("stage 12: cannot find the shim's moveNode");
  const { morphInto } = new Function("document", moveSrc + morphSrc + "\n  return { morphInto };")(document);

  const app = new MElement("div");
  parseHtml('<ul><li data-key="a"><input id="a" value="alpha"></li><li data-key="b"><input id="b" value="beta"></li></ul>', app);
  const list = app.childNodes[0];
  const inputA = list.childNodes[0].childNodes[0];
  const inputB = list.childNodes[1].childNodes[0];
  document.activeElement = inputA;
  inputA.value = "alpha typed";
  // A repaint: the rows swap, both values change in the markup, a row joins.
  morphInto(app, '<ul><li data-key="b"><input id="b" value="BETA"></li><li data-key="a"><input id="a" value="ALPHA"></li><li data-key="c"><input id="c" value="gamma"></li></ul>');
  const rows = app.childNodes[0].childNodes;
  const keys = rows.map((r) => r.getAttribute("data-key")).join(",");
  if (keys !== "b,a,c") throw new Error("stage 12: keyed rows not reordered: " + keys);
  if (rows[1].childNodes[0] !== inputA) throw new Error("stage 12: the focused input lost its identity across the move");
  if (document.activeElement !== inputA) throw new Error("stage 12: the focused input lost its focus when its row moved");
  if (inputA.value !== "alpha typed") throw new Error("stage 12: the focused input's live value was overwritten: " + inputA.value);
  if (rows[0].childNodes[0] !== inputB) throw new Error("stage 12: an unfocused keyed input lost its identity");
  if (inputB.value !== "BETA") throw new Error("stage 12: an unfocused input did not follow the markup: " + inputB.value);
  if (rows[2].childNodes[0].getAttribute("value") !== "gamma") throw new Error("stage 12: the new row is missing");
  document.activeElement = null;
  morphInto(app, '<ul><li data-key="a"><input id="a" value="ALPHA"></li></ul>');
  if (app.childNodes[0].childNodes.length !== 1 || inputA.value !== "ALPHA") throw new Error("stage 12: blur then repaint did not adopt the markup value");
  console.log("stage 12: morph keeps the focused control's value and identity ok");

  // ── stage 13: the shim's patcher answers the keeps it could not honor ──
  // The real `patchInto`, over the same DOM model: nested children and
  // strings flatten as they are walked, a `keeps` list stands for its
  // rows, a kept element is left exactly as it is (moved if the order
  // changed), and a `keep` naming an element that is not there comes back
  // as missing — what `dom.patch` hands the SDK so a stale memo heals.
  const pStart = shimSrc.indexOf("  const SVG_NS =");
  const pEnd = shimSrc.indexOf("  function morphInto(el, html)");
  const keyOfAt = shimSrc.indexOf("  const keyOf =");
  if (pStart < 0 || pEnd < pStart || keyOfAt < 0) throw new Error("stage 13: cannot find the shim's patch functions");
  const keyOfSrc = shimSrc.slice(keyOfAt, shimSrc.indexOf("\n", keyOfAt));
  document.createTextNode = (data) => new MText(data);
  document.createElementNS = (ns, tag) => new MElement(tag);
  const { patchInto } = new Function("document", keyOfSrc + "\n" + shimSrc.slice(pStart, pEnd) + "\n  return { patchInto };")(document);
  const li = (key, label) => ({ tag: "li", attrs: { "data-key": key }, children: [label] });
  const host = new MElement("div");
  let missing = patchInto(host, { tag: "ul", attrs: {}, children: [[li("rows:1", "one"), [li("rows:2", "two")]], { tag: "", attrs: {}, children: [li("rows:3", "three")] }] });
  const ul = host.childNodes[0];
  const labels = () => ul.childNodes.map((n) => n.getAttribute("data-key") + "=" + n.childNodes[0].data).join(",");
  if (missing.length !== 0 || labels() !== "rows:1=one,rows:2=two,rows:3=three")
    throw new Error("stage 13: nested children did not flatten: " + labels() + " missing " + missing);
  const [one, two, three] = ul.childNodes;
  missing = patchInto(host, { tag: "ul", attrs: {}, children: [{ keeps: ["rows:1", "rows:2", "rows:3"] }] });
  if (missing.length !== 0 || ul.childNodes[0] !== one || ul.childNodes[2] !== three)
    throw new Error("stage 13: a keeps list did not keep its rows");
  missing = patchInto(host, { tag: "ul", attrs: {}, children: [{ keep: "rows:3" }, li("rows:2", "TWO"), { keep: "rows:1" }] });
  if (missing.length !== 0 || labels() !== "rows:3=three,rows:2=TWO,rows:1=one" || ul.childNodes[0] !== three || ul.childNodes[1] !== two)
    throw new Error("stage 13: kept rows were not moved in place: " + labels());
  // A kept row that moves keeps the focus it held.
  const field = new MElement("input");
  one.appendChild(field);
  field.focus();
  missing = patchInto(host, { tag: "ul", attrs: {}, children: [{ keep: "rows:1" }, { keep: "rows:3" }, { keep: "rows:2" }] });
  if (document.activeElement !== field || ul.childNodes[0] !== one)
    throw new Error("stage 13: a moved row lost the focus inside it");
  one.removeChild(field);
  document.activeElement = null;
  missing = patchInto(host, { tag: "ul", attrs: {}, children: [{ keep: "rows:1" }, { keep: "drawer" }, { keeps: ["rows:9"] }] });
  if (missing.join(",") !== "drawer,rows:9" || labels() !== "rows:1=one")
    throw new Error("stage 13: missing keeps not answered: " + missing + " / " + labels());
  console.log("stage 13: the patcher flattens as it walks and answers the keeps it could not honor ok");
}

console.log("DOM BRIDGE END-TO-END PASSED (incl. fetch payloads + random)");
