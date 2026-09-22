use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use mapgen_core::{render, RenderOptions, SvgOptions};
use mapgen_data::Source;

/// Deterministic SVG map generator.
#[derive(Parser)]
#[command(name = "mapgen", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render one region to an SVG file.
    Render(RenderArgs),
}

#[derive(Clone, Copy, ValueEnum)]
enum Dataset {
    /// GADM 4.1 levels GeoPackage.
    Gadm,
    /// Natural Earth 1:10m Admin-0 (countries).
    NeAdmin0,
    /// Natural Earth 1:10m Admin-1 (states/provinces).
    NeAdmin1,
}

#[derive(clap::Args)]
struct RenderArgs {
    /// GeoPackage to read from (see scripts/fetch-data.sh).
    #[arg(long, conflicts_with = "geojson", required_unless_present = "geojson")]
    gpkg: Option<PathBuf>,

    /// Layout of the GeoPackage passed with --gpkg.
    #[arg(long, value_enum, default_value = "gadm")]
    dataset: Dataset,

    /// Administrative level for GADM (0 = country, 1 = states, 2 = counties...).
    #[arg(long, default_value_t = 1)]
    level: u8,

    /// Region code to filter on, e.g. ISO 3166-1 alpha-3 `FRA`.
    #[arg(long)]
    region: Option<String>,

    /// GeoJSON FeatureCollection to read instead of a GeoPackage.
    #[arg(long)]
    geojson: Option<PathBuf>,

    /// Output SVG path.
    #[arg(short, long)]
    out: PathBuf,

    /// Output width in pixels.
    #[arg(long, default_value_t = 1000)]
    width: u32,

    /// Simplification tolerance in pixels (0 disables).
    #[arg(long, default_value_t = 0.5)]
    simplify: f64,

    /// Document title.
    #[arg(long)]
    title: Option<String>,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Render(args) => run_render(args),
    }
}

fn run_render(args: RenderArgs) -> Result<()> {
    let features = if let Some(path) = &args.geojson {
        mapgen_data::geojson::read_features(path, "id", "name", "subdivision")
            .with_context(|| format!("reading {}", path.display()))?
    } else if let Some(path) = &args.gpkg {
        let source = match args.dataset {
            Dataset::Gadm => Source::Gadm { level: args.level },
            Dataset::NeAdmin0 => Source::NaturalEarthAdmin0,
            Dataset::NeAdmin1 => Source::NaturalEarthAdmin1,
        };
        mapgen_data::gpkg::read_features(path, &source.layer_query(), args.region.as_deref())
            .with_context(|| format!("reading {}", path.display()))?
    } else {
        unreachable!("clap requires --gpkg or --geojson");
    };
    if features.is_empty() {
        bail!("no features matched region {:?}", args.region);
    }

    let opts = RenderOptions {
        svg: SvgOptions {
            width: args.width,
            title: args.title,
            ..SvgOptions::default()
        },
        simplify_px: args.simplify,
    };
    let svg = render(&features, &opts)?;
    std::fs::write(&args.out, &svg).with_context(|| format!("writing {}", args.out.display()))?;
    eprintln!(
        "wrote {} ({} features, {} bytes)",
        args.out.display(),
        features.len(),
        svg.len()
    );
    Ok(())
}
