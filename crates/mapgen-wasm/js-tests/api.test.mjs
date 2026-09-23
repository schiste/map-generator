// Integration tests of the published package (wasm-pack build --target nodejs).
// Run: npm test  (from crates/mapgen-wasm)
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..", "..");
const require = createRequire(import.meta.url);
const mapgen = require(join(here, "..", "pkg", "node", "mapgen_wasm.js"));
const { MapGenerator, themes, bboxPresets, version, reshape, parseRecipe, recipeToCsv } = mapgen;

const fixtures = join(repo, "crates", "mapgen-data", "tests", "fixtures");
const read = (p) => readFileSync(p, "utf8");
const twin = read(join(fixtures, "twin-regions.geojson"));

function twinGenerator() {
  const gen = new MapGenerator();
  assert.equal(gen.setSubject(twin), 2);
  return gen;
}

test("output is byte-identical to the native golden SVG", () => {
  const out = twinGenerator().render({ width: 400, title: "Twin regions", labels: true });
  assert.equal(out.svg, read(join(fixtures, "twin-regions.svg")));
});

test("result is a plain JS object with metadata", () => {
  const out = twinGenerator().render();
  assert.equal(Object.getPrototypeOf(out), Object.prototype);
  assert.equal(out.width, 1000);
  assert.ok(out.height > 0);
  assert.equal(out.projection, "laea");
  assert.equal(out.regions, 2);
  assert.deepEqual(out.outsideFrame, []);
  assert.equal(out.center.length, 2);
  assert.equal(out.html, undefined);
  assert.equal(out.contract, 1);
  assert.equal(out.legendSlots.length, 9);
  const best = out.legendSlots[0];
  assert.ok(best.width >= 48 && best.height >= 48, JSON.stringify(best));
  assert.ok(best.landShare >= 0 && best.landShare <= 1);
});

test("matchCodes and reshape: data from another boundary year", () => {
  const m = twinGenerator().matchCodes({ table: "id,v\n01,1\n09,2\n", codeColumn: "id", codePrefix: "XA-" });
  assert.equal(m.matched, 1);
  assert.deepEqual(m.dataNotOnMap, ["XA-09"]);
  assert.deepEqual(m.mapWithoutData, ["XA-02"]);
  assert.throws(() => twinGenerator().matchCodes({ codez: [] }), { message: /codez/ });

  // Valdez-Cordova (02261) split in 2019; Wade Hampton (02270) was renamed.
  const crosswalk = { table: "from,to,weight\n02261,02063,0.278325\n02261,02066,0.721675\n02270,02158,1\n" };
  const r = reshape({ table: "fips,pop\n02261,1000\n02270,8000\n02020,5\n", codeColumn: "fips", crosswalk });
  assert.equal(r.csv, "fips,pop\n02020,5\n02063,278.325\n02066,721.675\n02158,8000\n");
  assert.deepEqual([r.direct, r.weighted, r.conflicts.length], [2, 1, 0]);
  const blocked = reshape({ table: "fips,pop\n02261,1\n", codeColumn: "fips", crosswalk: { table: "from,to\n02261,02063\n02261,02066\n" } });
  assert.equal(blocked.csv, undefined);
  assert.match(blocked.conflicts[0].reason, /split without weights/);
  assert.throws(() => reshape({ table: "a\n", codeColumn: "a", crosswalk: "us-counties-2010-2020" }), { message: /HTTP API/ });
});

test("recipes: parse, validate, and write back", () => {
  const r = parseRecipe("# custom\nkey,value\ndataset,countries\nregion,France\nregions,DEU;Italy\ntitle,\"A, B\"\nwidth,1200\nlabels,true\ncolor-water,#c6ecff\nlanguages,fr;de\n");
  assert.equal(r.dataset, "ne-admin0");
  assert.deepEqual(r.regions, ["France", "DEU", "Italy"]);
  assert.deepEqual(r.spec, { title: "A, B", width: 1200, labels: true, colors: { water: "#c6ecff" }, languages: ["fr", "de"] });
  const csv = recipeToCsv({ dataset: "ne-admin0", regions: ["FRA", "DEU"], spec: { ...r.spec, cssVars: true } });
  assert.match(csv, /^# map-generator recipe v1/);
  assert.match(csv, /key,value\ndataset,ne-admin0\nregion,FRA\nregion,DEU\ncolor-water,#c6ecff\ncss-vars,true\nlabels,true\nlanguages,fr;de\ntitle,"A, B"\nwidth,1200\n/);
  assert.deepEqual(parseRecipe(csv).spec, { ...r.spec, cssVars: true });
  assert.throws(() => parseRecipe("key,value\ncolour-water,red\n"), { message: /colour-water/ });
  assert.throws(() => recipeToCsv({ spec: { colours: {} } }), { message: /colours/ });
  assert.deepEqual(parseRecipe("country,category\nFrance,A\n").regions, ["France"]);
});

test("errors are thrown as Error with helpful messages", () => {
  assert.throws(() => new MapGenerator().render(), { message: /setSubject/ });
  const gen = twinGenerator();
  assert.throws(() => gen.render({ colours: {} }), { message: /colours/ });
  assert.throws(() => gen.render({ theme: "neon" }), { message: /wikimedia/ });
  assert.throws(() => gen.render({ colors: { water: "red;}</style>" } }), { message: /invalid colour/ });
  assert.throws(() => gen.render({ width: 5 }), { message: /width/ });
  assert.throws(() => gen.render("FRA"), { message: /plain object/ });
  assert.throws(() => new MapGenerator().setSubject("{nope"), Error);
});

test("the same generator renders many variations deterministically", () => {
  const gen = twinGenerator();
  const a = gen.render({ theme: "dark" }).svg;
  gen.render({ theme: "light", labels: true });
  assert.equal(gen.render({ theme: "dark" }).svg, a);
});

test("html output embeds the colour editor", () => {
  const out = twinGenerator().render({ format: "html", title: "Demo" });
  assert.match(out.html, /^<!doctype html>/);
  assert.match(out.html, /Download SVG/);
  assert.match(out.svg, /var\(--mg-water,#c6ecff\)/);
});

test("lookup tables and version", () => {
  assert.equal(themes().wikimedia.water, "#c6ecff");
  assert.deepEqual(Object.keys(themes()).sort(), ["dark", "light", "mono", "wikimedia"]);
  assert.deepEqual(bboxPresets().europe, [-25, 34, 45, 72]);
  const pkg = JSON.parse(read(join(here, "..", "pkg", "node", "package.json")));
  assert.equal(version(), pkg.version);
});

test("TypeScript definitions declare the typed API", () => {
  const dts = read(join(here, "..", "pkg", "node", "mapgen_wasm.d.ts"));
  for (const s of [
    "export interface RenderSpec",
    "export interface MapOutput",
    "export interface LayerSpec",
    "render(spec?: RenderSpec): MapOutput;",
    "setSubject(geojson: string, spec?: LayerSpec): number;",
    "regions(): string[];",
  ]) {
    assert.ok(dts.includes(s), `d.ts should contain ${JSON.stringify(s)}`);
  }
  assert.ok(!/render\(spec: /.test(dts), "render must not also be declared with a required spec");
});

// Real-data parity with the native CLI: the gallery in docs/examples was made
// by the native binary; WebAssembly must reproduce it byte for byte. Skipped
// when the datasets are not downloaded (they are git-ignored), unless
// MAPGEN_REQUIRE_DATA=1 (as in CI), where missing data is a failure.
const requireData = process.env.MAPGEN_REQUIRE_DATA === "1";
const skipUnless = (ok) => (ok || requireData ? false : "no data/ (run scripts/fetch-data.sh)");
const data = join(repo, "data");
const examples = join(repo, "docs", "examples");
const hasNE = [
  "ne_10m_admin_0.geojson",
  "ne_10m_admin_1.geojson",
  "ne_10m_lakes.geojson",
  "ne_10m_disputed_lines.geojson",
].every((f) =>
  existsSync(join(data, f)),
);
const ne = hasNE || requireData
  ? {
      admin0: read(join(data, "ne_10m_admin_0.geojson")),
      admin1: read(join(data, "ne_10m_admin_1.geojson")),
      lakes: read(join(data, "ne_10m_lakes.geojson")),
      disputed: read(join(data, "ne_10m_disputed_lines.geojson")),
    }
  : null;

function withContext(subject, layerSpec) {
  const gen = new MapGenerator();
  gen.setSubject(subject, layerSpec);
  gen.setContext(ne.admin0);
  gen.setLakes(ne.lakes);
  gen.setDisputed(ne.disputed);
  return gen;
}

test("parity: France départements (Natural Earth)", { skip: skipUnless(hasNE) }, () => {
  const t = performance.now();
  const out = withContext(ne.admin1, { dataset: "ne-admin1" }).render({
    region: "FRA",
    labels: true,
    width: 900,
    title: "France — départements",
  });
  console.log(`  france-departements: ${(performance.now() - t).toFixed(0)} ms incl. parsing`);
  assert.equal(out.svg, read(join(examples, "france-departements.svg")));
});

test("parity: Europe (continent filter + bbox preset)", { skip: skipUnless(hasNE) }, () => {
  const out = withContext(ne.admin0, { dataset: "ne-admin0", filterProperty: "CONTINENT" }).render({
    region: "Europe",
    bbox: "europe",
    width: 900,
    title: "Europe",
  });
  assert.equal(out.svg, read(join(examples, "europe.svg")));
});

test("parity: South America (curved label along Chile)", { skip: skipUnless(hasNE) }, () => {
  const out = withContext(ne.admin0, { dataset: "ne-admin0", filterProperty: "CONTINENT" }).render({
    region: "South America",
    bbox: "south-america",
    labels: true,
    width: 700,
    title: "South America",
  });
  // Commons target (the default): rotated letters, since librsvg has no textPath.
  assert.match(out.svg, /<g class="mg-label" aria-label="Chile"><text transform=/);
  assert.equal(out.svg, read(join(examples, "south-america.svg")));
});

test("parity: world map (Equal Earth, seam splitting)", { skip: skipUnless(hasNE) }, () => {
  const gen = new MapGenerator();
  gen.setSubject(ne.admin0, { dataset: "ne-admin0" });
  gen.setLakes(ne.lakes);
  gen.setDisputed(ne.disputed);
  const out = gen.render({ frame: "world", theme: "dark", padding: 10, width: 1200, title: "World" });
  assert.equal(out.projection, "equal-earth");
  assert.equal(out.svg, read(join(examples, "world-dark.svg")));
});

test("parity: Japan labelled in four languages", { skip: skipUnless(hasNE) }, () => {
  const languages = ["ja", "ko", "zh-Hans", "zh-Hant"];
  const out = withContext(ne.admin1, { dataset: "ne-admin1", languages }).render({
    region: "JPN",
    labels: true,
    languages,
    theme: "light",
    width: 700,
    title: "Japan",
  });
  assert.match(out.svg, /<switch>\n<text systemLanguage="zh-Hans,zh-CN,zh-SG,zh-MY"/);
  assert.equal(out.svg, read(join(examples, "japan-light.svg")));
});

const capitalsFile = join(data, "ne_10m_capitals.geojson");
test("parity: Italy with capitals and neighbour names", { skip: skipUnless(hasNE && existsSync(capitalsFile)) }, () => {
  const gen = withContext(ne.admin1, { dataset: "ne-admin1" });
  assert.ok(gen.setPlaces(read(capitalsFile), ["de"]) > 2000);
  const out = gen.render({
    region: "ITA",
    capitals: "all",
    contextLabels: true,
    width: 700,
    title: "Italy — capitals",
  });
  assert.match(out.svg, /<circle class="mg-place mg-capital"[^>]*data-name="Rome"[^>]*data-wikidata="Q220"/);
  assert.equal(out.svg, read(join(examples, "italy-capitals.svg")));
  // Names follow the viewer's language like region labels.
  const de = gen.render({ region: "ITA", capitals: "countries", languages: ["de"], width: 700 });
  assert.match(de.svg, /<text systemLanguage="de" class="mg-place-label"[^>]*>Rom</);
  assert.equal(gen.setPlaces(), 0);
  assert.doesNotMatch(gen.render({ region: "ITA", capitals: "all" }).svg, /mg-place/);
});

test("mixed levels: countries and single subdivisions", { skip: skipUnless(hasNE) }, () => {
  const gen = withContext(ne.admin0, { dataset: "ne-admin0" });
  assert.equal(gen.setSubdivisions(ne.admin1, { dataset: "ne-admin1" }), gen.subdivisions().length);
  assert.ok(gen.subdivisions().some((s) => s.code === "FR-67" && s.parentName));
  const { codes, unknown } = gen.resolveRegions(["Switzerland", "Bayern", "fr-67", "Atlantis"]);
  assert.deepEqual(codes, ["CHE", "DE-BY", "FR-67"]);
  assert.deepEqual(unknown, ["Atlantis"]);
  const out = gen.render({ regions: codes, width: 600 });
  assert.equal(out.regions, 3);
  for (const code of codes) assert.ok(out.svg.includes(`data-code="${code}"`), code);
  // Countries only: the subdivisions play no part.
  assert.equal(gen.render({ regions: ["CHE"] }).regions, 1);
  assert.throws(() => gen.render({ regions: ["DEU", "DE-BY"] }), /lies in Germany/);
  gen.setSubdivisions();
  assert.throws(() => gen.render({ regions: ["DE-BY"] }), /DE-BY/);
});

test("parity: Fiji with CSS custom properties", { skip: skipUnless(hasNE) }, () => {
  const out = withContext(ne.admin1, { dataset: "ne-admin1" }).render({
    region: "FJI",
    cssVars: true,
    width: 500,
    title: "Fiji (straddles 180°)",
  });
  assert.equal(out.svg, read(join(examples, "fiji.svg")));
});

test("custom maps: several countries, picked by name", { skip: skipUnless(hasNE) }, () => {
  const gen = withContext(ne.admin0, { dataset: "ne-admin0" });
  const countries = gen.countries();
  assert.ok(countries.length > 200);
  assert.deepEqual(countries.find((c) => c.code === "FRA"), { code: "FRA", name: "France", iso2: "fr", wikidata: "Q142", names: {} });
  const { codes, unknown } = gen.resolveRegions(["France", "de", "ITA", "Atlantis", "france"]);
  assert.deepEqual([codes, unknown], [["FRA", "DEU", "ITA"], ["Atlantis"]]);
  // Clipperton, Baikonur, Brazilian Island and Australian territories share
  // their country's ISO-2 code: the country wins.
  assert.deepEqual(gen.resolveRegions(["fr", "kz", "br", "au"]).codes, ["FRA", "KAZ", "BRA", "AUS"]);
  // Wikidata items, as the Choropleth map template takes them.
  assert.deepEqual(gen.resolveRegions(["Q142", "q183"]).codes, ["FRA", "DEU"]);
  assert.equal(countries.find((c) => c.code === "FRA").wikidata, "Q142");
  const out = gen.render({ regions: codes, width: 600, labels: true });
  assert.match(out.svg, /<path id="DEU" class="mg-land country de"[^>]* data-wikidata="Q183"/);
  for (const c of codes) assert.match(out.svg, new RegExp(`<path id="${c}" class="mg-land`));
  assert.doesNotMatch(out.svg, /<path id="ESP" class="mg-land/);
  assert.match(out.svg, /<path id="ESP" class="mg-context/);
  // Subdivisions of two countries, named through the neighbouring countries.
  const sub = withContext(ne.admin1, { dataset: "ne-admin1" });
  assert.deepEqual(sub.resolveRegions(["Belgium", "NLD"]).codes, ["BEL", "NLD"]);
  assert.ok(sub.countries().some((c) => c.code === "BEL" && c.name === "Belgium"));
  assert.throws(() => sub.render({ regions: ["BEL", "XXX"] }), { message: /XXX/ });
});

const indFile = join(data, "ne_10m_admin_0_ind.geojson");
const areasFile = join(data, "ne_10m_disputed_areas.geojson");
test("parity: Kashmir from India's point of view", { skip: skipUnless(hasNE && existsSync(indFile) && existsSync(areasFile)) }, () => {
  const india = read(indFile);
  const gen = new MapGenerator();
  gen.setSubject(india, { dataset: "ne-admin0", worldview: "IND" });
  gen.setContext(india, { dataset: "ne-admin0", worldview: "IND" });
  gen.setLakes(ne.lakes);
  gen.setDisputed(ne.disputed);
  gen.setDisputedAreas(read(areasFile));
  const out = gen.render({ bbox: "66,26,84,38", labels: true, width: 600, title: "Kashmir (IND view)" });
  assert.match(out.svg, /Natural Earth \(IND view\)/);
  assert.equal(out.svg, read(join(examples, "kashmir-ind.svg")));
});

const gbFile = join(data, "geoboundaries", "FRA-ADM1.geojson");
test("parity: France régions (geoBoundaries, credited)", { skip: skipUnless(hasNE && existsSync(gbFile)) }, () => {
  const lic = JSON.parse(read(gbFile.replace(/\.geojson$/, ".license.json")));
  const out = withContext(read(gbFile), {
    dataset: "geoboundaries",
    attribution: `${lic.source} (${lic.license}) via ${lic.via}`,
  }).render({
    labels: true,
    credit: true,
    width: 900,
    title: "France — régions",
    boundaryYear: lic.year,
    sourceRelease: lic.release,
  });
  // The gallery map is rendered from a GeoPackage made by `mapgen convert
  // --ids-from`, which replaces geoBoundaries' obsolete code for Corsica
  // (FR-20R) with the current one (FR-COR). The id also changes the order of
  // the paths, and of the border runs within each border path. Everything
  // else must be identical.
  const sortRuns = (line) =>
    line.replace(/ d="([^"]*)"/, (_, d) => ` d="${d.split(/(?=M)/).sort().join("")}"`);
  const lines = (svg) =>
    svg
      .replaceAll('id="FR-20R"', 'id="FR-COR"')
      .replaceAll('data-code="FR-20R"', 'data-code="FR-COR"')
      .split("\n")
      .map(sortRuns)
      .sort();
  assert.deepEqual(lines(out.svg), lines(read(join(examples, "france-regions.svg"))));
});
