use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use mapgen_core::frame::BBOX_PRESETS;
use mapgen_core::validate::{self, CheckOptions, IssueKind};
use mapgen_core::{
    html_page, render, Color, FrameMode, GeoBBox, InsetMode, MapFeature, MapLayers, MapLine,
    ProjectionChoice, RenderOptions, Theme,
};
use mapgen_data::crosswalk::{crosswalk, load_reference, ReferenceSpec};
use mapgen_data::geojson::{read_lines, read_records};
use mapgen_data::gpkg_write::{write_gpkg, WriteOptions};
use mapgen_data::{
    list_regions, read_grouped, read_layer, read_layer_in, Format, LayerQuery, Source,
};
use rayon::prelude::*;

/// Deterministic SVG map generator.
#[derive(Parser)]
#[command(name = "mapgen", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render one map to an SVG (or interactive HTML) file.
    Render(RenderArgs),
    /// Render one map per region (e.g. every country) in parallel.
    Batch(BatchArgs),
    /// List built-in colour themes and frame presets.
    Themes,
    /// Convert a GeoJSON layer into an indexed GeoPackage (fast region and
    /// bounding-box reads), optionally repairing it and adding readable ids.
    Convert(ConvertArgs),
    /// Check a layer for invalid polygons, slivers, overlaps, near-miss
    /// borders and duplicate ids.
    Check(CheckArgs),
}

#[derive(Clone, Copy, ValueEnum)]
enum Dataset {
    /// Any layer; set --table (GeoPackage) and --id-column/--name-column as needed.
    Custom,
    /// geoBoundaries gbOpen GeoJSON (see `scripts/fetch-data.sh geoboundaries`).
    Geoboundaries,
    /// Natural Earth 1:10m Admin-0 (countries).
    NeAdmin0,
    /// Natural Earth 1:10m Admin-1 (states, provinces, départements).
    NeAdmin1,
}

#[derive(clap::Args)]
struct InputArgs {
    /// Layout of the input file(s).
    #[arg(long, value_enum, default_value = "custom")]
    dataset: Dataset,

    /// GeoPackage table (overrides the dataset preset).
    #[arg(long)]
    table: Option<String>,

    /// Column/property holding the id (overrides the dataset preset).
    #[arg(long)]
    id_column: Option<String>,

    /// Column/property holding the display name, e.g. `name_fr` for Natural Earth.
    #[arg(long)]
    name_column: Option<String>,

    /// Column/property that --region is compared against.
    #[arg(long)]
    filter_column: Option<String>,

    /// Column/property with the code of the enclosing unit (e.g. the région of
    /// a département); borders between different parents are drawn thicker.
    #[arg(long)]
    parent_column: Option<String>,

    /// Neighbouring countries for context: Natural Earth Admin-0 (.gpkg or .geojson).
    #[arg(long)]
    context: Option<PathBuf>,

    /// Lakes: Natural Earth lakes (.gpkg or .geojson).
    #[arg(long)]
    lakes: Option<PathBuf>,

    /// Disputed and claimed boundaries, drawn dashed: Natural Earth
    /// `ne_10m_admin_0_boundary_lines_disputed_areas` (.geojson).
    #[arg(long)]
    disputed: Option<PathBuf>,

    /// Data credit embedded in the SVG. Default: built from the inputs
    /// (`<file>.license.json` next to a data file, or Natural Earth).
    #[arg(long)]
    attribution: Option<String>,

    /// Also draw the data credit in the bottom-right corner.
    #[arg(long)]
    credit: bool,
}

impl InputArgs {
    fn query(&self) -> LayerQuery {
        let mut q = match self.dataset {
            Dataset::Custom => LayerQuery {
                table: None,
                id_columns: vec!["id".into()],
                name_column: "name".into(),
                filter_column: None,
                parent_column: None,
                class: "region".into(),
            },
            Dataset::Geoboundaries => Source::GeoBoundaries.layer_query(),
            Dataset::NeAdmin0 => Source::NaturalEarthAdmin0.layer_query(),
            Dataset::NeAdmin1 => Source::NaturalEarthAdmin1.layer_query(),
        };
        if let Some(t) = &self.table {
            q.table = Some(t.clone());
        }
        if let Some(c) = &self.id_column {
            q.id_columns = vec![c.clone()];
        }
        if let Some(c) = &self.name_column {
            q.name_column = c.clone();
        }
        if let Some(c) = &self.filter_column {
            q.filter_column = Some(c.clone());
        }
        if let Some(c) = &self.parent_column {
            q.parent_column = Some(c.clone());
        }
        q
    }

    /// Credits for a data file and the context layers, and whether any
    /// licence is share-alike.
    fn attribution(&self, data: &Path) -> (Option<String>, bool) {
        if let Some(a) = &self.attribution {
            return (Some(a.clone()), false);
        }
        let mut credits: Vec<String> = Vec::new();
        let mut share_alike = false;
        let inputs = [
            Some(data),
            self.context.as_deref(),
            self.lakes.as_deref(),
            self.disputed.as_deref(),
        ];
        for (i, path) in inputs.into_iter().enumerate() {
            let Some(path) = path else { continue };
            let credit = match read_license(path) {
                Some(l) => {
                    share_alike |= l.share_alike();
                    l.credit()
                }
                None if i > 0 || matches!(self.dataset, Dataset::NeAdmin0 | Dataset::NeAdmin1) => {
                    "Natural Earth".to_owned()
                }
                None => continue,
            };
            if !credits.contains(&credit) {
                credits.push(credit);
            }
        }
        (
            (!credits.is_empty()).then(|| credits.join("; ")),
            share_alike,
        )
    }

    /// Neighbouring countries, lakes and disputed lines, limited to `bbox`
    /// when given (fast on GeoPackages written by `mapgen convert`).
    fn load_context(&self, bbox: Option<[f64; 4]>) -> Result<Surroundings> {
        let load = |path: &Option<PathBuf>, source: Source| -> Result<Vec<MapFeature>> {
            match path {
                Some(p) => read_layer_in(p, &source.layer_query(), None, bbox)
                    .with_context(|| format!("reading {}", p.display())),
                None => Ok(Vec::new()),
            }
        };
        let disputed = match &self.disputed {
            Some(p) => read_lines(p, &Source::NaturalEarthDisputedLines.layer_query())
                .with_context(|| format!("reading {}", p.display()))?,
            None => Vec::new(),
        };
        Ok(Surroundings {
            countries: load(&self.context, Source::NaturalEarthAdmin0)?,
            lakes: load(&self.lakes, Source::NaturalEarthLakes)?,
            disputed,
        })
    }
}

/// Layers drawn around the mapped regions.
struct Surroundings {
    countries: Vec<MapFeature>,
    lakes: Vec<MapFeature>,
    disputed: Vec<MapLine>,
}

/// Lon/lat box around the subject, generously padded, for loading only the
/// nearby context. `None` (load everything) for very wide or antimeridian-
/// crossing subjects.
fn context_bbox(subject: &[MapFeature]) -> Option<[f64; 4]> {
    use geo::BoundingRect;
    let b = subject
        .iter()
        .filter_map(|f| f.geometry.bounding_rect())
        .reduce(|a, b| {
            geo_types::Rect::new(
                geo_types::coord! { x: a.min().x.min(b.min().x), y: a.min().y.min(b.min().y) },
                geo_types::coord! { x: a.max().x.max(b.max().x), y: a.max().y.max(b.max().y) },
            )
        })?;
    if b.width() > 180.0 {
        return None;
    }
    let (px, py) = (b.width() * 0.5 + 5.0, b.height() * 0.5 + 5.0);
    Some([
        (b.min().x - px).max(-180.0),
        (b.min().y - py).max(-90.0),
        (b.max().x + px).min(180.0),
        (b.max().y + py).min(90.0),
    ])
}

#[derive(clap::Args)]
struct StyleArgs {
    /// Colour theme (see `mapgen themes`).
    #[arg(long, default_value = "wikimedia", value_parser = clap::builder::PossibleValuesParser::new(Theme::NAMES))]
    theme: String,
    /// Canvas colour (visible in padding, around world maps, or through `--water none`).
    #[arg(long)]
    background: Option<Color>,
    /// Sea and lakes.
    #[arg(long)]
    water: Option<Color>,
    /// The mapped regions.
    #[arg(long, visible_alias = "earth")]
    land: Option<Color>,
    /// Neighbouring countries.
    #[arg(long)]
    context_land: Option<Color>,
    /// Borders between mapped regions.
    #[arg(long)]
    border: Option<Color>,
    /// Outer edge of the mapped area (coasts, borders with neighbours).
    #[arg(long)]
    outline: Option<Color>,
    #[arg(long)]
    context_border: Option<Color>,
    #[arg(long)]
    lake_border: Option<Color>,
    /// Disputed boundaries (dashed).
    #[arg(long)]
    disputed_border: Option<Color>,
    #[arg(long)]
    label_color: Option<Color>,
    #[arg(long)]
    border_width: Option<f64>,
    /// Width of borders between regions with different parents.
    #[arg(long)]
    parent_border_width: Option<f64>,
    #[arg(long)]
    outline_width: Option<f64>,
    #[arg(long)]
    context_border_width: Option<f64>,
    #[arg(long)]
    disputed_border_width: Option<f64>,
    #[arg(long)]
    label_size: Option<f64>,
    /// Draw region names.
    #[arg(long)]
    labels: bool,
    /// Don't place labels of small regions outside them with leader lines.
    #[arg(long)]
    no_leaders: bool,
    /// Don't curve labels along long, thin regions.
    #[arg(long)]
    no_curved_labels: bool,
    /// Smallest label size, as a fraction of --label-size (labels shrink to fit).
    #[arg(long, default_value_t = 0.7)]
    label_min_scale: f64,
    /// Emit colours as CSS custom properties (`var(--mg-water, …)`) so a web
    /// page can restyle an inline SVG.
    #[arg(long)]
    css_vars: bool,
}

impl StyleArgs {
    fn theme(&self) -> Theme {
        let mut t = Theme::builtin(&self.theme).expect("validated by clap");
        let set = |slot: &mut Color, v: &Option<Color>| {
            if let Some(v) = v {
                *slot = v.clone();
            }
        };
        set(&mut t.background, &self.background);
        set(&mut t.water, &self.water);
        set(&mut t.land, &self.land);
        set(&mut t.context_land, &self.context_land);
        set(&mut t.border, &self.border);
        set(&mut t.outline, &self.outline);
        set(&mut t.context_border, &self.context_border);
        set(&mut t.lake_border, &self.lake_border);
        set(&mut t.disputed_border, &self.disputed_border);
        set(&mut t.label, &self.label_color);
        t.border_width = self.border_width.unwrap_or(t.border_width);
        t.parent_border_width = self.parent_border_width.unwrap_or(t.parent_border_width);
        t.outline_width = self.outline_width.unwrap_or(t.outline_width);
        t.context_border_width = self.context_border_width.unwrap_or(t.context_border_width);
        t.disputed_border_width = self
            .disputed_border_width
            .unwrap_or(t.disputed_border_width);
        t.label_size = self.label_size.unwrap_or(t.label_size);
        t
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum FrameArg {
    /// Main landmass; far overseas territories are left out.
    Auto,
    /// Every feature.
    All,
    /// The whole globe (Equal Earth).
    World,
}

#[derive(Clone, Copy, ValueEnum)]
enum InsetArg {
    /// Far-away parts (overseas territories, Alaska, Hawaii…) in corner boxes.
    Auto,
    /// Leave far-away parts out.
    None,
}

/// `auto`, `laea`, `equal-earth`, `albers`, `lcc`, or `epsg:<code>`.
fn parse_projection(s: &str) -> std::result::Result<ProjectionChoice, String> {
    let lower = s.trim().to_ascii_lowercase();
    Ok(match lower.as_str() {
        "auto" => ProjectionChoice::Auto,
        "laea" => ProjectionChoice::Laea,
        "equal-earth" => ProjectionChoice::EqualEarth,
        "albers" => ProjectionChoice::Albers { parallels: None },
        "lcc" => ProjectionChoice::Lcc { parallels: None },
        other => match other.strip_prefix("epsg:").map(str::parse::<u32>) {
            Some(Ok(code)) => ProjectionChoice::Epsg(code),
            _ => {
                return Err(format!(
                    "expected auto, laea, equal-earth, albers, lcc or epsg:<code>, got {s:?}"
                ))
            }
        },
    })
}

fn parse_parallels(s: &str) -> std::result::Result<(f64, f64), String> {
    let v: Vec<f64> = s
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| e.to_string())?;
    match v[..] {
        [a, b] if a.abs() < 90.0 && b.abs() < 90.0 && a != -b => Ok((a, b)),
        _ => Err(format!("expected two latitudes like 29.5,45.5, got {s:?}")),
    }
}

#[derive(clap::Args)]
struct LayoutArgs {
    /// Output width in pixels.
    #[arg(long, default_value_t = 1000)]
    width: u32,
    /// Margin in pixels, painted in the background colour.
    #[arg(long, default_value_t = 0)]
    padding: u32,
    /// Simplification tolerance in pixels (0 disables).
    #[arg(long, default_value_t = 0.5)]
    simplify: f64,
    /// Drop islands/lakes smaller than this many square pixels.
    #[arg(long, default_value_t = 0.5)]
    min_area: f64,
    /// Decimal places in path coordinates.
    #[arg(long, default_value_t = 1)]
    precision: usize,
    #[arg(long, value_enum, default_value = "auto")]
    frame: FrameArg,
    /// Fixed frame: `west,south,east,north` or a preset (see `mapgen themes`).
    #[arg(long, allow_hyphen_values = true)]
    bbox: Option<String>,
    /// Room around the framed features, as a fraction of the frame size.
    #[arg(long, default_value_t = 0.04)]
    margin: f64,
    /// auto, laea, equal-earth, albers, lcc, or epsg:<code> (EPSG needs a
    /// build with `--features proj`).
    #[arg(long, default_value = "auto", value_parser = parse_projection)]
    projection: ProjectionChoice,
    /// Standard parallels for albers/lcc, e.g. `29.5,45.5` (default: from the region).
    #[arg(long, allow_hyphen_values = true, value_parser = parse_parallels)]
    parallels: Option<(f64, f64)>,
    /// Central meridian override (e.g. 150 for a Pacific-centred world map).
    #[arg(long, allow_hyphen_values = true)]
    center_lon: Option<f64>,
    /// Snap neighbouring countries within this many pixels onto the mapped
    /// area's outline (0 disables).
    #[arg(long, default_value_t = 2.0)]
    snap: f64,
    #[arg(long, value_enum, default_value = "auto")]
    insets: InsetArg,
    /// At most this many insets.
    #[arg(long, default_value_t = 6)]
    max_insets: usize,
    /// Repair the input before rendering (duplicate points, invalid polygons,
    /// near-miss borders); see `mapgen check`.
    #[arg(long)]
    repair: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    /// Pick from the output file extension.
    Auto,
    Svg,
    /// Standalone page with the map inline and live colour pickers.
    Html,
}

#[derive(clap::Args)]
struct RenderArgs {
    /// Input file: GeoPackage (.gpkg) or GeoJSON (.geojson, .json).
    #[arg(short, long)]
    input: PathBuf,
    #[command(flatten)]
    data: InputArgs,
    /// Region code to keep, compared against the filter column (e.g. `FRA`).
    #[arg(long)]
    region: Option<String>,
    /// Natural Earth continent name (e.g. `Europe`, `South America`); implies
    /// `--filter-column CONTINENT` and a matching frame preset.
    #[arg(long, conflicts_with = "region")]
    continent: Option<String>,
    /// Output file (.svg or .html).
    #[arg(short, long)]
    out: PathBuf,
    #[arg(long, value_enum, default_value = "auto")]
    format: OutputFormat,
    /// Document title.
    #[arg(long)]
    title: Option<String>,
    #[command(flatten)]
    style: StyleArgs,
    #[command(flatten)]
    layout: LayoutArgs,
}

#[derive(clap::Args)]
struct BatchArgs {
    /// Input files and/or directories (every .geojson, .json and .gpkg inside,
    /// `.license.json` sidecars excepted). Shell globs work too.
    #[arg(short, long, num_args = 1.., required = true)]
    input: Vec<PathBuf>,
    #[command(flatten)]
    data: InputArgs,
    /// Directory to write the maps into. With one input file, maps are named
    /// `<region>.svg`; with several, `<file stem>.svg` (or `<file stem>-<region>.svg`
    /// when a file holds several regions).
    #[arg(long)]
    out_dir: PathBuf,
    /// Only these region codes (comma-separated); default: every value of the
    /// filter column in every file.
    #[arg(long, value_delimiter = ',')]
    regions: Option<Vec<String>>,
    #[arg(long, value_enum, default_value = "svg")]
    format: OutputFormat,
    /// Check each map's input (see `mapgen check`) and report issue counts.
    #[arg(long)]
    check: bool,
    #[command(flatten)]
    style: StyleArgs,
    #[command(flatten)]
    layout: LayoutArgs,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Render(args) => run_render(args),
        Command::Batch(args) => run_batch(args),
        Command::Themes => {
            print_themes();
            Ok(())
        }
        Command::Convert(args) => run_convert(args),
        Command::Check(args) => run_check(args),
    }
}

fn options(
    input: &InputArgs,
    data: &Path,
    style: &StyleArgs,
    layout: &LayoutArgs,
    title: Option<String>,
    html: bool,
) -> Result<RenderOptions> {
    let frame = match (&layout.bbox, layout.frame) {
        (Some(b), _) => FrameMode::BBox(GeoBBox::parse(b)?),
        (None, FrameArg::Auto) => FrameMode::Auto,
        (None, FrameArg::All) => FrameMode::All,
        (None, FrameArg::World) => FrameMode::World,
    };
    Ok(RenderOptions {
        width: layout.width,
        padding: layout.padding,
        precision: layout.precision,
        title,
        attribution: input.attribution(data).0,
        credit: input.credit,
        theme: style.theme(),
        css_vars: style.css_vars || html,
        labels: style.labels,
        label_leaders: !style.no_leaders,
        label_curved: !style.no_curved_labels,
        label_min_scale: style.label_min_scale,
        simplify_px: layout.simplify,
        min_area_px: layout.min_area,
        snap_px: layout.snap,
        projection: match (layout.projection, layout.parallels) {
            (ProjectionChoice::Albers { .. }, Some(p)) => {
                ProjectionChoice::Albers { parallels: Some(p) }
            }
            (ProjectionChoice::Lcc { .. }, Some(p)) => ProjectionChoice::Lcc { parallels: Some(p) },
            (choice, _) => choice,
        },
        frame,
        insets: match layout.insets {
            InsetArg::Auto => InsetMode::Auto,
            InsetArg::None => InsetMode::None,
        },
        max_insets: layout.max_insets,
        margin: layout.margin,
        center_lon: layout.center_lon,
    })
}

fn is_html(format: OutputFormat, out: &Path) -> bool {
    match format {
        OutputFormat::Html => true,
        OutputFormat::Svg => false,
        OutputFormat::Auto => out
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("html")),
    }
}

fn run_render(args: RenderArgs) -> Result<()> {
    let mut query = args.data.query();
    let mut layout = args.layout;
    let region = if let Some(continent) = &args.continent {
        query.filter_column = Some("CONTINENT".into());
        if layout.bbox.is_none()
            && matches!(layout.frame, FrameArg::Auto)
            && GeoBBox::parse(continent).is_ok()
        {
            layout.bbox = Some(continent.clone());
        }
        Some(continent.clone())
    } else {
        args.region.clone()
    };
    if region.is_some() && query.filter_column.is_none() {
        bail!("--region needs a filter column: pass --filter-column or a --dataset preset");
    }

    let mut subject = read_layer(&args.input, &query, region.as_deref())
        .with_context(|| format!("reading {}", args.input.display()))?;
    if subject.is_empty() {
        bail!(
            "no features matched region {:?}",
            region.unwrap_or_default()
        );
    }
    if layout.repair {
        report_repair(&validate::repair(&mut subject, &CheckOptions::default()));
    }
    let context = args.data.load_context(context_bbox(&subject))?;

    let html = is_html(args.format, &args.out);
    let opts = options(
        &args.data,
        &args.input,
        &args.style,
        &layout,
        args.title.clone(),
        html,
    )?;
    let mut layers = MapLayers {
        subject,
        context: context.countries,
        lakes: context.lakes,
        disputed: context.disputed,
    };
    layers.exclude_subject_from_context(region.as_deref());
    let rendered = render(&layers, &opts)?;
    let body = if html {
        to_html(&rendered.svg, &args.out, args.title.as_deref(), &opts.theme)
    } else {
        rendered.svg
    };
    std::fs::write(&args.out, &body).with_context(|| format!("writing {}", args.out.display()))?;

    eprintln!(
        "wrote {} ({}×{} px, {} regions, {} bytes, {})",
        args.out.display(),
        rendered.width,
        rendered.height,
        layers.subject.len() - rendered.outside_frame.len(),
        body.len(),
        rendered.projection.name(),
    );
    for inset in &rendered.insets {
        let names: Vec<&str> = inset
            .ids
            .iter()
            .map(|id| {
                layers
                    .subject
                    .iter()
                    .find(|f| &f.id == id)
                    .map_or(id.as_str(), |f| f.name.as_str())
            })
            .take(4)
            .collect();
        let more = inset.ids.len().saturating_sub(4);
        eprintln!(
            "inset: {}{} ({})",
            names.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            },
            inset.projection
        );
    }
    report_license(&args.data, &args.input);
    if !rendered.outside_frame.is_empty() {
        let names: Vec<&str> = rendered
            .outside_frame
            .iter()
            .map(|id| {
                layers
                    .subject
                    .iter()
                    .find(|f| &f.id == id)
                    .map_or(id.as_str(), |f| f.name.as_str())
            })
            .collect();
        let shown = names.iter().take(8).copied().collect::<Vec<_>>().join(", ");
        let more = names.len().saturating_sub(8);
        eprintln!(
            "note: {} region(s) outside the frame were left out: {shown}{} (use --frame all, or more --max-insets)",
            names.len(),
            if more > 0 { format!(" and {more} more") } else { String::new() },
        );
    }
    Ok(())
}

fn to_html(svg: &str, out: &Path, title: Option<&str>, theme: &Theme) -> String {
    let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("map");
    html_page(svg, title.unwrap_or(stem), theme, &format!("{stem}.svg"))
}

/// Shared, read-only state for a batch run.
struct Batch<'a> {
    args: &'a BatchArgs,
    query: LayerQuery,
    context: Surroundings,
    wanted: Option<BTreeSet<String>>,
    multi: bool,
    html: bool,
}

/// `(output name, Ok(note) | Err)`; the note summarises `--check` findings.
type JobResult = (String, Result<String>);

fn run_batch(args: BatchArgs) -> Result<()> {
    let files = expand_inputs(&args.input)?;
    if files.is_empty() {
        bail!("no .geojson, .json or .gpkg files found in the given inputs");
    }
    let context = args.data.load_context(None)?;
    std::fs::create_dir_all(&args.out_dir)?;
    let batch = Batch {
        query: args.data.query(),
        context,
        wanted: args.regions.clone().map(|r| r.into_iter().collect()),
        multi: files.len() > 1,
        html: args.format == OutputFormat::Html,
        args: &args,
    };

    // Parallel over files, and over regions within a file. Each file is read
    // only by the job that renders it, so memory stays bounded.
    let results: Vec<JobResult> = files
        .par_iter()
        .filter(|f| batch.may_contain_wanted(f))
        .map(|f| batch.file_jobs(f))
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect();

    let mut ok = 0;
    for (label, r) in &results {
        match r {
            Ok(note) => {
                ok += 1;
                if !note.is_empty() {
                    eprintln!("{label}: {note}");
                }
            }
            Err(e) => eprintln!("{label}: {e:#}"),
        }
    }
    eprintln!(
        "rendered {ok}/{} maps from {} file(s) into {}",
        results.len(),
        files.len(),
        args.out_dir.display()
    );
    let share_alike: Vec<String> = files
        .iter()
        .filter(|f| batch.may_contain_wanted(f))
        .filter(|f| read_license(f).is_some_and(|l| l.share_alike()))
        .map(|f| stem(f))
        .collect();
    if !share_alike.is_empty() {
        eprintln!(
            "note: share-alike data licence for {}; maps made from them must be shared under the same licence",
            share_alike.join(", ")
        );
    }
    if results.is_empty() {
        bail!("no regions matched");
    }
    if ok == 0 {
        bail!("every map failed");
    }
    Ok(())
}

impl Batch<'_> {
    /// geoBoundaries files are named `<ISO3>-<LEVEL>`, so `--regions` can skip
    /// files without reading them.
    fn may_contain_wanted(&self, file: &Path) -> bool {
        match (&self.wanted, self.args.data.dataset) {
            (Some(w), Dataset::Geoboundaries) => {
                let s = stem(file);
                let iso = s.split('-').next().unwrap_or(&s);
                w.contains(iso)
            }
            _ => true,
        }
    }

    fn file_jobs(&self, file: &Path) -> Vec<JobResult> {
        let fail = |e: anyhow::Error| vec![(file.display().to_string(), Err(e))];
        let keep = |code: &String| self.wanted.as_ref().is_none_or(|w| w.contains(code));
        let file_stem = stem(file);

        if self.query.filter_column.is_none() {
            // No region column: the whole file is one map.
            let job = || -> Result<String> {
                let subject = read_layer(file, &self.query, None)?;
                self.render_to(file, None, subject, &file_stem)
            };
            return vec![(file_stem.clone(), job())];
        }

        let format = match Format::of(file) {
            Ok(f) => f,
            Err(e) => return fail(e.into()),
        };
        match format {
            Format::GeoJson => {
                let groups = match read_grouped(file, &self.query) {
                    Ok(g) => g,
                    Err(e) => return fail(anyhow::Error::from(e).context("reading file")),
                };
                let groups: Vec<(String, Vec<MapFeature>)> =
                    groups.into_iter().filter(|(c, _)| keep(c)).collect();
                let single = groups.len() == 1;
                groups
                    .into_par_iter()
                    .map(|(code, subject)| {
                        let out = self.out_stem(&file_stem, &code, single);
                        let r = self.render_to(file, Some(&code), subject, &out);
                        (out, r)
                    })
                    .collect()
            }
            Format::GeoPackage => {
                let regions = match list_regions(file, &self.query) {
                    Ok(r) => r,
                    Err(e) => return fail(anyhow::Error::from(e).context("listing regions")),
                };
                let regions: Vec<String> = regions.into_iter().filter(|c| keep(c)).collect();
                let single = regions.len() == 1;
                regions
                    .par_iter()
                    .map(|code| {
                        let out = self.out_stem(&file_stem, code, single);
                        let r = read_layer(file, &self.query, Some(code))
                            .map_err(anyhow::Error::from)
                            .and_then(|subject| self.render_to(file, Some(code), subject, &out));
                        (out, r)
                    })
                    .collect()
            }
        }
    }

    fn out_stem(&self, file_stem: &str, code: &str, single_region: bool) -> String {
        match (self.multi, single_region) {
            (false, _) => file_safe(code),
            (true, true) => file_safe(file_stem),
            (true, false) => file_safe(&format!("{file_stem}-{code}")),
        }
    }

    fn render_to(
        &self,
        file: &Path,
        code: Option<&str>,
        mut subject: Vec<MapFeature>,
        out_stem: &str,
    ) -> Result<String> {
        if subject.is_empty() {
            bail!("no features");
        }
        let args = self.args;
        let mut notes = Vec::new();
        if args.check {
            let issues = validate::check(&subject, &CheckOptions::default());
            if !issues.is_empty() {
                notes.push(format!(
                    "{} input issue(s): {}",
                    issues.len(),
                    issue_summary(&issues)
                ));
            }
        }
        if args.layout.repair {
            let r = validate::repair(&mut subject, &CheckOptions::default());
            if r != Default::default() {
                notes.push(repair_summary(&r));
            }
        }
        let mut layers = MapLayers {
            context: self.context.countries.clone(),
            lakes: self.context.lakes.clone(),
            disputed: self.context.disputed.clone(),
            subject,
        };
        layers.exclude_subject_from_context(code);
        let opts = options(
            &args.data,
            file,
            &args.style,
            &args.layout,
            Some(out_stem.to_owned()),
            self.html,
        )?;
        let rendered = render(&layers, &opts)?;
        let ext = if self.html { "html" } else { "svg" };
        let out = args.out_dir.join(format!("{out_stem}.{ext}"));
        let body = if self.html {
            to_html(&rendered.svg, &out, Some(out_stem), &opts.theme)
        } else {
            rendered.svg
        };
        std::fs::write(&out, body).with_context(|| format!("writing {}", out.display()))?;
        if !rendered.outside_frame.is_empty() {
            notes.push(format!(
                "{} region(s) left out",
                rendered.outside_frame.len()
            ));
        }
        Ok(notes.join("; "))
    }
}

/// Expands directories into their data files, drops licence sidecars, sorts,
/// and rejects duplicate file stems (they would overwrite each other's maps).
fn expand_inputs(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let is_sidecar = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".license.json"))
    };
    let mut files = BTreeSet::new();
    for p in paths {
        if p.is_dir() {
            for entry in std::fs::read_dir(p).with_context(|| format!("reading {}", p.display()))? {
                let path = entry?.path();
                if path.is_file() && !is_sidecar(&path) && Format::of(&path).is_ok() {
                    files.insert(path);
                }
            }
        } else if !is_sidecar(p) {
            files.insert(p.clone());
        }
    }
    let files: Vec<PathBuf> = files.into_iter().collect();
    if files.len() > 1 {
        let mut seen = BTreeSet::new();
        for f in &files {
            if !seen.insert(stem(f)) {
                bail!(
                    "two inputs share the file name {:?}; maps would overwrite each other",
                    stem(f)
                );
            }
        }
    }
    Ok(files)
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("map")
        .to_owned()
}

#[derive(clap::Args)]
struct ConvertArgs {
    /// Input GeoJSON.
    #[arg(short, long)]
    input: PathBuf,
    /// Output GeoPackage (.gpkg); replaced if it exists.
    #[arg(short, long)]
    out: PathBuf,
    #[command(flatten)]
    data: InputArgs,
    /// Repair geometries while converting (see `mapgen check`).
    #[arg(long)]
    repair: bool,
    /// Reference GeoJSON to borrow readable codes from, by spatial overlap;
    /// they are stored in `code` and `parent` columns, which the presets use
    /// as ids and parents.
    #[arg(long)]
    ids_from: Option<PathBuf>,
    /// Property of the reference holding the code (`id` for the feature id).
    /// Reference features sharing a code are merged first, so e.g.
    /// `region_cod` on Natural Earth Admin-1 yields régions.
    #[arg(long, requires = "ids_from")]
    ids_column: Option<String>,
    /// Property of the reference holding the parent code.
    #[arg(long, requires = "ids_from")]
    ids_parent_column: Option<String>,
    /// Prefix for borrowed codes, e.g. `US-` for FIPS codes.
    #[arg(long, default_value = "", requires = "ids_from")]
    ids_prefix: String,
    /// Each of a region and its reference unit must cover at least this share
    /// of the other to be considered the same place.
    #[arg(long, default_value_t = 0.5, requires = "ids_from")]
    ids_min_share: f64,
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Input file: GeoPackage (.gpkg) or GeoJSON (.geojson, .json).
    #[arg(short, long)]
    input: PathBuf,
    #[command(flatten)]
    data: InputArgs,
    /// Region code to check (default: the whole layer).
    #[arg(long)]
    region: Option<String>,
    /// Near-miss border tolerance, in degrees (1e-4 ≈ 10 m).
    #[arg(long, default_value_t = 1e-4)]
    tolerance: f64,
    /// Machine-readable output.
    #[arg(long)]
    json: bool,
    /// Exit with status 1 when issues are found.
    #[arg(long)]
    strict: bool,
}

fn run_convert(args: ConvertArgs) -> Result<()> {
    let started = std::time::Instant::now();
    if Format::of(&args.input)? != Format::GeoJson {
        bail!("convert reads GeoJSON (.geojson, .json)");
    }
    let mut records =
        read_records(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let query = args.data.query();

    if args.repair {
        let mut features: Vec<MapFeature> = records
            .iter()
            .enumerate()
            .map(|(i, r)| MapFeature {
                id: query
                    .id_columns
                    .iter()
                    .find_map(|c| r.prop(c))
                    .unwrap_or_else(|| i.to_string()),
                name: String::new(),
                class: String::new(),
                parent: None,
                geometry: polygons_of(&r.geometry),
            })
            .collect();
        report_repair(&validate::repair(&mut features, &CheckOptions::default()));
        for (r, f) in records.iter_mut().zip(features) {
            if is_areal(&r.geometry) {
                r.geometry = geo_types::Geometry::MultiPolygon(f.geometry);
            }
        }
    }

    if let Some(reference) = &args.ids_from {
        let spec = ReferenceSpec {
            code_column: args.ids_column.clone().unwrap_or_else(|| "id".into()),
            parent_column: args.ids_parent_column.clone(),
            prefix: args.ids_prefix.clone(),
        };
        let refs = load_reference(reference, &spec)
            .with_context(|| format!("reading {}", reference.display()))?;
        let targets: Vec<_> = records.iter().map(|r| polygons_of(&r.geometry)).collect();
        let matches = crosswalk(&targets, &refs, args.ids_min_share);
        let mut used: BTreeSet<&str> = BTreeSet::new();
        let mut duplicates = 0;
        let mut unmatched: Vec<String> = Vec::new();
        for (i, (r, m)) in records.iter_mut().zip(&matches).enumerate() {
            match m {
                Some(m) => {
                    if !used.insert(m.code.as_str()) {
                        duplicates += 1;
                    }
                    r.properties.insert("code".into(), m.code.clone().into());
                    if let Some(p) = &m.parent {
                        r.properties.insert("parent".into(), p.clone().into());
                    }
                }
                None => unmatched.push(
                    r.prop(&query.name_column)
                        .unwrap_or_else(|| format!("#{i}")),
                ),
            }
        }
        eprintln!(
            "ids: {}/{} regions matched a code from {}{}",
            records.len() - unmatched.len(),
            records.len(),
            reference.display(),
            if duplicates > 0 {
                format!(" ({duplicates} codes used twice)")
            } else {
                String::new()
            }
        );
        if !unmatched.is_empty() {
            let shown: Vec<&str> = unmatched.iter().take(8).map(String::as_str).collect();
            eprintln!(
                "     unmatched (keep their original id): {}{}",
                shown.join(", "),
                if unmatched.len() > 8 {
                    format!(" and {} more", unmatched.len() - 8)
                } else {
                    String::new()
                }
            );
        }
    }

    let table = query.table.clone().unwrap_or_else(|| stem(&args.input));
    let mut index_columns = query.id_columns.clone();
    index_columns.extend(query.filter_column.clone());
    index_columns.push("code".into());
    write_gpkg(
        &args.out,
        &records,
        &WriteOptions {
            table: table.clone(),
            index_columns,
        },
    )
    .with_context(|| format!("writing {}", args.out.display()))?;
    // Carry the licence sidecar over, so maps from the GeoPackage keep their credit.
    let sidecar = args.input.with_extension("license.json");
    if sidecar.exists() {
        std::fs::copy(&sidecar, args.out.with_extension("license.json"))?;
    }
    eprintln!(
        "wrote {} (layer {table}, {} features, {} bytes, {:.1}s)",
        args.out.display(),
        records.len(),
        std::fs::metadata(&args.out)?.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn is_areal(g: &geo_types::Geometry<f64>) -> bool {
    matches!(
        g,
        geo_types::Geometry::Polygon(_) | geo_types::Geometry::MultiPolygon(_)
    )
}

fn polygons_of(g: &geo_types::Geometry<f64>) -> geo_types::MultiPolygon<f64> {
    match g {
        geo_types::Geometry::Polygon(p) => geo_types::MultiPolygon(vec![p.clone()]),
        geo_types::Geometry::MultiPolygon(mp) => mp.clone(),
        geo_types::Geometry::GeometryCollection(gc) => {
            geo_types::MultiPolygon(gc.0.iter().flat_map(|g| polygons_of(g).0).collect())
        }
        _ => geo_types::MultiPolygon(vec![]),
    }
}

fn run_check(args: CheckArgs) -> Result<()> {
    let query = args.data.query();
    if args.region.is_some() && query.filter_column.is_none() {
        bail!("--region needs a filter column: pass --filter-column or a --dataset preset");
    }
    let features = read_layer(&args.input, &query, args.region.as_deref())
        .with_context(|| format!("reading {}", args.input.display()))?;
    let opts = CheckOptions {
        border_tolerance: args.tolerance,
        ..CheckOptions::default()
    };
    let issues = validate::check(&features, &opts);
    if args.json {
        let items: Vec<serde_json::Value> = issues
            .iter()
            .map(|i| {
                serde_json::json!({
                    "feature": i.feature,
                    "kind": kind_name(&i.kind),
                    "other": i.other,
                    "message": i.to_string(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        for i in &issues {
            println!("{i}");
        }
        eprintln!(
            "{} feature(s) checked: {}",
            features.len(),
            if issues.is_empty() {
                "no issues".to_owned()
            } else {
                issue_summary(&issues)
            }
        );
        if !issues.is_empty() {
            eprintln!(
                "fix what can be fixed with `mapgen convert --repair` or `mapgen render --repair`"
            );
        }
    }
    if args.strict && !issues.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn kind_name(k: &IssueKind) -> &'static str {
    match k {
        IssueKind::Empty => "empty",
        IssueKind::OutOfRange => "out-of-range",
        IssueKind::Invalid(_) => "invalid",
        IssueKind::DuplicatePoints(_) => "duplicate-points",
        IssueKind::Sliver => "sliver",
        IssueKind::Overlap(_) => "overlap",
        IssueKind::NearMissBorder(_) => "near-miss-border",
        IssueKind::DuplicateId => "duplicate-id",
    }
}

/// e.g. "3 invalid, 12 near-miss-border".
fn issue_summary(issues: &[validate::Issue]) -> String {
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for i in issues {
        *counts.entry(kind_name(&i.kind)).or_default() += 1;
    }
    counts
        .iter()
        .map(|(k, n)| format!("{n} {k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn repair_summary(r: &validate::RepairSummary) -> String {
    let snapping = if r.snapping_reverted {
        "border snapping undone (it did not reduce near-miss borders)".to_owned()
    } else {
        format!("{} border vertices snapped", r.snapped_vertices)
    };
    format!(
        "repaired: {} repeated points removed, {} degenerate rings dropped, {} invalid polygons rebuilt, {snapping}",
        r.removed_points, r.dropped_rings, r.fixed_polygons
    )
}

fn report_repair(r: &validate::RepairSummary) {
    eprintln!("{}", repair_summary(r));
}

/// Licence metadata written by `scripts/fetch-data.sh` next to a data file.
#[derive(serde::Deserialize)]
struct LicenseFile {
    license: String,
    source: String,
    #[serde(default)]
    via: Option<String>,
}

impl LicenseFile {
    fn credit(&self) -> String {
        match &self.via {
            Some(via) => format!("{} ({}) via {via}", self.source, self.license),
            None => format!("{} ({})", self.source, self.license),
        }
    }

    fn share_alike(&self) -> bool {
        let l = self.license.to_ascii_lowercase();
        l.contains("sharealike")
            || l.contains("share-alike")
            || l.contains("by-sa")
            || l.contains("odbl")
    }
}

/// Reads `<data>.license.json`. A missing file is normal; a file that exists
/// but can't be read or parsed is reported once, since silently dropping it
/// would drop a required credit.
fn read_license(data: &Path) -> Option<LicenseFile> {
    let path = data.with_extension("license.json");
    if !path.exists() {
        return None;
    }
    let parsed = std::fs::read_to_string(&path)
        .map_err(anyhow::Error::from)
        .and_then(|text| serde_json::from_str(&text).map_err(anyhow::Error::from));
    match parsed {
        Ok(l) => Some(l),
        Err(e) => {
            static WARNED: std::sync::Mutex<BTreeSet<PathBuf>> =
                std::sync::Mutex::new(BTreeSet::new());
            if WARNED.lock().is_ok_and(|mut w| w.insert(path.clone())) {
                eprintln!(
                    "warning: ignoring unreadable licence file {}: {e}",
                    path.display()
                );
            }
            None
        }
    }
}

fn report_license(input: &InputArgs, data: &Path) {
    let (attribution, share_alike) = input.attribution(data);
    if let Some(a) = attribution {
        eprintln!("attribution: {a}");
    }
    if share_alike {
        eprintln!(
            "note: share-alike data licence; maps made from it must be shared under the same licence"
        );
    }
}

fn file_safe(code: &str) -> String {
    code.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn print_themes() {
    for name in Theme::NAMES {
        let t = Theme::builtin(name).expect("built-in");
        println!("{name}");
        for (slot, color) in t.colors() {
            println!("  {slot:<15} {color}");
        }
    }
    println!("\nframe presets (--bbox):");
    for (name, b) in BBOX_PRESETS {
        println!("  {name:<14} {},{},{},{}", b.west, b.south, b.east, b.north);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn license(l: &str) -> LicenseFile {
        LicenseFile {
            license: l.into(),
            source: "Src".into(),
            via: Some("geoBoundaries".into()),
        }
    }

    #[test]
    fn detects_share_alike_licences() {
        assert!(license("Creative Commons Attribution-ShareAlike 2.0").share_alike());
        assert!(license("CC BY-SA 4.0").share_alike());
        assert!(license("Open Database License (ODbL) v1.0").share_alike());
        assert!(!license("Public Domain").share_alike());
        assert!(!license("Etalab Open License 2.0").share_alike());
    }

    #[test]
    fn expands_directories_and_skips_sidecars() {
        let dir = std::env::temp_dir().join(format!("mapgen-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in [
            "FRA-ADM1.geojson",
            "FRA-ADM1.license.json",
            "notes.txt",
            "sub/X.gpkg",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let files = expand_inputs(&[dir.clone(), dir.join("sub/X.gpkg")]).unwrap();
        let names: Vec<String> = files.iter().map(|f| stem(f)).collect();
        assert_eq!(names, ["FRA-ADM1", "X"]);

        std::fs::write(dir.join("sub/FRA-ADM1.geojson"), "").unwrap();
        let dup = expand_inputs(&[dir.clone(), dir.join("sub/FRA-ADM1.geojson")]);
        assert!(dup.is_err(), "duplicate stems must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_utf8_sidecars_and_rejects_broken_ones() {
        let dir = std::env::temp_dir().join(format!("mapgen-lic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data = dir.join("FRA-ADM1.geojson");
        let sidecar = dir.join("FRA-ADM1.license.json");
        std::fs::write(
            &sidecar,
            r#"{"license":"Etalab","source":"géographique","via":"geoBoundaries"}"#,
        )
        .unwrap();
        assert_eq!(read_license(&data).unwrap().source, "géographique");
        // cp1252 "é" is invalid UTF-8: must be reported (not a panic), and yield no credit.
        std::fs::write(&sidecar, b"{\"license\":\"x\",\"source\":\"g\xe9o\"}").unwrap();
        assert!(read_license(&data).is_none());
        assert!(read_license(&dir.join("missing.geojson")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn credit_names_source_licence_and_distributor() {
        assert_eq!(
            license("Public Domain").credit(),
            "Src (Public Domain) via geoBoundaries"
        );
    }
}
