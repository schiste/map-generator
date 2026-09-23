//! Joining data to maps, and moving data between boundary versions
//! (`mapgen match`, `mapgen crosswalk`, `mapgen reshape`).
//!
//! Boundaries change (counties split, merge, get new codes), and data keyed
//! to one year's boundaries silently misses regions on another year's map.
//! `match_codes` reports those mismatches; `reshape` moves values through a
//! crosswalk, applying renames and merges automatically and refusing splits
//! it has no rule for; `overlap_crosswalk` derives weights from geometry
//! when no published relationship table exists.

use std::collections::{BTreeMap, BTreeSet};

use geo::{Area, BooleanOps, BoundingRect};
use mapgen_core::MapFeature;
use rstar::primitives::{GeomWithData, Rectangle};
use rstar::{RTree, AABB};

use crate::table::Table;

/// Codes that appear on one side only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatchReport {
    pub matched: usize,
    /// In the data, not on the map: a missing region, or a boundary change.
    pub data_not_on_map: Vec<String>,
    /// On the map, without data.
    pub map_without_data: Vec<String>,
}

impl MatchReport {
    /// Share of data codes that have no region on the map.
    pub fn missing_share(&self) -> f64 {
        let total = self.matched + self.data_not_on_map.len();
        if total == 0 {
            0.0
        } else {
            self.data_not_on_map.len() as f64 / total as f64
        }
    }
}

/// Compares the codes of a data table with those of a map. `map_units` maps
/// region codes to the data units they belong to: data keyed by unit counts
/// as present when the map has a region in that unit.
pub fn match_codes(
    map_codes: &BTreeSet<String>,
    map_units: &BTreeMap<String, Vec<String>>,
    data_codes: &BTreeSet<String>,
) -> MatchReport {
    let unit_codes: BTreeSet<&String> = map_units.values().flatten().collect();
    let mut report = MatchReport::default();
    for code in data_codes {
        if map_codes.contains(code) || unit_codes.contains(code) {
            report.matched += 1;
        } else {
            report.data_not_on_map.push(code.clone());
        }
    }
    report.map_without_data = map_codes
        .iter()
        .filter(|c| {
            !data_codes.contains(*c)
                && !map_units
                    .get(*c)
                    .is_some_and(|us| us.iter().any(|u| data_codes.contains(u)))
        })
        .cloned()
        .collect();
    report
}

/// Codes and data units of a rendered map, read from its paths'
/// `data-code` and `data-unit` attributes.
pub fn svg_codes(svg: &str) -> (BTreeSet<String>, BTreeMap<String, Vec<String>>) {
    let attr = |tag: &str, name: &str| -> Option<String> {
        let key = format!(" {name}=\"");
        let start = tag.find(&key)? + key.len();
        let end = tag[start..].find('"')? + start;
        Some(unescape(&tag[start..end]))
    };
    let mut codes = BTreeSet::new();
    let mut units = BTreeMap::new();
    for tag in svg
        .split("<path")
        .skip(1)
        .map(|t| t.split('>').next().unwrap_or(t))
    {
        // Only regions of the mapped area, not neighbours, lakes or borders.
        if !attr(tag, "class").is_some_and(|c| c.split_whitespace().any(|t| t == "mg-land")) {
            continue;
        }
        let Some(code) = attr(tag, "data-code") else {
            continue;
        };
        if let Some(u) = attr(tag, "data-unit") {
            units.insert(
                code.clone(),
                u.split_whitespace().map(str::to_owned).collect(),
            );
        }
        codes.insert(code);
    }
    (codes, units)
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// One row of a version crosswalk: `weight` is the share of the `from`
/// region's value that goes to `to` (1 when omitted).
#[derive(Debug, Clone, PartialEq)]
pub struct CrosswalkRow {
    pub from: String,
    pub to: String,
    pub weight: Option<f64>,
}

/// A value that could not be carried over without a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub from: String,
    pub targets: Vec<String>,
    pub reason: &'static str,
}

/// Values per target code, and what needs a decision.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reshaped {
    /// `target code → value per column`, sorted by code.
    pub values: BTreeMap<String, Vec<f64>>,
    pub conflicts: Vec<Conflict>,
    /// Number of source codes carried over 1:1 or merged.
    pub direct: usize,
    /// Number of source codes shared out by weight.
    pub weighted: usize,
}

/// Moves numeric values from source codes to target codes.
///
/// - one target: carried over (a rename), and several sources with the same
///   target are summed (a merge);
/// - several targets with weights summing to ~1: shared out by weight
///   (a split or redrawn boundaries with a rule);
/// - several targets without weights, or weights that don't sum to 1: a
///   conflict, and the value is not carried over;
/// - not in the crosswalk: kept under the same code, since published
///   change lists (e.g. Census substantial changes) only list what changed;
///   with `complete`, the crosswalk must list every code and this is a
///   conflict too.
pub fn reshape(
    data: &[(String, Vec<f64>)],
    crosswalk: &[CrosswalkRow],
    complete: bool,
) -> Reshaped {
    let mut by_from: BTreeMap<&str, Vec<&CrosswalkRow>> = BTreeMap::new();
    for r in crosswalk {
        by_from.entry(r.from.as_str()).or_default().push(r);
    }
    let mut out = Reshaped::default();
    for (from, values) in data {
        let add = |out: &mut Reshaped, to: &str, share: f64| {
            let acc = out
                .values
                .entry(to.to_owned())
                .or_insert_with(|| vec![0.0; values.len()]);
            for (a, v) in acc.iter_mut().zip(values) {
                *a += v * share;
            }
        };
        let targets = by_from
            .get(from.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let names = || targets.iter().map(|r| r.to.clone()).collect::<Vec<_>>();
        match targets {
            [] if !complete => {
                add(&mut out, from, 1.0);
                out.direct += 1;
            }
            [] => out.conflicts.push(Conflict {
                from: from.clone(),
                targets: vec![],
                reason: "no target in the crosswalk",
            }),
            [one] if one.weight.is_none_or(|w| (w - 1.0).abs() < 1e-6) => {
                add(&mut out, &one.to, 1.0);
                out.direct += 1;
            }
            many => {
                let weights: Option<Vec<f64>> = many.iter().map(|r| r.weight).collect();
                match weights {
                    None => out.conflicts.push(Conflict {
                        from: from.clone(),
                        targets: names(),
                        reason: "split without weights: choose a rule (area, population…)",
                    }),
                    Some(w) if (w.iter().sum::<f64>() - 1.0).abs() > 1e-3 => {
                        out.conflicts.push(Conflict {
                            from: from.clone(),
                            targets: names(),
                            reason: "weights do not sum to 1",
                        })
                    }
                    Some(w) => {
                        for (r, share) in many.iter().zip(w) {
                            add(&mut out, &r.to, share);
                        }
                        out.weighted += 1;
                    }
                }
            }
        }
    }
    out
}

/// Crosswalk from one set of regions to another by area overlap: each
/// `from` region is shared among the `to` regions it overlaps, weighted by
/// the share of its area in each. Overlaps under `min_share` (slivers from
/// boundaries drawn slightly differently) are dropped and the rest
/// renormalised.
pub fn overlap_crosswalk(
    from: &[MapFeature],
    to: &[MapFeature],
    min_share: f64,
) -> Vec<CrosswalkRow> {
    let tree: RTree<GeomWithData<Rectangle<[f64; 2]>, usize>> = RTree::bulk_load(
        to.iter()
            .enumerate()
            .filter_map(|(i, f)| f.geometry.bounding_rect().map(|b| (i, b)))
            .map(|(i, b)| {
                GeomWithData::new(
                    Rectangle::from_corners([b.min().x, b.min().y], [b.max().x, b.max().y]),
                    i,
                )
            })
            .collect(),
    );
    let mut rows = Vec::new();
    for f in from {
        let (Some(b), area) = (f.geometry.bounding_rect(), f.geometry.unsigned_area()) else {
            continue;
        };
        if area <= 0.0 {
            continue;
        }
        let env = AABB::from_corners([b.min().x, b.min().y], [b.max().x, b.max().y]);
        let mut hits: Vec<usize> = tree
            .locate_in_envelope_intersecting(&env)
            .map(|h| h.data)
            .collect();
        hits.sort_unstable();
        let shares: Vec<(usize, f64)> = hits
            .into_iter()
            .map(|j| {
                (
                    j,
                    f.geometry.intersection(&to[j].geometry).unsigned_area() / area,
                )
            })
            .filter(|(_, s)| *s >= min_share)
            .collect();
        let total: f64 = shares.iter().map(|(_, s)| s).sum();
        for (j, s) in shares {
            let weight = (s / total * 1e6).round() / 1e6;
            rows.push(CrosswalkRow {
                from: f.id.clone(),
                to: to[j].id.clone(),
                weight: Some(weight),
            });
        }
    }
    rows
}

/// Reads crosswalk rows from a table (weight column optional).
pub fn crosswalk_rows(
    table: &Table,
    from_column: &str,
    to_column: &str,
    weight_column: Option<&str>,
) -> crate::error::Result<Vec<CrosswalkRow>> {
    let (f, t) = (table.column(from_column)?, table.column(to_column)?);
    let w = weight_column.map(|c| table.column(c)).transpose()?;
    let mut rows = Vec::new();
    for r in &table.rows {
        let (Some(from), Some(to)) = (table.cell(r, f), table.cell(r, t)) else {
            continue;
        };
        let weight = match w.and_then(|w| table.cell(r, w)) {
            Some(v) => Some(v.parse::<f64>().map_err(|_| {
                crate::error::Error::Table(format!(
                    "weight {v:?} for {from} → {to} is not a number"
                ))
            })?),
            None => None,
        };
        rows.push(CrosswalkRow { from, to, weight });
    }
    Ok(rows)
}

/// A data table moved to new codes by [`reshape_table`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReshapedTable {
    pub code_column: String,
    /// The value columns carried over.
    pub columns: Vec<String>,
    pub result: Reshaped,
}

impl ReshapedTable {
    /// The table on the new codes, as CSV (values rounded to 1e-9).
    pub fn csv(&self) -> String {
        let mut out = csv_cell(&self.code_column);
        for c in &self.columns {
            out.push(',');
            out.push_str(&csv_cell(c));
        }
        out.push('\n');
        for (key, values) in &self.result.values {
            out.push_str(&csv_cell(key));
            for v in values {
                out.push_str(&format!(",{}", (v * 1e9).round() / 1e9));
            }
            out.push('\n');
        }
        out
    }

    /// The values that need a decision, as CSV (`from,targets,reason`).
    pub fn conflicts_csv(&self) -> String {
        let mut text = String::from("from,targets,reason\n");
        for c in &self.result.conflicts {
            text.push_str(&format!(
                "{},{},{}\n",
                csv_cell(&c.from),
                csv_cell(&c.targets.join(" ")),
                csv_cell(c.reason)
            ));
        }
        text
    }
}

/// Moves the numeric `columns` of `table` (default: every other column
/// whose non-empty cells are all numbers) from the codes in `code_column`
/// to new codes through `crosswalk` (see [`reshape`]).
pub fn reshape_table(
    table: &Table,
    code_column: &str,
    columns: Option<&[String]>,
    crosswalk: &[CrosswalkRow],
    complete: bool,
) -> crate::error::Result<ReshapedTable> {
    let code = table.column(code_column)?;
    let columns = match columns {
        Some(cs) => cs.to_vec(),
        None => numeric_columns(table, code),
    };
    if columns.is_empty() {
        return Err(crate::error::Error::Table(
            "no numeric columns to reshape: name them".into(),
        ));
    }
    let idx = columns
        .iter()
        .map(|c| table.column(c))
        .collect::<crate::error::Result<Vec<_>>>()?;
    let mut data = Vec::new();
    for r in &table.rows {
        let Some(key) = table.cell(r, code) else {
            continue;
        };
        let mut values = Vec::with_capacity(idx.len());
        for (&i, name) in idx.iter().zip(&columns) {
            let cell = table.cell(r, i).unwrap_or_default();
            values.push(cell.parse::<f64>().map_err(|_| {
                crate::error::Error::Table(format!("{key}: {name} is {cell:?}, not a number"))
            })?);
        }
        data.push((key, values));
    }
    Ok(ReshapedTable {
        code_column: code_column.to_owned(),
        result: reshape(&data, crosswalk, complete),
        columns,
    })
}

/// Columns other than `skip` whose non-empty cells all parse as numbers.
pub fn numeric_columns(table: &Table, skip: usize) -> Vec<String> {
    table
        .header
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != skip)
        .filter(|(i, _)| {
            let mut cells = table
                .rows
                .iter()
                .filter_map(|r| table.cell(r, *i))
                .peekable();
            cells.peek().is_some() && cells.all(|c| c.parse::<f64>().is_ok())
        })
        .map(|(_, h)| h.clone())
        .collect()
}

/// Codes of a data table's column, with an optional prefix added.
pub fn table_codes(
    table: &Table,
    code_column: &str,
    prefix: &str,
) -> crate::error::Result<BTreeSet<String>> {
    let col = table.column(code_column)?;
    Ok(table
        .rows
        .iter()
        .filter_map(|r| table.cell(r, col))
        .map(|c| format!("{prefix}{c}"))
        .collect())
}

/// Quotes a CSV cell when needed.
pub fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::{coord, MultiPolygon, Rect};

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn cw(from: &str, to: &str, w: Option<f64>) -> CrosswalkRow {
        CrosswalkRow {
            from: from.into(),
            to: to.into(),
            weight: w,
        }
    }

    #[test]
    fn match_reports_both_sides_and_accepts_unit_codes() {
        // 2018 map with Valdez-Cordova (02261); data already on 2019 codes.
        let map = set(&["02261", "02020", "36005", "36047"]);
        let units = BTreeMap::from([
            ("36005".to_string(), vec!["NYC".to_string()]),
            ("36047".to_string(), vec!["NYC".to_string()]),
        ]);
        let data = set(&["02063", "02066", "02020", "NYC"]);
        let r = match_codes(&map, &units, &data);
        assert_eq!(r.matched, 2);
        assert_eq!(r.data_not_on_map, ["02063", "02066"]);
        assert_eq!(r.map_without_data, ["02261"]);
        assert!((r.missing_share() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn reads_codes_and_units_from_rendered_svg() {
        let svg = r#"<g id="context"><path id="CAN" class="mg-context country ca" data-code="CAN" d="M0 0Z"/></g>
<path id="US-36005" class="mg-land subdivision us" data-name="Bronx" data-code="US-36005" data-unit="NYC" d="M0 0Z"><title>Bronx</title></path>
<path id="A_B" class="mg-land region" data-name="x" data-code="A&amp;B" d="M0 0Z"/>"#;
        let (codes, units) = svg_codes(svg);
        assert_eq!(codes, set(&["A&B", "US-36005"]));
        assert_eq!(units["US-36005"], ["NYC"]);
    }

    #[test]
    fn reshape_renames_merges_splits_and_reports_conflicts() {
        let data = vec![
            ("OLD-A".to_string(), vec![10.0]), // renamed
            ("OLD-B".to_string(), vec![1.0]),  // merged with C
            ("OLD-C".to_string(), vec![2.0]),
            ("OLD-D".to_string(), vec![100.0]), // split with weights
            ("OLD-E".to_string(), vec![5.0]),   // split without weights
            ("OLD-F".to_string(), vec![7.0]),   // not in the crosswalk
        ];
        let cws = vec![
            cw("OLD-A", "NEW-A", None),
            cw("OLD-B", "NEW-BC", None),
            cw("OLD-C", "NEW-BC", Some(1.0)),
            cw("OLD-D", "NEW-D1", Some(0.25)),
            cw("OLD-D", "NEW-D2", Some(0.75)),
            cw("OLD-E", "NEW-E1", None),
            cw("OLD-E", "NEW-E2", None),
        ];
        let r = reshape(&data, &cws, true);
        assert_eq!(r.values["NEW-A"], [10.0]);
        assert_eq!(r.values["NEW-BC"], [3.0]);
        assert_eq!((r.values["NEW-D1"][0], r.values["NEW-D2"][0]), (25.0, 75.0));
        assert!(!r.values.contains_key("NEW-E1"));
        assert_eq!((r.direct, r.weighted), (3, 1));
        let reasons: Vec<(&str, &str)> = r
            .conflicts
            .iter()
            .map(|c| (c.from.as_str(), c.reason))
            .collect();
        assert_eq!(reasons.len(), 2);
        assert!(reasons[0].0 == "OLD-E" && reasons[0].1.starts_with("split without weights"));
        assert_eq!(reasons[1], ("OLD-F", "no target in the crosswalk"));

        // A change list: unlisted codes keep their code.
        let r = reshape(&data, &cws, false);
        assert_eq!(r.values["OLD-F"], [7.0]);
        assert_eq!((r.direct, r.conflicts.len()), (4, 1));
    }

    #[test]
    fn reshapes_tables_to_csv() {
        let table =
            crate::table::parse_table("fips,pop,name\n02261,100,VC\n02020,5,Anc\n").unwrap();
        let cws = vec![
            cw("02261", "02063", Some(0.4)),
            cw("02261", "02066", Some(0.6)),
        ];
        let t = reshape_table(&table, "fips", None, &cws, false).unwrap();
        assert_eq!(t.columns, ["pop"]);
        assert_eq!(t.csv(), "fips,pop\n02020,5\n02063,40\n02066,60\n");
        assert_eq!(t.conflicts_csv(), "from,targets,reason\n");
        let e = reshape_table(&table, "fips", Some(&["name".into()]), &cws, false).unwrap_err();
        assert!(e.to_string().contains("not a number"), "{e}");
    }

    #[test]
    fn overlap_crosswalk_weights_by_area() {
        let sq = |id: &str, x0: f64, x1: f64| MapFeature {
            id: id.into(),
            geometry: MultiPolygon(vec![Rect::new(
                coord! { x: x0, y: 0.0 },
                coord! { x: x1, y: 1.0 },
            )
            .to_polygon()]),
            ..MapFeature::default()
        };
        // Old county [0,4] split into [0,1] and [1,4]; a sliver of [4,4.001] is ignored.
        let from = vec![sq("OLD", 0.0, 4.0)];
        let to = vec![
            sq("N1", 0.0, 1.0),
            sq("N2", 1.0, 4.0),
            sq("N3", 3.999, 10.0),
        ];
        let rows = overlap_crosswalk(&from, &to, 0.01);
        assert_eq!(
            rows,
            vec![cw("OLD", "N1", Some(0.25)), cw("OLD", "N2", Some(0.75))]
        );
    }
}
