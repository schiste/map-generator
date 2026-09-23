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
