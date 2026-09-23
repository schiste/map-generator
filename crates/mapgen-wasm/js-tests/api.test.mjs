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
const { MapGenerator, themes, bboxPresets, version } = mapgen;

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
const hasNE = ["ne_10m_admin_0.geojson", "ne_10m_admin_1.geojson", "ne_10m_lakes.geojson"].every((f) =>
  existsSync(join(data, f)),
);
const ne = hasNE || requireData
  ? {
      admin0: read(join(data, "ne_10m_admin_0.geojson")),
      admin1: read(join(data, "ne_10m_admin_1.geojson")),
      lakes: read(join(data, "ne_10m_lakes.geojson")),
    }
  : null;

function withContext(subject, layerSpec) {
  const gen = new MapGenerator();
  gen.setSubject(subject, layerSpec);
  gen.setContext(ne.admin0);
  gen.setLakes(ne.lakes);
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

test("parity: world map (Equal Earth, seam splitting)", { skip: skipUnless(hasNE) }, () => {
  const gen = new MapGenerator();
  gen.setSubject(ne.admin0, { dataset: "ne-admin0" });
  gen.setLakes(ne.lakes);
  const out = gen.render({ frame: "world", theme: "dark", padding: 10, width: 1200, title: "World" });
  assert.equal(out.projection, "equal-earth");
  assert.equal(out.svg, read(join(examples, "world-dark.svg")));
});

const gbFile = join(data, "geoboundaries", "FRA-ADM1.geojson");
test("parity: France régions (geoBoundaries, credited)", { skip: skipUnless(hasNE && existsSync(gbFile)) }, () => {
  const lic = JSON.parse(read(gbFile.replace(/\.geojson$/, ".license.json")));
  const out = withContext(read(gbFile), {
    dataset: "geoboundaries",
    attribution: `${lic.source} (${lic.license}) via ${lic.via}`,
  }).render({ labels: true, credit: true, width: 900, title: "France — régions" });
  assert.equal(out.svg, read(join(examples, "france-regions.svg")));
});
