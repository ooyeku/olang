// tracker frontend: a spreadsheet over the tracker API. Plain JS, no deps.
//
// The full backend surface, polished: server-driven search/filter/sort/
// paging with the view state mirrored into the URL hash (refresh and
// share keep your filters); a comments drawer with ⌘-Enter submit; stats
// as CSS bars whose colors follow the status everywhere; a live activity
// feed with relative times; dark mode; keyboard shortcuts (/ search,
// n new issue, Esc closes); two-step delete; envelope-aware error
// toasts; and a 401 that prompts once for the bearer token.

const STATUSES = ["open", "in-progress", "done"];
const PRIORITIES = ["low", "medium", "high"];
const PAGE_SIZE = 50;

const query = { q: "", status: "", assignee: "", sort: "id", order: "asc", offset: 0 };
let issues = [];
let total = 0;
let drawerIssue = null;
let inFlight = 0;

const $ = id => document.getElementById(id);
const rowsEl = $("rows");

// ── theme ────────────────────────────────────────────────────────────

function applyTheme(t) {
  document.documentElement.dataset.theme = t;
  localStorage.setItem("tracker_theme", t);
}
applyTheme(localStorage.getItem("tracker_theme") ||
  (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"));
$("theme").addEventListener("click", () =>
  applyTheme(document.documentElement.dataset.theme === "dark" ? "light" : "dark"));

// ── view state ⇄ URL hash ────────────────────────────────────────────

function readHash() {
  const p = new URLSearchParams(location.hash.slice(1));
  for (const k of ["q", "status", "assignee", "sort", "order"]) {
    if (p.has(k)) query[k] = p.get(k);
  }
  if (p.has("offset")) query.offset = parseInt(p.get("offset"), 10) || 0;
  $("search").value = query.q;
  $("f-status").value = query.status;
  $("f-assignee").value = query.assignee;
}

function writeHash() {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v !== "" && !(k === "sort" && v === "id") && !(k === "order" && v === "asc") &&
        !(k === "offset" && v === 0)) p.set(k, v);
  }
  history.replaceState(null, "", p.size ? `#${p}` : location.pathname);
}

// ── API plumbing: loading bar, bearer token, the error envelope ──────

function loadingOn() { inFlight++; $("loading").classList.add("on"); }
function loadingOff() {
  if (--inFlight <= 0) { inFlight = 0; $("loading").classList.remove("on"); }
}

function authHeaders() {
  const token = localStorage.getItem("tracker_token");
  return token ? { "Authorization": `Bearer ${token}` } : {};
}

async function api(method, path, body, retried) {
  loadingOn();
  try {
    const res = await fetch(path, {
      method,
      headers: { ...(body ? { "Content-Type": "application/json" } : {}), ...authHeaders() },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (res.status === 401 && !retried) {
      const token = prompt("This tracker requires a write token (TRACKER_TOKEN):");
      if (token) {
        localStorage.setItem("tracker_token", token);
        return await api(method, path, body, true);
      }
    }
    const data = await res.json().catch(() => ({}));
    if (!res.ok) {
      const e = data.error || {};
      const details = (e.details || []).map(d => `${d.field}: ${d.message}`).join("; ");
      throw new Error(details ? `${e.message} — ${details}` : (e.message || `${method} ${path}: ${res.status}`));
    }
    return data;
  } finally { loadingOff(); }
}

function toast(message, kind = "err") {
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.textContent = message;
  $("toasts").append(el);
  setTimeout(() => el.remove(), kind === "err" ? 6000 : 3200);
}

// ── time, initials ───────────────────────────────────────────────────

function relativeTime(iso) {
  if (!iso) return "";
  const s = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

const initials = name => (name || "?").trim().slice(0, 2) || "?";

// ── loading the list ─────────────────────────────────────────────────

function listPath() {
  const p = new URLSearchParams();
  if (query.q) p.set("q", query.q);
  if (query.status) p.set("status", query.status);
  if (query.assignee) p.set("assignee", query.assignee);
  p.set("sort", query.sort);
  p.set("order", query.order);
  p.set("limit", PAGE_SIZE);
  p.set("offset", query.offset);
  return `/api/issues?${p}`;
}

async function load() {
  writeHash();
  try {
    const page = await api("GET", listPath());
    issues = page.items;
    total = page.total;
    render();
    refreshPanels();
  } catch (err) { toast(err.message); }
}

// ── the grid ─────────────────────────────────────────────────────────

function selectCell(kind, value, options, extraClass) {
  const sel = document.createElement("select");
  sel.classList.add("cell-pill");
  for (const o of options) {
    const opt = document.createElement("option");
    opt.value = o; opt.textContent = o;
    if (o === value) opt.selected = true;
    sel.append(opt);
  }
  sel.classList.add(`${extraClass}-${value}`);
  return sel;
}

function commit(id, col, value, refreshRow) {
  api("PATCH", `/api/issues/${id}`, { [col]: value }).then(updated => {
    const i = issues.findIndex(x => x.id === id);
    if (i >= 0) issues[i] = { ...issues[i], ...updated };
    if (refreshRow) render(); else updateFooter();
    refreshPanels();
  }).catch(err => { toast(err.message); load(); });
}

function inputCell(issue, col, opts = {}) {
  const td = document.createElement("td");
  const input = document.createElement("input");
  input.value = issue[col];
  if (opts.numeric) td.className = "num";
  if (col === "title") input.className = "title-input";
  input.addEventListener("change", () => {
    const value = opts.numeric ? (parseInt(input.value, 10) || 0) : input.value;
    commit(issue.id, col, value, false);
  });
  input.addEventListener("keydown", e => {
    if (e.key === "Enter") {
      input.blur();
      const rows = [...rowsEl.querySelectorAll("tr")];
      const rowIdx = rows.findIndex(r => r.dataset.id == issue.id);
      const next = rows[rowIdx + 1];
      if (next) {
        const target = next.querySelector(`[data-col="${col}"] input`);
        if (target) { target.focus(); target.select(); }
      } else {
        $("new-title").focus();
      }
    }
  });
  td.dataset.col = col;
  td.append(input);
  return td;
}

function render() {
  rowsEl.textContent = "";
  for (const issue of issues) {
    const tr = document.createElement("tr");
    tr.dataset.id = issue.id;
    tr.className = `status-${issue.status}`;

    const idTd = document.createElement("td");
    idTd.className = "id";
    idTd.textContent = issue.id;
    tr.append(idTd);

    tr.append(inputCell(issue, "title"));

    const statusTd = document.createElement("td");
    const statusSel = selectCell("status", issue.status, STATUSES, "status");
    statusSel.addEventListener("change", () => commit(issue.id, "status", statusSel.value, true));
    statusTd.append(statusSel);
    tr.append(statusTd);

    const priTd = document.createElement("td");
    const priSel = selectCell("priority", issue.priority, PRIORITIES, "pill");
    priSel.addEventListener("change", () => commit(issue.id, "priority", priSel.value, true));
    priTd.append(priSel);
    tr.append(priTd);

    tr.append(inputCell(issue, "assignee"));
    tr.append(inputCell(issue, "points", { numeric: true }));
    tr.append(inputCell(issue, "notes"));

    const talkTd = document.createElement("td");
    talkTd.className = "talk";
    const talk = document.createElement("button");
    const n = issue.comments || 0;
    talk.textContent = n > 0 ? `💬${n}` : "💬";
    if (n > 0) talk.className = "has";
    talk.title = "comments";
    talk.addEventListener("click", () => openDrawer(issue));
    talkTd.append(talk);
    tr.append(talkTd);

    // Two-step delete: first click arms, second confirms; blur disarms.
    const delTd = document.createElement("td");
    delTd.className = "del";
    const del = document.createElement("button");
    del.textContent = "×";
    del.title = "delete";
    del.addEventListener("click", async () => {
      if (!del.classList.contains("arm")) {
        del.classList.add("arm");
        del.textContent = "sure?";
        setTimeout(() => { del.classList.remove("arm"); del.textContent = "×"; }, 2500);
        return;
      }
      try {
        const gone = await api("DELETE", `/api/issues/${issue.id}`);
        toast(`deleted #${gone.deleted}: ${gone.title}`, "ok");
        load();
      } catch (err) { toast(err.message); }
    });
    delTd.append(del);
    tr.append(delTd);

    rowsEl.append(tr);
  }
  $("empty").classList.toggle("on", issues.length === 0);
  updateFooter();
  updateHeaders();
  updatePager();
  updateApplied();
}

function updateFooter() {
  const filtered = query.q || query.status || query.assignee;
  $("count").textContent = `${total} issue${total === 1 ? "" : "s"}${filtered ? " (filtered)" : ""}`;
  const by = s => issues.filter(i => i.status === s).length;
  $("summary").textContent =
    `this page: ${by("open")} open · ${by("in-progress")} in progress · ${by("done")} done`;
}

function updateHeaders() {
  for (const th of document.querySelectorAll("th[data-col]")) {
    th.querySelector(".dir").textContent =
      th.dataset.col === query.sort ? (query.order === "asc" ? "▲" : "▼") : "";
  }
}

function updatePager() {
  const from = total === 0 ? 0 : query.offset + 1;
  const to = Math.min(query.offset + PAGE_SIZE, total);
  $("page-info").textContent = `${from}–${to} of ${total}`;
  $("prev").disabled = query.offset === 0;
  $("next").disabled = to >= total;
}

// Chips naming the active filters, each with its own clear button.
function updateApplied() {
  const box = $("applied");
  box.textContent = "";
  const chip = (label, clear) => {
    const el = document.createElement("span");
    el.className = "fchip";
    el.textContent = label;
    const x = document.createElement("button");
    x.textContent = "×";
    x.addEventListener("click", () => { clear(); query.offset = 0; load(); });
    el.append(x);
    box.append(el);
  };
  if (query.q) chip(`“${query.q}”`, () => { query.q = ""; $("search").value = ""; });
  if (query.status) chip(query.status, () => { query.status = ""; $("f-status").value = ""; });
  if (query.assignee) chip(`@${query.assignee}`, () => { query.assignee = ""; $("f-assignee").value = ""; });
}

// ── the comments drawer ──────────────────────────────────────────────

async function openDrawer(issue) {
  drawerIssue = issue;
  $("drawer-title").textContent = `#${issue.id} — ${issue.title}`;
  const meta = $("drawer-meta");
  meta.textContent = "";
  const pill = (cls, text) => {
    const el = document.createElement("span");
    el.className = `pill ${cls}`;
    el.textContent = text;
    meta.append(el);
  };
  pill(`s-${issue.status}`, issue.status);
  pill(`p-${issue.priority}`, issue.priority);
  pill("pts", `${issue.points} pts`);
  if (issue.assignee) pill("pts", `@${issue.assignee}`);
  $("drawer").classList.add("open");
  await renderComments();
  $("c-text").focus();
}

function closeDrawer() {
  $("drawer").classList.remove("open");
  drawerIssue = null;
}

async function renderComments() {
  if (!drawerIssue) return;
  try {
    const full = await api("GET", `/api/issues/${drawerIssue.id}`);
    const box = $("comments");
    box.textContent = "";
    if (!full.comments.length) {
      const empty = document.createElement("p");
      empty.className = "none";
      empty.textContent = "No comments yet — start the thread below.";
      box.append(empty);
    }
    for (const c of full.comments) {
      const div = document.createElement("div");
      div.className = "comment";
      const av = document.createElement("span");
      av.className = "avatar";
      av.textContent = initials(c.author);
      const body = document.createElement("div");
      body.className = "body";
      const who = document.createElement("span");
      who.className = "who";
      who.textContent = c.author || "anonymous";
      const when = document.createElement("span");
      when.className = "when";
      when.textContent = relativeTime(c.at);
      when.title = c.at;
      const text = document.createElement("div");
      text.className = "text";
      text.textContent = c.text;
      body.append(who, when, text);
      div.append(av, body);
      box.append(div);
    }
    box.scrollTop = box.scrollHeight;
  } catch (err) { toast(err.message); }
}

$("drawer-close").addEventListener("click", closeDrawer);

async function submitComment() {
  if (!drawerIssue || !$("c-text").value.trim()) return;
  try {
    await api("POST", `/api/issues/${drawerIssue.id}/comments`, {
      text: $("c-text").value,
      author: $("c-author").value,
    });
    $("c-text").value = "";
    await renderComments();
    load();
  } catch (err) { toast(err.message); }
}

$("comment-form").addEventListener("submit", e => { e.preventDefault(); submitComment(); });
$("c-text").addEventListener("keydown", e => {
  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); submitComment(); }
});

// ── stats + activity panels ──────────────────────────────────────────

function barRow(label, value, max, cls, suffix) {
  const row = document.createElement("div");
  row.className = "bar-row";
  const l = document.createElement("span");
  l.className = "label";
  l.textContent = label;
  const bar = document.createElement("span");
  bar.className = "bar";
  const fill = document.createElement("i");
  fill.className = cls;
  fill.style.width = max > 0 ? `${Math.max(3, (value / max) * 100)}%` : "0";
  bar.append(fill);
  const v = document.createElement("span");
  v.className = "val";
  v.textContent = `${value}${suffix || ""}`;
  row.append(l, bar, v);
  return row;
}

async function refreshPanels() {
  if ($("stats-panel").classList.contains("open")) {
    try {
      const s = await api("GET", "/api/stats");
      $("stat-totals").innerHTML =
        `${s.totals.issues}<small>issues</small> &nbsp; ${s.totals.points}<small>points</small>`;
      const q = s.point_quantiles;
      $("stat-quantiles").innerHTML =
        `point quantiles p25 / p50 / p75: <b>${q.p25} / ${q.p50} / ${q.p75}</b>`;
      const st = $("stat-status");
      st.textContent = "";
      const maxN = Math.max(...s.by_status.map(r => r.n), 1);
      for (const row of s.by_status) {
        st.append(barRow(row.status, row.n, maxN, `s-${row.status}`, ` (${row.points ?? 0}p)`));
      }
      const asg = $("stat-assignee");
      asg.textContent = "";
      const maxP = Math.max(...s.by_assignee.map(r => r.points ?? 0), 1);
      for (const row of s.by_assignee.slice(0, 8)) {
        asg.append(barRow(row.assignee, row.points ?? 0, maxP, "s-points", "p"));
      }
    } catch (err) { toast(err.message); }
  }
  if ($("activity-panel").classList.contains("open")) {
    try {
      const events = await api("GET", "/api/activity?limit=15");
      const list = $("activity-list");
      list.textContent = "";
      for (const ev of events) {
        const li = document.createElement("li");
        const act = document.createElement("span");
        act.className = `act ${ev.action}`;
        act.textContent = ev.action;
        const what = document.createElement("span");
        what.textContent = ev.issue_id ? `#${ev.issue_id} ${ev.detail}` : ev.detail;
        const when = document.createElement("span");
        when.className = "when";
        when.textContent = relativeTime(ev.at);
        when.title = ev.at;
        li.append(act, what, when);
        list.append(li);
      }
    } catch (err) { toast(err.message); }
  }
}

function bindToggle(buttonId, panelId) {
  $(buttonId).addEventListener("click", () => {
    $(panelId).classList.toggle("open");
    $(buttonId).classList.toggle("on");
    refreshPanels();
  });
}
bindToggle("toggle-stats", "stats-panel");
bindToggle("toggle-activity", "activity-panel");
setInterval(() => {
  if ($("activity-panel").classList.contains("open")) refreshPanels();
}, 15000);

$("backup").addEventListener("click", async () => {
  try {
    const made = await api("POST", "/api/admin/backup");
    toast(`backup written: ${made.backup} (${made.issues} issues)`, "ok");
  } catch (err) { toast(err.message); }
});

// ── toolbar → query state ────────────────────────────────────────────

let searchTimer = null;
$("search").addEventListener("input", e => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    query.q = e.target.value.trim();
    query.offset = 0;
    load();
  }, 250);
});

$("f-status").addEventListener("change", e => {
  query.status = e.target.value;
  query.offset = 0;
  load();
});

let assigneeTimer = null;
$("f-assignee").addEventListener("input", e => {
  clearTimeout(assigneeTimer);
  assigneeTimer = setTimeout(() => {
    query.assignee = e.target.value.trim();
    query.offset = 0;
    load();
  }, 250);
});

for (const th of document.querySelectorAll("th[data-col]")) {
  th.addEventListener("click", () => {
    const col = th.dataset.col;
    if (query.sort === col) query.order = query.order === "asc" ? "desc" : "asc";
    else { query.sort = col; query.order = "asc"; }
    load();
  });
}

$("prev").addEventListener("click", () => {
  query.offset = Math.max(0, query.offset - PAGE_SIZE);
  load();
});
$("next").addEventListener("click", () => {
  query.offset += PAGE_SIZE;
  load();
});

// The whole new-issue row submits together on Enter in the title.
$("new-title").addEventListener("keydown", async e => {
  if (e.key === "Enter" && e.target.value.trim()) {
    try {
      const made = await api("POST", "/api/issues", {
        title: e.target.value.trim(),
        status: $("new-status").value,
        priority: $("new-priority").value,
        assignee: $("new-assignee").value.trim(),
        points: parseInt($("new-points").value, 10) || 0,
      });
      toast(`created #${made.id}: ${made.title}`, "ok");
      e.target.value = "";
      $("new-assignee").value = "";
      $("new-points").value = "";
      load();
    } catch (err) { toast(err.message); }
  }
});

// ── keyboard shortcuts ───────────────────────────────────────────────

document.addEventListener("keydown", e => {
  const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName);
  if (e.key === "Escape") {
    if (drawerIssue) closeDrawer();
    else if (typing) document.activeElement.blur();
    return;
  }
  if (typing) return;
  if (e.key === "/") { e.preventDefault(); $("search").focus(); }
  if (e.key === "n") { e.preventDefault(); $("new-title").focus(); }
});

// ── health in the footer ─────────────────────────────────────────────

async function health() {
  try {
    const h = await api("GET", "/health");
    const up = Math.round(h.uptime_ms / 1000);
    const mins = up >= 60 ? `${Math.floor(up / 60)}m ${up % 60}s` : `${up}s`;
    $("health").innerHTML =
      `backend: <span class="ok">olang · sqlite (${h.db}) · ${h.issues} rows · up ${mins}</span>`;
  } catch { /* footer stays generic */ }
}

readHash();
load();
health();
setInterval(health, 30000);
