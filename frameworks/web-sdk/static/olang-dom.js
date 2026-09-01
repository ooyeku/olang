// olang-dom.js — the page-side shim: makes this browser an olang host.
// Loads the wasm build, implements the dom host imports over the real
// DOM, boots a persistent session from /app.ol, and routes events and
// fetch responses back into the live interpreter.

(async function () {
  // currentScript is only valid during synchronous execution — capture
  // the page's chosen program before the first await.
  const src = document.currentScript?.dataset?.src ?? "/app.ol";
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

  // Canvas contexts render at devicePixelRatio: the backing store
  // scales up once and the context pre-scales, so every draw call keeps
  // working in design units while the pixels stay retina-crisp. The
  // ratio is capped at 2, and a canvas can opt down with
  // data-olang-dpr="1.5" — for a per-frame animated piece, a smaller
  // backing store cuts GPU fill rate quadratically and motion hides
  // the softness.
  function ctx2d(el) {
    if (!el.__olangCtx) {
      const attr = parseFloat(el.dataset && el.dataset.olangDpr);
      const dpr = attr > 0 ? attr : Math.min(window.devicePixelRatio || 1, 2);
      const w = el.width, h = el.height;
      if (!el.style.width) {
        el.style.width = w + "px";
        el.style.height = h + "px";
      }
      if (dpr !== 1) {
        el.width = Math.round(w * dpr);
        el.height = Math.round(h * dpr);
      }
      el.__olangCtx = el.getContext("2d");
      if (el.__olangCtx && dpr !== 1) el.__olangCtx.scale(dpr, dpr);
      el.__olangW = w;
      el.__olangH = h;
      watchVisibility(el);
    }
    return el.__olangCtx;
  }

  // Offscreen canvases don't paint. An IntersectionObserver tracks
  // whether each draw target is actually in the viewport; draw and
  // draw_points become no-ops for one that isn't. A page can animate
  // several heavy canvases and only ever pay for what's on screen —
  // scrolled away, the GPU cost is zero. (Feature-detected: in a
  // headless host without IntersectionObserver everything counts as
  // visible.)
  let visObserver = null;
  function watchVisibility(el) {
    if (typeof IntersectionObserver === "undefined") return;
    if (!visObserver) {
      visObserver = new IntersectionObserver((entries) => {
        for (const e of entries) e.target.__olangOffscreen = !e.isIntersecting;
      });
    }
    visObserver.observe(el);
  }
  const canPaint = (el) => !el.__olangOffscreen;

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
    dispatchRawJson(id, JSON.stringify(obj));
  }

  // Same dispatch, but the payload is already JSON text (worker messages,
  // fetch_json responses) — no stringify round-trip.
  function dispatchRawJson(id, text) {
    const bytes = new TextEncoder().encode(text);
    const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
    mem().set(bytes, ptr);
    readResult(ex.olang_dispatch_event_json(BigInt(id), ptr, bytes.length));
    ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
  }

  // Every DOM event delivers the same shape; handlers pick what they use.
  // `data` walks up to the nearest [data-action] carrier (self first),
  // so a click landing on a styled child still names its action;
  // `zone` walks to the nearest [data-zone] container — how a drop (or
  // any region-scoped handler) learns which region it happened in.
  function eventPayload(e, type) {
    const t = e.target ?? {};
    const a = (t.closest && t.closest("[data-action]")) || t;
    const z = (t.closest && t.closest("[data-zone]")) || null;
    return {
      type,
      id: t.id ?? "",
      value: t.value ?? "",
      key: e.key ?? "",
      tag: (t.tagName ?? "").toLowerCase(),
      x: Math.round(e.clientX ?? 0),
      y: Math.round(e.clientY ?? 0),
      alt: !!e.altKey, ctrl: !!e.ctrlKey, shift: !!e.shiftKey, meta: !!e.metaKey,
      data: { ...(a.dataset ?? {}) },
      zone: { ...((z && z.dataset) ?? {}) },
    };
  }

  const JSON_CALLBACK_BIT = 2 ** 40;
  // Session state: dom.state_get/set live here (see the two host imports).
  const sessionState = {};

  // Web Workers: each handle is a second olang instance off the main
  // thread, bridged over postMessage. Values cross as JSON both ways.
  const workers = [null];

  // dom.on_frame callback ids, all serviced by one shared rAF loop.
  const frameCallbacks = [];

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
        } else if (ev === "drop") {
          // Subscribing to "drop" makes the element a drop zone: the
          // browser only permits a drop where dragover is cancelled.
          el.addEventListener("dragover", (e) => e.preventDefault());
          el.addEventListener("drop", (e) => {
            e.preventDefault();
            dispatchJson(cb, eventPayload(e, "drop"));
          });
        } else if (ev === "dragstart") {
          el.addEventListener("dragstart", (e) => {
            // Firefox refuses to drag until dataTransfer holds data;
            // the payload itself crosses through the event map.
            if (e.dataTransfer) {
              e.dataTransfer.setData("text/plain", "olang-drag");
              e.dataTransfer.effectAllowed = "move";
            }
            dispatchJson(cb, eventPayload(e, "dragstart"));
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
        // Bit 40 marks a fetch_json callback: deliver the response through
        // the JSON dispatch so the handler receives a parsed value.
        const raw = Number(id);
        const wantsJson = raw >= JSON_CALLBACK_BIT;
        const cb = wantsJson ? raw - JSON_CALLBACK_BIT : raw;
        const deliver = wantsJson
          ? (text) => dispatchRawJson(cb, text)
          : (text) => dispatch(cb, text);
        fetch(path, {
          method,
          headers: body ? { "Content-Type": "application/json" } : {},
          body: body || undefined,
        })
          .then((r) => r.text())
          .then(deliver)
          .catch((e) => deliver(JSON.stringify({ error: String(e) })));
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
      host_dom_insert_before: (p, c, b) => {
        const parent = elements[Number(p)];
        const child = elements[Number(c)];
        const before = Number(b) ? elements[Number(b)] : null;
        parent.insertBefore(child, before);
      },
      host_dom_push_state: (ptr, len) => {
        history.pushState({}, "", readStr(ptr, len));
      },
      host_dom_location: () =>
        giveStr(JSON.stringify({
          path: location.pathname,
          query: Object.fromEntries(new URLSearchParams(location.search)),
        })),
      host_dom_on_route: (id) => {
        const cb = Number(id);
        window.addEventListener("popstate", () =>
          dispatchJson(cb, {
            type: "route",
            path: location.pathname,
            query: Object.fromEntries(new URLSearchParams(location.search)),
          }));
      },
      host_dom_storage_get: (ptr, len) =>
        giveStr(localStorage.getItem(readStr(ptr, len)) ?? ""),
      host_dom_storage_set: (kp, kl, vp, vl) => {
        localStorage.setItem(readStr(kp, kl), readStr(vp, vl));
      },
      host_dom_storage_remove: (ptr, len) => {
        localStorage.removeItem(readStr(ptr, len));
      },
      // Session state: an in-memory, page-lifetime store (not persisted
      // — that is localStorage above). Values are JSON strings the
      // olang side encodes/decodes, so the store itself is dumb.
      host_dom_state_get: (ptr, len) => giveStr(sessionState[readStr(ptr, len)] ?? ""),
      host_dom_state_set: (kp, kl, vp, vl) => {
        sessionState[readStr(kp, kl)] = readStr(vp, vl);
      },
      // All frame callbacks share ONE rAF loop, throttled to ~60fps.
      // Per-callback chains on a 120Hz display doubled every cost for
      // no visible gain — these are data animations, not games — and
      // N callbacks meant N interleaved rAF timers. The 14ms floor
      // passes every tick on a 60Hz display and every second tick on
      // ProMotion's 120Hz.
      host_dom_on_frame: (id) => {
        frameCallbacks.push(Number(id));
        if (frameCallbacks.length === 1) {
          let last = performance.now();
          const tick = (now) => {
            if (now - last >= 14) {
              const delta = now - last;
              last = now;
              for (const cb of frameCallbacks)
                dispatchJson(cb, { type: "frame", delta });
            }
            requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        }
      },
      host_dom_worker_spawn: (ptr, len) => {
        const path = readStr(ptr, len);
        const w = new Worker("/olang-worker.js");
        const entry = { w, cb: null, inbox: [] };
        w.onmessage = (e) => {
          if (e.data.olang == null) return;
          if (entry.cb == null) entry.inbox.push(e.data.olang);
          else dispatchRawJson(entry.cb, e.data.olang);
        };
        fetch(path)
          .then((r) => (r.ok ? r.text() : Promise.reject(r.status)))
          .then((source) =>
            w.postMessage({ boot: { wasmUrl: "/olang.wasm", source } }))
          .catch((e) => console.error("olang worker boot:", path, e));
        return BigInt(workers.push(entry) - 1);
      },
      host_dom_worker_send: (h, ptr, len) => {
        workers[Number(h)]?.w.postMessage({ olang: readStr(ptr, len) });
      },
      host_dom_worker_on: (h, id) => {
        const entry = workers[Number(h)];
        if (!entry) return;
        entry.cb = Number(id);
        while (entry.inbox.length) dispatchRawJson(entry.cb, entry.inbox.shift());
      },
      host_dom_worker_close: (h) => {
        workers[Number(h)]?.w.terminate();
        workers[Number(h)] = null;
      },
      // dom.post / dom.on_message only exist inside a worker; on the page
      // they are inert (the worker harness implements the live versions).
      host_dom_post: () => {},
      host_dom_on_message: () => {},
      // The bulk point path: one packed f64 buffer, read as a
      // zero-copy typed-array view; an optional affine (sx,sy,tx,ty)
      // maps data coordinates to pixels host-side.
      host_dom_draw_points: (h, ptr, n, sp, sl) => {
        const el = elements[Number(h)];
        const ctx = ctx2d(el);
        if (!ctx || !canPaint(el)) return;
        const style = JSON.parse(readStr(sp, sl));
        const pts = new Float64Array(ex.memory.buffer, Number(ptr), n * 2);
        const sx = style.sx ?? 1, sy = style.sy ?? 1;
        const tx = style.tx ?? 0, ty = style.ty ?? 0;
        // Optional rotation (radians) around the data origin, applied
        // before scale/translate — spinning a point cloud costs one
        // parameter, not a recompute.
        const cr = Math.cos(style.rot ?? 0), sr = Math.sin(style.rot ?? 0);
        const color = style.color ?? "#5aa9e6";
        if (style.alpha != null) ctx.globalAlpha = style.alpha;
        if (style.mode === "path") {
          ctx.beginPath();
          for (let i = 0; i < n; i++) {
            const px = pts[2 * i], py = pts[2 * i + 1];
            const x = (px * cr - py * sr) * sx + tx, y = (px * sr + py * cr) * sy + ty;
            i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
          }
          ctx.strokeStyle = color;
          ctx.lineWidth = style.size ?? 1;
          ctx.stroke();
        } else {
          ctx.fillStyle = color;
          const r = style.size ?? 1.5;
          for (let i = 0; i < n; i++) {
            const px = pts[2 * i], py = pts[2 * i + 1];
            ctx.fillRect((px * cr - py * sr) * sx + tx - r / 2, (px * sr + py * cr) * sy + ty - r / 2, r, r);
          }
        }
        if (style.alpha != null) ctx.globalAlpha = 1;
      },
      // The draw-list: one JSON scene per call, replayed onto Canvas 2D.
      host_dom_draw: (h, ptr, len) => {
        const el = elements[Number(h)];
        const ctx = ctx2d(el);
        if (!ctx || !canPaint(el)) return;
        const paint = (op, fillStroke) => {
          if (op.fill != null) { ctx.fillStyle = op.fill; fillStroke.fill(); }
          if (op.stroke != null) {
            ctx.strokeStyle = op.stroke;
            ctx.lineWidth = op.line_width ?? 1;
            fillStroke.stroke();
          }
        };
        for (const op of JSON.parse(readStr(ptr, len))) {
          switch (op.op) {
            case "clear":
              if (op.color != null) {
                ctx.fillStyle = op.color;
                ctx.fillRect(0, 0, el.__olangW, el.__olangH);
              } else ctx.clearRect(0, 0, el.__olangW, el.__olangH);
              break;
            case "rect":
              paint(op, {
                fill: () => ctx.fillRect(op.x, op.y, op.w, op.h),
                stroke: () => ctx.strokeRect(op.x, op.y, op.w, op.h),
              });
              break;
            case "circle":
              ctx.beginPath();
              ctx.arc(op.x, op.y, op.r, 0, Math.PI * 2);
              paint(op, { fill: () => ctx.fill(), stroke: () => ctx.stroke() });
              break;
            case "line":
              ctx.beginPath();
              ctx.moveTo(op.x1, op.y1);
              ctx.lineTo(op.x2, op.y2);
              ctx.strokeStyle = op.stroke ?? "#000";
              ctx.lineWidth = op.line_width ?? 1;
              ctx.stroke();
              break;
            case "path":
              ctx.beginPath();
              (op.points ?? []).forEach(([x, y], i) =>
                i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y));
              if (op.close) ctx.closePath();
              paint(op, { fill: () => ctx.fill(), stroke: () => ctx.stroke() });
              break;
            case "text":
              if (op.font != null) ctx.font = op.font;
              if (op.align != null) ctx.textAlign = op.align;
              ctx.fillStyle = op.fill ?? "#000";
              ctx.fillText(op.text ?? "", op.x, op.y);
              break;
            case "save": ctx.save(); break;
            case "restore": ctx.restore(); break;
            case "translate": ctx.translate(op.x, op.y); break;
            case "rotate": ctx.rotate(op.rad); break;
            case "scale": ctx.scale(op.x, op.y); break;
          }
        }
      },
    },
  };

  // Streaming instantiation compiles the module WHILE it downloads —
  // for a multi-megabyte runtime that overlap is most of the boot.
  // Fall back to the buffered path (with its diagnostics) when
  // streaming is unavailable or refuses (wrong MIME, old server).
  async function instantiateWasm() {
    if (WebAssembly.instantiateStreaming) {
      try {
        return await WebAssembly.instantiateStreaming(fetch("/olang.wasm"), imports);
      } catch (e) { /* buffered fallback below */ }
    }
    const wasmBytes = await fetch("/olang.wasm").then((r) => r.arrayBuffer());
    if (wasmBytes.byteLength < 8) {
      document.body.insertAdjacentHTML(
        "beforeend",
        `<pre style="color:#c33;padding:1rem">/olang.wasm came back empty.
Two known causes:
  1. the server binary predates body_file support — reinstall olang (make install)
  2. static/olang_playground.wasm is missing — build it:
     cargo build -p olang-playground --target wasm32-unknown-unknown --release
     cp target/wasm32-unknown-unknown/release/olang_playground.wasm examples/web/app/static/</pre>`
      );
      throw new Error("empty wasm");
    }
    return WebAssembly.instantiate(wasmBytes, imports);
  }

  const [wasmModule, source] = await Promise.all([
    instantiateWasm(),
    fetch(src).then((r) => r.text()),
  ]);
  ({ instance: { exports: ex } } = wasmModule);

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
