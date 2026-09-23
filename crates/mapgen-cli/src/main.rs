use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use mapgen_core::frame::BBOX_PRESETS;
use mapgen_core::{
    html_page, render, Color, FrameMode, GeoBBox, MapFeature, MapLayers, ProjectionChoice,
    RenderOptions, Theme,
};
use mapgen_data::{list_regions, read_grouped, read_layer, Format, LayerQuery, Source};
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
    /// Input file: GeoPackage (.gpkg) or GeoJSON (.geojson, .json).
    #[arg(short, long)]
    input: PathBuf,

    /// Layout of the input file.
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

    /// Neighbouring countries for context: Natural Earth Admin-0 (.gpkg or .geojson).
    #[arg(long)]
    context: Option<PathBuf>,

    /// Lakes: Natural Earth lakes (.gpkg or .geojson).
    #[arg(long)]
    lakes: Option<PathBuf>,

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
        q
    }

    /// Credits for every input, and whether any licence is share-alike.
    fn attribution(&self) -> (Option<String>, bool) {
        if let Some(a) = &self.attribution {
            return (Some(a.clone()), false);
        }
        let mut credits: Vec<String> = Vec::new();
        let mut share_alike = false;
        let inputs = [
            Some(&self.input),
            self.context.as_ref(),
            self.lakes.as_ref(),
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

    fn load_context(&self) -> Result<(Vec<MapFeature>, Vec<MapFeature>)> {
        let load = |path: &Option<PathBuf>, source: Source| -> Result<Vec<MapFeature>> {
            match path {
                Some(p) => read_layer(p, &source.layer_query(), None)
                    .with_context(|| format!("reading {}", p.display())),
                None => Ok(Vec::new()),
            }
        };
        Ok((
            load(&self.context, Source::NaturalEarthAdmin0)?,
            load(&self.lakes, Source::NaturalEarthLakes)?,
        ))
    }
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
    #[arg(long)]
    context_border: Option<Color>,
    #[arg(long)]
    lake_border: Option<Color>,
    #[arg(long)]
    label_color: Option<Color>,
    #[arg(long)]
    border_width: Option<f64>,
    #[arg(long)]
    context_border_width: Option<f64>,
    #[arg(long)]
    label_size: Option<f64>,
    /// Draw region names.
    #[arg(long)]
    labels: bool,
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
        set(&mut t.context_border, &self.context_border);
        set(&mut t.lake_border, &self.lake_border);
        set(&mut t.label, &self.label_color);
        t.border_width = self.border_width.unwrap_or(t.border_width);
        t.context_border_width = self.context_border_width.unwrap_or(t.context_border_width);
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
enum ProjectionArg {
    Auto,
    Laea,
    EqualEarth,
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
    #[arg(long, value_enum, default_value = "auto")]
    projection: ProjectionArg,
    /// Central meridian override (e.g. 150 for a Pacific-centred world map).
    #[arg(long, allow_hyphen_values = true)]
    center_lon: Option<f64>,
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
    #[command(flatten)]
    input: InputArgs,
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
    #[command(flatten)]
    input: InputArgs,
    /// Directory to write one file per region into.
    #[arg(long)]
    out_dir: PathBuf,
    /// Only these region codes (comma-separated); default: every value of the filter column.
    #[arg(long, value_delimiter = ',')]
    regions: Option<Vec<String>>,
    #[arg(long, value_enum, default_value = "svg")]
    format: OutputFormat,
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
    }
}

fn options(
    input: &InputArgs,
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
        attribution: input.attribution().0,
        credit: input.credit,
        theme: style.theme(),
        css_vars: style.css_vars || html,
        labels: style.labels,
        simplify_px: layout.simplify,
        min_area_px: layout.min_area,
        projection: match layout.projection {
            ProjectionArg::Auto => ProjectionChoice::Auto,
            ProjectionArg::Laea => ProjectionChoice::Laea,
            ProjectionArg::EqualEarth => ProjectionChoice::EqualEarth,
        },
        frame,
        margin: layout.margin,
        center_lon: layout.center_lon,
    })
}

/// Context layer minus the countries being mapped.
fn context_for(
    all: &[MapFeature],
    subject: &[MapFeature],
    region: Option<&str>,
) -> Vec<MapFeature> {
    all.iter()
        .filter(|c| Some(c.id.as_str()) != region && !subject.iter().any(|s| s.id == c.id))
        .cloned()
        .collect()
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
    let mut query = args.input.query();
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

    let subject = read_layer(&args.input.input, &query, region.as_deref())
        .with_context(|| format!("reading {}", args.input.input.display()))?;
    if subject.is_empty() {
        bail!(
            "no features matched region {:?}",
            region.unwrap_or_default()
        );
    }
    let (context, lakes) = args.input.load_context()?;
    let context = context_for(&context, &subject, region.as_deref());

    let html = is_html(args.format, &args.out);
    let opts = options(&args.input, &args.style, &layout, args.title.clone(), html)?;
    let layers = MapLayers {
        subject,
        context,
        lakes,
    };
    let rendered = render(&layers, &opts)?;
    let body = if html {
        to_html(&rendered.svg, &args.out, args.title.as_deref(), &opts.theme)
    } else {
        rendered.svg
    };
    std::fs::write(&args.out, &body).with_context(|| format!("writing {}", args.out.display()))?;

    eprintln!(
        "wrote {} ({}×{} px, {} regions, {} bytes, {:?})",
        args.out.display(),
        rendered.width,
        rendered.height,
        layers.subject.len() - rendered.outside_frame.len(),
        body.len(),
        rendered.projection,
    );
    report_license(&args.input);
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
            "note: {} region(s) outside the frame were left out: {shown}{} (use --frame all to include them)",
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

fn run_batch(args: BatchArgs) -> Result<()> {
    let path = &args.input.input;
    let query = args.input.query();
    let regions = match &args.regions {
        Some(r) => r.clone(),
        None => list_regions(path, &query)
            .with_context(|| format!("listing regions in {}", path.display()))?,
    };
    // GeoJSON has no index, so read it once; GeoPackages are queried per region.
    let grouped = match Format::of(path)? {
        Format::GeoJson => Some(read_grouped(path, &query)?),
        Format::GeoPackage => None,
    };
    let (context, lakes) = args.input.load_context()?;
    std::fs::create_dir_all(&args.out_dir)?;
    let html = args.format == OutputFormat::Html;
    let ext = if html { "html" } else { "svg" };

    let results: Vec<(String, Result<usize>)> = regions
        .par_iter()
        .map(|code| {
            let job = || -> Result<usize> {
                let subject = match &grouped {
                    Some(g) => g.get(code).cloned().unwrap_or_default(),
                    None => read_layer(path, &query, Some(code))?,
                };
                if subject.is_empty() {
                    bail!("no features");
                }
                let layers = MapLayers {
                    context: context_for(&context, &subject, Some(code)),
                    lakes: lakes.clone(),
                    subject,
                };
                let opts = options(
                    &args.input,
                    &args.style,
                    &args.layout,
                    Some(code.clone()),
                    html,
                )?;
                let rendered = render(&layers, &opts)?;
                let out = args.out_dir.join(format!("{}.{ext}", file_safe(code)));
                let body = if html {
                    to_html(&rendered.svg, &out, Some(code), &opts.theme)
                } else {
                    rendered.svg
                };
                std::fs::write(&out, &body)?;
                Ok(body.len())
            };
            (code.clone(), job())
        })
        .collect();

    let mut ok = 0;
    for (code, r) in &results {
        match r {
            Ok(_) => ok += 1,
            Err(e) => eprintln!("{code}: {e:#}"),
        }
    }
    eprintln!(
        "rendered {ok}/{} maps into {}",
        results.len(),
        args.out_dir.display()
    );
    if ok == 0 && !results.is_empty() {
        bail!("every map failed");
    }
    Ok(())
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

fn read_license(data: &Path) -> Option<LicenseFile> {
    let text = std::fs::read_to_string(data.with_extension("license.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn report_license(input: &InputArgs) {
    let (attribution, share_alike) = input.attribution();
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
    fn credit_names_source_licence_and_distributor() {
        assert_eq!(
            license("Public Domain").credit(),
            "Src (Public Domain) via geoBoundaries"
        );
    }
}
