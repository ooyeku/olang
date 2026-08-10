// olang-dom.js — the page-side shim: makes this browser an olang host.
// Loads the wasm build, implements the dom host imports over the real
// DOM, boots a persistent session from /app.ol, and routes events and
// fetch responses back into the live interpreter.

(async function () {
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

  // Element handles: index into this array (0 reserved = not found).
  const elements = [null];
  const handleOf = (el) => {
    const i = elements.indexOf(el);
    return i > 0 ? i : elements.push(el) - 1;
  };

  function readResult(res) {
    const view = new DataView(ex.memory.buffer);
    const len = view.getUint32(res, true);
    const json = JSON.parse(new TextDecoder().decode(mem().slice(res + 4, res + 4 + len)));
    ex.olang_result_free(res);
    if (json.output) console.log(json.output.trimEnd());
    if (json.error) console.error("olang:", json.error);
    return json;
  }

  function dispatch(id, payload) {
    if (payload == null) {
      readResult(ex.olang_dispatch_event(BigInt(id)));
    } else {
      const bytes = new TextEncoder().encode(payload);
      const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
      mem().set(bytes, ptr);
      readResult(ex.olang_dispatch_event_with(BigInt(id), ptr, bytes.length));
      ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
    }
  }

  const imports = {
    env: {
      host_now_ms: () => performance.now(),
      host_epoch_ms: () => Date.now(),
      host_random_bytes: (ptr, len) =>
        crypto.getRandomValues(new Uint8Array(ex.memory.buffer, ptr, len)),
      host_dom_query: (ptr, len) => {
        const el = document.querySelector(readStr(ptr, len));
        return BigInt(el ? handleOf(el) : 0);
      },
      host_dom_set_text: (h, ptr, len) => { elements[Number(h)].textContent = readStr(ptr, len); },
      host_dom_get_text: (h) => giveStr(elements[Number(h)].textContent ?? ""),
      host_dom_set_html: (h, ptr, len) => { elements[Number(h)].innerHTML = readStr(ptr, len); },
      host_dom_get_value: (h) => giveStr(elements[Number(h)].value ?? ""),
      host_dom_set_value: (h, ptr, len) => { elements[Number(h)].value = readStr(ptr, len); },
      host_dom_on: (h, ptr, len, id) => {
        const cb = Number(id);
        const ev = readStr(ptr, len);
        const el = elements[Number(h)];
        // Payload conventions (delivered to 1-argument handlers):
        //   "enter"  keydown filtered to Enter; payload = element value
        //   "click"  payload = the clicked target's id (delegation)
        //   "change" payload = target id + "\n" + target value
        if (ev === "enter") {
          el.addEventListener("keydown", (e) => {
            if (e.key === "Enter") dispatch(cb, el.value ?? "");
          });
        } else if (ev === "click") {
          el.addEventListener("click", (e) => dispatch(cb, e.target.id ?? ""));
        } else if (ev === "change") {
          el.addEventListener("change", (e) =>
            dispatch(cb, (e.target.id ?? "") + "\n" + (e.target.value ?? "")));
        } else {
          el.addEventListener(ev, () => dispatch(cb, null));
        }
      },
      host_dom_focus: (h) => { elements[Number(h)].focus(); },
      host_dom_fetch: (mp, ml, pp, pl, bp, bl, id) => {
        const method = readStr(mp, ml);
        const path = readStr(pp, pl);
        const body = readStr(bp, bl);
        const cb = Number(id);
        fetch(path, {
          method,
          headers: body ? { "Content-Type": "application/json" } : {},
          body: body || undefined,
        })
          .then((r) => r.text())
          .then((text) => dispatch(cb, text))
          .catch((e) => dispatch(cb, JSON.stringify({ error: String(e) })));
      },
    },
  };

  const [wasmBytes, source] = await Promise.all([
    fetch("/olang.wasm").then((r) => r.arrayBuffer()),
    fetch("/app.ol").then((r) => r.text()),
  ]);
  if (wasmBytes.byteLength < 8) {
    document.body.insertAdjacentHTML(
      "beforeend",
      `<pre style="color:#c33;padding:1rem">/olang.wasm came back empty.
Two known causes:
  1. the server binary predates body_file support — reinstall olang (make install)
  2. static/olang_playground.wasm is missing — build it:
     cargo build -p olang-playground --target wasm32-unknown-unknown --release
     cp target/wasm32-unknown-unknown/release/olang_playground.wasm examples/app/static/</pre>`
    );
    return;
  }
  ({ instance: { exports: ex } } = await WebAssembly.instantiate(wasmBytes, imports));

  const enc = new TextEncoder().encode(source);
  const ptr = ex.olang_alloc(enc.length);
  mem().set(enc, ptr);
  const boot = readResult(ex.olang_session_start(ptr, enc.length));
  ex.olang_dealloc(ptr, enc.length);
  if (boot.error) {
    document.body.insertAdjacentHTML(
      "beforeend",
      `<pre style="color:#c33">olang boot error: ${boot.error}</pre>`
    );
  }
})();
