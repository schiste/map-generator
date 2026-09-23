//! WebAssembly build of `map-generator`.
//!
//! ```js
//! import init, { MapGenerator } from "mapgen-wasm";
//! await init();
//! const gen = new MapGenerator();
//! gen.setSubject(admin1GeoJson, { dataset: "ne-admin1" }); // parsed once
//! gen.setContext(countriesGeoJson);                        // Natural Earth Admin-0
//! const { svg } = gen.render({ region: "FRA", theme: "dark", colors: { water: "#123" } });
//! ```
//!
//! The same engine as the CLI, so the same input and options give
//! byte-identical SVG in the browser, in Node.js and natively.

use mapgen_spec as spec;

pub use spec::{
    bbox_table, match_layer, parse_units, render_map, reshape_with, theme_table, CrosswalkSource,
    Dataset, LayerSpec, LoadedLayer, LoadedLines, MapOutput, MatchOutput, MatchSpec, RenderSpec,
    ReshapeOutput, ReshapeSpec, Sources, SpecError, UnitColumns,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r##"
export type Dataset =
  | "custom" | "ne-admin0" | "ne-admin1" | "ne-lakes" | "ne-disputed" | "ne-disputed-areas"
  | "geoboundaries";

/** How to read a GeoJSON layer. */
export interface LayerSpec {
  /** Property layout preset. Default: "custom" (`id` and `name` properties). */
  dataset?: Dataset;
  idProperty?: string;
  nameProperty?: string;
  /** Property that `region` is compared against, e.g. "CONTINENT". */
  filterProperty?: string;
  /** Property with the enclosing unit's code; borders between different parents are drawn thicker. */
  parentProperty?: string;
  /** Property with the enclosing unit's name, for tooltips ("Lancaster, Nebraska"). */
  parentNameProperty?: string;
  /** Property with the country (ISO alpha-2 or alpha-3), emitted as a lowercase alpha-2 class (Maphue). */
  countryProperty?: string;
  /** Languages to read names in (BCP 47 tags, e.g. ["fr", "zh-Hant"]), for RenderSpec.languages. */
  languages?: string[];
  /** Property with names in other languages, "{lang}" standing for the language (Natural Earth: "NAME_{lang}"). */
  nameLanguageProperty?: string;
  /** Data credit for this layer (Natural Earth presets default to "Natural Earth (de facto view)"). */
  attribution?: string;
  /** For Natural Earth point-of-view files: the view, named in the credit ("IND"). */
  worldview?: string;
}

export type ColorSlot =
  | "background" | "water" | "land" | "earth" | "contextLand"
  | "border" | "outline" | "coast" | "contextBorder" | "lakeBorder" | "disputedBorder" | "label";

/** Options for one render. Mirrors the CLI flags; everything is optional. */
export interface RenderSpec {
  /** Keep only features whose region property equals this, e.g. "FRA". */
  region?: string;
  /** Several regions in one map, e.g. ["FRA", "DEU", "ITA"]; instead of `region`. */
  regions?: string[];
  width?: number;
  padding?: number;
  precision?: number;
  title?: string;
  attribution?: string;
  /** Draw the data credit in the bottom-right corner. */
  credit?: boolean;
  /** With a data-unit table (setUnits): merge each unit's regions into one shape. */
  dissolve?: boolean;
  /** Year the boundaries represent: `data-boundary-year` on <svg>, and in the credit. */
  boundaryYear?: string;
  /** Boundary dataset release: `data-source-release` on <svg>. */
  sourceRelease?: string;
  theme?: "wikimedia" | "light" | "dark" | "mono";
  /** Any CSS colour, e.g. { water: "#c6ecff", earth: "tan", background: "none" }. */
  colors?: Partial<Record<ColorSlot, string>>;
  borderWidth?: number;
  /** Borders between regions with different parents. */
  parentBorderWidth?: number;
  /** Outer edge of the mapped area. */
  outlineWidth?: number;
  contextBorderWidth?: number;
  disputedBorderWidth?: number;
  labelSize?: number;
  labels?: boolean;
  /** Also label in these languages (read with LayerSpec.languages): a <switch> on systemLanguage per label. */
  languages?: string[];
  /** "layer" (default): borders drawn once, by kind; "regions": each region strokes its outline. */
  borderMode?: "layer" | "regions";
  /** "commons" (curved labels as rotated letters, for librsvg) or "web" (textPath). Default: "web" for HTML output, else "commons". */
  target?: "commons" | "web";
  /** Label small regions outside them with a leader line (default true). */
  leaders?: boolean;
  /** Curve labels along long, thin regions (default true). */
  curvedLabels?: boolean;
  /** Smallest label size as a fraction of labelSize (default 0.7). */
  labelMinScale?: number;
  /** Snap neighbours within this many pixels onto the mapped area's outline (default 2). */
  snap?: number;
  /** "auto": far-away parts (overseas territories…) in corner boxes. */
  insets?: "auto" | "none";
  maxInsets?: number;
  /** Standard parallels for albers/lcc. */
  parallels?: [number, number];
  /** Emit colours as `var(--mg-<slot>, …)` for restyling from page CSS. */
  cssVars?: boolean;
  /** Simplification tolerance in pixels (0 disables). */
  simplify?: number;
  /** Drop islands and lakes smaller than this many square pixels. */
  minArea?: number;
  margin?: number;
  frame?: "auto" | "all" | "world";
  /** "west,south,east,north" or a preset name from `bboxPresets()`. */
  bbox?: string;
  projection?: "auto" | "laea" | "equal-earth" | "albers" | "lcc";
  centerLon?: number;
  /** "html" also returns an interactive page with colour pickers. */
  format?: "svg" | "html";
}

/**
 * Holds parsed layers so a large GeoJSON is parsed once and rendered many times.
 * (Methods with optional arguments are declared here and merged into the class.)
 */
export interface MapGenerator {
  /** Loads the regions to map. Returns the number of features. */
  setSubject(geojson: string, spec?: LayerSpec): number;
  /** Loads neighbouring countries (default: Natural Earth Admin-0); omit `geojson` to remove them. */
  setContext(geojson?: string, spec?: LayerSpec): number;
  /** Loads lakes (default: Natural Earth lakes); omit `geojson` to remove them. */
  setLakes(geojson?: string, spec?: LayerSpec): number;
  /** Loads disputed boundary lines, drawn dashed (default: Natural Earth); omit to remove. */
  setDisputed(geojson?: string, spec?: LayerSpec): number;
  /** Loads disputed areas, drawn hatched (default: Natural Earth); omit to remove. */
  setDisputedAreas(geojson?: string, spec?: LayerSpec): number;
  /**
   * Loads a data-unit table (CSV, TSV or pipe-separated: which map regions make
   * up each data unit). Regions get `data-unit`, or merge with `dissolve`.
   * Omit to remove. Returns the number of rows.
   */
  setUnits(table?: string, columns?: UnitColumns): number;
  /** Renders a map. */
  render(spec?: RenderSpec): MapOutput;
  /**
   * Countries of the loaded layers, for pickers and search: the subject's regions
   * when they are countries, otherwise the neighbouring countries (setContext).
   */
  countries(): Country[];
  /** Region codes for what people type: codes, names (any loaded language), ISO-2. */
  resolveRegions(inputs: string[]): { codes: string[]; unknown: string[] };
  /**
   * Compares a data table's codes (or a list of codes) with the loaded regions:
   * codes the map lacks usually mean data and boundaries from different years.
   */
  matchCodes(spec: MatchSpec): MatchOutput;
}

export interface Country {
  /** Region code, e.g. "FRA". */
  code: string;
  name: string;
  /** Lowercase ISO 3166-1 alpha-2, when known. */
  iso2?: string;
  /** Names in the languages read (LayerSpec.languages). */
  names: Record<string, string>;
}

/**
 * A map recipe (docs/recipes.md): which regions and every design setting,
 * as a key,value CSV that spreadsheets can edit.
 */
export interface Recipe {
  /** "ne-admin0" (alias "countries"), "ne-admin1" ("subdivisions"), or an API dataset id. */
  dataset?: string;
  /** Codes or names, as written; resolve with MapGenerator.resolveRegions. */
  regions: string[];
  worldview?: string;
  /** The design settings as a RenderSpec. */
  spec: RenderSpec;
}

/** A data table, or codes, to compare with the map. */
export interface MatchSpec {
  /** Region of the layer to compare against (default: the whole layer). */
  region?: string;
  /** CSV, TSV or pipe-separated text... */
  table?: string;
  /** ...and its code column (default "code"). */
  codeColumn?: string;
  /** Or the codes directly. */
  codes?: string[];
  /** Prefix added to the data's codes, e.g. "US-" for bare FIPS codes. */
  codePrefix?: string;
}

export interface MatchOutput {
  matched: number;
  dataNotOnMap: string[];
  mapWithoutData: string[];
  /** Share of the data's codes that have no region on the map. */
  missingShare: number;
  /** Hosted crosswalks that know the missing codes (HTTP API only). */
  hints?: { crosswalk: string; direction: "old-data" | "new-data"; codes: string[]; message: string }[];
}

/** Moves numeric data from old codes to new ones through a crosswalk. */
export interface ReshapeSpec {
  table: string;
  codeColumn: string;
  /** Default: every other numeric column. Use counts, not rates. */
  columns?: string[];
  /** A crosswalk table; the HTTP API also accepts a hosted crosswalk's id. */
  crosswalk: string | { table: string; fromColumn?: string; toColumn?: string; weightColumn?: string };
  /** The crosswalk lists every code: codes it lacks are conflicts. */
  complete?: boolean;
  /** Return the table even with conflicts, leaving those values out. */
  allowConflicts?: boolean;
}

export interface ReshapeOutput {
  /** The table on the new codes; absent when there are conflicts, unless allowConflicts. */
  csv?: string;
  conflicts: { from: string; targets: string[]; reason: string }[];
  direct: number;
  weighted: number;
  columns: string[];
}

/** Column names of a data-unit table (defaults: map_id, data_unit_id, data_unit_name). */
export interface UnitColumns {
  mapColumn?: string;
  unitColumn?: string;
  nameColumn?: string;
}

export interface LegendSlot {
  position:
    | "top-left" | "top-center" | "top-right"
    | "middle-left" | "center" | "middle-right"
    | "bottom-left" | "bottom-center" | "bottom-right";
  x: number;
  y: number;
  width: number;
  height: number;
  /** Share of land under a box a third of the map wide and high at this position. */
  landShare: number;
}

export interface MapOutput {
  svg: string;
  html?: string;
  width: number;
  height: number;
  projection: "laea" | "equal-earth" | "albers" | "lcc";
  /** Projection centre [lon, lat]. */
  center: [number, number];
  /** Number of subject regions drawn (main map and insets). */
  regions: number;
  /** Ids of regions shown nowhere (outside the frame and not in an inset). */
  outsideFrame: string[];
  /** Inset boxes: the regions in each and its projection. */
  insets: { ids: string[]; projection: string }[];
  /** Empty areas for a legend or title, in pixels, largest first; width 0 when nothing fits. */
  legendSlots: LegendSlot[];
  /** Version of the SVG contract (docs/contract.md). */
  contract: number;
  /** How the data-unit table fits the map (when set). */
  units?: {
    unknownRegions: string[];
    /** [region, units]: regions in several units, i.e. units that don't nest. */
    overlapping: [string, string[]][];
    /** [unit, parts]: units that are not one contiguous shape. */
    splitUnits: [string, number][];
    unassigned: string[];
  };
}
"##;

#[wasm_bindgen]
extern "C" {
    /// Options object typed as `RenderSpec` in TypeScript.
    #[wasm_bindgen(typescript_type = "RenderSpec")]
    pub type RenderSpecJs;
    /// Options object typed as `LayerSpec` in TypeScript.
    #[wasm_bindgen(typescript_type = "LayerSpec")]
    pub type LayerSpecJs;
    /// Result object typed as `MapOutput` in TypeScript.
    #[wasm_bindgen(typescript_type = "MapOutput")]
    pub type MapOutputJs;
}

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

fn js_err(e: SpecError) -> JsError {
    JsError::new(&e.0)
}

/// Reads an options object. It goes through `JSON.stringify` and `serde_json`
/// rather than `serde_wasm_bindgen::from_value`, because the latter only looks
/// up declared fields and so silently ignores typos like `colours`; this way
/// the browser applies exactly the validation the native tests cover.
fn from_js<T: serde::de::DeserializeOwned + Default>(
    value: Option<impl Into<JsValue>>,
) -> Result<T, JsError> {
    // wasm-bindgen does not type-check `Object` parameters at runtime, so a
    // string or array can still arrive here; reject it explicitly.
    let Some(value) = value.map(Into::into) else {
        return Ok(T::default());
    };
    if value.is_undefined() || value.is_null() {
        return Ok(T::default());
    }
    if !value.is_object() || js_sys::Array::is_array(&value) {
        return Err(JsError::new("options must be a plain object"));
    }
    let json: String = js_sys::JSON::stringify(&value)
        .map_err(|_| JsError::new("options must be JSON-serialisable"))?
        .into();
    serde_json::from_str(&json).map_err(|e| JsError::new(&e.to_string()))
}

fn to_js<T: serde::Serialize>(value: &T) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| JsError::new(&e.to_string()))
}

/// Holds parsed layers so a large GeoJSON is parsed once and rendered many times.
#[wasm_bindgen]
#[derive(Default)]
pub struct MapGenerator {
    subject: Option<LoadedLayer>,
    context: Option<LoadedLayer>,
    lakes: Option<LoadedLayer>,
    disputed_areas: Option<LoadedLayer>,
    disputed: Option<LoadedLines>,
    units: Option<Vec<mapgen_core::units::UnitRow>>,
}

#[wasm_bindgen]
impl MapGenerator {
    #[wasm_bindgen(constructor)]
    pub fn new() -> MapGenerator {
        MapGenerator::default()
    }

    /// Loads the regions to map. Returns the number of features.
    #[wasm_bindgen(js_name = setSubject, skip_typescript)]
    pub fn set_subject(
        &mut self,
        geojson: &str,
        spec: Option<LayerSpecJs>,
    ) -> Result<usize, JsError> {
        let layer = LoadedLayer::parse(geojson, &from_js(spec)?).map_err(js_err)?;
        let n = layer.len();
        self.subject = Some(layer);
        Ok(n)
    }

    /// Loads neighbouring countries (default preset: Natural Earth Admin-0).
    /// Pass `undefined` to remove them.
    #[wasm_bindgen(js_name = setContext, skip_typescript)]
    pub fn set_context(
        &mut self,
        geojson: Option<String>,
        spec: Option<LayerSpecJs>,
    ) -> Result<usize, JsError> {
        self.context = load_optional(geojson, spec, Dataset::NeAdmin0)?;
        Ok(self.context.as_ref().map_or(0, LoadedLayer::len))
    }

    /// Loads lakes (default preset: Natural Earth lakes). Pass `undefined` to remove them.
    #[wasm_bindgen(js_name = setLakes, skip_typescript)]
    pub fn set_lakes(
        &mut self,
        geojson: Option<String>,
        spec: Option<LayerSpecJs>,
    ) -> Result<usize, JsError> {
        self.lakes = load_optional(geojson, spec, Dataset::NeLakes)?;
        Ok(self.lakes.as_ref().map_or(0, LoadedLayer::len))
    }

    /// Loads a data-unit table. Pass `undefined` to remove it.
    #[wasm_bindgen(js_name = setUnits, skip_typescript)]
    pub fn set_units(
        &mut self,
        table: Option<String>,
        columns: Option<js_sys::Object>,
    ) -> Result<usize, JsError> {
        self.units = match table {
            None => None,
            Some(text) => {
                let cols: UnitColumns = from_js(columns)?;
                Some(parse_units(&text, &cols).map_err(js_err)?)
            }
        };
        Ok(self.units.as_ref().map_or(0, Vec::len))
    }

    /// Loads disputed areas (default preset: Natural Earth). Pass `undefined`
    /// to remove them.
    #[wasm_bindgen(js_name = setDisputedAreas, skip_typescript)]
    pub fn set_disputed_areas(
        &mut self,
        geojson: Option<String>,
        spec: Option<LayerSpecJs>,
    ) -> Result<usize, JsError> {
        self.disputed_areas = load_optional(geojson, spec, Dataset::NeDisputedAreas)?;
        Ok(self.disputed_areas.as_ref().map_or(0, LoadedLayer::len))
    }

    /// Loads disputed boundary lines (default preset: Natural Earth). Pass
    /// `undefined` to remove them.
    #[wasm_bindgen(js_name = setDisputed, skip_typescript)]
    pub fn set_disputed(
        &mut self,
        geojson: Option<String>,
        spec: Option<LayerSpecJs>,
    ) -> Result<usize, JsError> {
        self.disputed = match geojson {
            None => None,
            Some(text) => {
                let spec = match spec {
                    None => LayerSpec::with_dataset(Dataset::NeDisputed),
                    Some(s) => from_js(Some(s))?,
                };
                Some(LoadedLines::parse(&text, &spec).map_err(js_err)?)
            }
        };
        Ok(self.disputed.as_ref().map_or(0, LoadedLines::len))
    }

    /// Distinct region codes in the subject layer, sorted.
    pub fn regions(&self) -> Result<Vec<String>, JsError> {
        Ok(self.subject()?.regions())
    }

    /// Renders a map.
    #[wasm_bindgen(skip_typescript)]
    pub fn render(&self, spec: Option<RenderSpecJs>) -> Result<MapOutputJs, JsError> {
        let spec: RenderSpec = from_js(spec)?;
        let src = Sources {
            subject: self.subject()?,
            context: self.context.as_ref(),
            lakes: self.lakes.as_ref(),
            disputed_areas: self.disputed_areas.as_ref(),
            disputed: self.disputed.as_ref(),
            units: self.units.as_deref(),
        };
        Ok(to_js(&render_map(src, &spec).map_err(js_err)?)?.unchecked_into())
    }

    /// Countries of the loaded layers (see the TypeScript docs).
    #[wasm_bindgen(skip_typescript)]
    pub fn countries(&self) -> Result<JsValue, JsError> {
        let subject = self.subject()?;
        let regions: std::collections::BTreeSet<String> = subject.regions().into_iter().collect();
        let own: Vec<&mapgen_core::MapFeature> = subject
            .features()
            .filter(|f| regions.contains(&f.id))
            .collect();
        let list: Vec<serde_json::Value> = if !own.is_empty() {
            own.into_iter().map(country_json).collect()
        } else {
            self.context
                .iter()
                .flat_map(|c| c.features())
                .filter(|f| regions.contains(&f.id))
                .map(country_json)
                .collect()
        };
        to_js(&list)
    }

    #[wasm_bindgen(js_name = resolveRegions, skip_typescript)]
    pub fn resolve_regions(&self, inputs: Vec<String>) -> Result<JsValue, JsError> {
        let (codes, unknown) = self
            .subject()?
            .resolve_regions(&inputs, self.context.as_ref());
        to_js(&serde_json::json!({ "codes": codes, "unknown": unknown }))
    }

    /// Compares a data table's codes with the loaded regions.
    #[wasm_bindgen(js_name = matchCodes, skip_typescript)]
    pub fn match_codes(&self, spec: js_sys::Object) -> Result<JsValue, JsError> {
        let spec: MatchSpec = from_js(Some(spec))?;
        to_js(&match_layer(self.subject()?, &spec).map_err(js_err)?)
    }

    fn subject(&self) -> Result<&LoadedLayer, JsError> {
        self.subject
            .as_ref()
            .ok_or_else(|| JsError::new("call setSubject() before rendering"))
    }
}

fn load_optional(
    geojson: Option<String>,
    spec: Option<LayerSpecJs>,
    default: Dataset,
) -> Result<Option<LoadedLayer>, JsError> {
    let Some(text) = geojson else { return Ok(None) };
    let spec = match spec {
        None => LayerSpec::with_dataset(default),
        Some(s) => from_js(Some(s))?,
    };
    LoadedLayer::parse(&text, &spec).map(Some).map_err(js_err)
}

/// Built-in themes: `{ name: { slot: colour } }`.
#[wasm_bindgen(unchecked_return_type = "Record<string, Record<string, string>>")]
pub fn themes() -> Result<JsValue, JsError> {
    to_js(&theme_table())
}

/// Frame presets: `{ name: [west, south, east, north] }`.
#[wasm_bindgen(js_name = bboxPresets, unchecked_return_type = "Record<string, [number, number, number, number]>")]
pub fn bbox_presets() -> Result<JsValue, JsError> {
    to_js(&bbox_table())
}

/// Moves numeric data from old codes to new ones through a crosswalk table.
#[wasm_bindgen(unchecked_return_type = "ReshapeOutput")]
pub fn reshape(
    #[wasm_bindgen(unchecked_param_type = "ReshapeSpec")] spec: js_sys::Object,
) -> Result<JsValue, JsError> {
    let value: JsValue = spec.into();
    if !value.is_object() || js_sys::Array::is_array(&value) {
        return Err(JsError::new("options must be a plain object"));
    }
    let json: String = js_sys::JSON::stringify(&value)
        .map_err(|_| JsError::new("options must be JSON-serialisable"))?
        .into();
    let spec: ReshapeSpec =
        serde_json::from_str(&json).map_err(|e| JsError::new(&e.to_string()))?;
    let crosswalk = match &spec.crosswalk {
        CrosswalkSource::Inline(c) => c,
        CrosswalkSource::Hosted(id) => {
            return Err(JsError::new(&format!(
                "hosted crosswalks ({id:?}) are only available through the HTTP API; pass the crosswalk table"
            )))
        }
    };
    to_js(&reshape_with(&spec, crosswalk).map_err(js_err)?)
}

/// Reads a map recipe (key,value CSV; see docs/recipes.md).
#[wasm_bindgen(js_name = parseRecipe, unchecked_return_type = "Recipe")]
pub fn parse_recipe(text: &str) -> Result<JsValue, JsError> {
    let recipe = spec::recipe::Recipe::parse(text).map_err(js_err)?;
    let parsed =
        spec::params::parse(&recipe.params, &[]).map_err(|e| JsError::new(&e.to_string()))?;
    to_js(&serde_json::json!({
        "dataset": recipe.dataset,
        "regions": recipe.regions,
        "worldview": recipe.worldview,
        "spec": parsed.spec,
    }))
}

/// Writes a map recipe: the regions and the design settings of a RenderSpec.
#[wasm_bindgen(js_name = recipeToCsv)]
pub fn recipe_to_csv(
    #[wasm_bindgen(unchecked_param_type = "Recipe")] recipe: js_sys::Object,
) -> Result<String, JsError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct In {
        dataset: Option<String>,
        #[serde(default)]
        regions: Vec<String>,
        worldview: Option<String>,
        #[serde(default)]
        spec: serde_json::Map<String, serde_json::Value>,
    }
    let json: String = js_sys::JSON::stringify(&recipe.into())
        .map_err(|_| JsError::new("the recipe must be JSON-serialisable"))?
        .into();
    let r: In = serde_json::from_str(&json).map_err(|e| JsError::new(&e.to_string()))?;
    // Check the settings before writing them.
    serde_json::from_value::<RenderSpec>(serde_json::Value::Object(r.spec.clone()))
        .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(spec::recipe::Recipe::from_spec(r.dataset, r.regions, r.worldview, &r.spec).to_csv())
}

fn country_json(f: &mapgen_core::MapFeature) -> serde_json::Value {
    serde_json::json!({ "code": f.id, "name": f.name, "iso2": f.country, "names": f.names })
}

/// Library version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
