//! Plain-Rust core of the WebAssembly API: option types, validation, and
//! rendering. Kept free of `wasm-bindgen` so it is unit-tested natively.

use std::collections::{BTreeMap, BTreeSet};

use mapgen_core::frame::BBOX_PRESETS;
use mapgen_core::units::{check_units, dissolve, tag_units, UnitRow};
use mapgen_core::{
    render, BorderMode, Color, FrameMode, GeoBBox, InsetMode, MapFeature, MapLayers, MapLine,
    ProjectionChoice, RenderOptions, Target, Theme,
};
use mapgen_data::{LayerQuery, Source};
use serde::{Deserialize, Serialize};

/// Error surfaced to JavaScript as an `Error` with this message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError(pub String);

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<E: std::error::Error> From<E> for SpecError {
    fn from(e: E) -> Self {
        SpecError(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, SpecError>;

/// Known layouts of a GeoJSON layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dataset {
    /// `id` / `name` properties, no region column.
    #[default]
    Custom,
    NeAdmin0,
    NeAdmin1,
    NeLakes,
    /// Natural Earth disputed and claimed boundary lines.
    NeDisputed,
    /// Natural Earth disputed areas (polygons).
    NeDisputedAreas,
    Geoboundaries,
}

/// How to read a GeoJSON layer (`MapGenerator.setSubject(text, layerSpec)`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayerSpec {
    #[serde(default)]
    pub dataset: Dataset,
    pub id_property: Option<String>,
    pub name_property: Option<String>,
    /// Property that `region` is compared against (e.g. `CONTINENT`).
    pub filter_property: Option<String>,
    /// Property with the code of the enclosing unit; borders between
    /// different parents are drawn thicker.
    pub parent_property: Option<String>,
    /// Property with the enclosing unit's name, for tooltips.
    pub parent_name_property: Option<String>,
    /// Property with the country (ISO 3166-1 alpha-2 or alpha-3), emitted as
    /// a lowercase alpha-2 class for colouring tools like Maphue.
    pub country_property: Option<String>,
    /// Languages to read names in (BCP 47 tags), for `RenderSpec.languages`.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Property with names in other languages, `{lang}` standing for the
    /// language (Natural Earth presets: `NAME_{lang}` / `name_{lang}`).
    pub name_language_property: Option<String>,
    /// Data credit for this layer. Natural Earth presets default to
    /// "Natural Earth (de facto view)"; for geoBoundaries, pass the source
    /// and licence.
    pub attribution: Option<String>,
    /// For Natural Earth point-of-view files (`ne_10m_admin_0_countries_ind`):
    /// the view, named in the credit ("Natural Earth (IND view)").
    pub worldview: Option<String>,
}

impl LayerSpec {
    pub fn with_dataset(dataset: Dataset) -> Self {
        LayerSpec {
            dataset,
            ..LayerSpec::default()
        }
    }

    pub fn query(&self) -> LayerQuery {
        let mut q = match self.dataset {
            Dataset::Custom => LayerQuery {
                table: None,
                id_columns: vec!["id".into()],
                name_column: "name".into(),
                class: "region".into(),
                ..LayerQuery::default()
            },
            Dataset::NeAdmin0 => Source::NaturalEarthAdmin0.layer_query(),
            Dataset::NeAdmin1 => Source::NaturalEarthAdmin1.layer_query(),
            Dataset::NeLakes => Source::NaturalEarthLakes.layer_query(),
            Dataset::NeDisputed => Source::NaturalEarthDisputedLines.layer_query(),
            Dataset::NeDisputedAreas => Source::NaturalEarthDisputedAreas.layer_query(),
            Dataset::Geoboundaries => Source::GeoBoundaries.layer_query(),
        };
        if let Some(p) = &self.id_property {
            q.id_columns = vec![p.clone()];
        }
        if let Some(p) = &self.name_property {
            q.name_column = p.clone();
        }
        if let Some(p) = &self.filter_property {
            q.filter_column = Some(p.clone());
        }
        if let Some(p) = &self.parent_property {
            q.parent_column = Some(p.clone());
        }
        if let Some(p) = &self.parent_name_property {
            q.parent_name_column = Some(p.clone());
        }
        if let Some(p) = &self.country_property {
            q.country_column = Some(p.clone());
        }
        if let Some(p) = &self.name_language_property {
            q.name_language_column = Some(p.clone());
        }
        q.languages = self.languages.clone();
        q
    }
}

/// A parsed layer: features plus their region value, parsed once and
/// reused across renders.
#[derive(Debug, Clone, Default)]
pub struct LoadedLayer {
    rows: Vec<(Option<String>, MapFeature)>,
    has_filter: bool,
    credit: Option<String>,
}

impl LoadedLayer {
    pub fn parse(text: &str, spec: &LayerSpec) -> Result<Self> {
        let query = spec.query();
        let rows = mapgen_data::geojson::rows_from_str(text, &query)?;
        if rows.is_empty() {
            return Err(SpecError("the GeoJSON contains no features".into()));
        }
        let credit = spec.attribution.clone().or_else(|| {
            matches!(
                spec.dataset,
                Dataset::NeAdmin0
                    | Dataset::NeAdmin1
                    | Dataset::NeLakes
                    | Dataset::NeDisputed
                    | Dataset::NeDisputedAreas
            )
            .then(|| natural_earth_credit(spec))
        });
        Ok(LoadedLayer {
            rows,
            has_filter: query.filter_column.is_some(),
            credit,
        })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Distinct region values, sorted (empty when the layer has no region column).
    pub fn regions(&self) -> Vec<String> {
        let set: BTreeSet<&String> = self.rows.iter().filter_map(|(v, _)| v.as_ref()).collect();
        set.into_iter().cloned().collect()
    }

    fn select(&self, region: Option<&str>) -> Result<Vec<MapFeature>> {
        match region {
            None => Ok(self.all()),
            Some(_) if !self.has_filter => Err(SpecError(
                "`region` needs a region column: load the layer with a dataset preset or `filterProperty`"
                    .into(),
            )),
            Some(code) => {
                let v: Vec<MapFeature> = self
                    .rows
                    .iter()
                    .filter(|(val, _)| val.as_deref() == Some(code))
                    .map(|(_, f)| f.clone())
                    .collect();
                if v.is_empty() {
                    return Err(SpecError(format!("no features matched region {code:?}")));
                }
                Ok(v)
            }
        }
    }

    fn all(&self) -> Vec<MapFeature> {
        self.rows.iter().map(|(_, f)| f.clone()).collect()
    }
}

/// "Natural Earth (de facto view)", or the layer's point of view.
fn natural_earth_credit(spec: &LayerSpec) -> String {
    match &spec.worldview {
        Some(v) => format!("Natural Earth ({} view)", v.to_ascii_uppercase()),
        None => "Natural Earth (de facto view)".to_owned(),
    }
}

/// A parsed line layer (disputed boundaries).
#[derive(Debug, Clone, Default)]
pub struct LoadedLines {
    lines: Vec<MapLine>,
    credit: Option<String>,
}

impl LoadedLines {
    pub fn parse(text: &str, spec: &LayerSpec) -> Result<Self> {
        let lines = mapgen_data::geojson::lines_from_str(text, &spec.query())?;
        if lines.is_empty() {
            return Err(SpecError("the GeoJSON contains no line features".into()));
        }
        let natural_earth = spec.dataset == Dataset::NeDisputed;
        let credit = spec
            .attribution
            .clone()
            .or_else(|| natural_earth.then(|| natural_earth_credit(spec)));
        Ok(LoadedLines { lines, credit })
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Frame {
    #[default]
    Auto,
    All,
    World,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Projection {
    #[default]
    Auto,
    Laea,
    EqualEarth,
    Albers,
    Lcc,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Borders {
    #[default]
    Layer,
    Regions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetSpec {
    Commons,
    Web,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Insets {
    #[default]
    Auto,
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    #[default]
    Svg,
    Html,
}

/// Options for one render (`MapGenerator.render(renderSpec)`). Mirrors the
/// CLI flags; every field is optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderSpec {
    pub region: Option<String>,
    pub width: Option<u32>,
    pub padding: Option<u32>,
    pub precision: Option<usize>,
    pub title: Option<String>,
    pub attribution: Option<String>,
    #[serde(default)]
    pub credit: bool,
    /// Year the boundaries represent, written to the SVG and the credit.
    pub boundary_year: Option<String>,
    /// Release of the boundary dataset.
    pub source_release: Option<String>,
    pub theme: Option<String>,
    /// Colour overrides by slot: `background`, `water`, `land` (or `earth`),
    /// `contextLand`, `border`, `outline`, `contextBorder`, `lakeBorder`,
    /// `disputedBorder`, `label`.
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    pub border_width: Option<f64>,
    pub parent_border_width: Option<f64>,
    pub outline_width: Option<f64>,
    pub context_border_width: Option<f64>,
    pub disputed_border_width: Option<f64>,
    pub label_size: Option<f64>,
    #[serde(default)]
    pub labels: bool,
    /// Also label in these languages (read with `LayerSpec.languages`), in a
    /// `<switch>` on `systemLanguage`.
    #[serde(default)]
    pub languages: Vec<String>,
    /// `layer` (default): borders drawn once in their own layer; `regions`:
    /// each region strokes its own outline.
    #[serde(default)]
    pub border_mode: Borders,
    /// `commons` (curved labels as rotated letters, for librsvg) or `web`
    /// (textPath). Default: `web` for HTML output, `commons` otherwise.
    pub target: Option<TargetSpec>,
    /// With a data-unit table: merge each unit's regions into one shape.
    #[serde(default)]
    pub dissolve: bool,
    /// Leader lines for small regions' labels (default true).
    pub leaders: Option<bool>,
    /// Curved labels along long, thin regions (default true).
    pub curved_labels: Option<bool>,
    pub label_min_scale: Option<f64>,
    /// Snap neighbours within this many pixels onto the outline (default 2).
    pub snap: Option<f64>,
    #[serde(default)]
    pub insets: Insets,
    pub max_insets: Option<usize>,
    /// Standard parallels for `albers`/`lcc`.
    pub parallels: Option<[f64; 2]>,
    #[serde(default)]
    pub css_vars: bool,
    pub simplify: Option<f64>,
    pub min_area: Option<f64>,
    pub margin: Option<f64>,
    #[serde(default)]
    pub frame: Frame,
    /// `west,south,east,north` or a preset name (`europe`...). Overrides `frame`.
    pub bbox: Option<String>,
    #[serde(default)]
    pub projection: Projection,
    pub center_lon: Option<f64>,
    #[serde(default)]
    pub format: Format,
}

impl RenderSpec {
    pub fn options(&self) -> Result<RenderOptions> {
        let d = RenderOptions::default();
        let theme_name = self.theme.as_deref().unwrap_or("wikimedia");
        let mut theme = Theme::builtin(theme_name).ok_or_else(|| {
            SpecError(format!(
                "unknown theme {theme_name:?} (expected one of: {})",
                Theme::NAMES.join(", ")
            ))
        })?;
        for (slot, value) in &self.colors {
            theme.set(slot, Color::parse(value)?)?;
        }
        theme.border_width =
            positive("borderWidth", self.border_width)?.unwrap_or(theme.border_width);
        theme.context_border_width = positive("contextBorderWidth", self.context_border_width)?
            .unwrap_or(theme.context_border_width);
        theme.parent_border_width = positive("parentBorderWidth", self.parent_border_width)?
            .unwrap_or(theme.parent_border_width);
        theme.outline_width =
            positive("outlineWidth", self.outline_width)?.unwrap_or(theme.outline_width);
        theme.disputed_border_width = positive("disputedBorderWidth", self.disputed_border_width)?
            .unwrap_or(theme.disputed_border_width);
        theme.label_size = positive("labelSize", self.label_size)?.unwrap_or(theme.label_size);

        let width = self.width.unwrap_or(d.width);
        if !(16..=20_000).contains(&width) {
            return Err(SpecError(format!(
                "width must be between 16 and 20000, got {width}"
            )));
        }
        let frame = match (&self.bbox, self.frame) {
            (Some(b), _) => FrameMode::BBox(GeoBBox::parse(b)?),
            (None, Frame::Auto) => FrameMode::Auto,
            (None, Frame::All) => FrameMode::All,
            (None, Frame::World) => FrameMode::World,
        };
        Ok(RenderOptions {
            width,
            padding: self.padding.unwrap_or(d.padding),
            precision: self.precision.unwrap_or(d.precision).min(6),
            title: self.title.clone(),
            attribution: self.attribution.clone(),
            credit: self.credit,
            boundary_year: self.boundary_year.clone(),
            source_release: self.source_release.clone(),
            theme,
            css_vars: self.css_vars || self.format == Format::Html,
            labels: self.labels,
            languages: self.languages.clone(),
            border_mode: match self.border_mode {
                Borders::Layer => BorderMode::Layer,
                Borders::Regions => BorderMode::Regions,
            },
            target: match (self.target, self.format) {
                (Some(TargetSpec::Commons), _) => Target::Commons,
                (Some(TargetSpec::Web), _) | (None, Format::Html) => Target::Web,
                (None, _) => Target::Commons,
            },
            label_leaders: self.leaders.unwrap_or(d.label_leaders),
            label_curved: self.curved_labels.unwrap_or(d.label_curved),
            label_min_scale: positive("labelMinScale", self.label_min_scale)?
                .unwrap_or(d.label_min_scale)
                .min(1.0),
            simplify_px: non_negative("simplify", self.simplify)?.unwrap_or(d.simplify_px),
            min_area_px: non_negative("minArea", self.min_area)?.unwrap_or(d.min_area_px),
            snap_px: non_negative("snap", self.snap)?.unwrap_or(d.snap_px),
            projection: {
                let parallels = match self.parallels {
                    Some([a, b]) if a.abs() < 90.0 && b.abs() < 90.0 && a != -b => Some((a, b)),
                    Some(p) => return Err(SpecError(format!("invalid parallels {p:?}"))),
                    None => None,
                };
                match self.projection {
                    Projection::Auto => ProjectionChoice::Auto,
                    Projection::Laea => ProjectionChoice::Laea,
                    Projection::EqualEarth => ProjectionChoice::EqualEarth,
                    Projection::Albers => ProjectionChoice::Albers { parallels },
                    Projection::Lcc => ProjectionChoice::Lcc { parallels },
                }
            },
            frame,
            insets: match self.insets {
                Insets::Auto => InsetMode::Auto,
                Insets::None => InsetMode::None,
            },
            max_insets: self.max_insets.unwrap_or(d.max_insets),
            margin: non_negative("margin", self.margin)?.unwrap_or(d.margin),
            center_lon: self.center_lon,
        })
    }
}

fn non_negative(name: &str, v: Option<f64>) -> Result<Option<f64>> {
    match v {
        Some(x) if !x.is_finite() || x < 0.0 => Err(SpecError(format!(
            "{name} must be a finite number ≥ 0, got {x}"
        ))),
        v => Ok(v),
    }
}

fn positive(name: &str, v: Option<f64>) -> Result<Option<f64>> {
    match v {
        Some(x) if !x.is_finite() || x <= 0.0 => Err(SpecError(format!(
            "{name} must be a finite number > 0, got {x}"
        ))),
        v => Ok(v),
    }
}

/// Column names of a data-unit table (`MapGenerator.setUnits`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct UnitColumns {
    pub map_column: String,
    pub unit_column: String,
    pub name_column: String,
}

impl Default for UnitColumns {
    fn default() -> Self {
        UnitColumns {
            map_column: "map_id".into(),
            unit_column: "data_unit_id".into(),
            name_column: "data_unit_name".into(),
        }
    }
}

/// Parses a data-unit table (CSV, TSV or pipe-separated).
pub fn parse_units(text: &str, cols: &UnitColumns) -> Result<Vec<UnitRow>> {
    let table = mapgen_data::table::parse_table(text).map_err(SpecError)?;
    let (m, u) = (
        table.column(&cols.map_column)?,
        table.column(&cols.unit_column)?,
    );
    let n = table.column(&cols.name_column).ok();
    Ok(table
        .rows
        .iter()
        .filter_map(|r| {
            Some(UnitRow {
                region: table.cell(r, m)?,
                unit: table.cell(r, u)?,
                unit_name: n.and_then(|n| table.cell(r, n)),
            })
        })
        .collect())
}

/// How a data-unit table fits the map.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitReportOutput {
    pub unknown_regions: Vec<String>,
    /// `[region, [units…]]`: regions in several units (units that don't nest).
    pub overlapping: Vec<(String, Vec<String>)>,
    /// `[unit, parts]`: units that are not one contiguous shape.
    pub split_units: Vec<(String, usize)>,
    pub unassigned: Vec<String>,
}

/// Result of a render, returned to JavaScript.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapOutput {
    /// The SVG document.
    pub svg: String,
    /// The interactive page, when `format: "html"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    pub width: u32,
    pub height: u32,
    /// `"laea"`, `"equal-earth"`, `"albers"` or `"lcc"`.
    pub projection: String,
    /// Projection centre `[lon, lat]` (lat is 0 for Equal Earth).
    pub center: [f64; 2],
    /// Number of subject regions drawn (main map and insets).
    pub regions: usize,
    /// Ids of subject regions shown nowhere (outside the frame, no inset).
    pub outside_frame: Vec<String>,
    pub insets: Vec<InsetOutput>,
    /// Present when a data-unit table is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<UnitReportOutput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsetOutput {
    pub ids: Vec<String>,
    pub projection: String,
}

/// The layers a render draws from.
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    pub subject: &'a LoadedLayer,
    pub context: Option<&'a LoadedLayer>,
    pub lakes: Option<&'a LoadedLayer>,
    pub disputed_areas: Option<&'a LoadedLayer>,
    pub disputed: Option<&'a LoadedLines>,
    pub units: Option<&'a [UnitRow]>,
}

impl Sources<'_> {
    /// Distinct layer credits, in subject/context/lakes order.
    fn credits(&self) -> Option<String> {
        let mut out: Vec<&str> = Vec::new();
        let layers = [
            Some(self.subject),
            self.context,
            self.lakes,
            self.disputed_areas,
        ]
        .into_iter()
        .flatten();
        let credits = layers.map(|l| l.credit.as_deref()).chain(std::iter::once(
            self.disputed.and_then(|d| d.credit.as_deref()),
        ));
        for c in credits.flatten() {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        (!out.is_empty()).then(|| out.join("; "))
    }
}

pub fn render_map(src: Sources, spec: &RenderSpec) -> Result<MapOutput> {
    let mut opts = spec.options()?;
    if opts.attribution.is_none() {
        opts.attribution = src.credits();
    }
    let region = spec.region.as_deref();
    let mut subject = src.subject.select(region)?;
    let unit_report = src.units.map(|rows| {
        let r = check_units(&subject, rows);
        UnitReportOutput {
            unknown_regions: r.unknown_regions,
            overlapping: r.overlapping,
            split_units: r.split_units,
            unassigned: r.unassigned,
        }
    });
    match (src.units, spec.dissolve) {
        (Some(rows), true) => subject = dissolve(subject, rows),
        (Some(rows), false) => tag_units(&mut subject, rows),
        (None, true) => {
            return Err(SpecError(
                "`dissolve` needs a data-unit table (setUnits)".into(),
            ))
        }
        (None, false) => {}
    }
    let mut layers = MapLayers {
        subject,
        context: src.context.map(LoadedLayer::all).unwrap_or_default(),
        lakes: src.lakes.map(LoadedLayer::all).unwrap_or_default(),
        disputed_areas: src.disputed_areas.map(LoadedLayer::all).unwrap_or_default(),
        disputed: src.disputed.map(|d| d.lines.clone()).unwrap_or_default(),
    };
    layers.exclude_subject_from_context(region);
    let rendered = render(&layers, &opts)?;
    let projection = rendered.projection.name();
    let center = rendered.projection.center();
    let html = (spec.format == Format::Html).then(|| {
        let title = spec.title.as_deref().unwrap_or("map");
        mapgen_core::html_page(&rendered.svg, title, &opts.theme, "map.svg")
    });
    Ok(MapOutput {
        regions: layers.subject.len() - rendered.outside_frame.len(),
        svg: rendered.svg,
        html,
        width: rendered.width,
        height: rendered.height,
        projection,
        center,
        outside_frame: rendered.outside_frame,
        insets: rendered
            .insets
            .into_iter()
            .map(|i| InsetOutput {
                ids: i.ids,
                projection: i.projection,
            })
            .collect(),
        units: unit_report,
    })
}

/// Built-in themes as `{ name: { slot: colour } }`.
pub fn theme_table() -> BTreeMap<&'static str, BTreeMap<&'static str, String>> {
    Theme::NAMES
        .iter()
        .map(|&n| {
            let t = Theme::builtin(n).expect("built-in theme");
            (
                n,
                t.colors()
                    .iter()
                    .map(|(s, c)| (*s, c.as_str().to_owned()))
                    .collect(),
            )
        })
        .collect()
}

/// Frame presets as `{ name: [west, south, east, north] }`.
pub fn bbox_table() -> BTreeMap<&'static str, [f64; 4]> {
    BBOX_PRESETS
        .iter()
        .map(|(n, b)| (*n, [b.west, b.south, b.east, b.north]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWIN: &str = include_str!("../../mapgen-data/tests/fixtures/twin-regions.geojson");

    fn spec(json: &str) -> Result<RenderSpec> {
        serde_json::from_str(json).map_err(SpecError::from)
    }

    fn twin() -> LoadedLayer {
        LoadedLayer::parse(TWIN, &LayerSpec::default()).unwrap()
    }

    fn render_twin(s: &RenderSpec) -> Result<MapOutput> {
        let layer = twin();
        render_map(
            Sources {
                subject: &layer,
                context: None,
                lakes: None,
                disputed_areas: None,
                disputed: None,
                units: None,
            },
            s,
        )
    }

    #[test]
    fn defaults_match_the_cli() {
        let o = RenderSpec::default().options().unwrap();
        assert_eq!(o, RenderOptions::default());
    }

    #[test]
    fn rejects_unknown_fields_and_values() {
        assert!(spec(r#"{"colours": {}}"#).is_err());
        assert!(spec(r#"{"frame": "globe"}"#).is_err());
        assert!(spec(r#"{"projection": "mercator"}"#).is_err());
        let bad = |j: &str| spec(j).unwrap().options().unwrap_err().0;
        assert!(bad(r#"{"theme": "neon"}"#).contains("wikimedia"));
        assert!(bad(r#"{"colors": {"sea": "red"}}"#).contains("unknown colour slot"));
        assert!(bad(r#"{"colors": {"water": "red;}</style>"}}"#).contains("invalid colour"));
        assert!(bad(r#"{"width": 5}"#).contains("width"));
        assert!(bad(r#"{"simplify": -1}"#).contains("simplify"));
        assert!(bad(r#"{"labelSize": 0}"#).contains("labelSize"));
        assert!(bad(r#"{"bbox": "atlantis"}"#).contains("europe"));
    }

    #[test]
    fn applies_theme_and_color_overrides() {
        let s =
            spec(r##"{"theme": "dark", "colors": {"water": "#123456", "earth": "tan"}}"##).unwrap();
        let t = s.options().unwrap().theme;
        assert_eq!(t.water.as_str(), "#123456");
        assert_eq!(t.land.as_str(), "tan");
        assert_eq!(t.background, Theme::builtin("dark").unwrap().background);
    }

    #[test]
    fn renders_the_golden_fixture_byte_for_byte() {
        // Same options as crates/mapgen-data/tests/golden.rs.
        let s = spec(r#"{"width": 400, "title": "Twin regions", "labels": true}"#).unwrap();
        let golden = include_str!("../../mapgen-data/tests/fixtures/twin-regions.svg");
        assert_eq!(render_twin(&s).unwrap().svg, golden);
    }

    #[test]
    fn reports_metadata() {
        let out = render_twin(&RenderSpec::default()).unwrap();
        assert_eq!(out.regions, 2);
        assert_eq!(out.projection, "laea");
        assert!(out.height > 0 && out.width == 1000);
        assert!(out.html.is_none());
        let html = render_twin(&spec(r#"{"format": "html"}"#).unwrap()).unwrap();
        assert!(html.html.unwrap().contains("Download SVG"));
        assert!(html.svg.contains("var(--mg-water,"));
    }

    #[test]
    fn region_filtering() {
        let layer = LoadedLayer::parse(
            TWIN,
            &LayerSpec {
                filter_property: Some("name".into()),
                ..LayerSpec::default()
            },
        )
        .unwrap();
        assert_eq!(layer.regions(), ["East & Co", "Westland"]);
        let src = Sources {
            subject: &layer,
            context: None,
            lakes: None,
            disputed_areas: None,
            disputed: None,
            units: None,
        };
        let out = render_map(src, &spec(r#"{"region": "Westland"}"#).unwrap()).unwrap();
        assert_eq!(out.regions, 1);
        let err = render_map(src, &spec(r#"{"region": "Nowhere"}"#).unwrap()).unwrap_err();
        assert!(err.0.contains("Nowhere"));
        // Without a region column, asking for a region is an error, not a silent no-op.
        let no_col = twin();
        let src = Sources {
            subject: &no_col,
            ..src
        };
        assert!(render_map(src, &spec(r#"{"region": "X"}"#).unwrap()).is_err());
    }

    #[test]
    fn parses_features_and_bare_geometries() {
        let feature = r#"{"type":"Feature","properties":{"id":"A"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}}"#;
        assert_eq!(
            LoadedLayer::parse(feature, &LayerSpec::default())
                .unwrap()
                .len(),
            1
        );
        let geometry = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}"#;
        assert_eq!(
            LoadedLayer::parse(geometry, &LayerSpec::default())
                .unwrap()
                .len(),
            1
        );
        assert!(LoadedLayer::parse("not json", &LayerSpec::default()).is_err());
        let empty = r#"{"type":"FeatureCollection","features":[]}"#;
        assert!(LoadedLayer::parse(empty, &LayerSpec::default()).is_err());
    }

    #[test]
    fn attribution_from_layers_and_override() {
        let subject = LoadedLayer::parse(
            TWIN,
            &LayerSpec {
                attribution: Some("IGN (Etalab 2.0)".into()),
                ..LayerSpec::default()
            },
        )
        .unwrap();
        // Context defaults to Natural Earth; duplicates are merged.
        let context =
            LoadedLayer::parse(TWIN, &LayerSpec::with_dataset(Dataset::NeAdmin0)).unwrap();
        let src = Sources {
            subject: &subject,
            context: Some(&context),
            lakes: Some(&context),
            disputed_areas: None,
            disputed: None,
            units: None,
        };
        let out = render_map(src, &RenderSpec::default()).unwrap();
        assert!(out.svg.contains(
            r#"<desc id="attribution">IGN (Etalab 2.0); Natural Earth (de facto view)</desc>"#
        ));
        // A point-of-view file names its view.
        let india = LoadedLayer::parse(
            TWIN,
            &LayerSpec {
                worldview: Some("ind".into()),
                ..LayerSpec::with_dataset(Dataset::NeAdmin0)
            },
        )
        .unwrap();
        let out = render_map(
            Sources {
                context: Some(&india),
                lakes: None,
                ..src
            },
            &RenderSpec::default(),
        )
        .unwrap();
        assert!(out.svg.contains("Natural Earth (IND view)"));
        let out = render_map(src, &spec(r#"{"attribution": "Mine"}"#).unwrap()).unwrap();
        assert!(out.svg.contains(">Mine</desc>"));
        // A custom layer without a credit adds none.
        assert!(!render_twin(&RenderSpec::default())
            .unwrap()
            .svg
            .contains("<desc"));
    }

    #[test]
    fn units_tag_dissolve_and_report() {
        let rows = parse_units(
            "map_id,data_unit_id,data_unit_name\nXA-01,U1,Union\nXA-02,U1,Union\nZZ,U2,Ghost\n",
            &UnitColumns::default(),
        )
        .unwrap();
        let layer = twin();
        let src = Sources {
            subject: &layer,
            context: None,
            lakes: None,
            disputed_areas: None,
            disputed: None,
            units: Some(&rows),
        };
        let tagged = render_map(src, &RenderSpec::default()).unwrap();
        assert!(tagged.svg.contains(r#"data-code="XA-01" data-unit="U1""#));
        let report = tagged.units.unwrap();
        assert_eq!(report.unknown_regions, ["ZZ"]);
        let merged = render_map(src, &spec(r#"{"dissolve": true}"#).unwrap()).unwrap();
        assert_eq!(merged.regions, 1);
        assert!(merged.svg.contains(r#"<path id="U1""#));
        let no_table = Sources { units: None, ..src };
        assert!(render_map(no_table, &spec(r#"{"dissolve": true}"#).unwrap()).is_err());
    }

    #[test]
    fn tables_are_complete() {
        assert_eq!(theme_table().len(), Theme::NAMES.len());
        assert_eq!(theme_table()["wikimedia"]["water"], "#c6ecff");
        assert_eq!(bbox_table()["europe"], [-25.0, 34.0, 45.0, 72.0]);
    }
}
