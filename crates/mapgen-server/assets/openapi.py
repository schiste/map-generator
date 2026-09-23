"""Generates openapi.json (python3 crates/mapgen-server/assets/openapi.py).
Kept as code so the description stays valid JSON and in one place."""
import json, os

def ref(name): return {"$ref": f"#/components/schemas/{name}"}
def problem(desc): return {"description": desc, "content": {"application/problem+json": {"schema": ref("Problem")}}}
def js(schema, desc="OK"): return {"description": desc, "content": {"application/json": {"schema": schema}}}
P_DATASET = {"name": "dataset", "in": "path", "required": True, "schema": {"type": "string"}, "example": "ne-admin1"}
P_REGION = {"name": "region", "in": "path", "required": True, "schema": {"type": "string"}, "example": "FRA"}
P_LANGS = {"name": "languages", "in": "query", "schema": {"type": "string"}, "description": "Comma-separated BCP 47 tags, e.g. `fr,zh-Hant`."}
MAP_HEADERS = {
    "ETag": {"description": "SHA-1 of the body, as Commons stores it.", "schema": {"type": "string"}},
    "Content-Location": {"description": "Canonical URL of this map.", "schema": {"type": "string"}},
    "X-Mapgen-Version": {"schema": {"type": "string"}},
    "X-Mapgen-Contract": {"description": "SVG contract version (docs/contract.md).", "schema": {"type": "string"}},
    "X-Dataset-Release": {"schema": {"type": "string"}},
    "Link": {"description": "`<licence url>; rel=\"license\"`", "schema": {"type": "string"}},
}

render_params = [
    ("width", "integer", "Width in pixels (at most 4000; default 1000)."),
    ("padding", "integer", "Padding in pixels."),
    ("precision", "integer", "Decimal places of coordinates (default 1)."),
    ("title", "string", "Document title."),
    ("show-title", "boolean", "Draw the title in a band above the map."),
    ("caption", "string", "Text drawn under the map (wrapped)."),
    ("alt", "string", "Description for screen readers (<desc id=\"description\">)."),
    ("height", "integer", "Fixed height in pixels; the frame widens to fill it."),
    ("attribution", "string", "Data credit (default: from the data)."),
    ("credit", "boolean", "Draw the credit in the bottom-right corner."),
    ("boundary-year", "string", "Year the boundaries represent (default: from the data)."),
    ("source-release", "string", "Dataset release (default: the hosted one)."),
    ("theme", "string", "`wikimedia` (default), `light`, `dark` or `mono`."),
    ("labels", "boolean", "Label the regions."),
    ("context-labels", "boolean", "Name the neighbouring countries, where room is left."),
    ("capitals", "string", "`none` (default), `countries` (national capitals) or `all` (also regional capitals inside the map)."),
    ("languages", "string", "Also label in these languages (comma-separated BCP 47 tags)."),
    ("target", "string", "`commons` (default for SVG; curved labels as rotated letters, for librsvg) or `web` (textPath)."),
    ("border-mode", "string", "`layer` (default) or `regions`."),
    ("css-vars", "boolean", "Colours as `var(--mg-<slot>, …)`."),
    ("frame", "string", "`auto`, `all` or `world`."),
    ("bbox", "string", "`west,south,east,north` or a preset name (see /bbox-presets)."),
    ("projection", "string", "`auto`, `laea`, `equal-earth`, `albers`, `lcc`."),
    ("parallels", "string", "Standard parallels for albers/lcc: `south,north`."),
    ("center-lon", "number", "Central meridian."),
    ("insets", "string", "`auto` (default) or `none`."),
    ("max-insets", "integer", "At most this many insets."),
    ("leaders", "boolean", "Leader lines for small regions' labels (default true)."),
    ("curved-labels", "boolean", "Curve labels along long, thin regions (default true)."),
    ("label-size", "number", "Label size in pixels."),
    ("label-min-scale", "number", "Smallest label size as a fraction of label-size."),
    ("border-width", "number", None), ("parent-border-width", "number", None), ("outline-width", "number", None),
    ("context-border-width", "number", None), ("disputed-border-width", "number", None),
    ("snap", "number", "Snap neighbours within this many pixels onto the outline."),
    ("simplify", "number", "Simplification tolerance in pixels."),
    ("min-area", "number", "Drop islands and lakes smaller than this many px²."),
    ("margin", "number", "Room around the frame, as a fraction of its size."),
    ("worldview", "string", "Natural Earth point of view for disputed borders, e.g. `IND` (see /datasets: worldviews)."),
    ("release", "string", "Pin the URL to a dataset release: the response is then cached as immutable; 404 once that release is gone."),
]
map_query = [{"name": n, "in": "query", "schema": {"type": t}, **({"description": d} if d else {})} for n, t, d in render_params]
map_query.append({"name": "color-{slot}", "in": "query", "schema": {"type": "string"},
                  "description": "Colour of a slot, e.g. `color-water=%23c6ecff` (slots: /themes)."})

paths = {
    "/api/v1/": {"get": {"summary": "Links to every endpoint", "responses": {"200": js({"type": "object"})}}},
    "/api/v1/openapi.json": {"get": {"summary": "This description", "responses": {"200": js({"type": "object"})}}},
    "/api/v1/client.js": {"get": {"summary": "A dependency-free ES module client", "responses": {"200": {"description": "JavaScript", "content": {"text/javascript": {}}}}}},
    "/api/v1/health": {"get": {"summary": "200 when the data is loaded", "responses": {"200": js({"type": "object", "properties": {"status": {"type": "string"}, "datasets": {"type": "integer"}}})}}},
    "/api/v1/version": {"get": {"summary": "mapgen version, commit, SVG contract version, dataset releases", "responses": {"200": js({"type": "object"})}}},
    "/api/v1/themes": {"get": {"summary": "Built-in themes: { name: { slot: colour } }", "responses": {"200": js({"type": "object"})}}},
    "/api/v1/bbox-presets": {"get": {"summary": "Frame presets: { name: [west, south, east, north] }", "responses": {"200": js({"type": "object"})}}},
    "/api/v1/datasets": {"get": {"summary": "Hosted datasets", "responses": {"200": js({"type": "array", "items": ref("Dataset")})}}},
    "/api/v1/datasets/{dataset}/regions": {"get": {"summary": "Regions a map can be made for", "parameters": [P_DATASET],
        "responses": {"200": js({"type": "array", "items": ref("Region")}), "404": problem("Unknown dataset")}}},
    "/api/v1/datasets/{dataset}/regions/{region}/features": {"get": {"summary": "The regions of a map, without geometry: codes, names, parents", "parameters": [P_DATASET, P_REGION, P_LANGS],
        "responses": {"200": js({"type": "array", "items": ref("Feature")}), "404": problem("Unknown dataset or region")}}},
    "/api/v1/maps/{dataset}/{file}": {"get": {
        "summary": "A map", "description": "`file` is `{region}.svg` (the map), `{region}.json` (its metadata) or `{region}.html` (interactive page with colour pickers). `{region}` can list several regions by code or name, comma-separated (`BEL,LUX,NLD.svg`): one map of all of them. Unknown or misspelled parameters are errors.",
        "parameters": [P_DATASET, {"name": "file", "in": "path", "required": True, "schema": {"type": "string"}, "example": "FRA.svg"}] + map_query
            + [{"name": "If-None-Match", "in": "header", "schema": {"type": "string"}}],
        "responses": {
            "200": {"description": "The map", "headers": MAP_HEADERS, "content": {"image/svg+xml": {}, "text/html": {}, "application/json": {"schema": ref("MapMetadata")}}},
            "304": {"description": "Not modified (ETag)"},
            "400": problem("Invalid parameter"), "404": problem("Unknown dataset, region, format, release or point of view"),
            "503": problem("Busy or too slow: retry after Retry-After seconds")}}},
    "/api/v1/render": {"post": {"summary": "A map from a JSON render spec",
        "description": "Same as the map GET, with the WASM `RenderSpec` (camelCase) as JSON, or a map recipe as CSV (docs/recipes.md). `region` may list several regions. `Accept: application/json` returns the metadata plus `svg`.",
        "requestBody": {"required": True, "content": {"application/json": {"schema": ref("RenderRequest")},
                                                       "text/csv": {"schema": {"type": "string", "description": "A map recipe: key,value rows (docs/recipes.md)."}}}},
        "responses": {"200": {"description": "SVG, HTML (spec.format = html) or JSON", "content": {"image/svg+xml": {}, "application/json": {"schema": ref("MapMetadata")}}},
                      "400": problem("Invalid spec"), "404": problem("Unknown dataset or region"), "503": problem("Busy")}}},
    "/api/v1/match": {"post": {"summary": "Compare a data table's codes with a map's regions",
        "requestBody": {"required": True, "content": {"application/json": {"schema": ref("MatchRequest")}}},
        "responses": {"200": js(ref("MatchOutput")), "400": problem("Invalid request"), "404": problem("Unknown dataset or region")}}},
    "/api/v1/crosswalks": {"get": {"summary": "Hosted crosswalks between boundary versions", "responses": {"200": js({"type": "array", "items": {"type": "object"}})}}},
    "/api/v1/crosswalks/{file}": {"get": {"summary": "A hosted crosswalk: `{id}.csv` (from,to,weight) or `{id}.json` (metadata)",
        "parameters": [{"name": "file", "in": "path", "required": True, "schema": {"type": "string"}, "example": "us-counties-2010-2020.csv"}],
        "responses": {"200": {"description": "CSV or JSON", "content": {"text/csv": {}, "application/json": {}}}, "404": problem("Unknown crosswalk")}}},
    "/api/v1/reshape": {"post": {"summary": "Move a data table to new codes through a crosswalk",
        "description": "Renames and merges are applied, splits shared out by weight. Values that need a rule give 422 with `conflicts`, unless `allowConflicts`. `Accept: text/csv` returns the table only.",
        "requestBody": {"required": True, "content": {"application/json": {"schema": ref("ReshapeSpec")}}},
        "responses": {"200": {"description": "Reshaped", "content": {"application/json": {"schema": ref("ReshapeOutput")}, "text/csv": {}}},
                      "400": problem("Invalid request"), "422": problem("Values need a decision (see `conflicts`)")}}},
    "/api/v1/contract/fixture.svg": {"get": {"summary": "A small map following the SVG contract, for consumers' tests", "responses": {"200": {"description": "SVG", "content": {"image/svg+xml": {}}}}}},
}

S = lambda **p: {"type": "object", "properties": p}
string, integer, number, boolean = ({"type": t} for t in ("string", "integer", "number", "boolean"))
strings = {"type": "array", "items": {"type": "string"}}
schemas = {
    "Problem": S(type=string, title=string, status=integer, detail=string, param=string),
    "Provenance": S(credit=string, licence=string, licenceUrl=string, shareAlike=boolean, release=string, boundaryYear=string),
    "Dataset": S(id=string, title=string, level=string, regions=integer, world=boolean, worldviews=boolean, languages=boolean,
                 credit=string, licence=string, licenceUrl=string, shareAlike=boolean, release=string, boundaryYear=string,
                 licencePerRegion={"type": "boolean", "description": "Licences differ per region: see /regions."}),
    "Region": S(code=string, name=string, kind={"type": "string", "enum": ["region", "continent", "world", "file"]}, provenance=ref("Provenance")),
    "Feature": S(code=string, name=string, names={"type": "object", "additionalProperties": {"type": "string"}},
                 parent=string, parentName=string, country={"type": "string", "description": "Lowercase ISO 3166-1 alpha-2"}, units=strings),
    "LegendSlot": S(position=string, x=number, y=number, width=number, height=number, landShare=number),
    "MapMetadata": S(width=integer, height=integer, projection=string, center={"type": "array", "items": number}, regions=integer,
                     outsideFrame=strings, insets={"type": "array", "items": S(ids=strings, projection=string)},
                     legendSlots={"type": "array", "items": ref("LegendSlot")}, contract=integer, dataset=string, region=string,
                     sha1=string, credit=string, licence=string, licenceUrl=string, shareAlike=boolean, boundaryYear=string,
                     sourceRelease=string, release=string, url=string, svg={"type": "string", "description": "POST /render with Accept: application/json only"}),
    "RenderRequest": {"type": "object", "required": ["dataset", "region"],
                      "properties": {"dataset": string, "region": string, "worldview": string,
                                     "spec": {"type": "object", "description": "The WASM RenderSpec (camelCase; see crates/mapgen-wasm TypeScript types)."}}},
    "MatchRequest": {"type": "object", "required": ["dataset", "region"],
                     "properties": {"dataset": string, "region": string, "table": {"type": "string", "description": "CSV, TSV or pipe-separated"},
                                    "codeColumn": {"type": "string", "default": "code"}, "codes": strings, "codePrefix": string}},
    "MatchOutput": S(matched=integer, dataNotOnMap=strings, mapWithoutData=strings, missingShare=number,
                     hints={"type": "array", "items": S(crosswalk=string, direction={"type": "string", "enum": ["old-data", "new-data"]}, codes=strings, message=string)}),
    "ReshapeSpec": {"type": "object", "required": ["table", "codeColumn", "crosswalk"],
                    "properties": {"table": string, "codeColumn": string, "columns": strings,
                                   "crosswalk": {"oneOf": [{"type": "string", "description": "Hosted crosswalk id"},
                                                           S(table=string, fromColumn=string, toColumn=string, weightColumn=string)]},
                                   "complete": boolean, "allowConflicts": boolean}},
    "ReshapeOutput": S(csv=string, conflicts={"type": "array", "items": S(**{"from": string}, targets=strings, reason=string)},
                       direct=integer, weighted=integer, columns=strings),
}

doc = {
    "openapi": "3.1.0",
    "info": {"title": "map-generator API", "version": "1",
             "description": "Deterministic SVG maps from open data, their metadata, and data joins across boundary versions. Read-only and anonymous. See https://github.com/schiste/map-generator/blob/main/docs/api.md",
             "license": {"name": "MIT", "identifier": "MIT"}},
    "servers": [{"url": "https://map-generator.toolforge.org"}],
    "paths": paths,
    "components": {"schemas": schemas},
}
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "openapi.json")
with open(out, "w", encoding="utf-8", newline="\n") as f:
    json.dump(doc, f, indent=1, ensure_ascii=False)
    f.write("\n")
