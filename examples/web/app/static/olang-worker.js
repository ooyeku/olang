// olang-worker.js — the Web Worker harness: a second olang, off the
// main thread. The page posts { wasmUrl, source } once; this worker
// instantiates its own wasm, boots the program as a persistent session,
// and bridges dom.post / dom.on_message over postMessage. There is no
// document here: DOM imports are inert stubs, exactly like the
// playground sandbox.

let ex = null;
let messageCb = null;
const queued = [];

const mem = () => new Uint8Array(ex.memory.buffer);
const readStr = (ptr, len) => new TextDecoder().decode(mem().slice(ptr, ptr + len));

function readResult(res) {
  const len = new DataView(ex.memory.buffer).getUint32(res, true);
  const json = JSON.parse(new TextDecoder().decode(mem().slice(res + 4, res + 4 + len)));
  ex.olang_result_free(res);
  if (json.output) console.log("[worker]", json.output.trimEnd());
  if (json.error) console.error("[worker] olang:", json.error);
  return json;
}

function dispatchJson(id, obj) {
  const bytes = new TextEncoder().encode(JSON.stringify(obj));
  const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
  mem().set(bytes, ptr);
  readResult(ex.olang_dispatch_event_json(BigInt(id), ptr, bytes.length));
  ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
}

const stub = () => {};
const imports = {
  env: new Proxy(
    {
      host_now_ms: () => performance.now(),
      host_epoch_ms: () => Date.now(),
      host_random_bytes: (ptr, len) =>
        crypto.getRandomValues(new Uint8Array(ex.memory.buffer, ptr, len)),
      host_dom_query: () => 0n,
      host_dom_get_text: () => 0,
      host_dom_get_value: () => 0,
      host_dom_get_attr: () => 0,
      host_dom_measure: () => 0,
      host_dom_location: () => 0,
      host_dom_storage_get: () => 0,
      host_dom_create: () => 0n,
      host_dom_set_interval: () => 0n,
      host_dom_worker_spawn: () => 0n,
      // The two live imports: values out, handler registration in.
      host_dom_post: (ptr, len) => {
        self.postMessage({ olang: readStr(ptr, len) });
      },
      host_dom_on_message: (id) => {
        messageCb = Number(id);
        while (queued.length) dispatchJson(messageCb, queued.shift());
      },
    },
    // Every other dom import is an inert stub — same posture as the
    // playground sandbox.
    { get: (t, name) => t[name] ?? stub }
  ),
};

self.onmessage = async (e) => {
  if (e.data.boot) {
    const { wasmUrl, source } = e.data.boot;
    const bytes = await fetch(wasmUrl).then((r) => r.arrayBuffer());
    ({ instance: { exports: ex } } = await WebAssembly.instantiate(bytes, imports));
    const enc = new TextEncoder().encode(source);
    const ptr = ex.olang_alloc(enc.length);
    mem().set(enc, ptr);
    readResult(ex.olang_session_start(ptr, enc.length));
    ex.olang_dealloc(ptr, enc.length);
    self.postMessage({ ready: true });
    return;
  }
  if (e.data.olang != null) {
    const value = JSON.parse(e.data.olang);
    if (messageCb == null) queued.push(value);
    else dispatchJson(messageCb, value);
  }
};
