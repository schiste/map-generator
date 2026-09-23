import init, { MapGenerator, themes, bboxPresets, version } from "./pkg/mapgen_wasm.js";

// Pinned to the same Natural Earth commit as scripts/fetch-data.sh. A
// deployment can serve the files itself (<meta name="mapgen-data" content="data/">),
// so visitors' browsers contact no third party (Toolforge's rule).
const NE =
  document.querySelector('meta[name="mapgen-data"]')?.content ||
  "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/ca96624a56bd078437bca8184e78163e5039ad19/geojson/";
const SAMPLES = {
  "ne-admin0": NE + "ne_10m_admin_0_countries.geojson",
  "ne-admin1": NE + "ne_10m_admin_1_states_provinces.geojson",
};
const CONTEXT = NE + "ne_10m_admin_0_countries.geojson";
const LAKES = NE + "ne_10m_lakes.geojson";
const DISPUTED = NE + "ne_10m_admin_0_boundary_lines_disputed_areas.geojson";
const LABELS = {
  background: "Background", water: "Water", land: "Land", "context-land": "Neighbours",
  border: "Borders", outline: "Outline", coast: "Coast", "context-border": "Neighbour borders",
  "lake-border": "Lake shores", "disputed-border": "Disputed", label: "Labels",
};

const $ = (id) => document.getElementById(id);
const gen = { current: null };
const colors = {};
let lastSvg = "";
let pending = 0;
const probe = document.createElement("canvas").getContext("2d");

await init();
const THEMES = themes();
$("version").textContent = `v${version()}`;

for (const name of Object.keys(THEMES)) $("theme").add(new Option(name, name));
for (const name of Object.keys(bboxPresets())) $("bbox").add(new Option(name, name));
buildColorControls();
$("theme").value = "wikimedia";
applyTheme("wikimedia");

async function fetchText(url, label) {
  setStatus(`Downloading ${label}…`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${label}: HTTP ${res.status}`);
  return res.text();
}

$("load-sample").addEventListener("click", () =>
  guard(async () => {
    const dataset = $("sample").value;
    const subject = await fetchText(SAMPLES[dataset], "sample data");
    const withContext = $("with-context").checked;
    const [context, lakes, disputed] = withContext
      ? await Promise.all([
          fetchText(CONTEXT, "neighbours"),
          fetchText(LAKES, "lakes"),
          fetchText(DISPUTED, "disputed boundaries"),
        ])
      : [undefined, undefined, undefined];
    loadGenerator(subject, { dataset }, context, lakes, dataset === "ne-admin1" ? "FRA" : "", disputed);
  }),
);

$("file").addEventListener("change", () =>
  guard(async () => {
    const file = $("file").files[0];
    if (!file) return;
    const spec = { dataset: $("file-dataset").value };
    const credit = $("file-credit").value.trim();
    if (credit) spec.attribution = credit;
    loadGenerator(await file.text(), spec, undefined, undefined, "");
  }),
);

function loadGenerator(subject, spec, context, lakes, preferredRegion, disputed) {
  const t = performance.now();
  const g = new MapGenerator();
  const n = g.setSubject(subject, spec);
  if (context) g.setContext(context);
  if (lakes) g.setLakes(lakes);
  if (disputed) g.setDisputed(disputed);
  gen.current?.free();
  gen.current = g;

  const regions = g.regions();
  const select = $("region");
  select.replaceChildren(new Option(regions.length ? "(all)" : "(no region column)", ""));
  for (const r of regions) select.add(new Option(r, r));
  select.value = regions.includes(preferredRegion) ? preferredRegion : "";
  $("frame").value = regions.length && !select.value && spec.dataset === "ne-admin0" ? "world" : "auto";

  setStatus(`${n.toLocaleString()} features parsed in ${Math.round(performance.now() - t)} ms.`);
  $("map-options").disabled = $("style-options").disabled = false;
  render();
}

function spec() {
  const s = {
    width: Number($("width").value) || 900,
    simplify: Number($("simplify").value),
    labels: $("labels").checked,
    credit: $("credit").checked,
    frame: $("frame").value,
    colors: { ...colors },
  };
  if ($("region").value) s.region = $("region").value;
  if ($("bbox").value) s.bbox = $("bbox").value;
  return s;
}

function render() {
  if (!gen.current) return;
  cancelAnimationFrame(pending);
  pending = requestAnimationFrame(() =>
    guard(() => {
      const t = performance.now();
      const out = gen.current.render(spec());
      const ms = Math.round(performance.now() - t);
      lastSvg = out.svg;
      $("map").innerHTML = out.svg;
      $("hover").textContent = "";
      $("download").disabled = false;
      const kb = (new Blob([out.svg]).size / 1024).toFixed(0);
      let info = `${out.regions} regions · ${out.width}×${out.height} px · ${kb} KB · ${out.projection} · ${ms} ms`;
      if (out.insets.length) info += ` · ${out.insets.length} inset(s)`;
      if (out.outsideFrame.length) info += ` · ${out.outsideFrame.length} outside the frame`;
      $("info").textContent = info;
    }),
  );
}

function buildColorControls() {
  for (const [slot, label] of Object.entries(LABELS)) {
    const row = document.createElement("label");
    row.className = "swatch";
    row.dataset.slot = slot;
    row.innerHTML = `<span>${label}</span><input type="color" aria-label="${label} colour"><input type="text" spellcheck="false" aria-label="${label} CSS colour">`;
    const [pick, text] = row.querySelectorAll("input");
    pick.addEventListener("input", () => setColor(slot, pick.value));
    text.addEventListener("change", () => setColor(slot, text.value.trim() || THEMES[$("theme").value][slot]));
    $("colors").append(row);
  }
}

// Resolves any CSS colour ("tomato", "rgb(1 2 3)") to #rrggbb for the picker.
function toHex(value) {
  probe.fillStyle = "#000000";
  probe.fillStyle = value;
  return /^#[0-9a-f]{6}$/i.test(probe.fillStyle) ? probe.fillStyle : null;
}

function setColor(slot, value, rerender = true) {
  colors[slot] = value;
  const row = document.querySelector(`[data-slot="${slot}"]`);
  row.querySelector("input[type=text]").value = value;
  const hex = toHex(value);
  if (hex) row.querySelector("input[type=color]").value = hex;
  if (rerender) render();
}

function applyTheme(name) {
  for (const [slot, value] of Object.entries(THEMES[name])) setColor(slot, value, false);
  render();
}

$("theme").addEventListener("change", (e) => applyTheme(e.target.value));
for (const id of ["region", "frame", "bbox", "width", "labels", "credit"]) $(id).addEventListener("change", render);
$("simplify").addEventListener("input", () => {
  $("simplify-out").textContent = `${$("simplify").value} px`;
  render();
});

$("map").addEventListener("mouseover", (e) => {
  const p = e.target.closest("path[data-name]");
  $("hover").textContent = p ? `${p.dataset.name} · #${p.id}` : "";
});

$("download").addEventListener("click", () => {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([lastSvg], { type: "image/svg+xml" }));
  a.download = `${$("region").value || "map"}.svg`;
  a.click();
  URL.revokeObjectURL(a.href);
});

function setStatus(text) {
  $("data-status").textContent = text;
}

async function guard(fn) {
  $("error").textContent = "";
  try {
    await fn();
  } catch (e) {
    $("error").textContent = e instanceof Error ? e.message : String(e);
    setStatus("");
  }
}
