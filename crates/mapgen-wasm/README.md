# mapgen-wasm

The [map-generator](https://github.com/schiste/map-generator) engine compiled to
WebAssembly: deterministic, restylable SVG maps in the browser or Node.js, with
**byte-identical output to the native CLI**.

```js
import init, { MapGenerator } from "./pkg/mapgen_wasm.js"; // wasm-pack --target web

await init();
const gen = new MapGenerator();
gen.setSubject(await (await fetch("ne_10m_admin_1.geojson")).text(), { dataset: "ne-admin1" });
gen.setContext(countriesGeoJson); // optional: neighbouring countries (Natural Earth Admin-0)
gen.setLakes(lakesGeoJson);       // optional

const map = gen.render({
  region: "FRA",
  theme: "wikimedia",
  colors: { water: "#bfe3f2", earth: "#fff8e7", background: "none" },
  labels: true,
});
document.body.innerHTML = map.svg; // also: map.width, map.height, map.projection, map.outsideFrame
```

- `MapGenerator` parses each GeoJSON **once**; `render()` can then be called
  repeatedly (typically 30–200 ms for a country).
- Options mirror the CLI flags (`RenderSpec` in the TypeScript definitions).
  Unknown or misspelled options are errors, not silently ignored.
- `format: "html"` also returns an interactive page with colour pickers (and
  `textPath` curved labels; SVG output defaults to `target: "commons"`, rotated
  letters that Wikimedia's renderer can draw).
- Data joins: `gen.matchCodes({ table, codeColumn })` lists codes the map lacks and
  regions without data; `reshape({ table, codeColumn, crosswalk: { table } })` moves
  numeric data to new codes (splits need weights; conflicts are returned, not guessed).
- Multilingual labels: read names with `setSubject(text, { languages: ["fr", "zh-Hant"] })`,
  then `render({ labels: true, languages: ["fr", "zh-Hant"] })`.
- `themes()`, `bboxPresets()` and `version()` expose the built-in tables.
- GeoPackage input is not available in WebAssembly (it needs SQLite); use GeoJSON.

## Build and test

```sh
npm install                 # TypeScript, for the type test
npm run build               # pkg/node (Node.js) and www/pkg (browser)
npm run typecheck           # compiles js-tests/types.ts against the generated .d.ts
npm test                    # node --test: API, errors, and native parity
wasm-pack test --node       # Rust-side tests of the JS API, in Node
WASM_BINDGEN_USE_BROWSER=1 wasm-pack test --headless --chrome   # …and in Chrome
npm run serve               # playground at http://localhost:8080 (after build:web)
```

The parity tests re-render the repository's example gallery in WebAssembly and
compare it byte for byte with the native output. They need the datasets
(`scripts/fetch-data.sh ne-geojson`, `geoboundaries FRA ADM1` and `ne-worldview IND`
from the repo root) and are skipped without them, unless `MAPGEN_REQUIRE_DATA=1`.
