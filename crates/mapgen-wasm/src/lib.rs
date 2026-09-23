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

mod spec;

pub use spec::{
    bbox_table, render_map, theme_table, Dataset, LayerSpec, LoadedLayer, MapOutput, RenderSpec,
    Sources, SpecError,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r##"
export type Dataset = "custom" | "ne-admin0" | "ne-admin1" | "ne-lakes" | "geoboundaries";

/** How to read a GeoJSON layer. */
export interface LayerSpec {
  /** Property layout preset. Default: "custom" (`id` and `name` properties). */
  dataset?: Dataset;
  idProperty?: string;
  nameProperty?: string;
  /** Property that `region` is compared against, e.g. "CONTINENT". */
  filterProperty?: string;
  /** Data credit for this layer (Natural Earth presets default to "Natural Earth"). */
  attribution?: string;
}

export type ColorSlot =
  | "background" | "water" | "land" | "earth" | "contextLand"
  | "border" | "contextBorder" | "lakeBorder" | "label";

/** Options for one render. Mirrors the CLI flags; everything is optional. */
export interface RenderSpec {
  /** Keep only features whose region property equals this, e.g. "FRA". */
  region?: string;
  width?: number;
  padding?: number;
  precision?: number;
  title?: string;
  attribution?: string;
  /** Draw the data credit in the bottom-right corner. */
  credit?: boolean;
  theme?: "wikimedia" | "light" | "dark" | "mono";
  /** Any CSS colour, e.g. { water: "#c6ecff", earth: "tan", background: "none" }. */
  colors?: Partial<Record<ColorSlot, string>>;
  borderWidth?: number;
  contextBorderWidth?: number;
  labelSize?: number;
  labels?: boolean;
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
  projection?: "auto" | "laea" | "equal-earth";
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
  /** Renders a map. */
  render(spec?: RenderSpec): MapOutput;
}

export interface MapOutput {
  svg: string;
  html?: string;
  width: number;
  height: number;
  projection: "laea" | "equal-earth";
  /** Projection centre [lon, lat]. */
  center: [number, number];
  /** Number of subject regions drawn. */
  regions: number;
  /** Ids of regions left out because they fell outside the frame. */
  outsideFrame: string[];
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
        };
        Ok(to_js(&render_map(src, &spec).map_err(js_err)?)?.unchecked_into())
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

/// Library version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
