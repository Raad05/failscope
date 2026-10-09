// failscope dashboard. Vanilla JS, inline SVG, no dependencies.
// All API strings (program ids, error names from on-chain IDLs) are
// untrusted: they only ever reach the DOM through textContent.
"use strict";

const SVG = "http://www.w3.org/2000/svg";

// Friendly names for well-known programs; anything else shows its id.
const KNOWN = {
  "11111111111111111111111111111111": "System Program",
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA": "SPL Token",
  "TokenzQdBNbLqP5VEhdkAS6EPFLC1PE9qPdWLZDGkNvEQ": "Token-2022",
  "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL": "Associated Token",
  "ComputeBudget111111111111111111111111111111": "Compute Budget",
  "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4": "Jupiter v6",
  "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc": "Orca Whirlpools",
  "MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD": "Marinade",
  "6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey": "fail_target",
  "BCUANLyTtzvYGheyo7ymmHhDDZQ74WGjwsC8Rv3VFDPb": "fail_callee",
};

const SOURCE_LABEL = {
  anchor_log: "Anchor error log",
  anchor_framework: "Anchor framework code",
  idl: "Program IDL",
  native: "Native program table",
  runtime: "Runtime (panic, compute)",
  unknown: "Not decoded",
};

const state = { hours: 24, program: null, data: null, tables: new Set() };

// ------------------------------------------------------------------ helpers

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") node.className = v;
    else if (k === "text") node.textContent = v;
    else node.setAttribute(k, v);
  }
  for (const c of children) if (c != null) node.append(c);
  return node;
}

function svg(tag, attrs = {}) {
  const node = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
  return node;
}

const shortId = (id) => (id && id.length > 12 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id || "–");
const programName = (id) => KNOWN[id] || shortId(id);
const fmt = new Intl.NumberFormat("en-US");
const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
const pct = (x) => `${(x * 100).toFixed(x > 0 && x < 0.1 ? 1 : 0)}%`;

function timeLabel(iso, long) {
  const d = new Date(iso);
  if (state.hours > 24 && !long) {
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }
  const t = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return long ? `${d.toLocaleDateString(undefined, { month: "short", day: "numeric" })} ${t}` : t;
}

function ago(iso) {
  if (!iso) return "–";
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return `${Math.round(s)}s ago`;
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

/** Rounds up to 1/2/5 × 10^n and returns [max, step] for ~4 ticks. */
function niceScale(max) {
  if (max <= 0) return [4, 1];
  const raw = max / 4;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * mag).find((s) => s >= raw);
  const s = Math.max(1, step);
  return [Math.ceil(max / s) * s, s];
}

async function api(path) {
  const r = await fetch(path, { headers: { accept: "application/json" } });
  const body = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(body.error || `HTTP ${r.status}`);
  return body;
}

// ------------------------------------------------------------------ tooltip

const tooltip = () => document.getElementById("tooltip");

function showTooltip(x, y, title, rows) {
  const tt = tooltip();
  tt.replaceChildren(el("div", { class: "tt-title", text: title }));
  for (const r of rows) {
    const key = el("span", { class: "key" });
    key.style.background = r.color;
    tt.append(el("div", { class: "tt-row" }, key, el("strong", { text: fmt.format(r.value) }), el("span", { text: r.name })));
  }
  tt.hidden = false;
  const pad = 14;
  const w = tt.offsetWidth, h = tt.offsetHeight;
  tt.style.left = `${Math.min(x + pad, window.innerWidth - w - 8)}px`;
  tt.style.top = `${Math.max(8, y - h - pad)}px`;
}

const hideTooltip = () => { tooltip().hidden = true; };

// ------------------------------------------------------------------ charts

function table(headers, rows, numeric = []) {
  const thead = el("thead", {}, el("tr", {}, ...headers.map((h, i) => el("th", { class: numeric.includes(i) ? "num" : "", text: h }))));
  const tbody = el("tbody");
  for (const r of rows) {
    tbody.append(el("tr", {}, ...r.map((c, i) => {
      const td = el("td", { class: numeric.includes(i) ? "num" : "" });
      if (c instanceof Node) td.append(c); else td.textContent = c;
      return td;
    })));
  }
  // Wide tables scroll inside their own box, never the page.
  return el("div", { class: "table-wrap" }, el("table", {}, thead, tbody));
}

/**
 * Line chart. series: [{name, color, values, area}] sharing `starts`.
 * Crosshair snaps to the nearest bucket; keyboard arrows move it.
 */
function lineChart(container, starts, series) {
  container.replaceChildren();
  if (!starts.length) return;
  const W = Math.max(320, container.clientWidth);
  const H = 220, m = { t: 10, r: 12, b: 26, l: 44 };
  const pw = W - m.l - m.r, ph = H - m.t - m.b;
  const n = starts.length;
  const [ymax, ystep] = niceScale(Math.max(0, ...series.flatMap((s) => s.values)));
  const x = (i) => m.l + (n === 1 ? pw / 2 : (i * pw) / (n - 1));
  const y = (v) => m.t + ph - (v / ymax) * ph;

  const root = svg("svg", { viewBox: `0 0 ${W} ${H}`, height: H, role: "img",
    "aria-label": `${series.map((s) => s.name).join(" and ")}: failures per bucket` });

  for (let v = 0; v <= ymax; v += ystep) {
    root.append(svg("line", { class: v === 0 ? "baseline" : "gridline", x1: m.l, x2: W - m.r, y1: y(v), y2: y(v) }));
    const t = svg("text", { class: "axis-label", x: m.l - 8, y: y(v) + 4, "text-anchor": "end" });
    t.textContent = compact.format(v);
    root.append(t);
  }
  // About one time label per 90px so they never collide on narrow screens.
  const ticks = Math.max(2, Math.min(6, n, Math.floor(pw / 90)));
  for (let k = 0; k < ticks; k++) {
    const i = Math.round((k * (n - 1)) / Math.max(1, ticks - 1));
    const t = svg("text", { class: "axis-label", x: x(i), y: H - 6,
      "text-anchor": k === 0 ? "start" : k === ticks - 1 ? "end" : "middle" });
    t.textContent = timeLabel(starts[i]);
    root.append(t);
  }

  // Paint context first so the accent series sits on top of it.
  for (const s of [...series].reverse()) {
    const pts = s.values.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`);
    if (s.area) {
      root.append(svg("path", { d: `M${x(0)},${y(0)}L${pts.join("L")}L${x(n - 1)},${y(0)}Z`, fill: s.wash, stroke: "none" }));
    }
    root.append(svg("polyline", { points: pts.join(" "), fill: "none", stroke: s.color,
      "stroke-width": 2, "stroke-linejoin": "round", "stroke-linecap": "round" }));
  }

  const cross = svg("line", { class: "crosshair", y1: m.t, y2: m.t + ph, visibility: "hidden" });
  const dots = series.map((s) => svg("circle", { r: 4, fill: s.color, stroke: "var(--surface)", "stroke-width": 2, visibility: "hidden" }));
  root.append(cross, ...dots);

  const hit = svg("rect", { x: m.l, y: m.t, width: pw, height: ph, fill: "transparent", tabindex: 0,
    "aria-label": "Chart readout; use arrow keys to move" });
  root.append(hit);
  let current = n - 1;
  const show = (i, cx, cy) => {
    current = i;
    cross.setAttribute("x1", x(i)); cross.setAttribute("x2", x(i));
    cross.setAttribute("visibility", "visible");
    dots.forEach((d, k) => {
      d.setAttribute("cx", x(i)); d.setAttribute("cy", y(series[k].values[i]));
      d.setAttribute("visibility", "visible");
    });
    showTooltip(cx, cy, timeLabel(starts[i], true),
      series.map((s) => ({ name: s.name, value: s.values[i], color: s.color })));
  };
  const hide = () => {
    cross.setAttribute("visibility", "hidden");
    dots.forEach((d) => d.setAttribute("visibility", "hidden"));
    hideTooltip();
  };
  hit.addEventListener("pointermove", (e) => {
    const box = root.getBoundingClientRect();
    const px = ((e.clientX - box.left) / box.width) * W;
    const i = Math.max(0, Math.min(n - 1, Math.round(((px - m.l) / pw) * (n - 1))));
    show(i, e.clientX, e.clientY);
  });
  hit.addEventListener("pointerleave", hide);
  hit.addEventListener("blur", hide);
  const keyShow = (i) => {
    const box = root.getBoundingClientRect();
    show(i, box.left + (x(i) / W) * box.width, box.top + (y(series[0].values[i]) / H) * box.height);
  };
  hit.addEventListener("focus", () => keyShow(current));
  hit.addEventListener("keydown", (e) => {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      keyShow(Math.max(0, Math.min(n - 1, current + (e.key === "ArrowLeft" ? -1 : 1))));
    }
  });
  container.append(root);
}

/** Horizontal bars, one color; value at the tip. `onSelect` makes rows buttons. */
function barList(container, rows, { onSelect, selected } = {}) {
  container.replaceChildren();
  const max = Math.max(1, ...rows.map((r) => r.value));
  const list = el("div", { class: "bars" });
  for (const r of rows) {
    const bar = el("div", { class: "bar" });
    bar.style.width = `${(r.value / max) * 100}%`;
    const label = el("span", { class: "bar-label", title: r.title || r.label });
    label.append(r.label);
    if (r.sub) label.append(" ", el("span", { class: "id", text: r.sub }));
    const parts = [label, el("span", { class: "bar-track" }, bar), el("span", { class: "bar-value", text: r.valueText })];
    let row;
    if (onSelect) {
      row = el("button", { type: "button", class: "bar-row", "aria-pressed": String(r.key === selected),
        "aria-label": `${r.title || r.label}: ${r.valueText}` }, ...parts);
      row.addEventListener("click", () => onSelect(r.key));
    } else {
      row = el("div", { class: "bar-row" }, ...parts);
    }
    list.append(row);
  }
  container.append(list);
}

function emptyState(container, text) {
  container.replaceChildren(el("div", { class: "empty", text }));
}

// ------------------------------------------------------------------ render

function render() {
  const d = state.data;
  if (!d) return;
  const total = d.coverage.total;
  const unknown = d.coverage.data.find((c) => c.decode_source === "unknown")?.failures || 0;
  document.getElementById("kpi-total").textContent = fmt.format(total);
  document.getElementById("kpi-programs").textContent = fmt.format(d.allPrograms.length);
  document.getElementById("kpi-decoded").textContent = total ? pct((total - unknown) / total) : "–";

  renderTimeseries(d);
  renderTop(d);
  renderCoverage(d);
  renderDetail(d);
}

function renderTimeseries(d) {
  const box = document.getElementById("ts");
  const legend = document.getElementById("ts-legend");
  const starts = d.ts.data.map((b) => b.start);
  const all = d.ts.data.map((b) => b.failures);
  document.getElementById("ts-sub").textContent =
    `Per ${d.ts.bucket}, last ${state.hours >= 168 ? "7 days" : `${state.hours}h`}`;
  legend.replaceChildren();
  if (!all.some((v) => v > 0)) { emptyState(box, "No failed transactions in this window"); return; }

  const css = getComputedStyle(document.documentElement);
  const accent = css.getPropertyValue("--series-1").trim();
  const accentWash = css.getPropertyValue("--series-1-wash").trim();
  const context = css.getPropertyValue("--context").trim();
  // Emphasis: the selected program in the accent, all failures as gray context.
  const series = [];
  if (state.program && d.programTs) {
    series.push({ name: programName(state.program), color: accent, wash: accentWash, area: true,
      values: d.programTs.data.map((b) => b.failures) });
    series.push({ name: "All programs", color: context, values: all });
  } else {
    series.push({ name: "All programs", color: accent, wash: accentWash, area: true, values: all });
  }
  if (series.length > 1) {
    for (const s of series) {
      const key = el("span", { class: "key" });
      key.style.background = s.color;
      legend.append(el("span", {}, key, s.name));
    }
  }
  if (state.tables.has("ts")) {
    box.replaceChildren(table(["Bucket start", ...series.map((s) => s.name)],
      starts.map((t, i) => [timeLabel(t, true), ...series.map((s) => fmt.format(s.values[i]))]),
      series.map((_, i) => i + 1)));
  } else {
    lineChart(box, starts, series);
  }
}

function renderTop(d) {
  const box = document.getElementById("top");
  const rows = d.allPrograms.slice(0, 10);
  if (!rows.length) { emptyState(box, "No failing programs in this window"); return; }
  if (state.tables.has("top")) {
    box.replaceChildren(table(["Program", "Failures", "Distinct errors", "Last seen"],
      rows.map((p) => [programName(p.program_id), fmt.format(p.failures), fmt.format(p.distinct_errors), ago(p.last_seen)]),
      [1, 2]));
    return;
  }
  barList(box, rows.map((p) => ({
    key: p.program_id,
    label: programName(p.program_id),
    sub: KNOWN[p.program_id] ? shortId(p.program_id) : null,
    title: p.program_id,
    value: p.failures,
    valueText: fmt.format(p.failures),
  })), { onSelect: selectProgram, selected: state.program });
}

function renderCoverage(d) {
  const box = document.getElementById("cov");
  const total = d.coverage.total;
  if (!total) { emptyState(box, "Nothing decoded in this window"); return; }
  const rows = d.coverage.data.map((c) => ({
    key: c.decode_source,
    label: SOURCE_LABEL[c.decode_source] || c.decode_source,
    value: c.failures,
    valueText: `${pct(c.share)} · ${fmt.format(c.failures)}`,
  }));
  if (state.tables.has("cov")) {
    box.replaceChildren(table(["Decode source", "Failures", "Share"],
      d.coverage.data.map((c) => [SOURCE_LABEL[c.decode_source] || c.decode_source, fmt.format(c.failures), pct(c.share)]), [1, 2]));
    return;
  }
  barList(box, rows);
}

function renderDetail(d) {
  const errors = document.getElementById("errors");
  const recent = document.getElementById("recent");
  if (!state.program) {
    document.getElementById("detail-title").textContent = "Program";
    document.getElementById("detail-sub").textContent = "";
    emptyState(errors, "Select a program above");
    recent.replaceChildren();
    return;
  }
  document.getElementById("detail-title").textContent = programName(state.program);
  document.getElementById("detail-sub").textContent = state.program;

  if (!d.errors.data.length) emptyState(errors, "No failures in this window");
  else errors.replaceChildren(table(["Error", "Code", "Decoded by", "Failures", "Last seen"],
    d.errors.data.map((e) => [
      e.error_name || `${e.error_kind}`,
      e.error_code == null ? "–" : String(e.error_code),
      el("span", { class: "pill", text: e.decode_source }),
      fmt.format(e.failures),
      ago(e.last_seen),
    ]), [1, 3]));

  if (!d.recent.data.length) emptyState(recent, "No failures in this window");
  else recent.replaceChildren(table(["When", "Signature", "Error", "CPI depth"],
    d.recent.data.map((f) => {
      const sig = el("span", { class: "mono", title: f.signature, text: shortId(f.signature) });
      return [ago(f.block_time), sig, f.error_name || f.error_kind, f.cpi_depth == null ? "?" : String(f.cpi_depth)];
    }), [3]));
}

// ------------------------------------------------------------------ data

function bucketFor(hours) {
  return hours <= 24 ? "minute" : "hour";
}

async function load() {
  const main = document.getElementById("main");
  const status = document.getElementById("status");
  main.classList.add("loading");
  status.textContent = "Loading…";
  const h = state.hours;
  try {
    const [allPrograms, coverage, ts] = await Promise.all([
      api(`/api/programs/top?hours=${h}&limit=500`),
      api(`/api/coverage?hours=${h}`),
      api(`/api/failures/timeseries?hours=${h}&bucket=${bucketFor(h)}`),
    ]);
    const programs = allPrograms.data;
    if (!state.program || !programs.some((p) => p.program_id === state.program)) {
      state.program = programs.length ? programs[0].program_id : null;
    }
    let programTs = null, errors = { data: [] }, recent = { data: [] };
    if (state.program) {
      const p = encodeURIComponent(state.program);
      [programTs, errors, recent] = await Promise.all([
        api(`/api/failures/timeseries?hours=${h}&bucket=${bucketFor(h)}&program=${p}`),
        api(`/api/programs/${p}/errors?hours=${h}&limit=20`),
        api(`/api/failures?hours=${h}&program=${p}&limit=15`),
      ]);
    }
    state.data = { allPrograms: programs, coverage, ts, programTs, errors, recent };
    saveHash();
    render();
    status.textContent = `Updated ${new Date().toLocaleTimeString()}`;
  } catch (e) {
    status.textContent = `Couldn't load: ${e.message}`;
  } finally {
    main.classList.remove("loading");
  }
}

function selectProgram(id) {
  state.program = id;
  load();
}

function saveHash() {
  const p = new URLSearchParams({ hours: String(state.hours) });
  if (state.program) p.set("program", state.program);
  history.replaceState(null, "", `#${p}`);
}

function readHash() {
  const p = new URLSearchParams(location.hash.slice(1));
  const hours = Number(p.get("hours"));
  if ([1, 6, 24, 168].includes(hours)) state.hours = hours;
  if (p.get("program")) state.program = p.get("program");
}

function syncWindowButtons() {
  for (const b of document.querySelectorAll(".segmented button")) {
    b.setAttribute("aria-checked", String(Number(b.dataset.hours) === state.hours));
  }
}

function setupTheme() {
  const btn = document.getElementById("theme");
  const stored = (() => { try { return localStorage.getItem("theme"); } catch { return null; } })();
  if (stored) document.documentElement.dataset.theme = stored;
  const isDark = () => document.documentElement.dataset.theme === "dark" ||
    (!document.documentElement.dataset.theme && matchMedia("(prefers-color-scheme: dark)").matches);
  const label = () => { btn.textContent = isDark() ? "Light" : "Dark"; };
  label();
  btn.addEventListener("click", () => {
    const next = isDark() ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem("theme", next); } catch { /* private mode */ }
    label();
    render();
  });
}

document.addEventListener("DOMContentLoaded", () => {
  readHash();
  syncWindowButtons();
  setupTheme();
  for (const b of document.querySelectorAll(".segmented button")) {
    b.addEventListener("click", () => {
      state.hours = Number(b.dataset.hours);
      syncWindowButtons();
      load();
    });
  }
  for (const b of document.querySelectorAll(".table-toggle")) {
    b.addEventListener("click", () => {
      const id = b.dataset.for;
      if (state.tables.has(id)) state.tables.delete(id); else state.tables.add(id);
      b.textContent = state.tables.has(id) ? "Chart" : "Table";
      render();
    });
  }
  let resize;
  new ResizeObserver(() => { clearTimeout(resize); resize = setTimeout(render, 100); })
    .observe(document.getElementById("ts"));
  load();
  setInterval(load, 30_000);
});
