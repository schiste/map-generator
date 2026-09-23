//! Topology-aware Visvalingam–Whyatt simplification.
//!
//! Simplifying each polygon on its own drops different vertices on each side
//! of a shared border and leaves slivers and gaps. Instead, as in TopoJSON:
//!
//! 1. find *junctions*, vertices where the set of neighbouring vertices
//!    differs between the rings that use them (i.e. where borders meet);
//! 2. cut every ring at its junctions into *arcs*, and deduplicate arcs so a
//!    border shared by two regions is stored once;
//! 3. simplify each arc once, keeping its endpoints;
//! 4. stitch rings back together from the simplified arcs.
//!
//! This relies on neighbours sharing bit-identical vertices, which holds for
//! Natural Earth and geoBoundaries (and is preserved by deterministic projection).

use std::collections::HashMap;

use geo::{Area, SimplifyVw};
use geo_types::{Coord, Line, LineString, MultiPolygon, Polygon};

/// Converts a tolerance in output pixels into a Visvalingam–Whyatt area
/// threshold in projected units² (VW removes vertices whose effective
/// triangle area is below the threshold).
pub fn vw_epsilon(tolerance_px: f64, pixels_per_unit: f64) -> f64 {
    let t = tolerance_px / pixels_per_unit;
    t * t
}

type Key = (u64, u64);

fn key(c: Coord<f64>) -> Key {
    // Normalise -0.0 so it matches 0.0.
    ((c.x + 0.0).to_bits(), (c.y + 0.0).to_bits())
}

/// Ring as a closed sequence of arc references `(arc index, reversed)`.
type RingArcs = Vec<(usize, bool)>;

/// A border arc between two geometries, or on the outline of one.
#[derive(Debug, Clone, PartialEq)]
pub struct BorderArc {
    pub coords: Vec<Coord<f64>>,
    /// Index of a geometry using the arc.
    pub a: usize,
    /// The geometry on the other side, or `None` when the arc is on the
    /// outline of the set (a coast, or the edge of the mapped area).
    pub b: Option<usize>,
    /// For outline arcs: whether `a`'s interior lies to the left when walking
    /// `coords` in order (y axis up). Tells which side is the outside.
    pub interior_left: Option<bool>,
}

/// Shared-border structure of a set of polygonal geometries.
///
/// Rings are cut into arcs at junctions and every arc is stored once, so a
/// border between two regions is simplified, moved (snapping) and drawn
/// exactly once. Arcs used twice by the *same* geometry (e.g. the edge where
/// a dataset cut an island at 180°) are internal to it and never drawn.
#[derive(Debug, Clone)]
pub struct Topology {
    arcs: Vec<Vec<Coord<f64>>>,
    /// geometry → polygon → ring (exterior first) → arcs, `None` if degenerate.
    plans: Vec<Vec<Vec<Option<RingArcs>>>>,
    originals: Vec<MultiPolygon<f64>>,
}

impl Topology {
    pub fn build(geoms: &[MultiPolygon<f64>]) -> Topology {
        // Open, de-duplicated rings: geoms → polygons → rings → vertices.
        let rings: Vec<Vec<Vec<Vec<Coord<f64>>>>> = geoms
            .iter()
            .map(|mp| {
                mp.0.iter()
                    .map(|p| {
                        std::iter::once(p.exterior())
                            .chain(p.interiors())
                            .map(open_ring)
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let junctions = find_junctions(rings.iter().flatten().flatten());
        let mut arcs: Vec<Vec<Coord<f64>>> = Vec::new();
        let mut index: HashMap<Vec<Key>, usize> = HashMap::new();
        let plans = rings
            .iter()
            .map(|g| {
                g.iter()
                    .map(|p| {
                        p.iter()
                            .map(|r| cut_ring(r, &junctions, &mut arcs, &mut index))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        Topology {
            arcs,
            plans,
            originals: geoms.to_vec(),
        }
    }

    /// Simplifies every arc once with Visvalingam–Whyatt (endpoints kept).
    pub fn simplify(&mut self, epsilon: f64) {
        if epsilon <= 0.0 {
            return;
        }
        for a in &mut self.arcs {
            *a = LineString(std::mem::take(a)).simplify_vw(&epsilon).0;
        }
    }

    /// Applies `f` to every arc vertex. Shared vertices move together, so
    /// borders stay shared.
    pub fn map_vertices(&mut self, mut f: impl FnMut(Coord<f64>) -> Coord<f64>) {
        for a in &mut self.arcs {
            for c in a.iter_mut() {
                *c = f(*c);
            }
        }
    }

    /// Segments of the arcs on the outline of the set (used by exactly one
    /// ring), e.g. to snap another layer's borders onto it.
    pub fn outline_segments(&self) -> Vec<Line<f64>> {
        let mut count = vec![0usize; self.arcs.len()];
        for refs in self.plans.iter().flatten().flatten().flatten() {
            for &(arc, _) in refs {
                count[arc] += 1;
            }
        }
        self.arcs
            .iter()
            .zip(count)
            .filter(|(_, n)| *n == 1)
            .flat_map(|(a, _)| a.windows(2).map(|w| Line::new(w[0], w[1])))
            .collect()
    }

    /// Stitches the geometries back together.
    ///
    /// Polygons smaller than `min_area` are dropped, except the largest part
    /// of geometries for which `keep_largest(index)` is true. A geometry whose
    /// polygons all collapse keeps its largest original polygon unsimplified.
    /// Returns the geometries (same order as the input) and the border arcs of
    /// the polygons that survived.
    pub fn finish(
        &self,
        min_area: f64,
        keep_largest: impl Fn(usize) -> bool,
    ) -> (Vec<MultiPolygon<f64>>, Vec<BorderArc>) {
        // (geometry, is its interior left of the stored arc?) for each use.
        let mut uses: Vec<Vec<(usize, bool)>> = vec![Vec::new(); self.arcs.len()];
        let mut geoms = Vec::with_capacity(self.plans.len());
        for (gi, (plan, original)) in self.plans.iter().zip(&self.originals).enumerate() {
            // (polygon, its arc refs, area)
            let mut polys: Vec<(Polygon<f64>, Vec<&RingArcs>, f64)> = Vec::new();
            for p in plan {
                let mut rings = p.iter().map(|r| {
                    r.as_ref()
                        .and_then(|refs| stitch(refs, &self.arcs).map(|ring| (ring, refs)))
                });
                let Some(Some((exterior, ext_refs))) = rings.next() else {
                    continue;
                };
                let holes: Vec<(LineString<f64>, &RingArcs)> = rings.flatten().collect();
                let mut refs = vec![ext_refs];
                refs.extend(holes.iter().map(|(_, r)| *r));
                let poly = Polygon::new(exterior, holes.into_iter().map(|(h, _)| h).collect());
                let area = poly.unsigned_area();
                polys.push((poly, refs, area));
            }
            if polys.is_empty() {
                geoms.push(largest(original));
                continue;
            }
            let largest_idx = (0..polys.len())
                .max_by(|&x, &y| polys[x].2.total_cmp(&polys[y].2).then(y.cmp(&x)))
                .unwrap_or(0);
            let keep = keep_largest(gi);
            let mut kept = Vec::new();
            for (i, (poly, refs, area)) in polys.into_iter().enumerate() {
                if area >= min_area || (keep && i == largest_idx) {
                    let rings = std::iter::once(poly.exterior()).chain(poly.interiors());
                    for (ri, (r, ring)) in refs.iter().zip(rings).enumerate() {
                        // The polygon's interior is left of a counter-clockwise
                        // exterior and right of a counter-clockwise hole.
                        let ccw = signed_area(ring) > 0.0;
                        let interior_left = if ri == 0 { ccw } else { !ccw };
                        for &(arc, reversed) in *r {
                            uses[arc].push((gi, interior_left != reversed));
                        }
                    }
                    kept.push(poly);
                }
            }
            geoms.push(MultiPolygon(kept));
        }

        let borders = self
            .arcs
            .iter()
            .zip(uses)
            .filter(|(coords, _)| coords.len() >= 2)
            .filter_map(|(coords, uses)| {
                let (a, left) = *uses.first()?;
                let total = uses.len();
                let mut users: Vec<usize> = uses.iter().map(|u| u.0).collect();
                users.sort_unstable();
                users.dedup();
                let b = match users.as_slice() {
                    // One geometry, used once: its outline.
                    [_] if total == 1 => None,
                    // One geometry using the arc twice: an internal cut, not a border.
                    [_] => return None,
                    [x, y, ..] => Some(if *x == a { *y } else { *x }),
                    [] => return None,
                };
                Some(BorderArc {
                    coords: coords.clone(),
                    a,
                    b,
                    interior_left: b.is_none().then_some(left),
                })
            })
            .collect();
        (geoms, borders)
    }
}

/// Simplifies a set of geometries together so shared borders stay shared.
///
/// Output has the same length and order as the input. A geometry is never
/// simplified away entirely: if every polygon of it collapses, its largest
/// original polygon is kept unsimplified.
pub fn simplify_shared(geoms: &[MultiPolygon<f64>], epsilon: f64) -> Vec<MultiPolygon<f64>> {
    if epsilon <= 0.0 {
        return geoms.to_vec();
    }
    let mut t = Topology::build(geoms);
    t.simplify(epsilon);
    t.finish(0.0, |_| true).0
}

/// Shoelace area of a closed ring: positive when counter-clockwise.
fn signed_area(ring: &LineString<f64>) -> f64 {
    ring.0
        .windows(2)
        .map(|w| w[0].x * w[1].y - w[1].x * w[0].y)
        .sum::<f64>()
        / 2.0
}

fn open_ring(ring: &LineString<f64>) -> Vec<Coord<f64>> {
    let mut v: Vec<Coord<f64>> = Vec::with_capacity(ring.0.len());
    for &c in &ring.0 {
        if v.last() != Some(&c) {
            v.push(c);
        }
    }
    if v.len() > 1 && v.first() == v.last() {
        v.pop();
    }
    v
}

fn find_junctions<'a>(rings: impl Iterator<Item = &'a Vec<Coord<f64>>>) -> HashMap<Key, bool> {
    // For each vertex: the unordered pair of its neighbours the first time it
    // was seen, and whether a different pair has been seen since.
    let mut seen: HashMap<Key, (Key, Key, bool)> = HashMap::new();
    for ring in rings {
        let n = ring.len();
        if n < 3 {
            continue;
        }
        for i in 0..n {
            let (a, b) = (key(ring[(i + n - 1) % n]), key(ring[(i + 1) % n]));
            let pair = if a <= b { (a, b) } else { (b, a) };
            seen.entry(key(ring[i]))
                .and_modify(|e| {
                    if (e.0, e.1) != pair {
                        e.2 = true;
                    }
                })
                .or_insert((pair.0, pair.1, false));
        }
    }
    seen.into_iter().map(|(k, (_, _, j))| (k, j)).collect()
}

/// Cuts a ring into arcs at its junctions (or treats it as one closed arc
/// starting at its smallest vertex), registering arcs in canonical orientation.
fn cut_ring(
    ring: &[Coord<f64>],
    junctions: &HashMap<Key, bool>,
    arcs: &mut Vec<Vec<Coord<f64>>>,
    index: &mut HashMap<Vec<Key>, usize>,
) -> Option<RingArcs> {
    let n = ring.len();
    if n < 3 {
        return None;
    }
    let is_junction = |c: Coord<f64>| junctions.get(&key(c)).copied().unwrap_or(false);
    let cuts: Vec<usize> = (0..n).filter(|&i| is_junction(ring[i])).collect();
    let cuts = if cuts.is_empty() {
        // Canonical start so identical rings (enclave hole vs. enclave) match.
        vec![(0..n).min_by_key(|&i| key(ring[i])).unwrap_or(0)]
    } else {
        cuts
    };

    let mut refs = Vec::with_capacity(cuts.len());
    for (ci, &start) in cuts.iter().enumerate() {
        let end = cuts[(ci + 1) % cuts.len()];
        let len = if end > start {
            end - start
        } else {
            end + n - start
        };
        let arc: Vec<Coord<f64>> = (0..=len).map(|k| ring[(start + k) % n]).collect();
        refs.push(register(arc, arcs, index));
    }
    Some(refs)
}

fn register(
    arc: Vec<Coord<f64>>,
    arcs: &mut Vec<Vec<Coord<f64>>>,
    index: &mut HashMap<Vec<Key>, usize>,
) -> (usize, bool) {
    let fwd: Vec<Key> = arc.iter().map(|&c| key(c)).collect();
    let rev: Vec<Key> = fwd.iter().rev().copied().collect();
    let (canon, reversed) = if rev < fwd { (rev, true) } else { (fwd, false) };
    if let Some(&id) = index.get(&canon) {
        return (id, reversed);
    }
    let mut stored = arc;
    if reversed {
        stored.reverse();
    }
    arcs.push(stored);
    index.insert(canon, arcs.len() - 1);
    (arcs.len() - 1, reversed)
}

fn stitch(refs: &RingArcs, arcs: &[Vec<Coord<f64>>]) -> Option<LineString<f64>> {
    let mut coords: Vec<Coord<f64>> = Vec::new();
    for &(id, reversed) in refs {
        let arc = &arcs[id];
        let iter: Box<dyn Iterator<Item = &Coord<f64>>> = if reversed {
            Box::new(arc.iter().rev())
        } else {
            Box::new(arc.iter())
        };
        let skip = usize::from(!coords.is_empty());
        coords.extend(iter.skip(skip));
    }
    let ring = LineString(coords);
    let closed = ring.0.first() == ring.0.last();
    (closed && ring.0.len() >= 4 && Polygon::new(ring.clone(), vec![]).unsigned_area() > 0.0)
        .then_some(ring)
}

fn largest(mp: &MultiPolygon<f64>) -> MultiPolygon<f64> {
    let best =
        mp.0.iter()
            .enumerate()
            .max_by(|(ia, a), (ib, b)| {
                a.unsigned_area()
                    .total_cmp(&b.unsigned_area())
                    .then(ib.cmp(ia))
            })
            .map(|(_, p)| p.clone());
    MultiPolygon(best.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly(pts: &[(f64, f64)]) -> MultiPolygon<f64> {
        MultiPolygon(vec![Polygon::new(LineString::from(pts.to_vec()), vec![])])
    }

    /// Vertices of `a`'s ring that lie on the x = 10 border.
    fn border(mp: &MultiPolygon<f64>) -> Vec<(u64, u64)> {
        let mut v: Vec<_> = mp.0[0]
            .exterior()
            .0
            .iter()
            .filter(|c| (9.0..=11.0).contains(&c.x) && c.y > 0.0 && c.y < 100.0)
            .map(|&c| key(c))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    #[test]
    fn shared_border_is_simplified_identically() {
        // A wiggly border at x≈10 between two regions, traversed in opposite
        // directions and starting at different vertices.
        let wiggle: Vec<(f64, f64)> = (1..100)
            .map(|i| (10.0 + if i % 2 == 0 { 0.3 } else { -0.2 }, f64::from(i)))
            .collect();
        let mut west = vec![(0.0, 0.0), (10.0, 0.0)];
        west.extend(wiggle.iter().copied());
        west.extend([(10.0, 100.0), (0.0, 100.0), (0.0, 0.0)]);
        let mut east = vec![(10.0, 100.0)];
        east.extend(wiggle.iter().rev().copied());
        east.extend([(10.0, 0.0), (20.0, 0.0), (20.0, 100.0), (10.0, 100.0)]);

        let out = simplify_shared(&[poly(&west), poly(&east)], 0.5);
        let (bw, be) = (border(&out[0]), border(&out[1]));
        assert!(
            bw.len() < 99,
            "border should be simplified, has {}",
            bw.len()
        );
        assert_eq!(bw, be);
    }

    fn sq(x: f64, y: f64) -> Polygon<f64> {
        Polygon::new(
            LineString::from(vec![
                (x, y),
                (x + 1.0, y),
                (x + 1.0, y + 1.0),
                (x, y + 1.0),
                (x, y),
            ]),
            vec![],
        )
    }

    fn length(b: &BorderArc) -> f64 {
        b.coords
            .windows(2)
            .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
            .sum()
    }

    #[test]
    fn shared_border_is_one_arc_between_two_geometries() {
        let t = Topology::build(&[
            MultiPolygon(vec![sq(0.0, 0.0)]),
            MultiPolygon(vec![sq(1.0, 0.0)]),
        ]);
        let (geoms, borders) = t.finish(0.0, |_| true);
        assert_eq!(geoms.len(), 2);
        let shared: Vec<&BorderArc> = borders.iter().filter(|b| b.b.is_some()).collect();
        assert_eq!(shared.len(), 1);
        assert_eq!(length(shared[0]), 1.0);
        let outline: f64 = borders.iter().filter(|b| b.b.is_none()).map(length).sum();
        assert_eq!(outline, 6.0, "outer perimeter of the 2x1 block");
    }

    #[test]
    fn outline_arcs_know_which_side_is_inside() {
        // Whatever the ring orientation in the input, walking an outline arc
        // with the interior on the stated side keeps a point inside it.
        for square in [
            sq(0.0, 0.0),
            Polygon::new(
                sq(0.0, 0.0).exterior().clone().into_iter().rev().collect(),
                vec![],
            ),
        ] {
            let t = Topology::build(&[MultiPolygon(vec![square.clone()])]);
            let (_, arcs) = t.finish(0.0, |_| true);
            for arc in arcs {
                let left = arc.interior_left.expect("outline arc");
                let (p, q) = (arc.coords[0], arc.coords[1]);
                let (mx, my) = ((p.x + q.x) / 2.0, (p.y + q.y) / 2.0);
                let (nx, ny) = (-(q.y - p.y), q.x - p.x); // left normal
                let s = if left { 0.01 } else { -0.01 };
                let probe = geo_types::Point::new(mx + s * nx, my + s * ny);
                assert!(geo::Contains::contains(&square, &probe));
            }
        }
    }

    #[test]
    fn cut_inside_one_geometry_is_not_a_border() {
        // One feature made of two touching parts (like an island cut at 180°).
        let t = Topology::build(&[MultiPolygon(vec![sq(0.0, 0.0), sq(1.0, 0.0)])]);
        let (_, borders) = t.finish(0.0, |_| true);
        assert!(borders.iter().all(|b| b.b.is_none()));
        let total: f64 = borders.iter().map(length).sum();
        assert_eq!(total, 6.0, "the internal cut is not drawn");
    }

    #[test]
    fn culled_islands_take_their_borders_with_them() {
        let islands = MultiPolygon(vec![
            sq(0.0, 0.0),
            Polygon::new(
                LineString::from(vec![(5.0, 5.0), (5.1, 5.0), (5.1, 5.1), (5.0, 5.0)]),
                vec![],
            ),
        ]);
        let t = Topology::build(&[islands]);
        let (geoms, borders) = t.finish(0.5, |_| false);
        assert_eq!(geoms[0].0.len(), 1);
        let total: f64 = borders.iter().map(length).sum();
        assert_eq!(total, 4.0);
    }

    #[test]
    fn outline_segments_exclude_shared_borders() {
        let t = Topology::build(&[
            MultiPolygon(vec![sq(0.0, 0.0)]),
            MultiPolygon(vec![sq(1.0, 0.0)]),
        ]);
        let segs = t.outline_segments();
        assert!(segs.iter().all(|l| !(l.start.x == 1.0 && l.end.x == 1.0)));
        assert_eq!(segs.len(), 6);
    }

    #[test]
    fn tiny_geometry_is_never_dropped() {
        let speck = poly(&[
            (0.0, 0.0),
            (0.001, 0.0),
            (0.001, 0.001),
            (0.0, 0.001),
            (0.0, 0.0),
        ]);
        let out = simplify_shared(std::slice::from_ref(&speck), 1000.0);
        assert_eq!(out[0], speck);
    }

    #[test]
    fn zero_epsilon_is_identity() {
        let p = poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]);
        assert_eq!(simplify_shared(std::slice::from_ref(&p), 0.0), vec![p]);
    }
}
