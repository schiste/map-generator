//! Data units made of several map regions (issue #7).
//!
//! Data is often reported for units that aren't the map's regions: New York
//! City for its five boroughs, Utah's health districts, "Dukes and
//! Nantucket". A *unit table* (`map_id → data unit`) links the two. Regions
//! are tagged with their unit(s) (`data-unit`), or dissolved into one shape
//! per unit: a unit becomes a single feature, so the borders between its
//! regions are internal and never drawn.

use std::collections::{BTreeMap, BTreeSet};

use geo_types::MultiPolygon;

use crate::feature::MapFeature;
use crate::simplify::Topology;

/// One row of a unit table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitRow {
    pub region: String,
    pub unit: String,
    pub unit_name: Option<String>,
}

/// What didn't fit cleanly, for warnings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitReport {
    /// Regions named in the table but not on the map.
    pub unknown_regions: Vec<String>,
    /// Regions in more than one unit: the units don't nest (e.g. Kansas City
    /// cutting across four counties). When dissolving, such a region goes to
    /// its first unit.
    pub overlapping: Vec<(String, Vec<String>)>,
    /// Units whose regions are not all connected by shared borders, with the
    /// number of separate parts (islands count as parts too).
    pub split_units: Vec<(String, usize)>,
    /// Map regions in no unit.
    pub unassigned: Vec<String>,
}

/// Units of each region, in table order.
fn assignments(rows: &[UnitRow]) -> BTreeMap<&str, Vec<&str>> {
    let mut by_region: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for r in rows {
        let units = by_region.entry(r.region.as_str()).or_default();
        if !units.contains(&r.unit.as_str()) {
            units.push(r.unit.as_str());
        }
    }
    by_region
}

/// Checks a unit table against the map's regions.
pub fn check_units(features: &[MapFeature], rows: &[UnitRow]) -> UnitReport {
    let by_region = assignments(rows);
    let on_map: BTreeSet<&str> = features.iter().map(|f| f.id.as_str()).collect();
    let mut report = UnitReport {
        unknown_regions: by_region
            .keys()
            .filter(|r| !on_map.contains(*r))
            .map(|r| r.to_string())
            .collect(),
        overlapping: by_region
            .iter()
            .filter(|(r, u)| u.len() > 1 && on_map.contains(*r))
            .map(|(r, u)| (r.to_string(), u.iter().map(|s| s.to_string()).collect()))
            .collect(),
        unassigned: features
            .iter()
            .filter(|f| !by_region.contains_key(f.id.as_str()))
            .map(|f| f.id.clone())
            .collect(),
        split_units: Vec::new(),
    };

    // Contiguity: regions (and their separate parts) linked by shared borders.
    let parts: Vec<(usize, MultiPolygon<f64>)> = features
        .iter()
        .enumerate()
        .flat_map(|(i, f)| {
            f.geometry
                .0
                .iter()
                .map(move |p| (i, MultiPolygon(vec![p.clone()])))
        })
        .collect();
    let geoms: Vec<MultiPolygon<f64>> = parts.iter().map(|(_, g)| g.clone()).collect();
    let (_, arcs) = Topology::build(&geoms).finish(0.0, |_| true);
    let mut parent: Vec<usize> = (0..parts.len()).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let unit_of = |part: usize| {
        by_region
            .get(features[parts[part].0].id.as_str())
            .and_then(|u| u.first().copied())
    };
    for arc in arcs {
        let Some(b) = arc.b else { continue };
        if unit_of(arc.a).is_some() && unit_of(arc.a) == unit_of(b) {
            let (x, y) = (find(&mut parent, arc.a), find(&mut parent, b));
            parent[x.max(y)] = x.min(y);
        }
    }
    let mut components: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for part in 0..parts.len() {
        if let Some(u) = unit_of(part) {
            let root = find(&mut parent, part);
            components.entry(u).or_default().insert(root);
        }
    }
    report.split_units = components
        .into_iter()
        .filter(|(_, c)| c.len() > 1)
        .map(|(u, c)| (u.to_string(), c.len()))
        .collect();
    report
}

/// Tags each feature with its data unit(s).
pub fn tag_units(features: &mut [MapFeature], rows: &[UnitRow]) {
    let by_region = assignments(rows);
    for f in features {
        if let Some(units) = by_region.get(f.id.as_str()) {
            f.units = units.iter().map(|u| u.to_string()).collect();
        }
    }
}

/// Merges the regions of each unit into one feature (id = unit, name = unit
/// name). A region in several units goes to its first; regions in no unit
/// are kept as they are. Parent and country are kept when all members agree.
pub fn dissolve(features: Vec<MapFeature>, rows: &[UnitRow]) -> Vec<MapFeature> {
    let by_region = assignments(rows);
    let names: BTreeMap<&str, &str> = rows
        .iter()
        .filter_map(|r| r.unit_name.as_deref().map(|n| (r.unit.as_str(), n)))
        .collect();
    let mut units: BTreeMap<String, Vec<MapFeature>> = BTreeMap::new();
    let mut out = Vec::new();
    for f in features {
        match by_region.get(f.id.as_str()).and_then(|u| u.first()) {
            Some(u) => units.entry(u.to_string()).or_default().push(f),
            None => out.push(f),
        }
    }
    for (unit, members) in units {
        let same = |get: fn(&MapFeature) -> &Option<String>| {
            let first = get(&members[0]).clone();
            members
                .iter()
                .all(|m| *get(m) == first)
                .then_some(first)
                .flatten()
        };
        out.push(MapFeature {
            name: names
                .get(unit.as_str())
                .map_or_else(|| unit.clone(), |n| n.to_string()),
            // Unit names come from the table, in one language.
            names: BTreeMap::new(),
            class: members[0].class.clone(),
            parent: same(|m| &m.parent),
            parent_name: same(|m| &m.parent_name),
            country: same(|m| &m.country),
            units: vec![unit.clone()],
            wikidata: None,
            geometry: MultiPolygon(members.into_iter().flat_map(|m| m.geometry.0).collect()),
            id: unit,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::{coord, Rect};

    fn region(id: &str, x: f64, parent: &str) -> MapFeature {
        MapFeature {
            id: id.into(),
            name: id.into(),
            class: "subdivision".into(),
            parent: Some(parent.into()),
            country: Some("us".into()),
            geometry: MultiPolygon(vec![Rect::new(
                coord! { x: x, y: 0.0 },
                coord! { x: x + 1.0, y: 1.0 },
            )
            .to_polygon()]),
            ..MapFeature::default()
        }
    }

    fn row(region: &str, unit: &str) -> UnitRow {
        UnitRow {
            region: region.into(),
            unit: unit.into(),
            unit_name: Some(format!("{unit} city")),
        }
    }

    /// Five boroughs in a row (x = 0..5), a separate county at x = 10.
    fn boroughs() -> Vec<MapFeature> {
        let mut v: Vec<MapFeature> = (0..5)
            .map(|i| region(&format!("B{i}"), f64::from(i), "NY"))
            .collect();
        v.push(region("C", 10.0, "NY"));
        v
    }

    #[test]
    fn dissolve_merges_members_and_keeps_the_rest() {
        let rows: Vec<UnitRow> = (0..5).map(|i| row(&format!("B{i}"), "NYC")).collect();
        let out = dissolve(boroughs(), &rows);
        assert_eq!(
            out.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            ["C", "NYC"]
        );
        let nyc = &out[1];
        assert_eq!((nyc.name.as_str(), nyc.geometry.0.len()), ("NYC city", 5));
        assert_eq!(
            (nyc.parent.as_deref(), nyc.country.as_deref()),
            (Some("NY"), Some("us"))
        );
        // The borders between boroughs are now internal: only the outline remains.
        let (_, arcs) = Topology::build(std::slice::from_ref(&nyc.geometry)).finish(0.0, |_| true);
        let length: f64 = arcs
            .iter()
            .flat_map(|a| a.coords.windows(2))
            .map(|w| (w[1].x - w[0].x).abs() + (w[1].y - w[0].y).abs())
            .sum();
        assert_eq!(length, 12.0, "perimeter of the 5×1 block");
    }

    #[test]
    fn report_flags_unknown_overlapping_split_and_unassigned() {
        let rows = vec![
            row("B0", "NYC"),
            row("B1", "NYC"),
            row("B2", "KC"),
            row("B3", "KC"),
            row("B3", "Jackson"), // B3 in two units: they don't nest
            row("C", "KC"),       // C is far from B2/B3: KC is split
            row("Z9", "NYC"),     // not on the map
        ];
        let r = check_units(&boroughs(), &rows);
        assert_eq!(r.unknown_regions, ["Z9"]);
        assert_eq!(
            r.overlapping,
            [(
                "B3".to_string(),
                vec!["KC".to_string(), "Jackson".to_string()]
            )]
        );
        assert_eq!(r.split_units, [("KC".to_string(), 2)]);
        assert_eq!(r.unassigned, ["B4"]);
    }

    #[test]
    fn tags_every_unit_of_a_region() {
        let mut fs = boroughs();
        tag_units(&mut fs, &[row("B3", "KC"), row("B3", "Jackson")]);
        assert_eq!(fs[3].units, ["KC", "Jackson"]);
        assert!(fs[0].units.is_empty());
    }
}
