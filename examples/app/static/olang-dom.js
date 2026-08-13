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

  // Structured events: the payload is a JSON object the wasm side parses
  // into the Map the olang handler receives.
  function dispatchJson(id, obj) {
    const bytes = new TextEncoder().encode(JSON.stringify(obj));
    const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
    mem().set(bytes, ptr);
    readResult(ex.olang_dispatch_event_json(BigInt(id), ptr, bytes.length));
    ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
  }

  // Every DOM event delivers the same shape; handlers pick what they use.
  function eventPayload(e, type) {
    const t = e.target ?? {};
    return {
      type,
      id: t.id ?? "",
      value: t.value ?? "",
      key: e.key ?? "",
      x: Math.round(e.clientX ?? 0),
      y: Math.round(e.clientY ?? 0),
      alt: !!e.altKey, ctrl: !!e.ctrlKey, shift: !!e.shiftKey, meta: !!e.metaKey,
      data: { ...(t.dataset ?? {}) },
    };
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
        // Every handler receives a structured event Map: type, target id,
        // value, key, pointer x/y, modifier flags, and data-* attributes.
        // "enter" stays as the keydown-filtered alias; delegation is the
        // model throughout (one listener per container, rebind-free).
        if (ev === "enter") {
          el.addEventListener("keydown", (e) => {
            if (e.key === "Enter") {
              const p = eventPayload(e, "enter");
              p.value = el.value ?? "";
              dispatchJson(cb, p);
            }
          });
        } else {
          el.addEventListener(ev, (e) => dispatchJson(cb, eventPayload(e, ev)));
        }
      },
      host_dom_focus: (h) => { elements[Number(h)].focus(); },
      host_dom_set_class: (h, ptr, len) => { elements[Number(h)].className = readStr(ptr, len); },
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
      host_dom_get_attr: (h, ptr, len) =>
        giveStr(elements[Number(h)].getAttribute(readStr(ptr, len)) ?? ""),
      host_dom_set_attr: (h, np, nl, vp, vl) => {
        elements[Number(h)].setAttribute(readStr(np, nl), readStr(vp, vl));
      },
      host_dom_remove_attr: (h, ptr, len) => {
        elements[Number(h)].removeAttribute(readStr(ptr, len));
      },
      host_dom_class_op: (h, op, ptr, len) => {
        const cl = elements[Number(h)].classList;
        const name = readStr(ptr, len);
        if (Number(op) === 0) cl.add(name);
        else if (Number(op) === 1) cl.remove(name);
        else cl.toggle(name);
      },
      host_dom_set_style: (h, np, nl, vp, vl) => {
        elements[Number(h)].style.setProperty(readStr(np, nl), readStr(vp, vl));
      },
      host_dom_measure: (h) => {
        const r = elements[Number(h)].getBoundingClientRect();
        return giveStr(JSON.stringify({
          x: r.x, y: r.y, width: r.width, height: r.height,
        }));
      },
      host_dom_create: (ptr, len) => {
        try {
          return BigInt(handleOf(document.createElement(readStr(ptr, len))));
        } catch {
          return 0n;
        }
      },
      host_dom_append: (p, c) => {
        elements[Number(p)].appendChild(elements[Number(c)]);
      },
      host_dom_remove: (h) => { elements[Number(h)].remove(); },
      host_dom_scroll_into_view: (h) => {
        elements[Number(h)].scrollIntoView({ block: "nearest" });
      },
      host_dom_set_timeout: (ms, id) => {
        setTimeout(() => dispatchJson(Number(id), { type: "timeout" }), ms);
      },
      host_dom_set_interval: (ms, id) => {
        return BigInt(setInterval(
          () => dispatchJson(Number(id), { type: "interval" }), ms));
      },
      host_dom_clear_interval: (t) => { clearInterval(Number(t)); },
      host_dom_request_frame: (id) => {
        const start = performance.now();
        requestAnimationFrame((now) =>
          dispatchJson(Number(id), { type: "frame", delta: now - start }));
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
