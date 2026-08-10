// tracker frontend: a spreadsheet over /api/issues. Plain JS, no deps.
// Cells commit on change/blur; Enter commits and moves down a row in the
// same column; headers sort; the filter box narrows rows client-side.

const STATUSES = ["open", "in-progress", "done"];
const PRIORITIES = ["low", "medium", "high"];

let issues = [];
let sortCol = "id";
let sortDir = 1;
let filterText = "";

const rowsEl = document.getElementById("rows");
const countEl = document.getElementById("count");
const summaryEl = document.getElementById("summary");

async function api(method, path, body) {
  const res = await fetch(path, {
    method,
    headers: body ? { "Content-Type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status}`);
  return res.json();
}

async function load() {
  // The list endpoint returns a paged envelope: { items, total, limit, offset }.
  const page = await api("GET", "/api/issues?limit=500");
  issues = page.items;
  render();
}

function visibleIssues() {
  const q = filterText.toLowerCase();
  const shown = issues.filter(i =>
    !q || [i.title, i.status, i.priority, i.assignee, i.notes]
      .some(v => String(v).toLowerCase().includes(q)));
  shown.sort((a, b) => {
    const [x, y] = [a[sortCol], b[sortCol]];
    const cmp = typeof x === "number" && typeof y === "number"
      ? x - y : String(x).localeCompare(String(y));
    return cmp * sortDir;
  });
  return shown;
}

function selectCell(kind, value, options, extraClass) {
  const sel = document.createElement("select");
  for (const o of options) {
    const opt = document.createElement("option");
    opt.value = o; opt.textContent = o;
    if (o === value) opt.selected = true;
    sel.append(opt);
  }
  sel.className = `${extraClass}-${value}`;
  return sel;
}

function commit(id, col, value, refreshRow) {
  api("PATCH", `/api/issues/${id}`, { [col]: value }).then(updated => {
    const i = issues.findIndex(x => x.id === id);
    if (i >= 0) issues[i] = updated;
    if (refreshRow) render(); else updateFooter();
  }).catch(err => { alert(err.message); load(); });
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
        document.getElementById("new-title").focus();
      }
    }
  });
  td.dataset.col = col;
  td.append(input);
  return td;
}

function render() {
  rowsEl.textContent = "";
  for (const issue of visibleIssues()) {
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

    const delTd = document.createElement("td");
    delTd.className = "del";
    const del = document.createElement("button");
    del.textContent = "×";
    del.title = "delete";
    del.addEventListener("click", async () => {
      await api("DELETE", `/api/issues/${issue.id}`);
      issues = issues.filter(x => x.id !== issue.id);
      render();
    });
    delTd.append(del);
    tr.append(delTd);

    rowsEl.append(tr);
  }
  updateFooter();
  updateHeaders();
}

function updateFooter() {
  countEl.textContent = `${issues.length} issues`;
  const by = s => issues.filter(i => i.status === s).length;
  summaryEl.textContent = `${by("open")} open · ${by("in-progress")} in progress · ${by("done")} done`;
}

function updateHeaders() {
  for (const th of document.querySelectorAll("th[data-col]")) {
    th.querySelector(".dir").textContent =
      th.dataset.col === sortCol ? (sortDir > 0 ? "▲" : "▼") : "";
  }
}

for (const th of document.querySelectorAll("th[data-col]")) {
  th.addEventListener("click", () => {
    const col = th.dataset.col;
    if (sortCol === col) sortDir = -sortDir;
    else { sortCol = col; sortDir = 1; }
    render();
  });
}

document.getElementById("filter").addEventListener("input", e => {
  filterText = e.target.value;
  render();
});

document.getElementById("new-title").addEventListener("keydown", async e => {
  if (e.key === "Enter" && e.target.value.trim()) {
    const made = await api("POST", "/api/issues", { title: e.target.value.trim() });
    issues.push(made);
    e.target.value = "";
    render();
  }
});

load();
