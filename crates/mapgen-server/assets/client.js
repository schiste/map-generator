// map-generator API client (v1). Dependency-free ES module:
//
//   import { MapgenClient } from "https://map-generator.toolforge.org/api/v1/client.js";
//   const api = new MapgenClient();
//   const svg = await api.map("ne-admin1", "FRA", { labels: true, languages: ["fr"] });
//   const meta = await api.metadata("ne-admin1", "FRA", { labels: true });
//
// Options are the render spec of the WebAssembly build (camelCase), e.g.
// { width, theme, labels, languages, target, colors: { water: "#c6ecff" } },
// plus `worldview` and `release`. Errors are thrown as MapgenError with the
// API's problem details.
// Documentation: https://github.com/schiste/map-generator/blob/main/docs/api.md

export const DEFAULT_BASE = "https://map-generator.toolforge.org/api/v1";

export class MapgenError extends Error {
  constructor(problem, status) {
    super(problem?.detail || problem?.title || `HTTP ${status}`);
    this.name = "MapgenError";
    this.status = status;
    this.problem = problem;
  }
}

const kebab = (s) => s.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());

/** Query string for a render spec: camelCase → kebab-case, colors → color-<slot>, lists comma-separated. */
export function specToQuery(spec = {}) {
  const params = new URLSearchParams();
  const entries = Object.entries(spec).filter(([, v]) => v !== undefined && v !== null);
  entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  for (const [key, value] of entries) {
    if (key === "colors") {
      for (const [slot, colour] of Object.entries(value).sort()) params.set(`color-${kebab(slot)}`, colour);
    } else if (Array.isArray(value)) {
      params.set(kebab(key), value.join(","));
    } else {
      params.set(kebab(key), String(value));
    }
  }
  return params.toString();
}

export class MapgenClient {
  /** @param {string} base API root, default https://map-generator.toolforge.org/api/v1 */
  constructor(base = DEFAULT_BASE, { fetch: fetcher = globalThis.fetch.bind(globalThis) } = {}) {
    this.base = base.replace(/\/$/, "");
    this.fetch = fetcher;
  }

  /** URL of a map: format "svg" (default), "json" or "html". */
  mapUrl(dataset, region, spec = {}, format = "svg") {
    const q = specToQuery(spec);
    return `${this.base}/maps/${encodeURIComponent(dataset)}/${encodeURIComponent(region)}.${format}${q ? "?" + q : ""}`;
  }

  async #request(path, init = {}, as = "json") {
    const res = await this.fetch(path.startsWith("http") ? path : this.base + path, init);
    if (!res.ok) {
      let problem = null;
      try {
        problem = await res.json();
      } catch {
        // not JSON
      }
      throw new MapgenError(problem, res.status);
    }
    return as === "text" ? res.text() : res.json();
  }

  #post(path, body) {
    return this.#request(path, {
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "application/json" },
      body: JSON.stringify(body),
    });
  }

  /** The map as SVG text. */
  map(dataset, region, spec = {}) {
    return this.#request(this.mapUrl(dataset, region, spec, "svg"), {}, "text");
  }

  /** Size, projection, insets, legendSlots, sha1, credit, licence, boundary version… */
  metadata(dataset, region, spec = {}) {
    return this.#request(this.mapUrl(dataset, region, spec, "json"));
  }

  /** POST /render with a JSON spec; returns the metadata plus `svg`. */
  render(dataset, region, spec = {}, worldview = undefined) {
    return this.#post("/render", { dataset, region, worldview, spec });
  }

  datasets() {
    return this.#request("/datasets");
  }

  regions(dataset) {
    return this.#request(`/datasets/${encodeURIComponent(dataset)}/regions`);
  }

  /** Codes, names (also in `languages`), parents and countries of a map's regions. */
  features(dataset, region, languages = []) {
    const q = languages.length ? `?languages=${encodeURIComponent(languages.join(","))}` : "";
    return this.#request(`/datasets/${encodeURIComponent(dataset)}/regions/${encodeURIComponent(region)}/features${q}`);
  }

  themes() {
    return this.#request("/themes");
  }

  /** Every map setting, with type, widget, default, choices, limits, group and conditions. */
  renderOptions() {
    return this.#request("/render-options");
  }

  version() {
    return this.#request("/version");
  }

  /** Compare data codes with a map: { dataset, region, table, codeColumn } or { dataset, region, codes }. */
  match(request) {
    return this.#post("/match", request);
  }

  crosswalks() {
    return this.#request("/crosswalks");
  }

  /**
   * Move a table to new codes: { table, codeColumn, crosswalk: id | { table } }.
   * Throws MapgenError with status 422 and `problem.conflicts` when values need a rule.
   */
  reshape(spec) {
    return this.#post("/reshape", spec);
  }
}
