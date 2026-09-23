// map-generator playground: custom maps of any countries (or their
// subdivisions), rendered in the browser with WebAssembly. Pick countries in
// the list or on the map, or apply a recipe CSV; export SVG, PNG, the recipe,
// or a link to the same map from the public API.
import init, { MapGenerator, themes, version, parseRecipe, recipeToCsv } from "./pkg/mapgen_wasm.js";

// Pinned to the same Natural Earth commit as scripts/fetch-data.sh. A
// deployment can serve the files itself (<meta name="mapgen-data" content="data/">),
// so visitors' browsers contact no third party (Toolforge's rule).
const NE =
  document.querySelector('meta[name="mapgen-data"]')?.content ||
  "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/ca96624a56bd078437bca8184e78163e5039ad19/geojson/";
const FILES = {
  countries: NE + "ne_10m_admin_0_countries.geojson",
  subdivisions: NE + "ne_10m_admin_1_states_provinces.geojson",
  lakes: NE + "ne_10m_lakes.geojson",
  disputed: NE + "ne_10m_admin_0_boundary_lines_disputed_areas.geojson",
  disputedAreas: NE + "ne_10m_admin_0_disputed_areas.geojson",
};
const API = location.hostname.endsWith("toolforge.org")
  ? `${location.origin}/api/v1`
  : "https://map-generator.toolforge.org/api/v1";
const DATASET = { countries: "ne-admin0", subdivisions: "ne-admin1" };
// Every language Natural Earth names places in (name_xx columns), so labels
// can switch to any of them and names in any of them are recognised.
const LANGUAGES = [
  "ar", "bn", "de", "el", "en", "es", "fa", "fr", "he", "hi", "hu", "id", "it", "ja", "ko",
  "nl", "pl", "pt", "ru", "sv", "tr", "uk", "ur", "vi", "zh", "zh-Hans", "zh-Hant",
];
const COLOR_LABELS = {
  background: "Background", water: "Water", land: "Selection", contextLand: "Neighbours",
  border: "Borders", outline: "Outline", coast: "Coast", contextBorder: "Neighbour borders",
  lakeBorder: "Lake shores", disputedBorder: "Disputed", label: "Labels",
};
const STORAGE = { theme: "mapgen-theme", recipe: "mapgen-recipe" };

const $ = (id) => document.getElementById(id);
const kebab = (s) => s.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
const state = {
  mode: "countries",
  selected: [],
  colors: {},
  /** Recipe settings without a control here (label size, bbox…), kept for export. */
  extra: {},
};
const generators = {};
const texts = {};
let countryList = [];
let byIso2 = new Map();
let lastOutput = null;
let pending = 0;
let toastTimer = 0;
const probe = document.createElement("canvas").getContext("2d");

await init();
const THEMES = themes();
$("version").textContent = `v${version()}`;
for (const name of Object.keys(THEMES)) $("theme").add(new Option(name, name));
buildColorControls();
initInterfaceTheme();
bindControls();
applyTheme("wikimedia", false);
await restore();

// ---------------------------------------------------------------- data

async function fetchText(key, label) {
  if (!texts[key]) {
    texts[key] = (async () => {
      $("loading-text").textContent = `Downloading ${label}…`;
      const res = await fetch(FILES[key]);
      if (!res.ok) throw new Error(`${label}: HTTP ${res.status}`);
      return res.text();
    })();
  }
  return texts[key];
}

/** The generator for a mode, loading its data on first use. */
async function generator(mode) {
  if (generators[mode]) return generators[mode];
  showLoading(true);
  try {
    const [countries, lakes, disputed, disputedAreas] = await Promise.all([
      fetchText("countries", "countries"),
      fetchText("lakes", "lakes"),
      fetchText("disputed", "disputed borders"),
      fetchText("disputedAreas", "disputed areas"),
    ]);
    const subject = mode === "subdivisions" ? await fetchText("subdivisions", "subdivisions (once, 12 MB)") : countries;
    $("loading-text").textContent = "Preparing the map…";
    await new Promise(requestAnimationFrame);
    const g = new MapGenerator();
    g.setSubject(subject, { dataset: DATASET[mode], languages: LANGUAGES });
    g.setContext(countries);
    g.setLakes(lakes);
    g.setDisputed(disputed);
    g.setDisputedAreas(disputedAreas);
    generators[mode] = g;
    return g;
  } finally {
    showLoading(false);
  }
}

async function setMode(mode, { rerender = true } = {}) {
  state.mode = mode;
  for (const b of document.querySelectorAll("[data-mode]")) {
    const on = b.dataset.mode === mode;
    b.classList.toggle("is-active", on);
    b.setAttribute("aria-pressed", String(on));
  }
  $("toolbar-label").textContent = `${mode === "countries" ? "Countries" : "Subdivisions"} · Natural Earth 1:10m`;
  const g = await generator(mode);
  countryList = g.countries().sort((a, b) => a.name.localeCompare(b.name));
  byIso2 = new Map(countryList.filter((c) => c.iso2).map((c) => [c.iso2, c.code]));
  const known = new Set(countryList.map((c) => c.code));
  state.selected = state.selected.filter((c) => known.has(c));
  $("country-search").disabled = false;
  $("data-status").textContent = `${countryList.length} countries`;
  renderCountryList();
  if (rerender) render();
}

// ---------------------------------------------------------------- selection

function toggleCountry(code) {
  const i = state.selected.indexOf(code);
  if (i >= 0) state.selected.splice(i, 1);
  else state.selected.push(code);
  renderCountryList();
  render();
}

function countryName(code) {
  return countryList.find((c) => c.code === code)?.name ?? code;
}

function renderCountryList() {
  const q = $("country-search").value.trim().toLowerCase();
  const selected = new Set(state.selected);
  const matches = countryList
    .filter((c) => !q || [c.name, c.code, c.iso2 ?? ""].some((v) => v.toLowerCase().includes(q)))
    .slice(0, q ? 80 : 60);
  $("country-list").replaceChildren(
    ...matches.map((c) => {
      const b = document.createElement("button");
      b.className = "country-option";
      b.type = "button";
      b.dataset.code = c.code;
      const on = selected.has(c.code);
      b.setAttribute("aria-pressed", String(on));
      b.setAttribute("aria-label", `${c.name}, ${on ? "selected. Activate to remove" : "activate to add"}`);
      b.innerHTML = `<span class="country-swatch" aria-hidden="true"></span><span class="country-option-name"></span><span class="country-option-code"></span>`;
      b.querySelector(".country-option-name").textContent = c.name;
      b.querySelector(".country-option-code").textContent = [c.iso2?.toUpperCase(), c.code].filter(Boolean).join(" · ");
      return b;
    }),
  );
  $("country-empty").hidden = matches.length > 0;
  $("selected-chips").replaceChildren(
    ...state.selected.map((code) => {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "selected-chip";
      chip.dataset.code = code;
      chip.title = `Remove ${countryName(code)}`;
      chip.textContent = countryName(code);
      return chip;
    }),
  );
  const n = state.selected.length;
  $("selected-count").textContent = n ? `${n} selected` : "none selected";
  $("clear-selection").disabled = n === 0;
}

// ---------------------------------------------------------------- design

function buildColorControls() {
  for (const [slot, label] of Object.entries(COLOR_LABELS)) {
    const row = document.createElement("label");
    row.className = "color-row";
    row.dataset.slot = slot;
    row.innerHTML = `<input class="color-picker" type="color"><span></span>`;
    row.querySelector("span").textContent = label;
    const pick = row.querySelector("input");
    pick.setAttribute("aria-label", `${label} colour`);
    pick.addEventListener("input", () => {
      state.colors[slot] = pick.value;
      render();
    });
    $("colors").append(row);
  }
}

function toHex(value) {
  probe.fillStyle = "#000000";
  probe.fillStyle = value ?? "#000000";
  return /^#[0-9a-f]{6}$/i.test(probe.fillStyle) ? probe.fillStyle : "#000000";
}

function applyTheme(name, rerender = true) {
  $("theme").value = THEMES[name] ? name : "wikimedia";
  state.colors = {};
  paintColorControls();
  if (rerender) render();
}

function paintColorControls() {
  const theme = THEMES[$("theme").value] ?? THEMES.wikimedia;
  for (const row of $("colors").children) {
    const slot = row.dataset.slot;
    row.querySelector("input").value = toHex(state.colors[slot] ?? theme[kebab(slot)] ?? theme[slot]);
  }
}

/** The render spec for the controls (camelCase, as the WASM API takes it). */
function currentSpec() {
  const spec = { ...state.extra };
  const theme = $("theme").value;
  if (theme !== "wikimedia") spec.theme = theme;
  const title = $("title").value.trim();
  if (title) spec.title = title;
  const width = Math.round(Number($("width").value));
  if (width && width !== 1000) spec.width = Math.min(4000, Math.max(100, width));
  if ($("labels").checked) spec.labels = true;
  const languages = $("languages").value.split(/[,;\s]+/).filter(Boolean);
  if (languages.length) spec.languages = languages;
  if ($("target").value !== "commons") spec.target = $("target").value;
  if ($("projection").value !== "auto") spec.projection = $("projection").value;
  if ($("frame").value !== "auto") spec.frame = $("frame").value;
  if (!$("insets").checked) spec.insets = "none";
  if ($("credit").checked) spec.credit = true;
  if (Object.keys(state.colors).length) spec.colors = { ...state.colors };
  return spec;
}

/** Sets the controls from a render spec; settings without a control are kept. */
function setControls(spec) {
  const s = { ...spec };
  const take = (k, d) => {
    const v = s[k];
    delete s[k];
    return v ?? d;
  };
  applyTheme(take("theme", "wikimedia"), false);
  state.colors = take("colors", {});
  paintColorControls();
  $("title").value = take("title", "");
  $("width").value = take("width", 1000);
  $("labels").checked = take("labels", false);
  $("languages").value = take("languages", []).join(", ");
  $("target").value = take("target", "commons");
  $("projection").value = take("projection", "auto");
  $("frame").value = take("frame", "auto");
  $("insets").checked = take("insets", "auto") !== "none";
  $("credit").checked = take("credit", false);
  delete s.region;
  delete s.regions;
  state.extra = s;
  const extra = Object.keys(s).map(kebab);
  $("extra-settings").hidden = extra.length === 0;
  $("extra-settings").textContent = extra.length ? `Also from the recipe, kept in exports: ${extra.join(", ")}.` : "";
}

// ---------------------------------------------------------------- render

function render() {
  cancelAnimationFrame(pending);
  pending = requestAnimationFrame(() => guard(renderNow));
}

async function renderNow() {
  if (!generators[state.mode]) return;
  const picking = state.selected.length === 0;
  // Nothing selected yet: the world, to pick countries from.
  const g = picking ? await generator("countries") : generators[state.mode];
  const spec = picking
    ? { frame: "world", width: 1200, theme: $("theme").value, colors: { ...state.colors } }
    : { ...currentSpec(), regions: [...state.selected] };
  const t = performance.now();
  const out = g.render(spec);
  const ms = Math.round(performance.now() - t);
  lastOutput = picking ? null : out;
  const container = $("map-container");
  $("map-tooltip").hidden = true;
  container.innerHTML = out.svg;
  container.classList.add("is-ready");
  container.classList.toggle("is-picking", picking);
  $("map-frame").style.aspectRatio = `${out.width} / ${out.height}`;
  $("map-frame").style.setProperty("--map-aspect", String(out.width / out.height));
  const kb = Math.round(new Blob([out.svg]).size / 1024);
  $("render-meta").textContent = picking
    ? "choose countries"
    : `${out.regions} regions · ${out.width}×${out.height} · ${out.projection} · ${kb} KB · ${ms} ms`;
  const n = state.selected.length;
  $("toolbar-title").textContent = picking
    ? "Click countries to build your map"
    : `${n} ${n === 1 ? "country" : "countries"} · click a neighbour to add it`;
  for (const id of ["download-svg", "download-png", "download-recipe", "copy-api"]) $(id).disabled = picking;
  save();
}

/** The country a clicked map path stands for. */
function countryOf(path) {
  const subdivision = state.mode === "subdivisions"
    && path.classList.contains("mg-land")
    && !$("map-container").classList.contains("is-picking");
  if (!subdivision) return path.dataset.code;
  // A subdivision stands for its country (its ISO-2 class).
  for (const cls of path.classList) if (byIso2.has(cls)) return byIso2.get(cls);
  return null;
}

// ---------------------------------------------------------------- recipes

async function applyRecipe(text) {
  const report = $("recipe-report");
  report.hidden = false;
  report.classList.remove("has-issues");
  try {
    const recipe = parseRecipe(text);
    const mode = !recipe.dataset || recipe.dataset === "ne-admin0" ? "countries"
      : recipe.dataset === "ne-admin1" ? "subdivisions" : null;
    if (!mode) {
      throw new Error(`The dataset ${recipe.dataset} is on the API only: POST this recipe to ${API}/render (Content-Type: text/csv).`);
    }
    setControls(recipe.spec);
    await setMode(mode, { rerender: false });
    const { codes, unknown } = generators[mode].resolveRegions(recipe.regions);
    state.selected = codes;
    renderCountryList();
    render();
    report.textContent = `Applied: ${codes.length} ${codes.length === 1 ? "country" : "countries"}`
      + (unknown.length ? `. Not found: ${unknown.join(", ")}.` : ".");
    report.classList.toggle("has-issues", unknown.length > 0);
  } catch (e) {
    report.textContent = messageOf(e);
    report.classList.add("has-issues");
  }
}

function recipeText() {
  return recipeToCsv({ dataset: DATASET[state.mode], regions: state.selected, spec: currentSpec() });
}

/** The same map from the public API. */
function apiLink() {
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(currentSpec()).sort()) {
    if (k === "colors") {
      for (const [slot, c] of Object.entries(v).sort()) params.set(`color-${kebab(slot)}`, c);
    } else {
      params.set(kebab(k), Array.isArray(v) ? v.join(",") : String(v));
    }
  }
  const q = params.toString();
  return `${API}/maps/${DATASET[state.mode]}/${[...state.selected].sort().join(",")}.svg${q ? "?" + q : ""}`;
}

// ---------------------------------------------------------------- export

function fileName(ext) {
  const base = $("title").value.trim() || state.selected.slice(0, 4).map(countryName).join(", ") || "map";
  return `${base.replace(/[\\/:*?"<>|#{}[\]]+/g, "-")}.${ext}`;
}

function download(blob, name) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

async function downloadPng() {
  const { svg, width, height } = lastOutput;
  const img = new Image();
  img.src = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = width * 2;
  canvas.height = height * 2;
  canvas.getContext("2d").drawImage(img, 0, 0, canvas.width, canvas.height);
  URL.revokeObjectURL(img.src);
  canvas.toBlob((blob) => download(blob, fileName("png")), "image/png");
}

// ---------------------------------------------------------------- wiring

function bindControls() {
  $("app-picker-button").addEventListener("click", () => {
    const open = $("app-picker-menu").classList.toggle("is-hidden") === false;
    $("app-picker-button").setAttribute("aria-expanded", String(open));
  });
  document.addEventListener("click", (e) => {
    if (!$("app-picker").contains(e.target)) {
      $("app-picker-menu").classList.add("is-hidden");
      $("app-picker-button").setAttribute("aria-expanded", "false");
    }
  });
  $("theme-light").addEventListener("click", () => setInterfaceTheme("light"));
  $("theme-dark").addEventListener("click", () => setInterfaceTheme("dark"));

  for (const b of document.querySelectorAll("[data-mode]")) {
    b.addEventListener("click", () => guard(() => setMode(b.dataset.mode)));
  }
  $("country-search").addEventListener("input", renderCountryList);
  for (const id of ["country-list", "selected-chips"]) {
    $(id).addEventListener("click", (e) => {
      const b = e.target.closest("[data-code]");
      if (b) toggleCountry(b.dataset.code);
    });
  }
  $("clear-selection").addEventListener("click", () => {
    state.selected = [];
    renderCountryList();
    render();
  });

  $("theme").addEventListener("change", (e) => applyTheme(e.target.value));
  for (const id of ["title", "width", "labels", "languages", "target", "projection", "frame", "insets", "credit"]) {
    $(id).addEventListener("change", render);
  }

  const map = $("map-container");
  map.addEventListener("click", (e) => {
    const p = e.target.closest("path[data-code]");
    const code = p && countryOf(p);
    if (code && countryList.some((c) => c.code === code)) toggleCountry(code);
  });
  map.addEventListener("mousemove", (e) => {
    const p = e.target.closest("path[data-name]");
    const tip = $("map-tooltip");
    tip.hidden = !p;
    if (p) {
      tip.textContent = p.dataset.parentName ? `${p.dataset.name}, ${p.dataset.parentName}` : p.dataset.name;
      tip.style.left = `${e.clientX}px`;
      tip.style.top = `${e.clientY}px`;
    }
  });
  map.addEventListener("mouseleave", () => ($("map-tooltip").hidden = true));

  $("choose-recipe").addEventListener("click", () => $("recipe-file").click());
  $("recipe-file").addEventListener("change", async () => {
    const file = $("recipe-file").files[0];
    if (!file) return;
    $("recipe-text").value = await file.text();
    $("recipe-file").value = "";
    applyRecipe($("recipe-text").value);
  });
  $("apply-recipe").addEventListener("click", () => applyRecipe($("recipe-text").value));

  $("download-svg").addEventListener("click", () =>
    download(new Blob([lastOutput.svg], { type: "image/svg+xml" }), fileName("svg")));
  $("download-png").addEventListener("click", () => guard(downloadPng));
  $("download-recipe").addEventListener("click", () =>
    guard(() => download(new Blob([recipeText()], { type: "text/csv" }), fileName("csv"))));
  $("copy-api").addEventListener("click", () =>
    guard(async () => {
      await navigator.clipboard.writeText(apiLink());
      toast("API link copied: the same map, from map-generator.toolforge.org");
    }));
  $("reset-button").addEventListener("click", () => {
    state.selected = [];
    setControls({});
    $("recipe-text").value = "";
    $("recipe-report").hidden = true;
    renderCountryList();
    render();
  });
}

// ---------------------------------------------------------------- interface theme & storage

function read(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage unavailable (private browsing): nothing to keep.
  }
}

function initInterfaceTheme() {
  const saved = read(STORAGE.theme);
  const dark = saved ? saved === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
  setInterfaceTheme(dark ? "dark" : "light", false);
}

function setInterfaceTheme(theme, persist = true) {
  const dark = theme === "dark";
  $("app-shell").classList.toggle("theme-light", !dark);
  $("theme-light").classList.toggle("active", !dark);
  $("theme-dark").classList.toggle("active", dark);
  $("theme-light").setAttribute("aria-pressed", String(!dark));
  $("theme-dark").setAttribute("aria-pressed", String(dark));
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  if (persist) write(STORAGE.theme, theme);
}

/** The current map as a recipe, so a reload brings it back. */
function save() {
  write(STORAGE.recipe, state.selected.length ? recipeText() : "");
}

async function restore() {
  const saved = read(STORAGE.recipe);
  if (saved) {
    $("recipe-text").value = saved;
    await applyRecipe(saved);
    $("recipe-report").hidden = true;
  } else {
    await guard(() => setMode("countries"));
  }
}

// ---------------------------------------------------------------- helpers

function showLoading(on) {
  $("map-loading").hidden = !on;
}

function toast(message) {
  clearTimeout(toastTimer);
  $("toast").textContent = message;
  $("toast").hidden = false;
  toastTimer = setTimeout(() => ($("toast").hidden = true), 3200);
}

function messageOf(e) {
  return e instanceof Error ? e.message : String(e);
}

async function guard(fn) {
  try {
    return await fn();
  } catch (e) {
    toast(messageOf(e));
    console.error(e);
  }
}
