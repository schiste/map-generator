# Public API (v1)

`https://map-generator.toolforge.org/api/v1/` serves maps, their metadata and data joins
over HTTP, for tools such as [Maphue](https://github.com/schiste/map-coloring) that
shouldn't have to bundle the engine or hundreds of MB of boundary data.

- **Same maps everywhere.** The options are the WebAssembly build's `RenderSpec`, and a map
  from the API is byte-identical to the CLI's and the WASM build's for the same data and options.
- **Cacheable.** Output is deterministic. The `ETag` is the SHA-1 of the body, the same hash
  Wikimedia Commons stores for each file version.
- **Read-only and anonymous.** No accounts or cookies, and nothing users send is stored.
  Requests are logged without IP addresses or user agents. CORS is open to every origin.
- **Described.** [`/api/v1/openapi.json`](https://map-generator.toolforge.org/api/v1/openapi.json)
  (OpenAPI 3.1) lists every endpoint, parameter and error.

## Quick start

```sh
# France's départements, labelled, 900 px wide
curl -o france.svg 'https://map-generator.toolforge.org/api/v1/maps/ne-admin1/FRA.svg?labels=true&width=900'

# Its metadata: size, insets, empty areas for a legend, SHA-1, credit, licence, boundary version
curl 'https://map-generator.toolforge.org/api/v1/maps/ne-admin1/FRA.json?labels=true&width=900'
```

```js
// From a web page on any origin, with no build step:
import { MapgenClient } from "https://map-generator.toolforge.org/api/v1/client.js";

const api = new MapgenClient();
const svg = await api.map("ne-admin1", "FRA", { labels: true, languages: ["fr"], colors: { water: "#c6ecff" } });
const regions = await api.features("ne-admin1", "FRA"); // codes, names, parents: no SVG parsing
```

## Endpoints

| | |
| --- | --- |
| `GET /api/v1/` | Links to everything below |
| `GET /api/v1/openapi.json` | OpenAPI description |
| `GET /api/v1/client.js` | Dependency-free ES module client (`MapgenClient`) |
| `GET /api/v1/health` | `200` when the data is loaded |
| `GET /api/v1/version` | mapgen version, commit, SVG contract version, dataset releases |
| `GET /api/v1/themes`, `/bbox-presets` | Colour themes and slots; frame presets |
| `GET /api/v1/datasets` | Hosted datasets: level, licence, share-alike, release, boundary year, points of view, languages |
| `GET /api/v1/datasets/{dataset}/regions` | What a map can be made for: countries, continents, `world`… with each one's licence |
| `GET /api/v1/datasets/{dataset}/regions/{region}/features` | The map's regions without geometry: `code`, `name`, `names` (`?languages=fr,zh-Hant`), `parent`, `parentName`, `country` |
| `GET /api/v1/maps/{dataset}/{region}.svg` | The map. `{region}` can list several regions, by code or name: `/maps/ne-admin0/BEL,LUX,NLD.svg` |
| `GET /api/v1/maps/{dataset}/{region}.json` | Its metadata (below) |
| `GET /api/v1/maps/{dataset}/{region}.html` | An interactive page with colour pickers |
| `POST /api/v1/render` | The same, with a JSON body `{"dataset", "region", "worldview"?, "spec": RenderSpec}`, or a [map recipe](recipes.md) (`Content-Type: text/csv`) |
| `POST /api/v1/match` | Compare a data table's codes with a map |
| `GET /api/v1/crosswalks`, `/crosswalks/{id}.csv` | Hosted crosswalks between boundary versions |
| `POST /api/v1/reshape` | Move a data table to new codes through a crosswalk |
| `GET /api/v1/contract/fixture.svg` | A small map following the [SVG contract](contract.md), for your tests |

### Datasets at launch

| id | Regions |
| --- | --- |
| `ne-admin0` | countries (`FRA`), continents (`Europe`, `South America`), `world`; points of view with `worldview=` |
| `ne-admin1` | first-level subdivisions of a country (`FRA` gives départements, with régions as parents) |
| `us-counties` | `USA`: counties with FIPS codes (`US-31109`), states as parents |
| `geoboundaries-adm1` | one country per file (`FRA`), each with its own licence |

`/api/v1/datasets` is authoritative.

### Map parameters

Query parameters are the `RenderSpec` fields in kebab-case:
- `width`, `theme`, `labels`, `languages=fr,zh-Hant`, `target=commons|web`, `css-vars`;
- `frame`, `bbox`, `projection`, `insets`, `border-mode`, `credit`;
- colour slots as `color-water=%23c6ecff`, `color-context-land=tan`.

Also:
- `worldview=IND` for a Natural Earth point of view (applied to the neighbouring countries,
  and to the regions themselves for `ne-admin0`);
- `release=<dataset release>`, which pins the URL: the response is then cached as immutable,
  and the URL returns 404 once that release is no longer hosted.

Unknown or misspelled parameters are errors, never silently ignored.

The `.json` metadata:
- `width`, `height`, `projection`, `center`;
- `insets`, `outsideFrame` (regions shown nowhere);
- `legendSlots`: empty areas for a legend or title, largest first, with `landShare`;
- `sha1`, `credit`, `licence`, `licenceUrl`, `shareAlike`;
- `boundaryYear`, `sourceRelease`, `release`;
- `contract`, and `url` (the canonical URL).

### Data joins

```sh
# Which codes of my table aren't on the map? (e.g. data on 2020 counties, map on older ones)
curl -X POST https://map-generator.toolforge.org/api/v1/match -H 'Content-Type: application/json' -d '{
  "dataset": "us-counties", "region": "USA",
  "table": "fips,pop\n02063,7000\n02066,2600\n", "codeColumn": "fips", "codePrefix": "US-"}'
# → dataNotOnMap, mapWithoutData, missingShare, and hints naming a hosted crosswalk

# Move a table to 2020 county codes
curl -X POST https://map-generator.toolforge.org/api/v1/reshape -H 'Content-Type: application/json' -d '{
  "table": "fips,pop\n02261,9600\n", "codeColumn": "fips", "crosswalk": "us-counties-2010-2020"}'
```

`reshape` adds values up and shares them out, so use it on counts, not rates. Values without
a rule, such as a split without weights, give `422` with a `conflicts` list, unless you pass
`"allowConflicts": true`.

## Responses

- **Content types:** `image/svg+xml`, `application/json`, `text/html` and `text/csv`, all in UTF-8.
- **Caching:** `ETag` gives `304` on `If-None-Match`. Maps are `public, max-age=2592000` (30
  days, since data changes only with a release), or `immutable` for a year with `release=`.
  Listings (datasets, regions, features, crosswalks) are cached for a day, and
  `health`/`version` not at all.
- **Provenance headers:** `Content-Location` (the canonical URL), `Link: <licence>; rel="license"`,
  `X-Mapgen-Version`, `X-Mapgen-Contract` and `X-Dataset-Release`. All are exposed to
  cross-origin scripts.
- **Errors:** [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) `application/problem+json`, as
  `{"type", "title", "status", "detail", "param"?}`:
  - `400`: invalid parameter;
  - `404`: unknown dataset, region, format, release or point of view;
  - `422`: reshape conflicts;
  - `503`: busy or too slow; retry after `Retry-After`.

## Limits

- `width` at most 4000 px.
- Request bodies up to 5 MB.
- 20 s per render.
- A few renders at a time; beyond that, requests wait up to 10 s, then get `503`.

Common maps are pre-rendered after every data update, so most requests are served from the
cache. There are no user-supplied geometries: for one-off maps from your own data, use the
WebAssembly build in the browser.

## Versioning

- `/api/v1` is stable. Additions (endpoints, parameters, response members) don't change the
  version, so clients should ignore members they don't know.
- Removing or changing something goes to `/api/v2`. `/api/v1` then keeps working for at
  least six months, announced with `Deprecation` and `Sunset` headers and in the changelog.
- The SVG contract (`data-mapgen-contract`) has its own version: see [contract.md](contract.md).

## Terms

The API runs on [Wikimedia Toolforge](https://wikitech.wikimedia.org/wiki/Portal:Toolforge)
under the [Wikimedia Cloud Services Terms of Use](https://wikitech.wikimedia.org/wiki/Wikitech:Cloud_Services_Terms_of_use).
The code is MIT-licensed. Each map carries its data's credit and licence (`credit`,
`licence`, `shareAlike`, and the `Link` header). Share-alike data (for example some
geoBoundaries countries) requires sharing derived maps under the same licence.

## Running it

```sh
scripts/prepare-server-data.sh target/server-data --worldviews IND,PAK   # add --geoboundaries ADM1 for all countries
MAPGEN_DATA_DIR=target/server-data MAPGEN_CACHE_DIR=target/server-cache PORT=8000 \
  cargo run --release -p mapgen-server
```

Settings (environment):

| Variable | Default | Meaning |
| --- | --- | --- |
| `MAPGEN_DATA_DIR` | `data` | the prepared data directory |
| `MAPGEN_CACHE_DIR` | none (no cache) | disk cache of rendered maps |
| `MAPGEN_CACHE_MAX_BYTES` | 2 GB | cache size limit |
| `MAPGEN_MAX_CONCURRENT` | 2 | renders at a time |
| `MAPGEN_RENDER_TIMEOUT` | 20 | seconds per render |
| `MAPGEN_WWW_DIR` | none | a static site served at `/` (the WebAssembly playground) |
| `MAPGEN_MAX_WIDTH` | 4000 | largest `width` |
| `PORT` | 8000 | port to listen on |

`datasets.toml` in the data directory lists what is served. `prepare-server-data.sh` writes
it, and `crates/mapgen-server/src/registry.rs` documents its fields. Deployment is described
in [toolforge.md](toolforge.md).
