// olang-dom.js — the page-side shim: makes this browser an olang host.
// Loads the wasm build, implements the dom host imports over the real
// DOM, boots a persistent session from the program image (or, failing
// that, its source), and routes events and fetch responses back into
// the live interpreter.

(async function () {
  // The shim runs as a module script, where `document.currentScript` is
  // always null — so the page's choices are read off the shim's own
  // <script> element, found by its src. (Reading currentScript silently
  // fell back for every attribute: the program path happened to match
  // the default, and the content-addressed wasm URL served no one.)
  const me =
    document.querySelector('script[src$="/olang-dom.js"]') ??
    document.querySelector("script[data-src], script[data-wasm]") ??
    document.currentScript;
  const src = me?.dataset?.src ?? "/app.ol";
  // The program image (data-bin): the bundle parsed on the server and
  // loaded here without parsing. The source at data-src is the
  // fallback — for a shell that offers no image, and for a runtime
  // whose version differs from the server's (an image names the olang
  // version that wrote it, and a runtime refuses another's).
  const bin = me?.dataset?.bin ?? null;
  // The runtime's URL: the shell passes the content-addressed form
  // (`/olang.<hash>.wasm`, immutable) so a repeat visit never asks the
  // server about it; the plain path is the fallback.
  const wasmUrl = me?.dataset?.wasm ?? "/olang.wasm";
  const bootMarks = { start: performance.now() };
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

  // A trap inside the runtime reaches the page as a bare
  // "RuntimeError: unreachable". The runtime's panic hook keeps the
  // message that preceded the trap, so it is read back and printed
  // before the error propagates — a report can then name the operation.
  function callRuntime(f) {
    try {
      return f();
    } catch (e) {
      const p = ex.olang_last_panic ? ex.olang_last_panic() : 0;
      if (p) {
        const len = new DataView(ex.memory.buffer).getUint32(p, true);
        console.error("olang panic:", new TextDecoder().decode(mem().slice(p + 4, p + 4 + len)));
        ex.olang_result_free(p);
      }
      throw e;
    }
  }

  // One dispatch at a time. A host call inside a handler can fire a DOM
  // event synchronously (focus → focusin, blur → change, click()), and
  // re-entering the runtime while the first dispatch is still on the
  // stack borrowed the session twice and killed the page. A dispatch
  // that arrives while one is running is queued and runs when the outer
  // one returns, in a microtask — never nested.
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
  function dispatch(id, payload) {
    enter(() => {
      if (payload == null) {
        readResult(callRuntime(() => ex.olang_dispatch_event(BigInt(id))));
      } else {
        const bytes = new TextEncoder().encode(payload);
        const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
        mem().set(bytes, ptr);
        readResult(callRuntime(() => ex.olang_dispatch_event_with(BigInt(id), ptr, bytes.length)));
        ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
      }

    });
  }

  // Structured events: the payload is a JSON object the wasm side parses
  // into the Map the olang handler receives.
  function dispatchJson(id, obj) {
    dispatchRawJson(id, JSON.stringify(obj));
  }

  // Same dispatch, but the payload is already JSON text (worker messages,
  // fetch_json responses) — no stringify round-trip.
  function dispatchRawJson(id, text) {
    enter(() => {
      const bytes = new TextEncoder().encode(text);
      const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
      mem().set(bytes, ptr);
      readResult(callRuntime(() => ex.olang_dispatch_event_json(BigInt(id), ptr, bytes.length)));
      ex.olang_dealloc(ptr, Math.max(bytes.length, 1));

    });
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
  const REQUEST_BIT = 2 ** 41;
  // Session state: dom.state_get/set live here (see the two host imports).
  const sessionState = {};

  // Web Workers: each handle is a second olang instance off the main
  // thread, bridged over postMessage. Values cross as JSON both ways.
  const workers = [null];

  // dom.on_frame callback ids, all serviced by one shared rAF loop.
  const frameCallbacks = [];

  // The one fetch path behind dom.fetch, dom.fetch_json, dom.request, and
  // dom.request_with.
  function doFetch(method, path, body, extra, id) {
    // Bit 40 marks a fetch_json callback: deliver the response through
    // the JSON dispatch so the handler receives a parsed value. Bit 41
    // marks a dom.request callback: the whole response — status,
    // headers, body — as one JSON value, so the handler can tell a
    // 404 from a 500 from a network failure (status 0).
    const raw = Number(id);
    const wantsStatus = raw >= REQUEST_BIT;
    const rest = wantsStatus ? raw - REQUEST_BIT : raw;
    const wantsJson = rest >= JSON_CALLBACK_BIT;
    const cb = wantsJson ? rest - JSON_CALLBACK_BIT : rest;
    const deliver = wantsJson
      ? (text) => dispatchRawJson(cb, text)
      : (text) => dispatch(cb, text);
    const request = fetch(path, {
      method,
      headers: Object.assign(body ? { "Content-Type": "application/json" } : {}, extra),
      body: body || undefined,
    });
    if (wantsStatus) {
      request
        .then(async (r) => dispatchRawJson(cb, JSON.stringify({
          status: r.status,
          headers: Object.fromEntries(r.headers.entries()),
          body: await r.text(),
        })))
        .catch((e) => dispatchRawJson(cb, JSON.stringify({
          status: 0, headers: {}, body: "", error: String(e),
        })));
      return;
    }
    request
      .then((r) => r.text())
      .then(deliver)
      .catch((e) => deliver(JSON.stringify({ error: String(e) })));
  }

  // Keyed reconciliation: bring `el`'s children to `html` by editing the
  // nodes that are already there — text updated in place, attributes
  // diffed, children matched by data-key (else by position and tag) —
  // instead of replacing the whole tree. What stays is what the browser
  // keeps: focus, caret, scroll position, an open dropdown.
  // dom.patch: the node tree as data, reconciled straight into the live
  // DOM — no markup to render on the wasm side, none to parse here.
  // Vnodes: { tag, attrs, children } | { text } | { raw } | { keep }.
  const SVG_NS = "http://www.w3.org/2000/svg";
  function patchInto(el, tree) {
    patchChildren(el, flatVNodes(tree.tag === "" ? tree.children : [tree]));
  }
  function flatVNodes(list, out = []) {
    for (const v of list) {
      if (v == null) continue;
      if (typeof v === "string") out.push({ text: v });
      else if (Array.isArray(v)) flatVNodes(v, out);
      else if (v.tag === "") flatVNodes(v.children || [], out);
      else if (v.raw !== undefined) {
        const tpl = document.createElement("template");
        tpl.innerHTML = String(v.raw);
        for (const n of tpl.content.childNodes) out.push({ dom: n });
      } else out.push(v);
    }
    return out;
  }
  const vkey = (v) => (v.attrs && v.attrs["data-key"] != null ? String(v.attrs["data-key"]) : null);
  function sameVKind(n, v) {
    if (v.text !== undefined) return n.nodeType === 3;
    if (v.dom) return false;
    return n.nodeType === 1 && n.tagName.toLowerCase() === String(v.tag).toLowerCase();
  }
  function patchChildren(from, wanted) {
    const keyed = new Map();
    for (const c of from.childNodes) { const k = keyOf(c); if (k) keyed.set(k, c); }
    let i = 0;
    for (const v of wanted) {
      const at = from.childNodes[i] || null;
      if (v.keep !== undefined) {
        // An unchanged memo subtree: the element it rendered last time
        // stays as it is, moved into place if the order changed.
        const m = keyed.get(String(v.keep));
        if (m) { if (m !== at) from.insertBefore(m, at); i++; }
        continue;
      }
      if (v.dom) {
        // Trusted markup: replaced wholesale, never diffed.
        if (at) from.replaceChild(v.dom.cloneNode(true), at); else from.appendChild(v.dom.cloneNode(true));
        i++;
        continue;
      }
      const k = vkey(v);
      let match = null;
      if (k && keyed.has(k)) match = keyed.get(k);
      else if (at && !keyOf(at) && !k && sameVKind(at, v)) match = at;
      if (match) {
        if (match !== at) from.insertBefore(match, at);
        patchNode(match, v);
      } else {
        from.insertBefore(createVNode(v, from), at);
      }
      i++;
    }
    while (from.childNodes.length > i) from.removeChild(from.lastChild);
  }
  function createVNode(v, parent) {
    if (v.text !== undefined) return document.createTextNode(String(v.text));
    const ns = v.tag === "svg" || (parent && parent.namespaceURI === SVG_NS) ? SVG_NS : null;
    const el = ns ? document.createElementNS(ns, v.tag) : document.createElement(v.tag);
    patchNode(el, v);
    return el;
  }
  function patchNode(from, v) {
    if (from.nodeType === 3) {
      const s = String(v.text);
      if (from.data !== s) from.data = s;
      return;
    }
    const attrs = v.attrs || {};
    const tag = from.tagName;
    const control = tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
    for (const name of Object.keys(attrs)) {
      const raw = attrs[name];
      if (raw === false || raw === null || raw === undefined) {
        if (from.hasAttribute(name)) from.removeAttribute(name);
        continue;
      }
      const value = raw === true ? "" : String(raw);
      if (from.getAttribute(name) !== value) from.setAttribute(name, value);
    }
    for (const { name } of [...from.attributes]) {
      if (!(name in attrs) || attrs[name] === false || attrs[name] == null) from.removeAttribute(name);
    }
    if (control) {
      // The control someone is typing in keeps its live value; every
      // other control follows the data.
      if (document.activeElement !== from) {
        if (tag === "INPUT" && (attrs.type === "checkbox" || attrs.type === "radio")) {
          from.checked = attrs.checked === true || (attrs.checked != null && attrs.checked !== false);
        } else if (attrs.value !== undefined) {
          const value = attrs.value == null || attrs.value === false ? "" : String(attrs.value);
          if (from.value !== value) from.value = value;
        }
      }
    }
    if (tag === "TEXTAREA") {
      if (document.activeElement === from) return;
      const inner = flatVNodes(v.children || []).map((c) => c.text ?? "").join("");
      if (from.value !== inner) from.value = inner;
      return;
    }
    patchChildren(from, flatVNodes(v.children || []));
  }
  function morphInto(el, html) {
    const tpl = document.createElement("template");
    tpl.innerHTML = html;
    morphChildren(el, tpl.content);
  }
  const sameKind = (a, b) =>
    a.nodeType === b.nodeType && (a.nodeType !== 1 || a.tagName === b.tagName);
  const keyOf = (n) => (n.nodeType === 1 && n.dataset && n.dataset.key) || null;
  function morphChildren(from, to) {
    const keyed = new Map();
    for (const c of from.childNodes) { const k = keyOf(c); if (k) keyed.set(k, c); }
    const wanted = [...to.childNodes];
    for (let i = 0; i < wanted.length; i++) {
      const t = wanted[i];
      const at = from.childNodes[i] || null;
      const k = keyOf(t);
      let match = null;
      if (k && keyed.has(k)) match = keyed.get(k);
      else if (at && !keyOf(at) && !k && sameKind(at, t)) match = at;
      if (match) {
        if (match !== at) from.insertBefore(match, at);
        morphNode(match, t);
      } else {
        from.insertBefore(t.cloneNode(true), at);
      }
    }
    while (from.childNodes.length > wanted.length) from.removeChild(from.lastChild);
  }
  function morphNode(from, to) {
    if (from.nodeType === 3 || from.nodeType === 8) {
      if (from.data !== to.data) from.data = to.data;
      return;
    }
    if (from.nodeType !== 1) return;
    for (const { name, value } of [...to.attributes]) {
      if (from.getAttribute(name) !== value) from.setAttribute(name, value);
    }
    for (const { name } of [...from.attributes]) {
      if (!to.hasAttribute(name)) from.removeAttribute(name);
    }
    const tag = from.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") {
      // The control someone is typing in keeps its live value; every
      // other control follows the markup.
      if (document.activeElement !== from) {
        if (tag === "INPUT" && (to.type === "checkbox" || to.type === "radio")) {
          from.checked = to.checked;
        } else if (from.value !== to.value) {
          from.value = to.value;
        }
      }
    }
    if (tag === "TEXTAREA" && document.activeElement === from) return;
    morphChildren(from, to);
  }

  // Read every file of a paste or drop to base64, then hand the payload
  // on with `files: [{ name, type, size, base64 }]` (empty when none).
  function withFiles(fileList, payload, k) {
    const files = fileList ? [...fileList] : [];
    if (!files.length) { payload.files = []; k(payload); return; }
    let pending = files.length;
    const out = new Array(files.length);
    files.forEach((file, i) => {
      const reader = new FileReader();
      reader.onload = () => {
        const url = String(reader.result);
        out[i] = { name: file.name, type: file.type, size: file.size, base64: url.slice(url.indexOf(",") + 1) };
        if (--pending === 0) { payload.files = out; k(payload); }
      };
      reader.onerror = () => {
        out[i] = { name: file.name, type: file.type, size: file.size, base64: "", error: String(reader.error) };
        if (--pending === 0) { payload.files = out; k(payload); }
      };
      reader.readAsDataURL(file);
    });
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
      host_dom_query_all: (ptr, len) =>
        giveStr(JSON.stringify([...document.querySelectorAll(readStr(ptr, len))].map(handleOf))),
      host_dom_set_text: (h, ptr, len) => { elements[Number(h)].textContent = readStr(ptr, len); },
      host_dom_get_text: (h) => giveStr(elements[Number(h)].textContent ?? ""),
      host_dom_set_html: (h, ptr, len) => { elements[Number(h)].innerHTML = readStr(ptr, len); },
      host_dom_morph: (h, ptr, len) => morphInto(elements[Number(h)], readStr(ptr, len)),
      host_dom_patch: (h, ptr, len) => patchInto(elements[Number(h)], JSON.parse(readStr(ptr, len))),
      host_dom_checked: (h) => (elements[Number(h)].checked ? 1n : 0n),
      host_dom_selection: (h) => {
        const el = elements[Number(h)];
        const from = el.selectionStart, to = el.selectionEnd;
        return giveStr(JSON.stringify([from == null ? 0 : from, to == null ? 0 : to]));
      },
      host_dom_set_selection: (h, from, to) => {
        const el = elements[Number(h)];
        if (el.setSelectionRange) { el.focus(); el.setSelectionRange(Number(from), Number(to)); }
      },
      host_dom_values: (h) => {
        const el = elements[Number(h)];
        const opts = el.selectedOptions ? [...el.selectedOptions].map((o) => o.value) : [];
        return giveStr(JSON.stringify(opts));
      },
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
          // browser only permits a drop where dragover is cancelled. The
          // dropped files travel in the payload as `files`, each read to
          // base64 — the shape dom.read_file hands back.
          el.addEventListener("dragover", (e) => e.preventDefault());
          el.addEventListener("drop", (e) => {
            e.preventDefault();
            withFiles(e.dataTransfer && e.dataTransfer.files, eventPayload(e, "drop"), (p) => dispatchJson(cb, p));
          });
        } else if (ev === "paste") {
          // A paste with files (an image from the clipboard) carries them
          // as `files`; a text paste carries `text` and is not prevented.
          el.addEventListener("paste", (e) => {
            const files = e.clipboardData && e.clipboardData.files;
            const p = eventPayload(e, "paste");
            p.text = (e.clipboardData && e.clipboardData.getData("text/plain")) || "";
            if (files && files.length) e.preventDefault();
            withFiles(files, p, (payload) => dispatchJson(cb, payload));
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
      host_dom_prefers_dark: () =>
        (window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches) ? 1 : 0,
      host_dom_active_id: () => giveStr((document.activeElement && document.activeElement.id) || ""),
      host_dom_confirm: (ptr, len) => (window.confirm(readStr(ptr, len)) ? 1 : 0),
      // A file input's first file, delivered as JSON through the same
      // dispatch a fetch_json callback uses: { name, size, type, base64 }.
      host_dom_read_file: (h, id) => {
        const el = elements[Number(h)];
        const file = el && el.files && el.files[0];
        const cb = Number(id);
        if (!file) { dispatchRawJson(cb, JSON.stringify({ error: "no file selected" })); return; }
        const reader = new FileReader();
        reader.onload = () => {
          const url = String(reader.result);
          const base64 = url.slice(url.indexOf(",") + 1);
          dispatchRawJson(cb, JSON.stringify({ name: file.name, size: file.size, type: file.type, base64 }));
        };
        reader.onerror = () => dispatchRawJson(cb, JSON.stringify({ error: String(reader.error) }));
        reader.readAsDataURL(file);
      },
      host_dom_set_class: (h, ptr, len) => { elements[Number(h)].className = readStr(ptr, len); },
      host_dom_fetch: (mp, ml, pp, pl, bp, bl, id) =>
        doFetch(readStr(mp, ml), readStr(pp, pl), readStr(bp, bl), {}, id),
      // dom.request_with: the same request with extra headers (a bearer
      // token, a content type other than JSON).
      host_dom_fetch_with: (mp, ml, pp, pl, bp, bl, hp, hl, id) => {
        let extra = {};
        try { extra = JSON.parse(readStr(hp, hl)) || {}; } catch (e) { extra = {}; }
        doFetch(readStr(mp, ml), readStr(pp, pl), readStr(bp, bl), extra, id);
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
            w.postMessage({ boot: { wasmUrl, source } }))
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
  // Every host_dom_* import runs its JavaScript under try/catch: an
  // exception there (an invalid selector, a selection range on an element
  // that has none) used to unwind through the runtime with the session
  // still borrowed. The message waits in `hostError`; the runtime reads it
  // through host_take_error after each dom call and raises it as an olang
  // error the handler can `attempt`.
  let hostError = null;
  const HANDLE_IMPORTS = new Set(["host_dom_query", "host_dom_create", "host_dom_set_interval",
    "host_dom_worker_spawn", "host_dom_prefers_dark", "host_dom_confirm", "host_dom_checked"]);
  const STRING_IMPORTS = new Set(["host_dom_query_all", "host_dom_get_text", "host_dom_get_value",
    "host_dom_get_attr", "host_dom_measure", "host_dom_location", "host_dom_storage_get",
    "host_dom_state_get", "host_dom_active_id", "host_dom_selection", "host_dom_values"]);
  for (const name of Object.keys(imports.env)) {
    if (!name.startsWith("host_dom_")) continue;
    const f = imports.env[name];
    imports.env[name] = (...a) => {
      try { return f(...a); }
      catch (e) {
        hostError = e && e.message ? e.message : String(e);
        return HANDLE_IMPORTS.has(name) ? 0n : STRING_IMPORTS.has(name) ? giveStr("") : undefined;
      }
    };
  }
  imports.env.host_take_error = () => {
    if (hostError == null) return 0;
    const m = hostError;
    hostError = null;
    return giveStr(m);
  };

  async function instantiateWasm() {
    if (WebAssembly.instantiateStreaming) {
      try {
        return await WebAssembly.instantiateStreaming(fetch(wasmUrl), imports);
      } catch (e) { /* buffered fallback below */ }
    }
    const wasmBytes = await fetch(wasmUrl).then((r) => r.arrayBuffer());
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

  async function fetchProgram() {
    if (bin) {
      try {
        const r = await fetch(bin);
        if (r.ok) return { image: new Uint8Array(await r.arrayBuffer()) };
      } catch (e) { /* the source below */ }
    }
    return { source: await fetch(src).then((r) => r.text()) };
  }
  const fetchSource = () => fetch(src).then((r) => r.text());

  const [wasmModule, program] = await Promise.all([instantiateWasm(), fetchProgram()]);
  ({ instance: { exports: ex } } = wasmModule);
  bootMarks.instantiated = performance.now();

  // Yield once between instantiation and the session start: loading
  // and running the bundle is one synchronous call, and without this
  // frame nothing paints — not even the shell — until it ends.
  await new Promise((resolve) => setTimeout(resolve, 0));

  function startSession(bytes, entry) {
    const ptr = ex.olang_alloc(Math.max(bytes.length, 1));
    mem().set(bytes, ptr);
    const r = readResult(callRuntime(() => entry(ptr, bytes.length)));
    ex.olang_dealloc(ptr, Math.max(bytes.length, 1));
    return r;
  }
  let boot, programKind;
  // A runtime built before images exist has no binary entry; it reads
  // the source like it always did.
  if (program.image && ex.olang_session_start_bin) {
    boot = startSession(program.image, ex.olang_session_start_bin);
    programKind = "image";
    if (boot.retry_with_source) {
      console.warn("olang: the program image is not loadable by this runtime; reading the source instead");
      boot = startSession(new TextEncoder().encode(await fetchSource()), ex.olang_session_start);
      programKind = "source";
    }
  } else {
    const source = program.source ?? (await fetchSource());
    boot = startSession(new TextEncoder().encode(source), ex.olang_session_start);
    programKind = "source";
  }
  bootMarks.started = performance.now();
  // Boot-phase timings, so an app can see what it pays for:
  // window.olangBoot = { fetch_instantiate_ms, session_start_ms, total_ms,
  //   program ("image" | "source"), load_ms (decode or parse), run_ms
  //   (the bundle's top level, the first render included) }.
  // Where a repaint's time goes: `olangProfile.start()`, act, then
  // `olangProfile.table()` — every olang function that ran, with its
  // tier, calls, and exact self and total milliseconds (instrumented,
  // not sampled). `report()` returns the rows; `stop()` also turns the
  // instrumentation off.
  window.olangProfile = {
    start: () => ex.olang_profile_start(),
    report: () => readResult(ex.olang_profile_report()),
    stop: () => readResult(ex.olang_profile_stop()),
    table: (top = 25) => {
      const r = readResult(ex.olang_profile_report());
      console.table(r.rows.slice(0, top));
      return r;
    },
  };
  window.olangBoot = {
    fetch_instantiate_ms: Math.round(bootMarks.instantiated - bootMarks.start),
    session_start_ms: Math.round(bootMarks.started - bootMarks.instantiated),
    total_ms: Math.round(bootMarks.started - bootMarks.start),
    program: programKind,
    load_ms: boot.load_ms ?? null,
    run_ms: boot.run_ms ?? null,
  };
  console.debug("olang boot", window.olangBoot);
  if (boot.error) {
    document.body.insertAdjacentHTML(
      "beforeend",
      `<pre style="color:#c33">olang boot error: ${boot.error}</pre>`
    );
  }
})();
