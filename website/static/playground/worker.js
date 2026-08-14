// The playground's execution sandbox: olang compiled to WebAssembly,
// running inside this worker. The live host functions are two clocks and
// an entropy source — no filesystem, network, process, or DOM access
// exists on the other side of the boundary. The `dom` module's imports
// must still be present for the instance to link (the wasm build carries
// them for pages that ARE a browser frontend, like examples/app), but in
// this sandbox they are inert: dom.query finds nothing (handle 0, a clean
// olang-level error), reads yield empty strings, writes are no-ops. The
// page terminates this worker on timeout, which is what bounds runaway
// programs.

let exportsRef = null;

async function instantiate() {
  const { instance } = await WebAssembly.instantiateStreaming(
    fetch('/playground/olang.wasm'),
    {
      env: {
        host_now_ms: () => performance.now(),
        host_epoch_ms: () => Date.now(),
        host_random_bytes: (ptr, len) => {
          crypto.getRandomValues(new Uint8Array(exportsRef.memory.buffer, ptr, len));
        },
        // dom stubs: there is no document in the sandbox.
        host_dom_query: () => 0n,
        host_dom_set_text: () => {},
        host_dom_get_text: () => 0,
        host_dom_set_html: () => {},
        host_dom_get_value: () => 0,
        host_dom_set_value: () => {},
        host_dom_on: () => {},
        host_dom_focus: () => {},
        host_dom_set_class: () => {},
        host_dom_fetch: () => {},
        host_dom_get_attr: () => 0,
        host_dom_set_attr: () => {},
        host_dom_remove_attr: () => {},
        host_dom_class_op: () => {},
        host_dom_set_style: () => {},
        host_dom_measure: () => 0,
        host_dom_create: () => 0n,
        host_dom_append: () => {},
        host_dom_remove: () => {},
        host_dom_scroll_into_view: () => {},
        host_dom_set_timeout: () => {},
        host_dom_set_interval: () => 0n,
        host_dom_clear_interval: () => {},
        host_dom_request_frame: () => {},
        host_dom_draw: () => {},
        host_dom_draw_points: () => {},
        host_dom_on_frame: () => {},
        host_dom_insert_before: () => {},
        host_dom_push_state: () => {},
        host_dom_location: () => 0,
        host_dom_on_route: () => {},
        host_dom_storage_get: () => 0,
        host_dom_storage_set: () => {},
        host_dom_storage_remove: () => {},
        host_dom_state_get: () => 0,
        host_dom_state_set: () => {},
        host_dom_worker_spawn: () => 0n,
        host_dom_worker_send: () => {},
        host_dom_worker_on: () => {},
        host_dom_worker_close: () => {},
        host_dom_post: () => {},
        host_dom_on_message: () => {},
      },
    }
  );
  exportsRef = instance.exports;
  return instance.exports;
}

const ready = instantiate();

function readResult(ex, ptr) {
  const len = new DataView(ex.memory.buffer).getUint32(ptr, true);
  const json = new TextDecoder().decode(new Uint8Array(ex.memory.buffer, ptr + 4, len));
  ex.olang_result_free(ptr);
  return JSON.parse(json);
}

self.onmessage = async (event) => {
  const { id, source } = event.data;
  try {
    const ex = await ready;
    const bytes = new TextEncoder().encode(source);
    const ptr = ex.olang_alloc(bytes.length);
    new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes);
    const resPtr = ex.olang_run(ptr, bytes.length);
    ex.olang_dealloc(ptr, bytes.length);
    self.postMessage({ id, result: readResult(ex, resPtr) });
  } catch (err) {
    // A trap poisons the instance; report it and let the page recycle us.
    let panic = null;
    try {
      const p = exportsRef?.olang_last_panic();
      if (p) {
        const len = new DataView(exportsRef.memory.buffer).getUint32(p, true);
        panic = new TextDecoder().decode(
          new Uint8Array(exportsRef.memory.buffer, p + 4, len)
        );
      }
    } catch {
      /* memory unreadable after a hard trap */
    }
    self.postMessage({ id, crash: panic || String(err) });
  }
};
