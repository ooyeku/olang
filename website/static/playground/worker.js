// The playground's execution sandbox: olang compiled to WebAssembly,
// running inside this worker. The wasm instance imports nothing but three
// host functions (two clocks and an entropy source) — no filesystem,
// network, process, or DOM access exists on the other side of the
// boundary. The page terminates this worker on timeout, which is what
// bounds runaway programs.

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
