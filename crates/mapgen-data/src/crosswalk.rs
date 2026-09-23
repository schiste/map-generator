//! Readable ids for layers that lack them (`mapgen convert --ids-from`).
//!
//! geoBoundaries often leaves `shapeISO` empty below the first level, so ids
//! are opaque hashes. A *crosswalk* borrows codes from a reference layer that
//! has them (Natural Earth's `iso_3166_2`, Census FIPS codes…) by spatial
//! overlap: a target region takes the code of the reference unit that covers
//! most of it, provided they are the same place — each must cover at least
//! `min_share` of the other.
//!
//! Reference features are grouped by code first, so grouping Natural Earth's
//! French départements by `region_cod` yields the 13 régions.

use std::collections::BTreeMap;
use std::path::Path;

use geo::{Area, BooleanOps, BoundingRect};
use geo_types::MultiPolygon;
use rstar::primitives::{GeomWithData, Rectangle};
use rstar::{RTree, AABB};

use crate::error::Result;
use crate::layer::{LayerQuery, Source};

/// One reference unit: all reference features sharing a code, unioned.
#[derive(Debug, Clone)]
pub struct Reference {
    pub code: String,
    pub parent: Option<String>,
    pub geometry: MultiPolygon<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub code: String,
    pub parent: Option<String>,
    /// Share of the target covered by the reference unit.
    pub share: f64,
}

/// How to read codes from a reference GeoJSON.
#[derive(Debug, Clone)]
pub struct ReferenceSpec {
    /// Property holding the code, or `id` for the feature's top-level id.
    pub code_column: String,
    pub parent_column: Option<String>,
    /// Prepended to codes and parents (e.g. `US-` for FIPS codes).
    pub prefix: String,
}

/// Reads and groups a reference layer. Features without a code are skipped.
pub fn load_reference(path: &Path, spec: &ReferenceSpec) -> Result<Vec<Reference>> {
    let records = crate::geojson::read_records(path)?;
    let mut groups: BTreeMap<String, (Option<String>, Vec<MultiPolygon<f64>>)> = BTreeMap::new();
    for r in records {
        let code = if spec.code_column == "id" {
            r.prop("id").or_else(|| r.feature_id.clone())
        } else {
            r.prop(&spec.code_column)
        };
        let Some(code) = code else { continue };
        let parent = spec.parent_column.as_deref().and_then(|c| r.prop(c));
        let geom = crate::geometry::into_multipolygon(r.geometry);
        let entry = groups.entry(code).or_insert((parent, Vec::new()));
        entry.1.push(geom);
    }
    let prefixed = |s: String| format!("{}{s}", spec.prefix);
    Ok(groups
        .into_iter()
        .map(|(code, (parent, parts))| Reference {
            code: prefixed(code),
            parent: parent.map(prefixed),
            geometry: union_all(parts),
        })
        .collect())
}

fn union_all(mut parts: Vec<MultiPolygon<f64>>) -> MultiPolygon<f64> {
    match parts.len() {
        0 => MultiPolygon(vec![]),
        1 => parts.pop().expect("one part"),
        _ => parts
            .iter()
            .skip(1)
            .fold(parts[0].clone(), |acc, p| acc.union(p)),
    }
}

/// Matches each target geometry to a reference unit (`None` when no unit is
/// the same place). Deterministic: ties go to the lexicographically first code.
pub fn crosswalk(
    targets: &[MultiPolygon<f64>],
    refs: &[Reference],
    min_share: f64,
) -> Vec<Option<Match>> {
    let tree: RTree<GeomWithData<Rectangle<[f64; 2]>, usize>> = RTree::bulk_load(
        refs.iter()
            .enumerate()
            .filter_map(|(i, r)| r.geometry.bounding_rect().map(|b| (i, b)))
            .map(|(i, b)| {
                GeomWithData::new(
                    Rectangle::from_corners([b.min().x, b.min().y], [b.max().x, b.max().y]),
                    i,
                )
            })
            .collect(),
    );
    let ref_areas: Vec<f64> = refs.iter().map(|r| r.geometry.unsigned_area()).collect();
    targets
        .iter()
        .map(|t| {
            let b = t.bounding_rect()?;
            let area = t.unsigned_area();
            if area <= 0.0 {
                return None;
            }
            let env = AABB::from_corners([b.min().x, b.min().y], [b.max().x, b.max().y]);
            let mut best: Option<(f64, usize)> = None;
            for hit in tree.locate_in_envelope_intersecting(&env) {
                let i = hit.data;
                let overlap = t.intersection(&refs[i].geometry).unsigned_area();
                if overlap / area < min_share
                    || ref_areas[i] <= 0.0
                    || overlap / ref_areas[i] < min_share
                {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some((o, j)) => overlap > o || (overlap == o && refs[i].code < refs[j].code),
                };
                if better {
                    best = Some((overlap, i));
                }
            }
            best.map(|(overlap, i)| Match {
                code: refs[i].code.clone(),
                parent: refs[i].parent.clone(),
                share: overlap / area,
            })
        })
        .collect()
}

/// What a table lookup changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LookupStats {
    pub matched: usize,
    pub unmatched: Vec<String>,
    /// Keys that map to more than one value in the table (left unassigned).
    pub ambiguous: Vec<String>,
}

/// Sets `code` (and `parent`, if `parent_column` is given) on each record
/// whose `match_property` equals a key of the table. The issue's "crosswalk
/// CSV" for sources without standard codes.
pub fn apply_code_table(
    records: &mut [crate::geojson::Record],
    match_property: &str,
    table: &crate::table::Table,
    key_column: &str,
    code_column: &str,
    parent_column: Option<&str>,
) -> Result<LookupStats> {
    let (codes, ambiguous) = table.lookup(key_column, code_column)?;
    let parents = match parent_column {
        Some(p) => table.lookup(key_column, p)?.0,
        None => BTreeMap::new(),
    };
    let mut stats = LookupStats {
        ambiguous,
        ..LookupStats::default()
    };
    for (i, r) in records.iter_mut().enumerate() {
        let key = r.prop(match_property);
        match key.as_ref().and_then(|k| codes.get(k)) {
            Some(code) => {
                stats.matched += 1;
                r.properties.insert("code".into(), code.clone().into());
                if let Some(p) = key.as_ref().and_then(|k| parents.get(k)) {
                    r.properties.insert("parent".into(), p.clone().into());
                }
            }
            None => stats.unmatched.push(key.unwrap_or_else(|| format!("#{i}"))),
        }
    }
    Ok(stats)
}

/// Sets `parent_name` on each record whose `parent` equals a key of the
/// table, optionally with `prefix` in front (`US-` + `31` = `US-31`).
pub fn apply_parent_names(
    records: &mut [crate::geojson::Record],
    table: &crate::table::Table,
    key_column: &str,
    name_column: &str,
    prefix: &str,
) -> Result<LookupStats> {
    let (names, ambiguous) = table.lookup(key_column, name_column)?;
    let by_parent: BTreeMap<String, &String> = names
        .iter()
        .flat_map(|(k, v)| [(k.clone(), v), (format!("{prefix}{k}"), v)])
        .collect();
    let mut stats = LookupStats {
        ambiguous,
        ..LookupStats::default()
    };
    for r in records.iter_mut() {
        let Some(parent) = r.prop("parent") else {
            continue;
        };
        match by_parent.get(&parent) {
            Some(name) => {
                stats.matched += 1;
                r.properties
                    .insert("parent_name".into(), (*name).clone().into());
            }
            None => {
                if !stats.unmatched.contains(&parent) {
                    stats.unmatched.push(parent);
                }
            }
        }
    }
    Ok(stats)
}

/// Reference presets for common crosswalks.
pub fn natural_earth_admin1_iso() -> ReferenceSpec {
    let q: LayerQuery = Source::NaturalEarthAdmin1.layer_query();
    ReferenceSpec {
        code_column: q.id_columns[0].clone(),
        parent_column: q.parent_column,
        prefix: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::{coord, Rect};

    fn sq(x: f64, y: f64, s: f64) -> MultiPolygon<f64> {
        MultiPolygon(vec![Rect::new(
            coord! { x: x, y: y },
            coord! { x: x + s, y: y + s },
        )
        .to_polygon()])
    }

    fn reference(code: &str, g: MultiPolygon<f64>) -> Reference {
        Reference {
            code: code.into(),
            parent: Some("P".into()),
            geometry: g,
        }
    }

    #[test]
    fn matches_same_place_despite_small_differences() {
        // Targets drawn slightly differently from the reference (different source).
        let refs = [
            reference("FR-75", sq(0.0, 0.0, 1.0)),
            reference("FR-92", sq(1.0, 0.0, 1.0)),
        ];
        let targets = [sq(0.02, 0.0, 0.97), sq(1.01, 0.01, 1.0)];
        let m = crosswalk(&targets, &refs, 0.5);
        assert_eq!(m[0].as_ref().unwrap().code, "FR-75");
        assert_eq!(m[1].as_ref().unwrap().code, "FR-92");
        assert_eq!(m[1].as_ref().unwrap().parent.as_deref(), Some("P"));
    }

    #[test]
    fn rejects_containment_that_is_not_the_same_unit() {
        // A region containing four reference units is none of them.
        let refs = [
            reference("a", sq(0.0, 0.0, 1.0)),
            reference("b", sq(1.0, 0.0, 1.0)),
            reference("c", sq(0.0, 1.0, 1.0)),
            reference("d", sq(1.0, 1.0, 1.0)),
        ];
        assert_eq!(crosswalk(&[sq(0.0, 0.0, 2.0)], &refs, 0.5), vec![None]);
    }

    fn record(props: &[(&str, &str)]) -> crate::geojson::Record {
        crate::geojson::Record {
            feature_id: None,
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), (*v).into()))
                .collect(),
            geometry: geo_types::Geometry::MultiPolygon(MultiPolygon(vec![])),
        }
    }

    #[test]
    fn code_tables_assign_codes_and_skip_ambiguous_keys() {
        let table = crate::table::parse_table(
            "name,fips,state\nAda,16001,16\nLancaster,31109,31\nLancaster,42071,42\n",
        )
        .unwrap();
        let mut recs = vec![
            record(&[("shapeName", "Ada")]),
            record(&[("shapeName", "Lancaster")]),
        ];
        let s = apply_code_table(
            &mut recs,
            "shapeName",
            &table,
            "name",
            "fips",
            Some("state"),
        )
        .unwrap();
        assert_eq!(s.matched, 1);
        assert_eq!(s.ambiguous, ["Lancaster"]);
        assert_eq!(recs[0].prop("code").as_deref(), Some("16001"));
        assert_eq!(recs[0].prop("parent").as_deref(), Some("16"));
        assert_eq!(recs[1].prop("code"), None);
    }

    #[test]
    fn parent_names_match_prefixed_codes() {
        let table = crate::table::parse_table("STATE|STATE_NAME\n31|Nebraska\n").unwrap();
        let mut recs = vec![
            record(&[("parent", "US-31")]),
            record(&[("parent", "US-99")]),
        ];
        let s = apply_parent_names(&mut recs, &table, "STATE", "STATE_NAME", "US-").unwrap();
        assert_eq!(s.matched, 1);
        assert_eq!(s.unmatched, ["US-99"]);
        assert_eq!(recs[0].prop("parent_name").as_deref(), Some("Nebraska"));
    }

    #[test]
    fn grouped_references_match_parent_level_units() {
        // Two départements of the same région, unioned, match the région.
        let unioned = union_all(vec![sq(0.0, 0.0, 1.0), sq(1.0, 0.0, 1.0)]);
        let refs = [reference("FR-IDF", unioned)];
        let m = crosswalk(&[sq(0.0, 0.0, 2.0).clone()], &refs, 0.5);
        assert!(m[0]
            .as_ref()
            .is_some_and(|m| m.code == "FR-IDF" && (m.share - 0.5).abs() < 1e-9));
    }
}
