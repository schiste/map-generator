//! `mapgen match`, `mapgen crosswalk` and `mapgen reshape`: checking that
//! data fits a map, and moving data between boundary versions.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use mapgen_data::join::{
    crosswalk_rows, csv_cell, match_codes, overlap_crosswalk, reshape_table, svg_codes,
    table_codes, CrosswalkRow,
};
use mapgen_data::read_layer;
use mapgen_data::table::read_table;

use crate::InputArgs;

#[derive(clap::Args)]
pub struct MatchArgs {
    /// Data table (CSV, TSV or pipe-separated) with one row per code.
    #[arg(long)]
    data: PathBuf,
    /// Column of the data table holding region (or data unit) codes.
    #[arg(long)]
    code_column: String,
    /// Prefix added to the data's codes, e.g. `US-` for bare FIPS codes.
    #[arg(long, default_value = "")]
    code_prefix: String,
    /// Map to match against: a rendered SVG (its `data-code` and `data-unit`
    /// attributes) or a layer (GeoPackage or GeoJSON, read with the dataset
    /// options below).
    #[arg(long)]
    map: PathBuf,
    #[command(flatten)]
    layer: InputArgs,
    /// Region of the layer to match against (default: the whole layer).
    #[arg(long)]
    region: Option<String>,
    /// Fail (status 1) when more than this share of the data's codes are
    /// not on the map, e.g. 0.01 for 1 %.
    #[arg(long)]
    max_missing: Option<f64>,
    /// Machine-readable output.
    #[arg(long)]
    json: bool,
}

pub fn run_match(args: MatchArgs) -> Result<()> {
    let table = read_table(&args.data)?;
    let data = table_codes(&table, &args.code_column, &args.code_prefix)?;
    let (codes, units) = if args
        .map
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        let svg = std::fs::read_to_string(&args.map)
            .with_context(|| format!("reading {}", args.map.display()))?;
        svg_codes(&svg)
    } else {
        let features = read_layer(&args.map, &args.layer.query(), args.region.as_deref())
            .with_context(|| format!("reading {}", args.map.display()))?;
        let units = features
            .iter()
            .filter(|f| !f.units.is_empty())
            .map(|f| (f.id.clone(), f.units.clone()))
            .collect();
        (features.into_iter().map(|f| f.id).collect(), units)
    };
    if codes.is_empty() {
        bail!("no region codes found in {}", args.map.display());
    }
    let report = match_codes(&codes, &units, &data);
    let share = report.missing_share();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "matched": report.matched,
                "dataNotOnMap": report.data_not_on_map,
                "mapWithoutData": report.map_without_data,
                "missingShare": share,
            }))?
        );
    } else {
        for c in &report.data_not_on_map {
            println!("data-not-on-map\t{c}");
        }
        for c in &report.map_without_data {
            println!("map-without-data\t{c}");
        }
        eprintln!(
            "{} code(s) matched; {} in the data but not on the map ({:.2} %), {} region(s) without data",
            report.matched,
            report.data_not_on_map.len(),
            share * 100.0,
            report.map_without_data.len()
        );
        if !report.data_not_on_map.is_empty() {
            eprintln!(
                "codes missing from the map usually mean the data and the boundaries are from \
                 different years: see `mapgen reshape`"
            );
        }
    }
    if args.max_missing.is_some_and(|max| share > max) {
        std::process::exit(1);
    }
    Ok(())
}

#[derive(clap::Args)]
pub struct CrosswalkArgs {
    /// Layer with the old boundaries.
    #[arg(long)]
    from: PathBuf,
    /// Layer with the new boundaries (read with the same dataset options).
    #[arg(long)]
    to: PathBuf,
    #[command(flatten)]
    layer: InputArgs,
    /// Region of both layers to cover (default: the whole layers).
    #[arg(long)]
    region: Option<String>,
    /// Overlaps covering less than this share of an old region are treated
    /// as slivers from boundaries drawn slightly differently, and dropped.
    #[arg(long, default_value_t = 0.01)]
    min_share: f64,
    /// Output table (CSV) with `from`, `to` and `weight` columns.
    #[arg(short, long)]
    out: PathBuf,
}

pub fn run_crosswalk(args: CrosswalkArgs) -> Result<()> {
    let query = args.layer.query();
    let read = |p: &Path| {
        read_layer(p, &query, args.region.as_deref())
            .with_context(|| format!("reading {}", p.display()))
    };
    let (from, to) = (read(&args.from)?, read(&args.to)?);
    let rows = overlap_crosswalk(&from, &to, args.min_share);
    let mut out = String::from("from,to,weight\n");
    for r in &rows {
        out.push_str(&format!(
            "{},{},{}\n",
            csv_cell(&r.from),
            csv_cell(&r.to),
            r.weight.unwrap_or(1.0)
        ));
    }
    std::fs::write(&args.out, out)?;
    let split = rows.windows(2).filter(|w| w[0].from == w[1].from).count();
    eprintln!(
        "wrote {} ({} row(s); {} old region(s), {} with more than one successor)",
        args.out.display(),
        rows.len(),
        from.len(),
        split
    );
    eprintln!(
        "weights are area shares: for counts of people or businesses, a published \
         relationship file (e.g. Census) or population weights are more accurate"
    );
    Ok(())
}

#[derive(clap::Args)]
pub struct ReshapeArgs {
    /// Data table on the old codes.
    #[arg(long)]
    data: PathBuf,
    /// Column of the data table holding the old codes.
    #[arg(long)]
    code_column: String,
    /// Columns to carry over (default: every other column that is numeric).
    /// Values are added up and shared out, so use counts, not rates or
    /// medians.
    #[arg(long, value_delimiter = ',')]
    columns: Option<Vec<String>>,
    /// Crosswalk table: old code, new code and optional weight per row
    /// (from `mapgen crosswalk`, or a published relationship file).
    #[arg(long)]
    crosswalk: PathBuf,
    #[arg(long, default_value = "from")]
    from_column: String,
    #[arg(long, default_value = "to")]
    to_column: String,
    /// Crosswalk column holding the share of the old region's value that
    /// goes to the new one.
    #[arg(long)]
    weight_column: Option<String>,
    /// The crosswalk lists every code: codes it lacks are conflicts instead
    /// of being kept as they are.
    #[arg(long)]
    complete: bool,
    /// Output table on the new codes.
    #[arg(short, long)]
    out: PathBuf,
    /// Where to list values that need a decision (default: next to --out,
    /// with a `.conflicts.csv` extension).
    #[arg(long)]
    conflicts: Option<PathBuf>,
    /// Write the output even with conflicts, leaving those values out.
    #[arg(long)]
    allow_conflicts: bool,
}

pub fn run_reshape(args: ReshapeArgs) -> Result<()> {
    let table = read_table(&args.data)?;
    let cw_table = read_table(&args.crosswalk)?;
    let cws: Vec<CrosswalkRow> = crosswalk_rows(
        &cw_table,
        &args.from_column,
        &args.to_column,
        args.weight_column.as_deref(),
    )?;
    let reshaped = reshape_table(
        &table,
        &args.code_column,
        args.columns.as_deref(),
        &cws,
        args.complete,
    )
    .with_context(|| format!("reshaping {}", args.data.display()))?;
    let result = &reshaped.result;

    let conflicts_path = args
        .conflicts
        .clone()
        .unwrap_or_else(|| args.out.with_extension("conflicts.csv"));
    if !result.conflicts.is_empty() {
        std::fs::write(&conflicts_path, reshaped.conflicts_csv())?;
        eprintln!(
            "{} value(s) need a decision, listed in {}: add weights to the crosswalk \
             (e.g. from `mapgen crosswalk`) or targets for codes it lacks",
            result.conflicts.len(),
            conflicts_path.display()
        );
        if !args.allow_conflicts {
            bail!(
                "not writing {} (pass --allow-conflicts to leave those values out)",
                args.out.display()
            );
        }
    }
    std::fs::write(&args.out, reshaped.csv())?;
    eprintln!(
        "wrote {}: {} code(s) carried over or merged, {} shared out by weight, {} new code(s)",
        args.out.display(),
        result.direct,
        result.weighted,
        result.values.len()
    );
    Ok(())
}
