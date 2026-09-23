//! Antimeridian (±180°) handling.
//!
//! Azimuthal projections (LAEA) have no seam at ±180°: each point is projected
//! relative to the centre with `wrap_longitude(lon - lon0)`, so Fiji or
//! Chukotka stay continuous *provided the centre is on the right side of the
//! globe*. [`center_longitude`] takes care of that.
//!
//! Pseudo-cylindrical projections (Equal Earth) do have a seam, at
//! `lon0 ± 180°`. [`split_at_seam`] cuts polygons along it before projecting.

use geo::{BooleanOps, MapCoords};
use geo_types::{coord, Coord, LineString, MultiLineString, MultiPolygon, Polygon, Rect};

/// Wraps a longitude difference or absolute longitude into `[-180, 180)`.
pub fn wrap_longitude(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Smallest arc of the circle containing every longitude, as `(start, length)`
/// going east from `start`. `None` for an empty input.
///
/// Finds the widest empty gap between sorted longitudes (including the
/// wrap-around gap); the covering arc is its complement.
pub fn covering_arc(lons: &[f64]) -> Option<(f64, f64)> {
    let mut v: Vec<f64> = lons.iter().map(|&l| wrap_longitude(l)).collect();
    v.sort_by(f64::total_cmp);
    v.dedup();
    let n = v.len();
    if n == 0 {
        return None;
    }
    let mut best_gap = v[0] + 360.0 - v[n - 1];
    let mut start = 0;
    for i in 1..n {
        let gap = v[i] - v[i - 1];
        if gap > best_gap {
            best_gap = gap;
            start = i;
        }
    }
    Some((v[start], 360.0 - best_gap))
}

/// Centre of the smallest arc containing every longitude, in `[-180, 180)`.
/// An empty slice returns `0.0`.
pub fn center_longitude(lons: &[f64]) -> f64 {
    covering_arc(lons).map_or(0.0, |(start, len)| wrap_longitude(start + len / 2.0))
}

/// Cuts polygons along the seam of a projection centred on `lon0`.
///
/// Output longitudes satisfy `lon - lon0 ∈ [-180, 180]`, with no ring
/// crossing the seam. Polygons that don't cross it are returned
/// coordinate-for-coordinate unchanged (apart from the ±360° shift), so
/// shared borders between neighbours stay bit-identical.
pub fn split_at_seam(mp: &MultiPolygon<f64>, lon0: f64) -> MultiPolygon<f64> {
    let mut out = Vec::new();
    for poly in mp {
        let mut ext = unwrap_relative(poly.exterior(), lon0);
        let (a, b) = x_range(&ext);
        let k = (((a + b) / 2.0 + 180.0) / 360.0).floor();
        shift_x(&mut ext, -360.0 * k);
        let (a, b) = (a - 360.0 * k, b - 360.0 * k);
        let mid = (a + b) / 2.0;
        let holes = poly
            .interiors()
            .iter()
            .map(|h| {
                let mut h = unwrap_relative(h, lon0);
                let (ha, hb) = x_range(&h);
                shift_x(&mut h, -360.0 * (((ha + hb) / 2.0 - mid) / 360.0).round());
                h
            })
            .collect();
        let rel = Polygon::new(ext, holes);

        if a >= -180.0 && b <= 180.0 {
            out.push(rel);
            continue;
        }
        let k_min = ((a + 180.0) / 360.0).floor() as i32;
        let k_max = ((b - 180.0) / 360.0).ceil() as i32;
        for k in k_min..=k_max {
            let off = 360.0 * f64::from(k);
            let strip = Rect::new(
                coord! { x: -180.0 + off, y: -90.0 },
                coord! { x: 180.0 + off, y: 90.0 },
            )
            .to_polygon();
            for part in rel.intersection(&strip) {
                out.push(part.map_coords(|c| coord! { x: c.x - off, y: c.y }));
            }
        }
    }
    MultiPolygon(out).map_coords(|c| coord! { x: c.x + lon0, y: c.y })
}

/// Longitudes relative to `lon0`, made continuous along the ring (no jump
/// larger than 180° between consecutive vertices).
///
/// Each value is `wrap(lon - lon0)` plus a whole multiple of 360°, never an
/// accumulated sum, so a vertex shared by two rings gets bit-identical output
/// in both (shared borders stay shared after seam splitting).
fn unwrap_relative(ring: &LineString<f64>, lon0: f64) -> LineString<f64> {
    let mut out: Vec<Coord<f64>> = Vec::with_capacity(ring.0.len());
    let mut prev: Option<f64> = None;
    for c in &ring.0 {
        let w = wrap_longitude(c.x - lon0);
        let rel = match prev {
            None => w,
            Some(p) => w + 360.0 * ((p - w) / 360.0).round(),
        };
        prev = Some(rel);
        out.push(coord! { x: rel, y: c.y });
    }
    LineString(out)
}

/// Rewrites longitude +180° as −180°. Datasets cut polygons at the
/// antimeridian with vertices on both; when the projection has no seam there
/// (LAEA, conic, or Equal Earth not centred on 0°) the two sides then share
/// exact vertices, so the cut is recognised as internal and never drawn.
pub fn canonicalize_antimeridian(mp: &MultiPolygon<f64>) -> MultiPolygon<f64> {
    mp.map_coords(|c| coord! { x: if c.x == 180.0 { -180.0 } else { c.x }, y: c.y })
}

/// [`split_at_seam`] for lines: output longitudes satisfy `lon - lon0 ∈ [-180, 180]`.
pub fn split_lines_at_seam(ml: &MultiLineString<f64>, lon0: f64) -> MultiLineString<f64> {
    let mut out = Vec::new();
    for line in ml {
        let mut rel = unwrap_relative(line, lon0);
        let (a, b) = x_range(&rel);
        let k = (((a + b) / 2.0 + 180.0) / 360.0).floor();
        shift_x(&mut rel, -360.0 * k);
        let (a, b) = (a - 360.0 * k, b - 360.0 * k);
        if a >= -180.0 && b <= 180.0 {
            out.push(rel);
            continue;
        }
        let k_min = ((a + 180.0) / 360.0).floor() as i32;
        let k_max = ((b - 180.0) / 360.0).ceil() as i32;
        let lines = MultiLineString(vec![rel]);
        for k in k_min..=k_max {
            let off = 360.0 * f64::from(k);
            let strip = Rect::new(
                coord! { x: -180.0 + off, y: -90.0 },
                coord! { x: 180.0 + off, y: 90.0 },
            )
            .to_polygon();
            for part in strip.clip(&lines, false) {
                out.push(part.map_coords(|c| coord! { x: c.x - off, y: c.y }));
            }
        }
    }
    MultiLineString(out).map_coords(|c| coord! { x: c.x + lon0, y: c.y })
}

fn x_range(ls: &LineString<f64>) -> (f64, f64) {
    ls.0.iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| {
            (a.min(c.x), b.max(c.x))
        })
}

fn shift_x(ls: &mut LineString<f64>, dx: f64) {
    if dx != 0.0 {
        for c in &mut ls.0 {
            c.x += dx;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::Area;

    fn angular_distance(a: f64, b: f64) -> f64 {
        wrap_longitude(a - b).abs()
    }

    #[test]
    fn wraps_into_range() {
        assert_eq!(wrap_longitude(190.0), -170.0);
        assert_eq!(wrap_longitude(-190.0), 170.0);
        assert_eq!(wrap_longitude(180.0), -180.0);
        assert_eq!(wrap_longitude(45.0), 45.0);
    }

    #[test]
    fn france_centre() {
        let c = center_longitude(&[-5.0, 8.0, 2.0]);
        assert!(angular_distance(c, 1.5) < 1e-9);
    }

    #[test]
    fn fiji_centre_is_near_180() {
        let c = center_longitude(&[177.0, 178.5, -179.8, -178.2]);
        assert!(angular_distance(c, 179.4) < 1e-9, "got {c}");
    }

    #[test]
    fn single_and_repeated_points() {
        assert_eq!(center_longitude(&[42.0]), 42.0);
        assert_eq!(center_longitude(&[-10.0, -10.0, -10.0]), -10.0);
        assert_eq!(center_longitude(&[]), 0.0);
    }

    #[test]
    fn span_of_russia_like_extent() {
        let (_, len) = covering_arc(&[20.0, 100.0, 179.9, -170.0]).unwrap();
        assert!((len - 170.0).abs() < 1e-9);
    }

    #[test]
    fn seam_split_preserves_area_and_range() {
        // A square from 170°E to 170°W, split by a seam at 180° (lon0 = 0).
        let sq = Polygon::new(
            LineString::from(vec![
                (170.0, 0.0),
                (-170.0, 0.0),
                (-170.0, 10.0),
                (170.0, 10.0),
            ]),
            vec![],
        );
        let out = split_at_seam(&MultiPolygon(vec![sq]), 0.0);
        assert_eq!(out.0.len(), 2);
        assert!((out.unsigned_area() - 200.0).abs() < 1e-6);
        for c in out.0.iter().flat_map(|p| p.exterior().0.iter()) {
            assert!(c.x >= -180.0 - 1e-9 && c.x <= 180.0 + 1e-9);
        }
    }

    #[test]
    fn shared_vertices_unwrap_identically_whatever_the_path() {
        // The same vertex reached from different predecessors must get the
        // same relative longitude, bit for bit.
        let v = 179.123456789;
        let a = unwrap_relative(&LineString::from(vec![(-179.9, 0.0), (v, 1.0)]), 37.3);
        let b = unwrap_relative(&LineString::from(vec![(178.0, 5.0), (v, 1.0)]), 37.3);
        assert_eq!(a.0[1].x.to_bits(), b.0[1].x.to_bits());
    }

    #[test]
    fn canonicalizes_plus_180() {
        let p = MultiPolygon(vec![Polygon::new(
            LineString::from(vec![(179.0, 0.0), (180.0, 0.0), (180.0, 1.0), (179.0, 0.0)]),
            vec![],
        )]);
        let xs: Vec<f64> = canonicalize_antimeridian(&p).0[0]
            .exterior()
            .0
            .iter()
            .map(|c| c.x)
            .collect();
        assert_eq!(xs, [179.0, -180.0, -180.0, 179.0]);
    }

    #[test]
    fn lines_split_at_seam() {
        let l = MultiLineString(vec![LineString::from(vec![(170.0, 0.0), (-170.0, 0.0)])]);
        let out = split_lines_at_seam(&l, 0.0);
        assert_eq!(out.0.len(), 2);
        for c in out.0.iter().flat_map(|l| l.0.iter()) {
            assert!(c.x.abs() <= 180.0 + 1e-9);
        }
    }

    #[test]
    fn non_crossing_polygon_is_untouched() {
        let p = Polygon::new(
            LineString::from(vec![(1.5, 2.25), (3.0, 2.0), (2.0, 4.0), (1.5, 2.25)]),
            vec![],
        );
        let out = split_at_seam(&MultiPolygon(vec![p.clone()]), 0.0);
        assert_eq!(out.0, vec![p]);
    }
}
