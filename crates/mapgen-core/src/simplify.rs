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
use geo_types::{Coord, LineString, MultiPolygon, Polygon};

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

/// Simplifies a set of geometries together so shared borders stay shared.
///
/// Output has the same length and order as the input. A geometry is never
/// simplified away entirely: if every polygon of it collapses, its largest
/// original polygon is kept unsimplified.
pub fn simplify_shared(geoms: &[MultiPolygon<f64>], epsilon: f64) -> Vec<MultiPolygon<f64>> {
    if epsilon <= 0.0 {
        return geoms.to_vec();
    }

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
    let plans: Vec<Vec<Vec<Option<RingArcs>>>> = rings
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

    let simplified: Vec<Vec<Coord<f64>>> = arcs
        .into_iter()
        .map(|a| LineString(a).simplify_vw(&epsilon).0)
        .collect();

    geoms
        .iter()
        .zip(&plans)
        .map(|(original, g)| {
            let polys: Vec<Polygon<f64>> = g
                .iter()
                .filter_map(|p| {
                    let mut rs = p
                        .iter()
                        .map(|r| r.as_ref().and_then(|r| stitch(r, &simplified)));
                    let exterior = rs.next().flatten()?;
                    Some(Polygon::new(exterior, rs.flatten().collect()))
                })
                .collect();
            if polys.is_empty() {
                largest(original)
            } else {
                MultiPolygon(polys)
            }
        })
        .collect()
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
