use std::collections::BTreeSet;
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

    /// Credits for a data file and the context layers, and whether any
    /// licence is share-alike.
    fn attribution(&self, data: &Path) -> (Option<String>, bool) {
        if let Some(a) = &self.attribution {
            return (Some(a.clone()), false);
        }
        let mut credits: Vec<String> = Vec::new();
        let mut share_alike = false;
        let inputs = [Some(data), self.context.as_deref(), self.lakes.as_deref()];
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

    let subject = read_layer(&args.input, &query, region.as_deref())
        .with_context(|| format!("reading {}", args.input.display()))?;
    if subject.is_empty() {
        bail!(
            "no features matched region {:?}",
            region.unwrap_or_default()
        );
    }
    let (context, lakes) = args.data.load_context()?;

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
        context,
        lakes,
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
        "wrote {} ({}×{} px, {} regions, {} bytes, {:?})",
        args.out.display(),
        rendered.width,
        rendered.height,
        layers.subject.len() - rendered.outside_frame.len(),
        body.len(),
        rendered.projection,
    );
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

/// Shared, read-only state for a batch run.
struct Batch<'a> {
    args: &'a BatchArgs,
    query: LayerQuery,
    context: Vec<MapFeature>,
    lakes: Vec<MapFeature>,
    wanted: Option<BTreeSet<String>>,
    multi: bool,
    html: bool,
}

type JobResult = (String, Result<()>);

fn run_batch(args: BatchArgs) -> Result<()> {
    let files = expand_inputs(&args.input)?;
    if files.is_empty() {
        bail!("no .geojson, .json or .gpkg files found in the given inputs");
    }
    let (context, lakes) = args.data.load_context()?;
    std::fs::create_dir_all(&args.out_dir)?;
    let batch = Batch {
        query: args.data.query(),
        context,
        lakes,
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
            Ok(()) => ok += 1,
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
            let job = || -> Result<()> {
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
        subject: Vec<MapFeature>,
        out_stem: &str,
    ) -> Result<()> {
        if subject.is_empty() {
            bail!("no features");
        }
        let args = self.args;
        let mut layers = MapLayers {
            context: self.context.clone(),
            lakes: self.lakes.clone(),
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
        Ok(())
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
