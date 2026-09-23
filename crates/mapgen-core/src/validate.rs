//! Input checks and repairs (`mapgen check`, `mapgen convert --repair`).
//!
//! Works on WGS84 features before any projection. Tolerances are in degrees.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use geo::line_intersection::{line_intersection, LineIntersection};
use geo::{Area, BooleanOps, BoundingRect, Closest, ClosestPoint, Euclidean, Length};
use geo_types::{Coord, Line, LineString, MultiPolygon, Point, Polygon};
use rstar::primitives::{GeomWithData, Rectangle};
use rstar::RTree;
use rstar::RTreeObject;

use crate::feature::MapFeature;
use crate::simplify::Topology;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckOptions {
    /// Borders of two features closer than this (but not touching) are a
    /// near miss: a sliver gap or overlap between neighbours. ~10 m.
    pub border_tolerance: f64,
    /// Parts less compact than this (Polsby–Popper, 1 = circle) and smaller
    /// than `sliver_max_area` are slivers.
    pub sliver_compactness: f64,
    pub sliver_max_area: f64,
    /// Overlaps smaller than this fraction of the smaller feature are ignored.
    pub overlap_min_share: f64,
}

impl Default for CheckOptions {
    fn default() -> Self {
        CheckOptions {
            border_tolerance: 1e-4,
            sliver_compactness: 0.02,
            sliver_max_area: 1e-4,
            overlap_min_share: 1e-4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueKind {
    /// No polygon at all.
    Empty,
    /// Coordinates outside ±180° / ±90° or non-finite.
    OutOfRange,
    /// Invalid polygon (self-intersection, bad rings…), with the reason.
    Invalid(String),
    /// Repeated consecutive vertices.
    DuplicatePoints(usize),
    /// A tiny, very thin part.
    Sliver,
    /// Overlaps another feature (share of the smaller one, in per mille).
    Overlap(u32),
    /// Border runs close to, but not on, a neighbour's border (vertex count).
    NearMissBorder(usize),
    /// Another feature has the same id.
    DuplicateId,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Issue {
    pub feature: String,
    pub kind: IssueKind,
    /// The other feature involved, for overlaps and near misses.
    pub other: Option<String>,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let other = self.other.as_deref().unwrap_or("?");
        match &self.kind {
            IssueKind::Empty => write!(f, "{}: no polygon", self.feature),
            IssueKind::OutOfRange => write!(
                f,
                "{}: coordinates out of range or non-finite",
                self.feature
            ),
            IssueKind::Invalid(why) => write!(f, "{}: invalid polygon: {why}", self.feature),
            IssueKind::DuplicatePoints(n) => write!(f, "{}: {n} repeated vertices", self.feature),
            IssueKind::Sliver => write!(f, "{}: sliver part", self.feature),
            IssueKind::Overlap(pm) => write!(
                f,
                "{}: overlaps {other} ({:.1}% of the smaller)",
                self.feature,
                f64::from(*pm) / 10.0
            ),
            IssueKind::NearMissBorder(n) => write!(
                f,
                "{}: border misses {other}'s by a hair ({n} vertices)",
                self.feature
            ),
            IssueKind::DuplicateId => write!(f, "{}: duplicate id", self.feature),
        }
    }
}

/// Everything found, sorted (deterministic).
pub fn check(features: &[MapFeature], opts: &CheckOptions) -> Vec<Issue> {
    let mut issues = BTreeSet::new();
    let issue = |f: &MapFeature, kind: IssueKind, other: Option<&MapFeature>| Issue {
        feature: f.id.clone(),
        kind,
        other: other.map(|o| o.id.clone()),
    };

    let mut seen = BTreeMap::new();
    for f in features {
        *seen.entry(f.id.as_str()).or_insert(0) += 1;
    }
    for f in features {
        if seen[f.id.as_str()] > 1 {
            issues.insert(issue(f, IssueKind::DuplicateId, None));
        }
        if f.geometry.0.is_empty() {
            issues.insert(issue(f, IssueKind::Empty, None));
            continue;
        }
        let coords = f
            .geometry
            .0
            .iter()
            .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
            .flat_map(|r| r.0.iter());
        if coords
            .clone()
            .any(|c| !c.x.is_finite() || !c.y.is_finite() || c.x.abs() > 180.0 || c.y.abs() > 90.0)
        {
            issues.insert(issue(f, IssueKind::OutOfRange, None));
        }
        let dups = rings(&f.geometry)
            .map(|r| r.0.windows(2).filter(|w| w[0] == w[1]).count())
            .sum::<usize>();
        if dups > 0 {
            issues.insert(issue(f, IssueKind::DuplicatePoints(dups), None));
        }
        for p in &f.geometry {
            if let Some(why) = polygon_problem(p) {
                issues.insert(issue(f, IssueKind::Invalid(why.into()), None));
            }
            if is_sliver(p, opts) {
                issues.insert(issue(f, IssueKind::Sliver, None));
            }
        }
    }

    // Overlaps between features whose bounding boxes intersect.
    let tree = bbox_tree(features);
    for (i, f) in features.iter().enumerate() {
        let Some(b) = f.geometry.bounding_rect() else {
            continue;
        };
        let env = rstar::AABB::from_corners([b.min().x, b.min().y], [b.max().x, b.max().y]);
        for hit in tree.locate_in_envelope_intersecting(&env) {
            let j = hit.data;
            if j <= i {
                continue;
            }
            let g = &features[j];
            let overlap = f.geometry.intersection(&g.geometry).unsigned_area();
            let smaller = f.geometry.unsigned_area().min(g.geometry.unsigned_area());
            if smaller > 0.0 && overlap / smaller > opts.overlap_min_share {
                let pm = (1000.0 * overlap / smaller)
                    .round()
                    .min(f64::from(u32::MAX)) as u32;
                issues.insert(issue(f, IssueKind::Overlap(pm), Some(g)));
            }
        }
    }

    // Near-miss borders: outline vertices close to another feature's outline.
    for ((a, b), n) in near_misses(features, opts.border_tolerance) {
        issues.insert(issue(
            &features[a],
            IssueKind::NearMissBorder(n),
            Some(&features[b]),
        ));
    }
    issues.into_iter().collect()
}

/// What [`repair`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RepairSummary {
    pub removed_points: usize,
    pub dropped_rings: usize,
    pub fixed_polygons: usize,
    pub snapped_vertices: usize,
    /// Snapping was tried but left more near-miss borders than before, so it
    /// was undone (only the clean-up was kept).
    pub snapping_reverted: bool,
}

/// Fixes what can be fixed safely: repeated vertices, degenerate rings,
/// invalid polygons (rebuilt with a boolean union), and near-miss borders
/// (each feature's vertices are snapped onto the borders of the features
/// before it). Overlaps and slivers are only reported by [`check`].
pub fn repair(features: &mut [MapFeature], opts: &CheckOptions) -> RepairSummary {
    // Measured like `check` reports it: pairs of features first, then vertices.
    let near_miss_total = |fs: &[MapFeature]| -> (usize, usize) {
        let m = near_misses(fs, opts.border_tolerance);
        (m.len(), m.values().sum())
    };
    let before = near_miss_total(features);
    let original = features.to_vec();

    let mut s = RepairSummary::default();
    if before.0 > 0 {
        s.snapped_vertices = snap_features(features, opts.border_tolerance);
    }
    clean_features(features, &mut s);

    // Snapping one side of a border can open near misses elsewhere (and
    // rebuilt polygons are re-quantised): keep it only if it helped.
    if s.snapped_vertices > 0 && near_miss_total(features) > before {
        features.clone_from_slice(&original);
        s = RepairSummary {
            snapping_reverted: true,
            ..RepairSummary::default()
        };
        clean_features(features, &mut s);
    }
    s
}

/// Snaps later features onto earlier ones: onto an earlier vertex when one is
/// within tolerance (so vertices shared before stay shared after), else onto
/// the nearest earlier edge. Returns the number of vertices moved.
fn snap_features(features: &mut [MapFeature], tolerance: f64) -> usize {
    let tol2 = tolerance * tolerance;
    let mut tree: RTree<GeomWithData<Line<f64>, usize>> = RTree::new();
    let mut vertices: RTree<[f64; 2]> = RTree::new();
    let mut moved = 0;
    for (i, f) in features.iter_mut().enumerate() {
        if tree.size() > 0 {
            for p in f.geometry.0.iter_mut() {
                p.exterior_mut(|r| moved += snap_ring(r, &vertices, &tree, tol2));
                p.interiors_mut(|rs| {
                    for r in rs {
                        moved += snap_ring(r, &vertices, &tree, tol2);
                    }
                });
            }
        }
        for r in rings(&f.geometry) {
            for w in r.0.windows(2) {
                tree.insert(GeomWithData::new(Line::new(w[0], w[1]), i));
            }
            for c in &r.0 {
                vertices.insert([c.x, c.y]);
            }
        }
    }
    moved
}

/// Removes repeated vertices and degenerate rings, and rebuilds invalid
/// polygons with a boolean union.
fn clean_features(features: &mut [MapFeature], s: &mut RepairSummary) {
    for f in features.iter_mut() {
        let mut polys = Vec::new();
        for p in f.geometry.0.drain(..) {
            let (ext, holes) = p.into_inner();
            let Some(ext) = clean_ring(ext, s) else {
                s.dropped_rings += 1 + holes.len();
                continue;
            };
            let holes = holes.into_iter().filter_map(|h| clean_ring(h, s)).collect();
            let p = Polygon::new(ext, holes);
            if polygon_problem(&p).is_none() {
                polys.push(p);
            } else {
                s.fixed_polygons += 1;
                polys.extend(MultiPolygon(vec![p]).union(&MultiPolygon::<f64>(vec![])).0);
            }
        }
        f.geometry = MultiPolygon(polys);
    }
}

fn snap_ring(
    r: &mut LineString<f64>,
    vertices: &RTree<[f64; 2]>,
    edges: &RTree<GeomWithData<Line<f64>, usize>>,
    tol2: f64,
) -> usize {
    let mut moved = 0;
    // The closing vertex repeats the first: snap distinct vertices, re-close.
    let n = if r.is_closed() {
        r.0.len().saturating_sub(1)
    } else {
        r.0.len()
    };
    for c in r.0.iter_mut().take(n) {
        let target = match vertices.nearest_neighbor(&[c.x, c.y]) {
            Some(v) if (v[0] - c.x).powi(2) + (v[1] - c.y).powi(2) <= tol2 => {
                Some(Coord { x: v[0], y: v[1] })
            }
            _ => {
                let p = Point::from(*c);
                edges
                    .nearest_neighbor(&p)
                    .and_then(|hit| match hit.geom().closest_point(&p) {
                        Closest::Intersection(q) | Closest::SinglePoint(q) => Some(q.into()),
                        Closest::Indeterminate => None,
                    })
            }
        };
        if let Some(q) = target {
            let d2 = (q.x - c.x).powi(2) + (q.y - c.y).powi(2);
            if d2 > 0.0 && d2 <= tol2 {
                *c = q;
                moved += 1;
            }
        }
    }
    if n < r.0.len() && n > 0 {
        r.0[n] = r.0[0];
    }
    moved
}

/// Why a polygon is invalid, if it is: a ring with fewer than 3 distinct
/// points, or edges crossing or overlapping (within a ring or between rings).
/// Edges merely touching at a vertex are accepted (unlike OGC validity).
///
/// `geo`'s `Validation` compares every pair of edges, which takes minutes on
/// full-resolution boundaries (rings of 100 000+ vertices); here edges are
/// only compared with those whose bounding boxes overlap, via an R-tree.
fn polygon_problem(p: &Polygon<f64>) -> Option<&'static str> {
    let rings: Vec<&LineString<f64>> = std::iter::once(p.exterior()).chain(p.interiors()).collect();
    for r in &rings {
        let distinct: BTreeSet<(u64, u64)> =
            r.0.iter().map(|c| (c.x.to_bits(), c.y.to_bits())).collect();
        if distinct.len() < 3 {
            return Some("a ring has fewer than 3 distinct points");
        }
    }
    // Repeated vertices are not an error (repair removes them), but they would
    // make neighbouring edges look non-adjacent: drop them first.
    let deduped: Vec<Vec<Coord<f64>>> = rings
        .iter()
        .map(|r| {
            let mut v = r.0.clone();
            v.dedup();
            v
        })
        .collect();
    // (ring, edge index, ring edge count) for each edge.
    let edges: Vec<GeomWithData<Line<f64>, (usize, usize, usize)>> = deduped
        .iter()
        .enumerate()
        .flat_map(|(ri, r)| {
            let n = r.len().saturating_sub(1);
            r.windows(2)
                .enumerate()
                .map(move |(ei, w)| GeomWithData::new(Line::new(w[0], w[1]), (ri, ei, n)))
        })
        .collect();
    let tree = RTree::bulk_load(edges.clone());
    for e in &edges {
        let (ri, ei, n) = e.data;
        for other in tree.locate_in_envelope_intersecting(&e.envelope()) {
            let (rj, ej, _) = other.data;
            if (rj, ej) <= (ri, ei) {
                continue;
            }
            // Consecutive edges of a ring share a vertex by construction.
            let adjacent = ri == rj && (ej == ei + 1 || (ei == 0 && ej + 1 == n));
            if adjacent {
                continue;
            }
            // Edges meeting at a vertex (a ring pinched at a point, a hole
            // touching the outline) are fine for rendering and are what
            // boolean operations produce; only crossings and overlaps count.
            let crossing = match line_intersection(*e.geom(), *other.geom()) {
                Some(LineIntersection::SinglePoint { is_proper, .. }) => is_proper,
                Some(LineIntersection::Collinear { intersection }) => {
                    intersection.start != intersection.end
                }
                None => false,
            };
            if crossing {
                return Some(if ri == rj {
                    "self-intersection"
                } else {
                    "rings intersect"
                });
            }
        }
    }
    None
}

fn clean_ring(mut r: LineString<f64>, s: &mut RepairSummary) -> Option<LineString<f64>> {
    let before = r.0.len();
    r.0.dedup();
    s.removed_points += before - r.0.len();
    r.close();
    let distinct: BTreeSet<(u64, u64)> =
        r.0.iter().map(|c| (c.x.to_bits(), c.y.to_bits())).collect();
    (distinct.len() >= 3 && r.0.iter().all(|c| c.x.is_finite() && c.y.is_finite())).then_some(r)
}

fn rings(mp: &MultiPolygon<f64>) -> impl Iterator<Item = &LineString<f64>> {
    mp.0.iter()
        .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
}

fn is_sliver(p: &Polygon<f64>, opts: &CheckOptions) -> bool {
    let area = p.unsigned_area();
    let perimeter = Euclidean.length(p.exterior());
    if area >= opts.sliver_max_area || perimeter <= 0.0 {
        return false;
    }
    4.0 * std::f64::consts::PI * area / (perimeter * perimeter) < opts.sliver_compactness
}

fn bbox_tree(features: &[MapFeature]) -> RTree<GeomWithData<Rectangle<[f64; 2]>, usize>> {
    RTree::bulk_load(
        features
            .iter()
            .enumerate()
            .filter_map(|(i, f)| f.geometry.bounding_rect().map(|b| (i, b)))
            .map(|(i, b)| {
                GeomWithData::new(
                    Rectangle::from_corners([b.min().x, b.min().y], [b.max().x, b.max().y]),
                    i,
                )
            })
            .collect(),
    )
}

/// `(feature, neighbour) → vertex count` for outline vertices lying within
/// `tolerance` of (but not on) another feature's outline.
fn near_misses(features: &[MapFeature], tolerance: f64) -> BTreeMap<(usize, usize), usize> {
    let geoms: Vec<MultiPolygon<f64>> = features.iter().map(|f| f.geometry.clone()).collect();
    let (_, arcs) = Topology::build(&geoms).finish(0.0, |_| true);
    let outline: Vec<(Vec<Coord<f64>>, usize)> = arcs
        .into_iter()
        .filter(|a| a.b.is_none())
        .map(|a| (a.coords, a.a))
        .collect();
    let tree: RTree<GeomWithData<Line<f64>, usize>> = RTree::bulk_load(
        outline
            .iter()
            .flat_map(|(c, owner)| {
                c.windows(2)
                    .map(move |w| GeomWithData::new(Line::new(w[0], w[1]), *owner))
            })
            .collect(),
    );
    let tol2 = tolerance * tolerance;
    let mut out = BTreeMap::new();
    for (coords, owner) in &outline {
        for c in coords {
            let p = Point::from(*c);
            // Only segments within the tolerance: a nearest-neighbour walk that
            // skips the feature's own segments would be quadratic along coasts.
            let nearest = tree
                .locate_within_distance(p, tol2)
                .filter(|hit| hit.data != *owner)
                .filter_map(|hit| match hit.geom().closest_point(&p) {
                    Closest::Intersection(q) | Closest::SinglePoint(q) => {
                        Some(((q.x() - c.x).powi(2) + (q.y() - c.y).powi(2), hit.data))
                    }
                    Closest::Indeterminate => None,
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some((d2, other)) = nearest {
                if d2 > 0.0 && d2 <= tol2 {
                    *out.entry((*owner.min(&other), *owner.max(&other)))
                        .or_insert(0) += 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::polygon;

    fn feature(id: &str, p: Polygon<f64>) -> MapFeature {
        MapFeature {
            id: id.into(),
            name: id.into(),
            class: "x".into(),
            geometry: MultiPolygon(vec![p]),
            ..MapFeature::default()
        }
    }

    fn sq(x: f64, y: f64, s: f64) -> Polygon<f64> {
        polygon![(x: x, y: y), (x: x + s, y: y), (x: x + s, y: y + s), (x: x, y: y + s), (x: x, y: y)]
    }

    #[test]
    fn clean_data_has_no_issues() {
        let fs = [
            feature("a", sq(0.0, 0.0, 1.0)),
            feature("b", sq(1.0, 0.0, 1.0)),
        ];
        assert!(check(&fs, &CheckOptions::default()).is_empty());
    }

    #[test]
    fn finds_bowtie_overlap_near_miss_and_duplicates() {
        let bowtie = polygon![(x: 0., y: 0.), (x: 1., y: 1.), (x: 1., y: 0.), (x: 0., y: 1.), (x: 0., y: 0.)];
        let fs = [
            feature("bow", bowtie),
            feature("a", sq(10.0, 0.0, 1.0)),
            feature("b", sq(10.5, 0.0, 1.0)), // overlaps a by half
            feature("c", sq(20.0, 0.0, 1.0)),
            feature("d", sq(21.00005, 0.0, 1.0)), // 5 m gap to c
            feature("c", sq(30.0, 0.0, 1.0)),     // duplicate id
        ];
        let issues = check(&fs, &CheckOptions::default());
        let has = |id: &str, pred: &dyn Fn(&IssueKind) -> bool| {
            issues.iter().any(|i| i.feature == id && pred(&i.kind))
        };
        assert!(
            has("bow", &|k| matches!(k, IssueKind::Invalid(_))),
            "{issues:#?}"
        );
        assert!(has("a", &|k| matches!(k, IssueKind::Overlap(500))));
        assert!(has("c", &|k| matches!(k, IssueKind::NearMissBorder(_))));
        assert!(has("c", &|k| *k == IssueKind::DuplicateId));
    }

    #[test]
    fn repair_fixes_bowties_and_closes_near_misses() {
        let bowtie = polygon![(x: 0., y: 0.), (x: 1., y: 1.), (x: 1., y: 1.), (x: 1., y: 0.), (x: 0., y: 1.), (x: 0., y: 0.)];
        let mut fs = vec![
            feature("bow", bowtie),
            feature("c", sq(20.0, 0.0, 1.0)),
            feature("d", sq(21.00005, 0.0, 1.0)),
        ];
        let s = repair(&mut fs, &CheckOptions::default());
        assert_eq!(s.removed_points, 1);
        assert_eq!(s.fixed_polygons, 1);
        assert!(fs[0]
            .geometry
            .0
            .iter()
            .all(|p| polygon_problem(p).is_none()));
        assert!(
            (fs[0].geometry.unsigned_area() - 0.5).abs() < 1e-9,
            "two triangles of 0.25"
        );
        assert_eq!(
            s.snapped_vertices, 2,
            "d's left edge moved onto c's right edge"
        );
        assert!(check(&fs, &CheckOptions::default())
            .iter()
            .all(|i| !matches!(i.kind, IssueKind::NearMissBorder(_))));
    }

    #[test]
    fn polygon_problems_match_geo_validation() {
        use geo::Validation;
        let cases = [
            sq(0.0, 0.0, 1.0),
            polygon![(x: 0., y: 0.), (x: 1., y: 1.), (x: 1., y: 0.), (x: 0., y: 1.), (x: 0., y: 0.)],
            polygon![(x: 0., y: 0.), (x: 2., y: 0.), (x: 2., y: 2.), (x: 1., y: 2.), (x: 1., y: -1.), (x: 0., y: 2.), (x: 0., y: 0.)],
            Polygon::new(
                sq(0.0, 0.0, 4.0).exterior().clone(),
                vec![sq(1.0, 1.0, 1.0).exterior().clone()],
            ),
            Polygon::new(
                sq(0.0, 0.0, 4.0).exterior().clone(),
                vec![sq(3.0, 1.0, 2.0).exterior().clone()],
            ),
            polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 0., y: 0.)],
        ];
        for (i, p) in cases.iter().enumerate() {
            assert_eq!(polygon_problem(p).is_none(), p.is_valid(), "case {i}");
        }
    }

    #[test]
    fn touching_at_a_vertex_is_accepted() {
        // A figure-eight pinched at (1, 1): OGC-invalid, but harmless to draw
        // and what boolean unions produce.
        let pinched = polygon![(x: 0., y: 0.), (x: 1., y: 1.), (x: 2., y: 0.), (x: 2., y: 2.), (x: 1., y: 1.), (x: 0., y: 2.), (x: 0., y: 0.)];
        assert_eq!(polygon_problem(&pinched), None);
        // Overlapping collinear edges are still a problem.
        let spike = polygon![(x: 0., y: 0.), (x: 2., y: 0.), (x: 2., y: 2.), (x: 1., y: 2.), (x: 1., y: 0.), (x: 1.5, y: 0.), (x: 0., y: 0.)];
        assert!(polygon_problem(&spike).is_some());
    }

    #[test]
    fn repeated_vertices_are_not_self_intersections() {
        let p = polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 1., y: 0.), (x: 1., y: 1.), (x: 0., y: 1.), (x: 0., y: 0.)];
        assert_eq!(polygon_problem(&p), None);
    }

    #[test]
    fn repair_prefers_existing_vertices() {
        // b's corner is near a's corner: it must land exactly on it, not on
        // the middle of a's edge.
        let mut fs = vec![
            feature("a", sq(0.0, 0.0, 1.0)),
            feature("b", sq(1.00003, 1.00002, 1.0)),
        ];
        repair(&mut fs, &CheckOptions::default());
        assert!(fs[1].geometry.0[0]
            .exterior()
            .0
            .contains(&Coord { x: 1.0, y: 1.0 }));
    }

    #[test]
    fn slivers_are_flagged() {
        let sliver = polygon![(x: 0., y: 0.), (x: 0.01, y: 0.), (x: 0.01, y: 0.00001), (x: 0., y: 0.00001), (x: 0., y: 0.)];
        let issues = check(&[feature("s", sliver)], &CheckOptions::default());
        assert!(issues.iter().any(|i| i.kind == IssueKind::Sliver));
    }
}
